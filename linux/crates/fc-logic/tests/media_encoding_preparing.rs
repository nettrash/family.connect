//! `media_encoding::preparing` — the pure half of Windows' `MediaPreparing` video and sound-file
//! paths, over a fake of the shell's seam. Not a C# suite (`MediaPreparing.cs` is Windows-platform
//! code with no tests of its own); these pin what the C# does, step by step: the plan, Rule E's
//! count and mix, the read-back and its frame check, rule D, rule C, the ceiling, the person's
//! cancel, and every temporary file removed.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::future::ready;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fc_core::rt::{Cancel, LocalBoxFuture};
use fc_core::testing::{FakeClock, drive};
use fc_logic::container_sound::SoundTrack;
use fc_logic::frame_turn;
use fc_logic::media_encoding::preparing::{
    AudioStream, EncodeAsk, MediaPreparing, Prepared, SoundJob, StreamReading, VideoJob,
    VideoStream, judge_video, prepare_sound, prepare_video, sound_source, video_input,
};
use fc_logic::media_encoding::{AacEncoding, H264Profile, VideoEncoding};
use fc_logic::transcode_attempts::{AttemptEnd, AttemptFailure, Attempted, OperationCanceled};
use fc_text::media;
use time::macros::datetime;

// --- the fake -----------------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct FakeFile {
    reading: Option<StreamReading>,
    tracks: Option<Vec<SoundTrack>>,
    /// The same frame at every moment.
    frame: Option<Vec<u8>>,
    bytes: Vec<u8>,
}

enum Outcome {
    Made(Box<FakeFile>),
    Refused,
    Wrong,
    Fail,
    /// Never answers until its cancel is raised.
    Hang,
}

fn made(file: FakeFile) -> Outcome {
    Outcome::Made(Box::new(file))
}

#[derive(Default)]
struct Fake {
    files: RefCell<HashMap<PathBuf, FakeFile>>,
    outcomes: RefCell<VecDeque<Outcome>>,
    asked: RefCell<Vec<EncodeAsk>>,
    made: RefCell<Vec<PathBuf>>,
    removed: RefCell<Vec<PathBuf>>,
    tracks_read: Cell<usize>,
}

impl Fake {
    fn with(source: &Path, file: FakeFile, outcomes: Vec<Outcome>) -> Self {
        let fake = Fake::default();
        fake.files.borrow_mut().insert(source.to_path_buf(), file);
        *fake.outcomes.borrow_mut() = outcomes.into();
        fake
    }

    fn file(&self, path: &Path) -> Option<FakeFile> {
        self.files.borrow().get(path).cloned()
    }

    fn profiles(&self) -> Vec<H264Profile> {
        self.asked
            .borrow()
            .iter()
            .filter_map(|ask| match ask {
                EncodeAsk::Video { encoding, .. } => Some(encoding.profile),
                EncodeAsk::Sound(_) => None,
            })
            .collect()
    }

    /// Every file an attempt made is gone again.
    fn assert_all_removed(&self) {
        let made = self.made.borrow();
        let removed = self.removed.borrow();
        for path in made.iter() {
            assert!(removed.contains(path), "{path:?} was left behind");
        }
    }
}

impl MediaPreparing for Fake {
    fn read_streams<'a>(&'a self, file: &'a Path) -> LocalBoxFuture<'a, Option<StreamReading>> {
        Box::pin(ready(self.file(file).and_then(|file| file.reading)))
    }

    fn sound_tracks<'a>(&'a self, file: &'a Path) -> LocalBoxFuture<'a, Option<Vec<SoundTrack>>> {
        self.tracks_read.set(self.tracks_read.get() + 1);
        Box::pin(ready(self.file(file).and_then(|file| file.tracks)))
    }

    fn frame<'a>(&'a self, file: &'a Path, _: f64) -> LocalBoxFuture<'a, Option<Vec<u8>>> {
        Box::pin(ready(self.file(file).and_then(|file| file.frame)))
    }

    fn encode<'a>(
        &'a self,
        _: &'a Path,
        ask: EncodeAsk,
        cancel: Cancel,
    ) -> LocalBoxFuture<'a, Result<Attempted<PathBuf>, AttemptFailure>> {
        self.asked.borrow_mut().push(ask);
        let outcome = self.outcomes.borrow_mut().pop_front().expect("an outcome");
        match outcome {
            Outcome::Made(file) => {
                let path =
                    PathBuf::from(format!("/cache/prepared/{}.mp4", self.made.borrow().len()));
                self.files.borrow_mut().insert(path.clone(), *file);
                self.made.borrow_mut().push(path.clone());
                Box::pin(ready(Ok(Attempted::Taken(path))))
            }
            Outcome::Refused => Box::pin(ready(Ok(Attempted::Refused))),
            Outcome::Wrong => Box::pin(ready(Ok(Attempted::Wrong))),
            Outcome::Fail => Box::pin(ready(Err(AttemptFailure::new("gst-stream-error-quark", 1)))),
            Outcome::Hang => Box::pin(async move {
                cancel.cancelled().await;
                Ok(Attempted::Refused)
            }),
        }
    }

    fn file_length(&self, file: &Path) -> Option<u64> {
        self.file(file).map(|file| file.bytes.len() as u64)
    }

    fn read_file<'a>(&'a self, file: &'a Path) -> LocalBoxFuture<'a, Option<Vec<u8>>> {
        Box::pin(ready(self.file(file).map(|file| file.bytes)))
    }

    fn remove(&self, file: &Path) {
        self.removed.borrow_mut().push(file.to_path_buf());
        self.files.borrow_mut().remove(file);
    }
}

// --- files --------------------------------------------------------------------------------------

/// A source's path with a name nobody may read in the log.
const SOURCE: &str = "/home/someone/Videos/Granny's secret birthday.mp4";

fn boxed(kind: &str, body: &[u8]) -> Vec<u8> {
    let mut out = ((8 + body.len()) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind.as_bytes());
    out.extend_from_slice(body);
    out
}

/// An MP4 with its index in front (or behind) — what `mp4mux faststart=true` writes.
fn mp4_bytes(moov_first: bool, media: usize) -> Vec<u8> {
    let ftyp = boxed("ftyp", b"isom\0\0\x02\0isomiso2avc1mp41");
    let moov = boxed("moov", &boxed("mvhd", &[0u8; 100]));
    let mdat = boxed("mdat", &vec![7u8; media]);
    if moov_first {
        [ftyp, moov, mdat].concat()
    } else {
        [ftyp, mdat, moov].concat()
    }
}

/// A frame with something in it: a ramp across, a little down — or the same picture a quarter
/// turn round.
fn picture(turned: bool) -> Vec<u8> {
    let side = frame_turn::SIDE;
    let mut bgra = Vec::with_capacity(4 * side * side);
    for y in 0..side {
        for x in 0..side {
            let (u, v) = if turned { (y, side - 1 - x) } else { (x, y) };
            let level = (u * 14 + v * 2) as u8;
            bgra.extend([level, level, level, 255]);
        }
    }
    bgra
}

fn h264(width: u32, height: u32) -> VideoStream {
    VideoStream {
        codec: "h264".into(),
        profile: Some("high".into()),
        width,
        height,
        frame_rate_numerator: 30,
        frame_rate_denominator: 1,
        ..VideoStream::default()
    }
}

fn aac_stream(channels: i64, sample_rate: i64, bitrate: i64) -> AudioStream {
    AudioStream {
        codec: "aac".into(),
        channels: Some(channels),
        sample_rate: Some(sample_rate),
        bitrate: Some(bitrate),
    }
}

fn sound_track(track_id: u32, channels: Option<u32>, sample_rate: Option<u32>) -> SoundTrack {
    SoundTrack {
        track_id,
        enabled: true,
        alternate_group: 0,
        channels,
        sample_rate,
    }
}

/// A 1080p phone clip at 8 Mbit/s with stereo AAC — over the profile's 720, so it is transcoded.
fn phone_clip() -> FakeFile {
    FakeFile {
        reading: Some(StreamReading {
            video: Some(VideoStream {
                bitrate: Some(8_000_000),
                ..h264(1920, 1080)
            }),
            audio: vec![aac_stream(2, 48_000, 128_000)],
        }),
        tracks: Some(vec![sound_track(2, Some(2), Some(48_000))]),
        frame: Some(picture(false)),
        bytes: Vec::new(),
    }
}

/// What a transcode of the phone clip writes when it comes out right.
fn its_transcode() -> FakeFile {
    FakeFile {
        reading: Some(StreamReading {
            video: Some(h264(1280, 720)),
            audio: vec![aac_stream(2, 48_000, 128_000)],
        }),
        tracks: Some(vec![sound_track(1, Some(2), Some(48_000))]),
        frame: Some(picture(false)),
        bytes: mp4_bytes(true, 4000),
    }
}

fn head() -> Vec<u8> {
    mp4_bytes(true, 0)[..32].to_vec()
}

fn video_job<'a>(head: &'a [u8], size: u64) -> VideoJob<'a> {
    VideoJob {
        file: Path::new(SOURCE),
        container: "video/mp4",
        size,
        head,
        duration_ms: Some(19_000),
    }
}

fn clock() -> FakeClock {
    FakeClock::at(datetime!(2026-10-29 10:00 UTC))
}

/// Runs a video's preparing to its end on the fake clock, and hands back what went and what was
/// said.
fn video(fake: &Fake, job: &VideoJob<'_>) -> (Result<Prepared, OperationCanceled>, Vec<String>) {
    let clock = clock();
    let mut lines = Vec::new();
    let mut log = |line: &str| lines.push(line.to_string());
    let prepared = drive(
        &clock,
        prepare_video(job, fake, &clock, &mut log, &Cancel::new()),
    );
    for line in &lines {
        assert!(!line.contains("secret"), "a name reached the log: {line}");
        assert!(!line.contains("/home"), "a path reached the log: {line}");
    }
    fake.assert_all_removed();
    (prepared, lines)
}

fn said(lines: &[String], part: &str) -> bool {
    lines.iter().any(|line| line.contains(part))
}

// --- a video ------------------------------------------------------------------------------------

#[test]
fn a_planned_video_is_transcoded_checked_and_sent() {
    let fake = Fake::with(Path::new(SOURCE), phone_clip(), vec![made(its_transcode())]);
    let head = head();
    let (prepared, lines) = video(&fake, &video_job(&head, 20_000_000));
    let Ok(Prepared::Made(made)) = prepared else {
        panic!("{prepared:?}");
    };
    assert_eq!(mp4_bytes(true, 4000), made.bytes);
    assert_eq!("video/mp4", made.mime);
    assert_eq!((Some(1280), Some(720)), (made.width, made.height));
    assert!(made.moov_first);
    assert_eq!(
        vec![EncodeAsk::Video {
            encoding: VideoEncoding {
                width: 1280,
                height: 720,
                rotation: 0,
                bitrate: 2_000_000,
                frame_rate_numerator: 30,
                frame_rate_denominator: 1,
                profile: H264Profile::High,
                audio: Some(AacEncoding {
                    sample_rate: 48_000,
                    channels: 2,
                    bitrate: 128_000
                }),
                to_sdr: false,
                keep_audio: false,
                keyframe_spacing: None,
            },
            mix: None,
        }],
        *fake.asked.borrow()
    );
    assert!(said(
        &lines,
        "transcoding a video to 1280x720, 30.000 fps, 2000000 bit/s, audio 128000 bit/s, sound tracks 1"
    ));
}

#[test]
fn a_refused_high_profile_is_asked_again_as_main() {
    let fake = Fake::with(
        Path::new(SOURCE),
        phone_clip(),
        vec![Outcome::Refused, made(its_transcode())],
    );
    let head = head();
    let (prepared, lines) = video(&fake, &video_job(&head, 20_000_000));
    assert!(matches!(prepared, Ok(Prepared::Made(_))), "{prepared:?}");
    assert_eq!(vec![H264Profile::High, H264Profile::Main], fake.profiles());
    assert!(said(&lines, "a transcode profile was refused (H.264 High"));
}

#[test]
fn a_pipeline_that_fails_is_the_next_ways_to_try() {
    let fake = Fake::with(
        Path::new(SOURCE),
        phone_clip(),
        vec![Outcome::Fail, made(its_transcode())],
    );
    let head = head();
    let (prepared, lines) = video(&fake, &video_job(&head, 20_000_000));
    assert!(matches!(prepared, Ok(Prepared::Made(_))), "{prepared:?}");
    assert!(said(
        &lines,
        "a transcode failed: gst-stream-error-quark 0x00000001"
    ));
}

/// The trap an unpinned raw format falls into (shell-design §1.4): `x264enc` negotiated High
/// 4:4:4, which no browser or Apple decoder plays — the next encoder is asked.
#[test]
fn a_result_in_a_profile_no_player_plays_is_the_next_encoders_to_get_right() {
    let mut unplayable = its_transcode();
    if let Some(reading) = unplayable.reading.as_mut() {
        reading.video.as_mut().unwrap().profile = Some("high-4:4:4".into());
    }
    let fake = Fake::with(
        Path::new(SOURCE),
        phone_clip(),
        vec![made(unplayable), made(its_transcode())],
    );
    let head = head();
    let (prepared, lines) = video(&fake, &video_job(&head, 20_000_000));
    assert!(matches!(prepared, Ok(Prepared::Made(_))), "{prepared:?}");
    assert_eq!(2, fake.made.borrow().len());
    assert!(said(
        &lines,
        "H.264 high-4:4:4, which not every player plays"
    ));
}

/// Rule E: a result with no sound track from a source with one is a failed transcode however small,
/// and no other way of asking would bring the sound back — the original goes.
#[test]
fn a_result_that_lost_the_sound_is_wrong_and_the_original_goes() {
    let mut silent = its_transcode();
    silent.tracks = Some(Vec::new());
    let fake = Fake::with(
        Path::new(SOURCE),
        phone_clip(),
        vec![made(silent), made(its_transcode())],
    );
    let head = head();
    let (prepared, lines) = video(&fake, &video_job(&head, 20_000_000));
    assert_eq!(Ok(Prepared::Original), prepared);
    assert_eq!(1, fake.asked.borrow().len(), "Wrong ends the asking");
    assert!(said(
        &lines,
        "a transcode came out with no sound track where its source had one"
    ));
}

#[test]
fn a_result_on_its_side_is_wrong() {
    let mut turned = its_transcode();
    turned.frame = Some(picture(true));
    let fake = Fake::with(Path::new(SOURCE), phone_clip(), vec![made(turned)]);
    let head = head();
    let (prepared, lines) = video(&fake, &video_job(&head, 20_000_000));
    assert_eq!(Ok(Prepared::Original), prepared);
    assert!(said(
        &lines,
        "a transcode does not look like its source (Turned)"
    ));
}

/// A source no frame can be read from cannot be judged and is let through on its numbers; a result
/// no frame can be read from, beside a source that gave one, is not a video to send.
#[test]
fn frames_that_cannot_be_read_judge_nothing_unless_only_the_results_are_missing() {
    let mut dark = phone_clip();
    dark.frame = None;
    let fake = Fake::with(Path::new(SOURCE), dark, vec![made(its_transcode())]);
    let head = head();
    let (prepared, lines) = video(&fake, &video_job(&head, 20_000_000));
    assert!(matches!(prepared, Ok(Prepared::Made(_))), "{prepared:?}");
    assert!(said(&lines, "it goes on its numbers alone"));

    let mut blind = its_transcode();
    blind.frame = None;
    let fake = Fake::with(Path::new(SOURCE), phone_clip(), vec![made(blind)]);
    let (prepared, _) = video(&fake, &video_job(&head, 20_000_000));
    assert_eq!(Ok(Prepared::Original), prepared);
}

/// Rule A: already within the profile — nothing is asked, and the container is not even read.
#[test]
fn a_video_within_the_profile_is_sent_as_it_is() {
    let mut within = phone_clip();
    within.reading = Some(StreamReading {
        video: Some(VideoStream {
            bitrate: Some(1_500_000),
            ..h264(1280, 720)
        }),
        audio: vec![aac_stream(2, 48_000, 128_000)],
    });
    let fake = Fake::with(Path::new(SOURCE), within, Vec::new());
    let head = head();
    let (prepared, _) = video(&fake, &video_job(&head, 5_000_000));
    assert_eq!(Ok(Prepared::Original), prepared);
    assert!(fake.asked.borrow().is_empty());
    assert_eq!(0, fake.tracks_read.get());
    // Within the profile and over the ceiling: refused, as 1.1 refused it.
    let (prepared, _) = video(&fake, &video_job(&head, media::SIZE_LIMIT + 1));
    assert_eq!(Ok(Prepared::TooLarge), prepared);
}

/// L1 of issue #98: the container lists a sound track the discoverer does not show. With the
/// container's numbers it is hidden sound before the plan; without them, Rule E's "unheard". Either
/// way the original goes and nothing is asked.
#[test]
fn sound_the_discoverer_does_not_show_is_left_alone() {
    let head = head();
    let mut hidden = phone_clip();
    hidden.reading.as_mut().unwrap().audio.clear();
    let fake = Fake::with(Path::new(SOURCE), hidden.clone(), Vec::new());
    let (prepared, lines) = video(&fake, &video_job(&head, 20_000_000));
    assert_eq!(Ok(Prepared::Original), prepared);
    assert!(said(
        &lines,
        "a video has sound its container lists and the discoverer does not"
    ));

    hidden.tracks = Some(vec![sound_track(2, None, None)]);
    let fake = Fake::with(Path::new(SOURCE), hidden, Vec::new());
    let (prepared, lines) = video(&fake, &video_job(&head, 20_000_000));
    assert_eq!(Ok(Prepared::Original), prepared);
    assert!(said(
        &lines,
        "a video has a sound track the discoverer does not show"
    ));
    assert!(fake.asked.borrow().is_empty());
}

fn screen_recording() -> FakeFile {
    let mut clip = phone_clip();
    clip.reading.as_mut().unwrap().audio = vec![
        aac_stream(2, 48_000, 128_000),
        aac_stream(1, 48_000, 64_000),
    ];
    clip.tracks = Some(vec![
        sound_track(2, Some(2), Some(48_000)),
        sound_track(3, Some(1), Some(48_000)),
    ]);
    clip
}

/// A screen recording's app sound and microphone: both voices MIXED into the one track, stereo at
/// the highest rate, at the profile's row capped by the two together.
#[test]
fn two_voices_are_mixed_into_one_track() {
    let fake = Fake::with(
        Path::new(SOURCE),
        screen_recording(),
        vec![made(its_transcode())],
    );
    let head = head();
    let (prepared, lines) = video(&fake, &video_job(&head, 20_000_000));
    assert!(matches!(prepared, Ok(Prepared::Made(_))), "{prepared:?}");
    let asked = fake.asked.borrow();
    let EncodeAsk::Video { encoding, mix } = &asked[0] else {
        panic!("{asked:?}");
    };
    assert_eq!(Some(vec![0, 1]), *mix);
    assert_eq!(
        Some(AacEncoding {
            sample_rate: 48_000,
            channels: 2,
            bitrate: 128_000
        }),
        encoding.audio
    );
    assert!(said(&lines, "mixing 2 sound tracks (track ids 2,3)"));
}

#[test]
fn a_mix_that_came_out_as_two_tracks_is_wrong() {
    let mut side_by_side = its_transcode();
    side_by_side.tracks = Some(vec![
        sound_track(1, Some(2), Some(48_000)),
        sound_track(2, Some(1), Some(48_000)),
    ]);
    let fake = Fake::with(
        Path::new(SOURCE),
        screen_recording(),
        vec![made(side_by_side)],
    );
    let head = head();
    let (prepared, lines) = video(&fake, &video_job(&head, 20_000_000));
    assert_eq!(Ok(Prepared::Original), prepared);
    assert!(said(&lines, "a mix came out with 2 sound tracks, not one"));
}

/// Fewer listed streams than sound tracks: which one is missing cannot be known, and a mix of the
/// wrong ones keeps fewer voices than the source has.
#[test]
fn streams_that_do_not_line_up_are_not_mixed() {
    let mut short = screen_recording();
    short.reading.as_mut().unwrap().audio.truncate(1);
    let fake = Fake::with(Path::new(SOURCE), short, Vec::new());
    let head = head();
    let (prepared, lines) = video(&fake, &video_job(&head, 20_000_000));
    assert_eq!(Ok(Prepared::Original), prepared);
    assert!(fake.asked.borrow().is_empty());
    assert!(said(
        &lines,
        "a video's 2 sound tracks do not line up with the 1 audio streams"
    ));
}

/// Rule D: a result bigger than a source that could go is thrown away.
#[test]
fn a_bigger_result_goes_as_the_original() {
    let fake = Fake::with(Path::new(SOURCE), phone_clip(), vec![made(its_transcode())]);
    let head = head();
    let (prepared, lines) = video(&fake, &video_job(&head, 1_000));
    assert_eq!(Ok(Prepared::Original), prepared);
    assert!(said(&lines, "a transcode came out bigger than its source"));
}

/// Its index behind its media data, and moved in front here — `mp4mux faststart=true` should have
/// done it, but a file that came out otherwise still goes right.
#[test]
fn an_index_at_the_end_is_moved_to_the_front() {
    let mut behind = its_transcode();
    behind.bytes = mp4_bytes(false, 4000);
    let fake = Fake::with(Path::new(SOURCE), phone_clip(), vec![made(behind)]);
    let head = head();
    let (prepared, _) = video(&fake, &video_job(&head, 20_000_000));
    let Ok(Prepared::Made(made)) = prepared else {
        panic!("{prepared:?}");
    };
    assert!(made.moov_first);
    assert_eq!(mp4_bytes(true, 4000).len(), made.bytes.len());
    assert_eq!(
        &b"moov"[..],
        &made.bytes[36..40],
        "the index right after ftyp"
    );
}

#[test]
fn a_result_whose_bytes_are_not_mp4_is_wrong() {
    let mut lying = its_transcode();
    lying.bytes = b"RIFF\0\0\0\0WAVEfmt not an mp4".to_vec();
    let fake = Fake::with(Path::new(SOURCE), phone_clip(), vec![made(lying)]);
    let head = head();
    let (prepared, lines) = video(&fake, &video_job(&head, 20_000_000));
    assert_eq!(Ok(Prepared::Original), prepared);
    assert!(said(
        &lines,
        "a transcode's bytes are not the type they are sent as"
    ));
}

/// Rule C for a source that could not have gone as it is: what 1.1 did — refused over the ceiling.
#[test]
fn an_unsendable_source_whose_transcode_fails_goes_the_way_it_went_before() {
    let fake = Fake::with(
        Path::new(SOURCE),
        phone_clip(),
        vec![Outcome::Refused, Outcome::Refused],
    );
    let head = head();
    let (prepared, lines) = video(&fake, &video_job(&head, media::SIZE_LIMIT + 1));
    assert_eq!(Ok(Prepared::TooLarge), prepared);
    assert!(said(&lines, "no transcode profile was taken"));
    // A source whose streams cannot be read at all is rule C too, without a transcode.
    let fake = Fake::with(Path::new(SOURCE), FakeFile::default(), Vec::new());
    let (prepared, lines) = video(&fake, &video_job(&head, 20_000_000));
    assert_eq!(Ok(Prepared::Original), prepared);
    assert!(said(&lines, "a video's streams could not be read"));
}

/// A shell attempt that finds the source no longer what was read of it ends the asking.
#[test]
fn an_attempt_the_shell_calls_wrong_ends_the_asking() {
    let fake = Fake::with(
        Path::new(SOURCE),
        phone_clip(),
        vec![Outcome::Wrong, made(its_transcode())],
    );
    let head = head();
    let (prepared, _) = video(&fake, &video_job(&head, 20_000_000));
    assert_eq!(Ok(Prepared::Original), prepared);
    assert_eq!(1, fake.asked.borrow().len());
}

/// One ceiling for every way of asking together: a pipeline that stalls is stopped there, and the
/// original goes.
#[test]
fn a_stalled_transcode_is_stopped_at_its_ceiling() {
    let fake = Fake::with(
        Path::new(SOURCE),
        phone_clip(),
        vec![Outcome::Hang, made(its_transcode())],
    );
    let head = head();
    let clock = clock();
    let mut lines = Vec::new();
    let mut log = |line: &str| lines.push(line.to_string());
    let prepared = drive(
        &clock,
        prepare_video(
            &video_job(&head, 20_000_000),
            &fake,
            &clock,
            &mut log,
            &Cancel::new(),
        ),
    );
    assert_eq!(Ok(Prepared::Original), prepared);
    assert_eq!(1, fake.asked.borrow().len());
    // Two minutes and three times the 19-second clip.
    assert!(clock.slept().contains(&Duration::from_secs(177)));
    assert!(said(&lines, "a transcode ran past its ceiling"));
}

/// The person's cancel is not a failure: never rule C's original for somebody who just took it
/// back.
#[test]
fn the_persons_cancel_is_not_a_failure() {
    let fake = Fake::with(Path::new(SOURCE), phone_clip(), vec![Outcome::Hang]);
    let head = head();
    let clock = clock();
    let cancel = Cancel::new();
    let mut log = |_: &str| {};
    let job = video_job(&head, 20_000_000);
    let (prepared, ()) = futures::executor::block_on(async {
        futures::join!(
            prepare_video(&job, &fake, &clock, &mut log, &cancel),
            async { cancel.cancel() }
        )
    });
    assert_eq!(Err(OperationCanceled), prepared);
    // A cancel raised before it starts is a cancel too.
    let mut log = |_: &str| {};
    let fake = Fake::with(Path::new(SOURCE), phone_clip(), Vec::new());
    assert_eq!(
        Err(OperationCanceled),
        drive(
            &clock,
            prepare_video(&job, &fake, &clock, &mut log, &cancel)
        )
    );
    assert!(fake.asked.borrow().is_empty());
}

// --- the numbers alone --------------------------------------------------------------------------

/// A portrait phone clip: a landscape frame with a 90° tag is DISPLAYED portrait, its HDR read from
/// the ISO code points, its rate from the container where the stream names none.
#[test]
fn what_the_planner_is_given_is_the_displayed_clip() {
    let reading = StreamReading {
        video: Some(VideoStream {
            codec: "hevc".into(),
            profile: None,
            width: 3840,
            height: 2160,
            rotation: Some(90),
            frame_rate_numerator: 0,
            frame_rate_denominator: 1,
            container_frame_rate: Some(59.94),
            pixel_aspect_numerator: 1,
            pixel_aspect_denominator: 1,
            transfer: Some(18),
            primaries: Some(9),
            bitrate: Some(40_000_000),
        }),
        audio: vec![aac_stream(2, 48_000, 256_000)],
    };
    let input = video_input(&reading, "video/quicktime", 150_000_000, Some(30_000), None).unwrap();
    assert_eq!((2160, 3840), (input.source.width, input.source.height));
    assert_eq!(Some(59.94), input.source.frame_rate);
    assert_eq!(90, input.rotation);
    assert!(input.hdr);
    assert_eq!(Some(48_000), input.audio_sample_rate);
    assert_eq!(Some(2), input.source.audio_channels);
    // Anamorphic pixels are left alone.
    let mut squashed = reading.clone();
    squashed.video.as_mut().unwrap().pixel_aspect_numerator = 4;
    squashed.video.as_mut().unwrap().pixel_aspect_denominator = 3;
    assert!(video_input(&squashed, "video/mp4", 1, None, None).is_err());
    // No video stream at all.
    assert!(video_input(&StreamReading::default(), "video/mp4", 1, None, None).is_err());
}

/// The read-back's order: the codec first, then the sound, then the picture's shape.
#[test]
fn a_result_is_judged_in_the_order_the_csharp_judges_it() {
    let asked = VideoEncoding {
        width: 1280,
        height: 720,
        rotation: 90,
        bitrate: 2_000_000,
        frame_rate_numerator: 30,
        frame_rate_denominator: 1,
        profile: H264Profile::High,
        audio: Some(AacEncoding {
            sample_rate: 48_000,
            channels: 2,
            bitrate: 128_000,
        }),
        to_sdr: true,
        keep_audio: false,
        keyframe_spacing: None,
    };
    let right = StreamReading {
        video: Some(VideoStream {
            rotation: Some(90),
            ..h264(1280, 720)
        }),
        audio: vec![aac_stream(2, 48_000, 128_000)],
    };
    assert_eq!(Ok(()), judge_video(&asked, &right, Some(1), Some(1)));
    let end = |back: &StreamReading| judge_video(&asked, back, Some(1), Some(1)).unwrap_err().end;
    let mut hevc = right.clone();
    hevc.video.as_mut().unwrap().codec = "hevc".into();
    assert_eq!(AttemptEnd::Wrong, end(&hevc));
    let mut sideways = right.clone();
    sideways.video.as_mut().unwrap().rotation = None;
    assert_eq!(AttemptEnd::Wrong, end(&sideways));
    let mut fast = right.clone();
    fast.video.as_mut().unwrap().frame_rate_numerator = 60;
    assert_eq!(AttemptEnd::Wrong, end(&fast));
    let mut still_hdr = right.clone();
    still_hdr.video.as_mut().unwrap().transfer = Some(16);
    assert_eq!(AttemptEnd::Wrong, end(&still_hdr));
    let mut loud = right.clone();
    loud.audio[0].bitrate = Some(192_000);
    assert_eq!(AttemptEnd::Refused, end(&loud));
    let mut mute = right.clone();
    mute.audio.clear();
    assert_eq!(AttemptEnd::Wrong, end(&mute));
    assert_eq!(
        AttemptEnd::Wrong,
        judge_video(&asked, &right, Some(1), Some(0))
            .unwrap_err()
            .end
    );
}

// --- a sound file -------------------------------------------------------------------------------

fn wav() -> FakeFile {
    FakeFile {
        reading: Some(StreamReading {
            video: None,
            audio: vec![AudioStream {
                codec: "pcm".into(),
                channels: Some(2),
                sample_rate: Some(44_100),
                bitrate: Some(1_411_200),
            }],
        }),
        ..FakeFile::default()
    }
}

fn m4a(channels: i64, bitrate: i64) -> FakeFile {
    FakeFile {
        reading: Some(StreamReading {
            video: None,
            audio: vec![aac_stream(channels, 44_100, bitrate)],
        }),
        bytes: mp4_bytes(true, 2000),
        ..FakeFile::default()
    }
}

fn sound(fake: &Fake, job: &SoundJob<'_>) -> (Result<Prepared, OperationCanceled>, Vec<String>) {
    let clock = clock();
    let mut lines = Vec::new();
    let mut log = |line: &str| lines.push(line.to_string());
    let prepared = drive(
        &clock,
        prepare_sound(job, fake, &clock, &mut log, &Cancel::new()),
    );
    fake.assert_all_removed();
    (prepared, lines)
}

const SONG: &str = "/home/someone/Music/take.wav";

/// The reasoning doc's fourth fixture: a WAV becomes AAC 128k in an M4A.
#[test]
fn a_wav_is_re_encoded_into_an_m4a() {
    let fake = Fake::with(Path::new(SONG), wav(), vec![made(m4a(2, 128_000))]);
    let head = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
    let job = SoundJob {
        file: Path::new(SONG),
        container: "audio/wav",
        as_audio: true,
        size: 31_752_000,
        head: &head,
        duration_ms: Some(180_000),
    };
    let (prepared, lines) = sound(&fake, &job);
    let Ok(Prepared::Made(made)) = prepared else {
        panic!("{prepared:?}");
    };
    assert_eq!("audio/mp4", made.mime);
    assert_eq!((None, None), (made.width, made.height));
    assert_eq!(
        vec![EncodeAsk::Sound(AacEncoding {
            sample_rate: 44_100,
            channels: 2,
            bitrate: 128_000
        })],
        *fake.asked.borrow()
    );
    assert!(said(
        &lines,
        "re-encoding a sound file (pcm) at 128000 bit/s"
    ));
}

/// A re-encode the encoder wrote at a rate of its own choosing is not taken; the WAV goes as it was.
#[test]
fn a_re_encode_at_a_rate_of_its_own_is_not_taken() {
    let mut mono = wav();
    mono.reading.as_mut().unwrap().audio[0].channels = Some(1);
    let fake = Fake::with(Path::new(SONG), mono, vec![made(m4a(1, 96_000))]);
    let head = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
    let job = SoundJob {
        file: Path::new(SONG),
        container: "audio/wav",
        as_audio: true,
        size: 15_876_000,
        head: &head,
        duration_ms: Some(180_000),
    };
    let (prepared, lines) = sound(&fake, &job);
    assert_eq!(Ok(Prepared::Original), prepared);
    assert!(said(
        &lines,
        "a re-encode came out at 96000 bit/s; asked 64000"
    ));
}

#[test]
fn a128k_mp3_is_sent_as_it_is() {
    let fake = Fake::with(
        Path::new(SONG),
        FakeFile {
            reading: Some(StreamReading {
                video: None,
                audio: vec![AudioStream {
                    codec: "mp3".into(),
                    channels: Some(2),
                    sample_rate: Some(44_100),
                    bitrate: Some(128_000),
                }],
            }),
            ..FakeFile::default()
        },
        Vec::new(),
    );
    let head = b"ID3\x04\0\0\0\0\0\0".to_vec();
    let job = SoundJob {
        file: Path::new(SONG),
        container: "audio/mpeg",
        as_audio: true,
        size: 2_880_000,
        head: &head,
        duration_ms: Some(180_000),
    };
    let (prepared, _) = sound(&fake, &job);
    assert_eq!(Ok(Prepared::Original), prepared);
    assert!(fake.asked.borrow().is_empty());
}

/// A FLAC the router sent as a file (the server takes no FLAC as audio): re-encoded when it can be,
/// and as the file it always went as when it cannot.
#[test]
fn a_flac_whose_re_encode_fails_goes_as_a_file() {
    let flac = FakeFile {
        reading: Some(StreamReading {
            video: None,
            audio: vec![AudioStream {
                codec: "flac".into(),
                channels: Some(2),
                sample_rate: Some(96_000),
                bitrate: None,
            }],
        }),
        ..FakeFile::default()
    };
    let fake = Fake::with(Path::new(SONG), flac.clone(), vec![Outcome::Refused]);
    let head = b"fLaC\0\0\0\x22".to_vec();
    let job = SoundJob {
        file: Path::new(SONG),
        container: "audio/flac",
        as_audio: false,
        size: 40_000_000,
        head: &head,
        duration_ms: Some(180_000),
    };
    let (prepared, _) = sound(&fake, &job);
    assert_eq!(Ok(Prepared::AsFile), prepared);
    assert_eq!(
        vec![EncodeAsk::Sound(AacEncoding {
            sample_rate: 48_000,
            channels: 2,
            bitrate: 128_000
        })],
        *fake.asked.borrow()
    );
    // And as audio when it comes out.
    let fake = Fake::with(Path::new(SONG), flac, vec![made(m4a(2, 128_000))]);
    let (prepared, _) = sound(&fake, &job);
    assert!(matches!(prepared, Ok(Prepared::Made(_))), "{prepared:?}");
}

/// A file nothing can read is judged as a codec nobody can name: left alone, as 1.1 left it.
#[test]
fn a_sound_nothing_can_read_is_left_as_it_was() {
    let source = sound_source(None, "audio/mpeg", 2_000_000, None);
    assert_eq!("unknown", source.codec);
    let fake = Fake::with(Path::new(SONG), FakeFile::default(), Vec::new());
    let head = b"ID3\x04".to_vec();
    let job = SoundJob {
        file: Path::new(SONG),
        container: "audio/mpeg",
        as_audio: true,
        size: media::SIZE_LIMIT + 1,
        head: &head,
        duration_ms: None,
    };
    let (prepared, _) = sound(&fake, &job);
    assert_eq!(Ok(Prepared::TooLarge), prepared);
}
