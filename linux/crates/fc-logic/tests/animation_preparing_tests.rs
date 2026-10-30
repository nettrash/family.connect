//! The port's own tests of `animation_preparing` — the pure half of Windows' `AnimationPreparing`
//! (an App service, untested in the C#): the frames an encoder is fed, the order profiles are asked
//! in, the poster, rule C and the cancel. The platform is played by a fake decoder (pixels made from
//! a still's own header) and a fake encoder that pulls every frame and writes an MP4 the way a muxer
//! would — the media data, then the index — timed in GStreamer's nanoseconds.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use fc_core::rt::{Cancel, LocalBoxFuture};
use fc_core::testing::FakeClock;
use fc_core::text::sticker_file;
use fc_logic::animated_pictures::animation_prep;
use fc_logic::animation_frames::FrameError;
use fc_logic::animation_preparing::{
    self, AnimationEncoder, DecodedStill, FrameFeed, H264Profile, StillDecoder, VideoFrame,
};
use fc_logic::animation_timing::{self, AnimationPlan};
use fc_logic::faststart;
use fc_logic::transcode_attempts::{AttemptFailure, Attempted, OperationCanceled};
use fc_logic::{animation_mp4, media_outbox::StagedMedia};
use futures::executor::LocalPool;
use time::macros::datetime;

// ---- pictures -----------------------------------------------------------------------------------

/// A GIF of `images` solid rectangles over a `width × height` screen, each `(left, top, width,
/// height, colour index, delay in hundredths)`, its data as literals and clears.
fn gif(width: u16, height: u16, images: &[(u16, u16, u16, u16, u8, u16)]) -> Vec<u8> {
    let mut out = b"GIF89a".to_vec();
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&[0x81, 0, 0]);
    // Four colours: black, red, green, blue.
    out.extend_from_slice(&[0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]);
    for &(left, top, w, h, colour, delay) in images {
        out.extend_from_slice(&[0x21, 0xF9, 0x04, 0x04]);
        out.extend_from_slice(&delay.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0x2C]);
        for value in [left, top, w, h] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&[0, 2]);
        // Code size 3 bits: clear (4), then the colour, a clear every two, then the end (5).
        let mut codes = vec![4u32];
        for at in 0..usize::from(w) * usize::from(h) {
            if at > 0 && at.is_multiple_of(2) {
                codes.push(4);
            }
            codes.push(u32::from(colour));
        }
        codes.push(5);
        let (mut bits, mut held, mut packed) = (0u32, 0u32, Vec::new());
        for code in codes {
            bits |= code << held;
            held += 3;
            while held >= 8 {
                packed.push(bits as u8);
                bits >>= 8;
                held -= 8;
            }
        }
        if held > 0 {
            packed.push(bits as u8);
        }
        for block in packed.chunks(255) {
            out.push(block.len() as u8);
            out.extend_from_slice(block);
        }
        out.push(0);
    }
    out.push(0x3B);
    out
}

fn chunk(kind: &str, payload: &[u8]) -> Vec<u8> {
    let mut chunk = (payload.len() as u32).to_be_bytes().to_vec();
    chunk.extend_from_slice(kind.as_bytes());
    chunk.extend_from_slice(payload);
    let check = fc_logic::animated_pictures::crc32::of(&chunk[4..]);
    chunk.extend_from_slice(&check.to_be_bytes());
    chunk
}

/// A 4 × 2 APNG of two whole-canvas frames of 100 ms whose data is a placeholder: the fake decoder
/// reads a still's size from its header and never inflates.
fn apng() -> Vec<u8> {
    let mut ihdr = [0u8; 13];
    ihdr[..4].copy_from_slice(&4u32.to_be_bytes());
    ihdr[4..8].copy_from_slice(&2u32.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 6;
    let fctl = |sequence: u32| {
        let mut fctl = Vec::new();
        for value in [sequence, 4, 2, 0, 0] {
            fctl.extend_from_slice(&value.to_be_bytes());
        }
        fctl.extend_from_slice(&[0, 1, 0, 10, 0, 0]);
        chunk("fcTL", &fctl)
    };
    [
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A][..],
        &chunk("IHDR", &ihdr),
        &chunk("acTL", &[0, 0, 0, 2, 0, 0, 0, 0]),
        &fctl(0),
        &chunk("IDAT", &[1, 2, 3]),
        &fctl(1),
        &chunk("fdAT", &[0, 0, 0, 2, 4, 5, 6]),
        &chunk("IEND", &[]),
    ]
    .concat()
}

// ---- the platform -------------------------------------------------------------------------------

/// Decodes a still to opaque grey at the size its own header states — or refuses the `refused`th
/// still it is asked for, or answers at the wrong size.
#[derive(Default)]
struct Decoder {
    refused: Option<usize>,
    wrong_size: bool,
    asked: RefCell<usize>,
}

impl StillDecoder for Decoder {
    fn decode_still(&self, still: Vec<u8>) -> LocalBoxFuture<'static, Option<DecodedStill>> {
        let index = {
            let mut asked = self.asked.borrow_mut();
            *asked += 1;
            *asked - 1
        };
        let answer = if self.refused == Some(index) {
            None
        } else {
            sticker_file::size(&still).map(|(width, height)| {
                let (width, height) = (width as i32 + i32::from(self.wrong_size), height as i32);
                DecodedStill {
                    width,
                    height,
                    bgra: [128, 128, 128, 255].repeat((width * height) as usize),
                }
            })
        };
        Box::pin(async move { answer })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encodes {
    /// Every frame, into an MP4.
    Well,
    /// This profile is refused.
    Refusing(H264Profile),
    /// Every profile is refused.
    Nothing,
    /// The bytes that come out are no MP4.
    Garbage,
    /// It stops after this many frames and writes what it has.
    StopsAfter(usize),
    /// It fails outright, as a pipeline error.
    Failing,
}

/// Pulls the frames, as the shell's GStreamer encoder will, and writes the MP4 a muxer would.
struct Encoder {
    encodes: Encodes,
    asked: RefCell<Vec<H264Profile>>,
    fed: RefCell<Vec<VideoFrame>>,
    posters: RefCell<Vec<(i32, i32, f64)>>,
}

impl Encoder {
    fn new(encodes: Encodes) -> Encoder {
        Encoder {
            encodes,
            asked: RefCell::default(),
            fed: RefCell::default(),
            posters: RefCell::default(),
        }
    }
}

impl AnimationEncoder for Encoder {
    fn encode<'a>(
        &'a self,
        plan: &'a AnimationPlan,
        profile: H264Profile,
        feed: &'a mut FrameFeed,
        _cancel: Cancel,
    ) -> LocalBoxFuture<'a, Result<Attempted<Vec<u8>>, AttemptFailure>> {
        Box::pin(async move {
            self.asked.borrow_mut().push(profile);
            match self.encodes {
                Encodes::Refusing(refused) if refused == profile => return Ok(Attempted::Refused),
                Encodes::Nothing => return Ok(Attempted::Refused),
                Encodes::Failing => return Err(AttemptFailure::new("gst-stream-error-quark", 6)),
                _ => {}
            }
            let mut durations = Vec::new();
            while let Some(frame) = feed.next().await {
                assert_eq!(
                    frame.nv12.len(),
                    (plan.width * plan.height * 3 / 2) as usize
                );
                durations.push(frame.duration_ns as u32);
                self.fed.borrow_mut().push(frame);
                if let Encodes::StopsAfter(count) = self.encodes
                    && durations.len() == count
                {
                    break;
                }
            }
            if self.encodes == Encodes::Garbage {
                return Ok(Attempted::Taken(vec![
                    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
                ]));
            }
            Ok(Attempted::Taken(muxed(1_000_000_000, &durations)))
        })
    }

    fn poster_jpeg(&self, bgra: &[u8], width: i32, height: i32, quality: f64) -> Option<Vec<u8>> {
        assert_eq!(bgra.len(), (width * height * 4) as usize);
        assert!(
            bgra.chunks(4).all(|pixel| pixel[3] == 255),
            "a poster is opaque"
        );
        self.posters.borrow_mut().push((width, height, quality));
        Some(vec![0xFF, 0xD8, 0xFF, width as u8, height as u8])
    }
}

fn bx(kind: &str, children: &[&[u8]]) -> Vec<u8> {
    let body = children.concat();
    let mut out = ((8 + body.len()) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind.as_bytes());
    out.extend_from_slice(&body);
    out
}

fn u32s(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_be_bytes())
        .collect()
}

/// What a muxer writes when it finishes: `ftyp`, the media data, then `moov` — one video track at
/// `scale` units a second, one sample per entry.
fn muxed(scale: u32, durations: &[u32]) -> Vec<u8> {
    let ftyp = bx("ftyp", &[b"mp42\0\0\0\0mp42isom"]);
    let mdat = bx("mdat", &[&[0u8; 64]]);
    let total = durations.iter().fold(0u32, |sum, d| sum.wrapping_add(*d));
    let count = durations.len() as u32;
    let stts: Vec<u32> = [0, count]
        .into_iter()
        .chain(durations.iter().flat_map(|value| [1, *value]))
        .collect();
    let stbl = bx(
        "stbl",
        &[
            &bx("stsd", &[&u32s(&[0, 0])]),
            &bx("stts", &[&u32s(&stts)]),
            &bx("stsz", &[&u32s(&[0, 0, count])]),
            &bx("stco", &[&u32s(&[0, 1, (ftyp.len() + 8) as u32])]),
        ],
    );
    let trak = bx(
        "trak",
        &[
            &bx("tkhd", &[&u32s(&[0, 0, 0, 1, 0, 999]), &[0; 60]]),
            &bx(
                "mdia",
                &[
                    &bx("mdhd", &[&u32s(&[0, 0, 0, scale, total]), &[0; 4]]),
                    &bx("hdlr", &[&u32s(&[0, 0]), b"vide", &[0; 13]]),
                    &bx("minf", &[&stbl]),
                ],
            ),
        ],
    );
    let moov = bx(
        "moov",
        &[
            &bx("mvhd", &[&u32s(&[0, 0, 0, 1000, 999]), &[0; 80]]),
            &trak,
        ],
    );
    [ftyp, mdat, moov].concat()
}

// ---- running it ---------------------------------------------------------------------------------

struct Made {
    answer: Result<Option<StagedMedia>, OperationCanceled>,
    log: Vec<String>,
}

fn prepare(bytes: &[u8], decoder: Decoder, encoder: &Encoder, cancel: &Cancel) -> Made {
    let clock = FakeClock::at(datetime!(2026-10-29 10:00 UTC));
    let mut log = Vec::new();
    let answer = LocalPool::new().run_until(animation_preparing::prepare(
        Rc::from(bytes),
        Rc::new(decoder),
        encoder,
        &clock,
        &mut |line: &str| log.push(line.to_string()),
        cancel,
    ));
    Made { answer, log }
}

/// A 4 × 2 GIF: red for 110 ms, a green left half for 20 ms (a flash no tick lands on), then blue
/// for 200 ms.
fn three_frames() -> Vec<u8> {
    gif(
        4,
        2,
        &[(0, 0, 4, 2, 1, 11), (0, 0, 2, 2, 2, 2), (0, 0, 4, 2, 3, 20)],
    )
}

#[test]
fn a_gif_becomes_a_silent_looping_video_with_its_poster_and_flag() {
    let bytes = three_frames();
    let encoder = Encoder::new(Encodes::Well);
    let made = prepare(&bytes, Decoder::default(), &encoder, &Cancel::new());
    let staged = made.answer.unwrap().expect("an animation");

    let plan = animation_timing::plan(4, 2, &[110, 20, 200]).unwrap();
    assert_eq!(
        (staged.kind.as_str(), staged.mime.as_str()),
        ("video", "video/mp4")
    );
    assert_eq!((staged.width, staged.height), (Some(4), Some(2)));
    assert_eq!(staged.duration_ms, Some(plan.duration_ms as i32));
    assert!(staged.animation);
    // The poster: the first frame on white within 600, as a JPEG at the preview's quality.
    assert_eq!(staged.preview, Some(vec![0xFF, 0xD8, 0xFF, 4, 2]));
    assert_eq!(*encoder.posters.borrow(), vec![(4, 2, 0.7)]);
    // High was taken, and only High asked.
    assert_eq!(*encoder.asked.borrow(), vec![H264Profile::High]);
    // The index in front, and every sample's ticks exact in a multiple of 30.
    assert_eq!(faststart::is_moov_first(&staged.bytes), Some(true));
    let (scale, durations) = animation_mp4::timing(&staged.bytes).unwrap();
    assert_eq!(scale % 30, 0);
    let ticks: Vec<u32> = plan
        .ticks()
        .iter()
        .map(|&t| t as u32 * scale / 30)
        .collect();
    assert_eq!(durations, ticks);
    assert!(made.log[0].starts_with("making a Gif animation of 4x2, 3 frame(s) into 4x2"));
}

/// What the encoder is fed: one NV12 frame per sample, each placed on the grid in nanoseconds and
/// lasting until the next — the flash never sampled, but drawn under what follows.
#[test]
fn the_encoder_is_fed_each_sample_on_the_grid() {
    let bytes = three_frames();
    let encoder = Encoder::new(Encodes::Well);
    let _ = prepare(&bytes, Decoder::default(), &encoder, &Cancel::new());
    let fed = encoder.fed.borrow();
    // 110 ms is ticks 0–3; the flash (110–130) holds no tick; 130–330 ms is ticks 4–9.
    assert_eq!(
        fed.iter()
            .map(|frame| (frame.index, frame.start_tick, frame.ticks))
            .collect::<Vec<_>>(),
        vec![(0, 0, 4), (1, 4, 6)]
    );
    assert_eq!(
        fed.iter()
            .map(|frame| (frame.pts_ns, frame.duration_ns))
            .collect::<Vec<_>>(),
        vec![(0, 133_333_333), (133_333_333, 200_000_000)]
    );
    // BT.709 luma of pure red, then of pure blue: the flash left nothing showing.
    assert!(fed[0].nv12[..8].iter().all(|&y| y == 63));
    assert!(fed[1].nv12[..8].iter().all(|&y| y == 32));
}

#[test]
fn main_is_asked_for_when_high_is_refused() {
    let bytes = three_frames();
    let encoder = Encoder::new(Encodes::Refusing(H264Profile::High));
    let made = prepare(&bytes, Decoder::default(), &encoder, &Cancel::new());
    assert!(made.answer.unwrap().is_some());
    assert_eq!(
        *encoder.asked.borrow(),
        vec![H264Profile::High, H264Profile::Main]
    );
}

/// RULE C: a picture that is not an animation, an encoder that takes nothing, bytes that are no MP4,
/// an encode that took fewer frames than planned — each answers `None`, and the picture goes as it
/// went before.
#[test]
fn whatever_does_not_come_out_goes_as_it_went_before() {
    let still = gif(4, 2, &[(0, 0, 4, 2, 1, 10)]);
    let encoder = Encoder::new(Encodes::Well);
    let made = prepare(&still, Decoder::default(), &encoder, &Cancel::new());
    assert_eq!(made.answer, Ok(None));
    assert!(encoder.asked.borrow().is_empty());
    assert_eq!(
        made.log,
        vec!["an animated picture's frames could not be walked; it goes as a file"]
    );

    for (encodes, said) in [
        (Encodes::Nothing, "no transcode profile was taken"),
        (Encodes::Garbage, "an animation's bytes are not an MP4's"),
        (
            Encodes::StopsAfter(1),
            "an animation's encode took 1 of 2 frames",
        ),
        (
            Encodes::Failing,
            "a transcode failed: gst-stream-error-quark 0x00000006 (Main)",
        ),
    ] {
        let encoder = Encoder::new(encodes);
        let made = prepare(
            &three_frames(),
            Decoder::default(),
            &encoder,
            &Cancel::new(),
        );
        assert_eq!(made.answer, Ok(None), "{encodes:?}");
        assert!(
            made.log.iter().any(|line| line == said),
            "{encodes:?}: {:?}",
            made.log
        );
    }
}

/// An APNG's frames are decoded by the platform, one standalone still each; a frame that will not
/// decode, or comes out another size, ends it before any encoder is asked — the poster is frame
/// zero.
#[test]
fn an_apngs_stills_are_the_platforms_and_a_bad_one_ends_it() {
    let encoder = Encoder::new(Encodes::Well);
    let made = prepare(&apng(), Decoder::default(), &encoder, &Cancel::new());
    let staged = made.answer.unwrap().expect("an animation");
    assert_eq!((staged.width, staged.height), (Some(4), Some(2)));
    assert_eq!(encoder.fed.borrow().len(), 2);

    for decoder in [
        Decoder {
            refused: Some(0),
            ..Decoder::default()
        },
        Decoder {
            wrong_size: true,
            ..Decoder::default()
        },
    ] {
        let encoder = Encoder::new(Encodes::Well);
        let made = prepare(&apng(), decoder, &encoder, &Cancel::new());
        assert_eq!(made.answer, Ok(None));
        assert!(encoder.asked.borrow().is_empty());
        assert!(
            made.log
                .iter()
                .any(|line| line.contains("frame 0 of an animation could not be decoded at 4x2")),
            "{:?}",
            made.log
        );
    }
}

/// A frame that fails mid-encode makes the result wrong, whatever the encoder wrote.
#[test]
fn a_frame_that_fails_mid_encode_makes_it_wrong() {
    let encoder = Encoder::new(Encodes::Well);
    let made = prepare(
        &apng(),
        Decoder {
            refused: Some(2),
            ..Decoder::default()
        },
        &encoder,
        &Cancel::new(),
    );
    assert_eq!(made.answer, Ok(None));
    assert_eq!(*encoder.asked.borrow(), vec![H264Profile::High]);
    assert!(
        made.log
            .iter()
            .any(|line| line.starts_with("an animation frame failed mid-encode: frame 1")),
        "{:?}",
        made.log
    );
}

/// The person's cancel is not a failure: it comes out as a cancel, and stages nothing.
#[test]
fn a_cancel_is_a_cancel() {
    let cancel = Cancel::new();
    cancel.cancel();
    let encoder = Encoder::new(Encodes::Well);
    let made = prepare(&three_frames(), Decoder::default(), &encoder, &cancel);
    assert_eq!(made.answer, Err(OperationCanceled));
    assert!(encoder.asked.borrow().is_empty());
}

/// A feed ends for good: after the last sample, a failure or a cancel, `None` and only `None`.
#[test]
fn a_feed_that_ended_stays_ended() {
    let bytes: Rc<[u8]> = Rc::from(apng());
    let layout = animation_prep::layout(&bytes).unwrap();
    let plan = animation_timing::plan(4, 2, &layout.stated_ms()).unwrap();
    let source = animation_preparing::frame_source(
        bytes,
        &layout,
        Rc::new(Decoder {
            refused: Some(1),
            ..Decoder::default()
        }),
    )
    .unwrap();
    let mut feed = FrameFeed::new(&source, plan, Cancel::new()).unwrap();
    let mut pool = LocalPool::new();
    assert!(pool.run_until(feed.next()).is_some());
    assert!(pool.run_until(feed.next()).is_none());
    assert!(matches!(
        feed.failed(),
        Some(FrameError::Undecodable { frame: 1, .. })
    ));
    assert!(pool.run_until(feed.next()).is_none());
    assert_eq!(feed.given(), 1);
}

/// One ceiling for every attempt: two minutes and three times the clip, or ten minutes for a clip
/// of no stated length (`MediaEncoding.TranscodeCeiling`).
#[test]
fn the_ceiling_follows_the_clips_length() {
    assert_eq!(
        animation_preparing::transcode_ceiling(Some(2100)),
        Duration::from_millis(126_300)
    );
    assert_eq!(
        animation_preparing::transcode_ceiling(None),
        Duration::from_secs(600)
    );
    assert_eq!(
        animation_preparing::transcode_ceiling(Some(0)),
        Duration::from_secs(600)
    );
}
