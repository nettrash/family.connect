//! An animated GIF, WebP or PNG made into the protocol's silent looping video — everything of it
//! but decoding a still and running the encoder (issue #96; docs/protocol.md, "Animations", "The
//! file"). The pure half of `win/src/FamilyConnect.App/Services/AnimationPreparing.cs`, moved here
//! so the shell implements only the two platform seams below, and the rest is tested.
//!
//! WHERE THE FRAMES COME FROM ([`frame_source`]). A GIF is decoded by this client
//! ([`GifPicture`]). An APNG and an animated WebP are cut into standalone stills
//! ([`ApngPicture`], [`WebPAnimation`]) and each still is decoded by the shell's
//! [`StillDecoder`] — gdk-pixbuf (glycin) reads a still PNG or WebP whatever it makes of the
//! animation around it. (Windows fell back to its WebP extension's own frames when the still path
//! refused the first frame; glycin's WebP loader is libwebp's, and that fallback is not ported.)
//!
//! HOW IT IS ENCODED ([`prepare`]). The plan (`animation_timing`) says the size, the samples and the
//! bitrate; the poster is the first frame on white fitted within the preview's 600, as a JPEG; then
//! the shell's [`AnimationEncoder`] is asked for H.264 High, then Main
//! (`transcode_attempts::first_taken`, one ceiling for both), and pulls the frames from a
//! [`FrameFeed`] — NV12 in BT.709 limited range, each with its presentation time and duration on
//! the 30-a-second grid. What comes back is checked (an MP4 by its bytes, every frame taken, none
//! failed), its sample table made exact (`animation_mp4::exact_ticks`), its index moved to the front
//! (`faststart::moov_first`) and held to the ceiling.
//!
//! RULE C. Anything that does not come out — a picture that will not decode, an encoder that
//! refuses or fails, a result that is not what was asked, one over the ceiling — answers `Ok(None)`,
//! and the caller sends it exactly as it went before #96: a GIF or a WebP as its original bytes, as
//! a file; an animated PNG down the photo route. Only the person's cancel is not a failure: it
//! comes out as `Err(OperationCanceled)`.
//!
//! LOGS say sizes, counts and formats — never a name or a path.

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use fc_core::rt::{Cancel, Clock, LocalBoxFuture};
use fc_text::media;

use crate::animated_pictures::{
    AnimationFormat, AnimationLayout, ApngPicture, GifPicture, WebPAnimation, animation_prep,
};
use crate::animation_frames::{AnimationFrames, FrameError, PixelsOf, animation_render};
use crate::animation_mp4;
use crate::animation_timing::{self, AnimationPlan};
use crate::faststart;
use crate::media_outbox::StagedMedia;
use crate::transcode_attempts::{self, AttemptFailure, Attempted, OperationCanceled};

/// One still picture as the platform decoded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedStill {
    pub width: i32,
    pub height: i32,
    /// Straight-alpha BGRA, `width × height × 4` bytes, rows top to bottom with no padding.
    pub bgra: Vec<u8>,
}

/// The platform's still-picture decoder — the shell implements it with gdk-pixbuf (glycin).
pub trait StillDecoder {
    /// Decode `still` — ONE still picture, a PNG or a WebP as [`ApngPicture::standalone`] and
    /// [`WebPAnimation::standalone`] cut them — into straight BGRA.
    ///
    /// The contract:
    /// - The FIRST frame only; the picture's own pixel size, never scaled.
    /// - Straight (NOT premultiplied) alpha, 8 bits a channel, in the byte order B, G, R, A, rows
    ///   top to bottom, `width × 4` bytes a row with no padding — `gdk::MemoryTextureBuilder`'s
    ///   `B8g8r8a8`, or a `Pixbuf` (R, G, B, A with a rowstride) reordered and unpadded. A picture
    ///   with no alpha comes out with every alpha 255.
    /// - The file's own colours: no colour management, no gamma correction, and EXIF orientation
    ///   ignored, as a browser draws an animation's frames.
    /// - `None` when it will not decode. Never panics, never blocks for long: it runs on the main
    ///   context between frames.
    /// - The future owns what it needs (`'static`): it is awaited from a [`PixelsOf`] closure.
    fn decode_still(&self, still: Vec<u8>) -> LocalBoxFuture<'static, Option<DecodedStill>>;
}

/// The H.264 profile an attempt asks for: High first, then Main.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum H264Profile {
    High,
    Main,
}

impl fmt::Display for H264Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            H264Profile::High => "High",
            H264Profile::Main => "Main",
        })
    }
}

/// The platform's video encoder and JPEG writer — the shell implements them with GStreamer and
/// gdk-pixbuf.
pub trait AnimationEncoder {
    /// Encode the frames `feed` gives as the profile's silent MP4 at `plan`'s numbers.
    ///
    /// The contract:
    /// - Pull `feed.next().await` until it answers `None`, and push each [`VideoFrame`] as one
    ///   buffer: NV12, `plan.width × plan.height`, `pts = pts_ns`, `duration = duration_ns`. Caps
    ///   `video/x-raw,format=NV12,width=…,height=…,framerate=30/1,pixel-aspect-ratio=1/1,
    ///   colorimetry=bt709` (limited range: the frames already ARE BT.709 16–235 — no
    ///   `videoconvert` may change them). `None` is the end of the stream (send EOS) — whether the
    ///   frames ran out, one failed or `cancel` was raised; the caller judges which.
    /// - `profile` [`H264Profile::High`]: `x264enc bitrate=<plan.video_bitrate / 1000>
    ///   key-int-max=60 ! video/x-h264,profile=high`; [`H264Profile::Main`]: the next encoder —
    ///   `openh264enc` with `profile=main` (carry-forward: x264enc High, then openh264enc Main). Then
    ///   `h264parse ! mp4mux` and NO audio track at all.
    /// - `mp4mux` WITHOUT `faststart`: `moov` must come LAST, after the media data, so the sample
    ///   table can be rewritten without moving a byte of it (`animation_mp4`); this module moves the
    ///   index to the front afterwards. (shell-design §8.4 names `faststart=true` here: with it the
    ///   timing cannot be made exact and the encoder's own goes.)
    /// - Read the result back (the discoverer): H.264, `plan.width × plan.height`, no sound —
    ///   anything else is `Ok(Attempted::Wrong)`.
    /// - `Ok(Attempted::Refused)` when this profile or encoder is not available or refuses the caps
    ///   (the next profile is tried); `Err(AttemptFailure)` for an error out of the pipeline (its
    ///   GLib error domain and code — never its message); `Ok(Attempted::Taken(bytes))` with the
    ///   whole file.
    /// - Watch `cancel` (the person's cancel or the ceiling): when raised, set the pipeline to
    ///   `NULL`, delete any temporary file and answer at once — anything. The future may also be
    ///   dropped, which must do the same.
    /// - Never block the main context: GStreamer runs its own threads; await the bus.
    fn encode<'a>(
        &'a self,
        plan: &'a AnimationPlan,
        profile: H264Profile,
        feed: &'a mut FrameFeed,
        cancel: Cancel,
    ) -> LocalBoxFuture<'a, Result<Attempted<Vec<u8>>, AttemptFailure>>;

    /// Opaque BGRA (`width × height × 4`, every alpha 255) as a baseline JPEG at `quality`
    /// (0.0–1.0 — gdk-pixbuf's `quality` is `round(quality × 100)`), no EXIF, sRGB. `None` when it
    /// cannot be written; the video then goes without a poster.
    fn poster_jpeg(&self, bgra: &[u8], width: i32, height: i32, quality: f64) -> Option<Vec<u8>>;
}

/// Where each frame's pixels come from, and the layout they are composited by.
#[derive(Clone)]
pub struct FrameSource {
    pub layout: AnimationLayout,
    pub pixels: PixelsOf,
}

impl fmt::Debug for FrameSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameSource")
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

/// The frames of an animated picture, read from its own bytes: a GIF decoded here, an APNG's or a
/// WebP's frames cut into stills for `decoder`, each held to its frame's size. Also how an animated
/// STICKER's frames are had (`animation_frames::sticker_frames::compose`). `None` when the bytes
/// are not the layout's format.
pub fn frame_source(
    bytes: Rc<[u8]>,
    layout: &AnimationLayout,
    decoder: Rc<dyn StillDecoder>,
) -> Option<FrameSource> {
    let pixels: PixelsOf = match layout.format {
        AnimationFormat::Gif => {
            let gif = Rc::new(GifPicture::read(bytes)?);
            Rc::new(move |index| {
                let pixels = gif.pixels(index);
                Box::pin(async move { pixels })
            })
        }
        AnimationFormat::Apng => {
            let apng = Rc::new(ApngPicture::read(bytes)?);
            let frames = layout.frames.clone();
            Rc::new(move |index| {
                let decoding = apng
                    .standalone(index)
                    .map(|still| decoder.decode_still(still));
                let frame = frames.get(index).map(|frame| (frame.width, frame.height));
                Box::pin(still(decoding, frame))
            })
        }
        AnimationFormat::WebP => {
            let webp = Rc::new(WebPAnimation::read(bytes)?);
            let frames = layout.frames.clone();
            Rc::new(move |index| {
                let decoding = webp
                    .standalone(index)
                    .map(|still| decoder.decode_still(still));
                let frame = frames.get(index).map(|frame| (frame.width, frame.height));
                Box::pin(still(decoding, frame))
            })
        }
    };
    Some(FrameSource {
        layout: layout.clone(),
        pixels,
    })
}

/// A decoded standalone still, as straight BGRA at exactly its frame's size; `None` when it would
/// not decode or came out another size.
async fn still(
    decoding: Option<LocalBoxFuture<'static, Option<DecodedStill>>>,
    frame: Option<(i32, i32)>,
) -> Option<Vec<u8>> {
    let decoded = decoding?.await?;
    let (width, height) = frame?;
    let fits = decoded.width == width
        && decoded.height == height
        && crate::animated_pictures::bgra_len(width, height) == Some(decoded.bgra.len());
    fits.then_some(decoded.bgra)
}

/// One video frame on its way into the encoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    /// Which sample of the plan, from 0.
    pub index: usize,
    /// NV12 at the plan's size, BT.709 limited range ([`animation_render::nv12`]).
    pub nv12: Vec<u8>,
    /// The tick the sample starts on, and how many it lasts.
    pub start_tick: i64,
    pub ticks: i32,
    /// Its presentation time and duration in nanoseconds ([`animation_timing::nanoseconds`]): each
    /// sample's time is placed on the grid, and its length is what reaches the next one.
    pub pts_ns: i64,
    pub duration_ns: i64,
}

/// The video frames of one encode, in order, for the shell's [`AnimationEncoder`] to pull.
pub struct FrameFeed {
    frames: AnimationFrames,
    starts: Vec<i64>,
    cancel: Cancel,
    failed: Option<FrameError>,
}

impl FrameFeed {
    /// The frames of `source` as `plan` samples them; `None` when the canvas cannot be made.
    pub fn new(source: &FrameSource, plan: AnimationPlan, cancel: Cancel) -> Option<FrameFeed> {
        let starts = plan.start_ticks();
        Some(FrameFeed {
            frames: AnimationFrames::new(source.layout.clone(), plan, source.pixels.clone())?,
            starts,
            cancel,
            failed: None,
        })
    }

    /// The next frame, or `None` — the end of the stream — once every sample has been given, a
    /// frame failed, or the cancel was raised. After one `None`, always `None`.
    pub async fn next(&mut self) -> Option<VideoFrame> {
        if self.failed.is_some() {
            return None;
        }
        let index = self.frames.given();
        match self.frames.next(&self.cancel).await {
            Ok(Some(bgra)) => {
                let plan = self.frames.plan();
                let start_tick = self.starts[index];
                let ticks = plan.samples[index].ticks;
                let pts_ns = animation_timing::nanoseconds(start_tick);
                let end_ns = animation_timing::nanoseconds(start_tick + i64::from(ticks));
                Some(VideoFrame {
                    index,
                    nv12: animation_render::nv12(&bgra, plan.width, plan.height),
                    start_tick,
                    ticks,
                    pts_ns,
                    duration_ns: end_ns - pts_ns,
                })
            }
            Ok(None) => None,
            Err(error) => {
                self.failed = Some(error);
                None
            }
        }
    }

    /// How many frames have been given.
    pub fn given(&self) -> usize {
        self.frames.given()
    }

    /// Why the stream ended early, if it did.
    pub fn failed(&self) -> Option<FrameError> {
        self.failed
    }

    /// The plan the frames follow.
    pub fn plan(&self) -> &AnimationPlan {
        self.frames.plan()
    }
}

/// How long all the attempts together may take: two minutes and three times the clip, or ten
/// minutes for a clip of no stated length — `MediaEncoding.TranscodeCeiling`, which is
/// [`crate::media_encoding::transcode_ceiling`] (it saturates; the `3 × duration` written here
/// before overflowed for a huge stated length).
pub fn transcode_ceiling(duration_ms: Option<i64>) -> Duration {
    crate::media_encoding::transcode_ceiling(duration_ms)
}

/// The made video, ready to stage — `Ok(None)` for rule C, `Err` when the person cancelled.
/// `log` is told what happened, in sizes and counts.
pub async fn prepare(
    bytes: Rc<[u8]>,
    decoder: Rc<dyn StillDecoder>,
    encoder: &dyn AnimationEncoder,
    clock: &dyn Clock,
    log: &mut dyn FnMut(&str),
    cancel: &Cancel,
) -> Result<Option<StagedMedia>, OperationCanceled> {
    let Some(layout) = animation_prep::layout(&bytes) else {
        log("an animated picture's frames could not be walked; it goes as a file");
        return Ok(None);
    };
    let Some(plan) = animation_timing::plan(
        i64::from(layout.width),
        i64::from(layout.height),
        &layout.stated_ms(),
    ) else {
        log(&format!(
            "an animated picture of {}x{} has no video size; it goes as a file",
            layout.width, layout.height
        ));
        return Ok(None);
    };
    log(&format!(
        "making a {:?} animation of {}x{}, {} frame(s) into {}x{}, {} video frame(s), {} ms, {} bit/s",
        layout.format,
        layout.width,
        layout.height,
        layout.frames.len(),
        plan.width,
        plan.height,
        plan.samples.len(),
        plan.duration_ms,
        plan.video_bitrate
    ));
    let Some(source) = frame_source(bytes, &layout, decoder) else {
        return Ok(None);
    };
    // The poster first: it is frame zero, and a picture whose first frame will not decode goes no
    // further.
    let poster = match poster(&source, &plan, encoder, cancel).await {
        Ok(poster) => poster,
        Err(FrameError::Canceled) => return Err(OperationCanceled),
        Err(error) => {
            log(&format!("making an animation: {error}"));
            return Ok(None);
        }
    };
    let attempts = [H264Profile::High, H264Profile::Main];
    let ceiling = transcode_ceiling(Some(plan.duration_ms));
    // The attempts' lines and the loop's, in the order they were said.
    let said: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let made = transcode_attempts::first_taken(
        &attempts,
        |profile, linked| attempt(encoder, &source, &plan, *profile, linked, &said),
        ceiling,
        clock,
        |line| said.borrow_mut().push(line.to_string()),
        cancel,
    )
    .await;
    for line in said.take() {
        log(&line);
    }
    let Some(made) = made? else {
        return Ok(None);
    };
    Ok(Some(StagedMedia {
        width: Some(plan.width),
        height: Some(plan.height),
        duration_ms: Some(i32::try_from(plan.duration_ms).unwrap_or(i32::MAX)),
        preview: poster,
        animation: true,
        ..StagedMedia::new("video", "video/mp4", made)
    }))
}

/// The first frame, on white, fitted within the preview's 600, as a JPEG — `Ok(None)` when the JPEG
/// could not be written (the video goes without), `Err` when the frame could not be had.
async fn poster(
    source: &FrameSource,
    plan: &AnimationPlan,
    encoder: &dyn AnimationEncoder,
    cancel: &Cancel,
) -> Result<Option<Vec<u8>>, FrameError> {
    let undecodable = FrameError::Undecodable {
        frame: 0,
        width: source.layout.width,
        height: source.layout.height,
    };
    let mut frames =
        AnimationFrames::new(source.layout.clone(), plan.clone(), source.pixels.clone())
            .ok_or(undecodable)?;
    let first = frames.next(cancel).await?.ok_or(undecodable)?;
    let (width, height) =
        media::fit_within(plan.width as u32, plan.height as u32, media::PREVIEW_EDGE);
    let (width, height) = (width as i32, height as i32);
    let fitted = animation_render::scale(&first, plan.width, plan.height, width, height);
    Ok(encoder.poster_jpeg(&fitted, width, height, media::PREVIEW_QUALITY))
}

/// One way of asking, into the shell's encoder; what it says goes into `said`.
async fn attempt(
    encoder: &dyn AnimationEncoder,
    source: &FrameSource,
    plan: &AnimationPlan,
    profile: H264Profile,
    cancel: Cancel,
    said: &RefCell<Vec<String>>,
) -> Result<Attempted<Vec<u8>>, AttemptFailure> {
    let Some(mut feed) = FrameFeed::new(source, plan.clone(), cancel.clone()) else {
        return Ok(Attempted::Wrong);
    };
    let bytes = match encoder.encode(plan, profile, &mut feed, cancel).await? {
        Attempted::Taken(bytes) => bytes,
        other => return Ok(other),
    };
    let mut lines = said.borrow_mut();
    if let Some(failed) = feed.failed() {
        lines.push(format!("an animation frame failed mid-encode: {failed}"));
        return Ok(Attempted::Wrong);
    }
    if feed.given() != plan.samples.len() {
        lines.push(format!(
            "an animation's encode took {} of {} frames",
            feed.given(),
            plan.samples.len()
        ));
        return Ok(Attempted::Wrong);
    }
    Ok(match finished(&bytes, plan, &mut lines) {
        Some(made) => Attempted::Taken(made),
        None => Attempted::Wrong,
    })
}

/// The encode taken in: an MP4 by its bytes, within the ceiling, its timing made exact where it can
/// be (the encoder's own, within a nanosecond a frame, otherwise), and its index in front — or
/// `None`, and rule C.
pub fn finished(bytes: &[u8], plan: &AnimationPlan, log: &mut Vec<String>) -> Option<Vec<u8>> {
    if !media::matches_magic("video/mp4", bytes) {
        log.push("an animation's bytes are not an MP4's".to_string());
        return None;
    }
    let timed = animation_mp4::exact_ticks(bytes, &plan.ticks());
    if timed.is_none() {
        log.push(
            "an animation's sample table could not be made exact; it keeps the encoder's timing"
                .to_string(),
        );
    }
    let Some(arranged) = faststart::moov_first(timed.as_deref().unwrap_or(bytes)) else {
        log.push("an animation's index could not be moved to the front".to_string());
        return None;
    };
    if arranged.len() as u64 > media::SIZE_LIMIT {
        log.push(format!(
            "an animation came out over the ceiling ({} bytes)",
            arranged.len()
        ));
        return None;
    }
    Some(match arranged {
        Cow::Borrowed(same) => same.to_vec(),
        Cow::Owned(moved) => moved,
    })
}
