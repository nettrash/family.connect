//! Preparing picked media (shell-design §10.7; fc-logic's `media_encoding::preparing` —
//! `MediaPreparing`'s rustdoc is the contract, `prepare_video`/`prepare_sound` decide everything):
//! the discoverer reads a picked video or sound, a transcode runs ONE pipeline per attempt
//! (`x264enc` High → `openh264enc` Main; `avenc_aac` → `voaacenc`), the result is read back and
//! judged by fc-logic, a cancel sets the pipeline to `Null` and deletes the temporary — a cancel is
//! not a failure and stages nothing.
//!
//! [`prepare_picked`] is what P1's composer hands each picked file to (through
//! `ComposerStaging::ingest`): a video, a sound file, or an animated picture — anything else is
//! [`Picked::NotMine`] and goes P1's/P2's way. The batch's status — a finishing old batch never
//! clears a newer one's "Preparing…" — is `ComposerStaging`'s (carry-forward 24(e)), not here.
//!
//! The frame grabs (a frame of a file at a time, upright, at a size and format asked) are here too:
//! `frame_turn`'s 16 × 16 BGRA, a picked video's poster, a video message's poster.
//!
//! Logs say sizes, counts, rates and codecs — never a name or a path.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use fc_core::rt::{Cancel, LocalBoxFuture};
use fc_logic::composer_staging::PrepFailure;
use fc_logic::container_sound::{self, SoundTrack};
use fc_logic::media_encoding::preparing::{
    AudioStream, EncodeAsk, MediaPreparing, Prepared, SoundJob, StreamReading, VideoJob,
    VideoStream,
};
use fc_logic::media_encoding::{self, preparing as logic};
use fc_logic::media_outbox::StagedMedia;
use fc_logic::transcode_attempts::{AttemptFailure, Attempted, OperationCanceled};
use fc_text::media;
use gst::prelude::*;
use gst_pbutils::prelude::*;

use super::gst_util::{self, PipelineError};
use super::pipelines::{self, AacEncoder};
use super::voice_recorder::Stopper;
use crate::shell::Shell;

/// How long the discoverer may read one file (§10.7).
const DISCOVER_LIMIT: Duration = Duration::from_secs(10);
/// How long a frame grab may take.
const GRAB_LIMIT: Duration = Duration::from_secs(10);
/// A temporary left in `prepared/` for longer than this is swept when the folder is made.
const STALE: Duration = Duration::from_secs(24 * 60 * 60);

/// The planner's name for a video stream's codec, from its caps.
pub fn video_codec(structure: &gst::StructureRef) -> String {
    match structure.name().as_str() {
        "video/x-h264" => "h264",
        "video/x-h265" => "hevc",
        "video/x-av1" => "av1",
        "video/x-vp9" => "vp9",
        _ => "unknown",
    }
    .to_string()
}

/// The planner's name for an audio stream's codec, from its caps.
pub fn audio_codec(structure: &gst::StructureRef) -> String {
    match structure.name().as_str() {
        "audio/mpeg" => match structure.get::<i32>("mpegversion").unwrap_or(0) {
            2 | 4 => "aac",
            1 if structure.get::<i32>("layer").unwrap_or(0) == 3 => "mp3",
            _ => "unknown",
        },
        "audio/x-raw" => "pcm",
        "audio/x-flac" => "flac",
        "audio/x-alac" => "alac",
        "audio/x-vorbis" => "vorbis",
        "audio/x-opus" => "opus",
        _ => "unknown",
    }
    .to_string()
}

/// The turn an `image-orientation` tag states, in degrees (a mirror is no turn).
fn turn_of(tags: Option<&gst::TagList>) -> Option<i64> {
    let orientation = tags?.get::<gst::tags::ImageOrientation>()?;
    match orientation.get() {
        "rotate-0" => Some(0),
        "rotate-90" => Some(90),
        "rotate-180" => Some(180),
        "rotate-270" => Some(270),
        _ => None,
    }
}

fn known(value: u32) -> Option<i64> {
    (value > 0).then_some(i64::from(value))
}

fn tag_bitrate(tags: Option<&gst::TagList>) -> Option<i64> {
    let tags = tags?;
    tags.get::<gst::tags::Bitrate>()
        .map(|rate| rate.get())
        .or_else(|| {
            tags.get::<gst::tags::NominalBitrate>()
                .map(|rate| rate.get())
        })
        .and_then(known)
}

/// What the discoverer reads of `file` as fc-logic's [`StreamReading`].
pub async fn read(file: &Path) -> Option<StreamReading> {
    let info = gst_util::discover(file, DISCOVER_LIMIT).await?;
    let video = info
        .video_streams()
        .into_iter()
        .find(|stream| !stream.is_image())
        .map(|stream| {
            let caps = stream.caps();
            let structure = caps.as_ref().and_then(|caps| caps.structure(0));
            let tags = stream.tags();
            let colorimetry = structure
                .and_then(|s| s.get::<String>("colorimetry").ok())
                .and_then(|text| text.parse::<gst_video::VideoColorimetry>().ok());
            let framerate = stream.framerate();
            let par = stream.par();
            VideoStream {
                codec: structure.map_or_else(|| "unknown".to_string(), video_codec),
                profile: structure.and_then(|s| s.get::<String>("profile").ok()),
                width: stream.width(),
                height: stream.height(),
                rotation: turn_of(tags.as_ref()),
                frame_rate_numerator: u32::try_from(framerate.numer()).unwrap_or(0),
                frame_rate_denominator: u32::try_from(framerate.denom()).unwrap_or(0),
                container_frame_rate: None,
                pixel_aspect_numerator: u32::try_from(par.numer()).unwrap_or(0),
                pixel_aspect_denominator: u32::try_from(par.denom()).unwrap_or(0),
                transfer: colorimetry
                    .as_ref()
                    .map(|c| i64::from(c.transfer().to_iso())),
                primaries: colorimetry
                    .as_ref()
                    .map(|c| i64::from(c.primaries().to_iso())),
                bitrate: known(stream.bitrate()).or_else(|| tag_bitrate(tags.as_ref())),
            }
        });
    let audio = info
        .audio_streams()
        .into_iter()
        .map(|stream| {
            let caps = stream.caps();
            let structure = caps.as_ref().and_then(|caps| caps.structure(0));
            AudioStream {
                codec: structure.map_or_else(|| "unknown".to_string(), audio_codec),
                channels: known(stream.channels()),
                sample_rate: known(stream.sample_rate()),
                bitrate: known(stream.bitrate()).or_else(|| tag_bitrate(stream.tags().as_ref())),
            }
        })
        .collect();
    let reading = StreamReading { video, audio };
    (reading.video.is_some() || !reading.audio.is_empty()).then_some(reading)
}

/// A file's length in milliseconds as the discoverer reads it.
pub async fn duration_ms(file: &Path) -> Option<i64> {
    let info = gst_util::discover(file, DISCOVER_LIMIT).await?;
    info.duration()
        .map(|duration| i64::try_from(duration.mseconds()).unwrap_or(i64::MAX))
        .filter(|ms| *ms > 0)
}

/// One frame of `file` at `at_seconds`, upright, as raw `format` pixels at `width × height`
/// (`None` for a side keeps the picture's own): `playbin3` paused, an accurate seek, the preroll.
/// Answers the frame's caps-described size and its rows (with their stride).
pub async fn grab(
    file: &Path,
    at_seconds: f64,
    format: &str,
    width: Option<u32>,
    height: Option<u32>,
) -> Option<Grabbed> {
    let playbin = gst::ElementFactory::make("playbin3")
        .property("uri", pipelines::file_uri(file))
        .build()
        .ok()?;
    playbin.set_property_from_str("flags", "video");
    let sink =
        gst::parse::bin_from_description(&pipelines::frame_sink(format, width, height), true)
            .ok()?;
    let appsink = sink.by_name("frame")?.downcast::<gst_app::AppSink>().ok()?;
    playbin.set_property("video-sink", &sink);
    let pipeline = gst::Pipeline::new();
    pipeline.add(&playbin).ok()?;
    let _stopper = Stopper(pipeline.clone());
    let mut messages = pipeline.bus()?.stream();
    if pipeline.set_state(gst::State::Paused).is_err() {
        return None;
    }
    gst_util::until_async_done(&pipeline, &mut messages, GRAB_LIMIT)
        .await
        .ok()?;
    if at_seconds > 0.0 {
        pipeline
            .seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst_util::clock_time(at_seconds),
            )
            .ok()?;
        gst_util::until_async_done(&pipeline, &mut messages, GRAB_LIMIT)
            .await
            .ok()?;
    }
    let sample = appsink.try_pull_preroll(gst::ClockTime::ZERO)?;
    let caps = sample.caps()?;
    let info = gst_video::VideoInfo::from_caps(caps).ok()?;
    let buffer = sample.buffer()?;
    let map = buffer.map_readable().ok()?;
    Some(Grabbed {
        width: info.width(),
        height: info.height(),
        stride: usize::try_from(*info.stride().first()?).ok()?,
        bytes: map.as_slice().to_vec(),
    })
}

/// A grabbed frame.
#[derive(Debug, Clone)]
pub struct Grabbed {
    pub width: u32,
    pub height: u32,
    /// Bytes from one row to the next.
    pub stride: usize,
    pub bytes: Vec<u8>,
}

impl Grabbed {
    /// The rows without their padding, `bytes_per_pixel` each.
    pub fn packed(&self, bytes_per_pixel: usize) -> Vec<u8> {
        let row = self.width as usize * bytes_per_pixel;
        let mut out = Vec::with_capacity(row * self.height as usize);
        for y in 0..self.height as usize {
            let at = y * self.stride;
            if let Some(line) = self.bytes.get(at..at + row) {
                out.extend_from_slice(line);
            }
        }
        out
    }

    /// An RGB grab as a baseline JPEG at `quality` (0–1), no EXIF.
    pub fn jpeg(&self, quality: f64) -> Option<Vec<u8>> {
        let width = i32::try_from(self.width).ok()?;
        let height = i32::try_from(self.height).ok()?;
        let packed = self.packed(3);
        let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_bytes(
            &glib::Bytes::from_owned(packed),
            gtk::gdk_pixbuf::Colorspace::Rgb,
            false,
            8,
            width,
            height,
            width * 3,
        );
        let quality = (quality.clamp(0.0, 1.0) * 100.0).round().to_string();
        pixbuf
            .save_to_bufferv("jpeg", &[("quality", quality.as_str())])
            .ok()
    }
}

/// A video's poster as the protocol has it: a frame at 0.5 s, else 0 s, else 2 s, fitted within
/// `edge` (600 for a picked video; the square's 480 for a video message), a JPEG.
pub async fn poster(file: &Path, displayed: Option<(u32, u32)>, edge: u32) -> Option<Vec<u8>> {
    let (width, height) = displayed
        .map(|(w, h)| media::fit_within(w.max(1), h.max(1), edge))
        .map_or((None, None), |(w, h)| (Some(even(w)), Some(even(h))));
    for at in [0.5, 0.0, 2.0] {
        if let Some(frame) = grab(file, at, "RGB", width, height).await
            && let Some(jpeg) = frame.jpeg(media::PREVIEW_QUALITY)
        {
            return Some(jpeg);
        }
    }
    None
}

fn even(side: u32) -> u32 {
    (side & !1).max(2)
}

/// fc-logic's `MediaPreparing` on GStreamer.
#[derive(Debug, Clone)]
pub struct GstMediaPreparing {
    folder: PathBuf,
}

impl GstMediaPreparing {
    /// Its temporaries under `$XDG_CACHE_HOME/family-connect/prepared/` — what an earlier run left
    /// there for over a day is swept now.
    pub fn new(cache: &Path) -> Self {
        let folder = cache.join("prepared");
        sweep(&folder);
        GstMediaPreparing { folder }
    }
}

fn sweep(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > STALE);
        if stale {
            gst_util::remove_file(&entry.path());
        }
    }
}

impl MediaPreparing for GstMediaPreparing {
    fn read_streams<'a>(&'a self, file: &'a Path) -> LocalBoxFuture<'a, Option<StreamReading>> {
        Box::pin(read(file))
    }

    fn sound_tracks<'a>(&'a self, file: &'a Path) -> LocalBoxFuture<'a, Option<Vec<SoundTrack>>> {
        Box::pin(async move {
            let mut opened = std::fs::File::open(file).ok()?;
            container_sound::read_from(&mut opened).ok().flatten()
        })
    }

    fn frame<'a>(&'a self, file: &'a Path, at_seconds: f64) -> LocalBoxFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move {
            let side = u32::try_from(fc_logic::frame_turn::SIDE).ok()?;
            let frame = grab(file, at_seconds, "BGRA", Some(side), Some(side)).await?;
            let packed = frame.packed(4);
            (packed.len() == 4 * fc_logic::frame_turn::SIDE * fc_logic::frame_turn::SIDE)
                .then_some(packed)
        })
    }

    fn encode<'a>(
        &'a self,
        source: &'a Path,
        ask: EncodeAsk,
        cancel: Cancel,
    ) -> LocalBoxFuture<'a, Result<Attempted<PathBuf>, AttemptFailure>> {
        Box::pin(encode(
            self.folder.clone(),
            source.to_path_buf(),
            ask,
            cancel,
        ))
    }

    fn file_length(&self, file: &Path) -> Option<u64> {
        std::fs::metadata(file).ok().map(|meta| meta.len())
    }

    fn read_file<'a>(&'a self, file: &'a Path) -> LocalBoxFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move { std::fs::read(file).ok() })
    }

    fn remove(&self, file: &Path) {
        gst_util::remove_file(file);
    }
}

/// The first AAC encoder this machine has.
fn aac_encoder() -> Option<AacEncoder> {
    AacEncoder::ORDER
        .into_iter()
        .find(|encoder| gst_util::has_element(encoder.factory()))
}

/// ONE attempt (see `MediaPreparing::encode`'s contract).
async fn encode(
    folder: PathBuf,
    source: PathBuf,
    ask: EncodeAsk,
    cancel: Cancel,
) -> Result<Attempted<PathBuf>, AttemptFailure> {
    let (extension, video, audio, mix) = match &ask {
        EncodeAsk::Video { encoding, mix } => ("mp4", Some(*encoding), encoding.audio, mix.clone()),
        EncodeAsk::Sound(aac) => ("m4a", None, Some(*aac), None),
    };
    if let Some(video) = &video
        && !gst_util::has_element(pipelines::h264_factory(video.profile))
    {
        return Ok(Attempted::Refused);
    }
    let encoder = match audio {
        Some(_) => match aac_encoder() {
            Some(encoder) => Some(encoder),
            None => return Ok(Attempted::Refused),
        },
        None => None,
    };
    let Ok(output) = gst_util::temp_file(&folder, "prepared", extension) else {
        return Err(AttemptFailure::new("temporary", 0));
    };
    let built = build_transcode(
        &source,
        &output,
        video.as_ref(),
        audio.zip(encoder),
        mix.as_deref(),
    );
    let (pipeline, missing) = match built {
        Ok(built) => built,
        Err(_) => {
            gst_util::remove_file(&output);
            return Ok(Attempted::Refused);
        }
    };
    let stopper = Stopper(pipeline.clone());
    let ended = gst_util::run_to_end(&pipeline, &cancel, Duration::from_secs(24 * 60 * 60)).await;
    drop(stopper);
    match ended {
        Ok(gst_util::Ended::Eos) if !missing.load(Ordering::SeqCst) => Ok(Attempted::Taken(output)),
        Ok(gst_util::Ended::Eos) => {
            // The source no longer has what was read of it (fewer sound streams than the mix names).
            gst_util::remove_file(&output);
            Ok(Attempted::Wrong)
        }
        Ok(_) => {
            gst_util::remove_file(&output);
            Ok(Attempted::Refused)
        }
        Err(error) => {
            gst_util::remove_file(&output);
            crate::fc_info!("preparing: an attempt failed ({error})");
            Err(error.attempt_failure())
        }
    }
}

/// The transcode's pipeline: `filesrc ! decodebin`, the video branch and the audio branch (one
/// stream, or a mix through `audiomixer`) into `mp4mux faststart=true ! filesink`. Unused streams go
/// to `fakesink`s. Answers the pipeline and a flag raised when a stream the ask names never came.
fn build_transcode(
    source: &Path,
    output: &Path,
    video: Option<&media_encoding::VideoEncoding>,
    audio: Option<(media_encoding::AacEncoding, AacEncoder)>,
    mix: Option<&[usize]>,
) -> Result<(gst::Pipeline, Arc<AtomicBool>), PipelineError> {
    let make = |factory: &str| {
        gst::ElementFactory::make(factory)
            .build()
            .map_err(|_| PipelineError::new("element", 0))
    };
    let pipeline = gst::Pipeline::new();
    let filesrc = make("filesrc")?;
    filesrc.set_property("location", source.to_string_lossy().as_ref());
    let decode = make("decodebin")?;
    let mux = gst::ElementFactory::make("mp4mux")
        .property("faststart", true)
        .build()
        .map_err(|_| PipelineError::new("element", 0))?;
    let sink = make("filesink")?;
    sink.set_property("location", output.to_string_lossy().as_ref());
    let link =
        |a: &gst::Element, b: &gst::Element| a.link(b).map_err(|_| PipelineError::new("link", 0));
    pipeline
        .add_many([&filesrc, &decode, &mux, &sink])
        .map_err(|_| PipelineError::new("pipeline", 0))?;
    link(&filesrc, &decode)?;
    link(&mux, &sink)?;
    let video_bin = match video {
        Some(asked) => {
            let bin = gst::parse::bin_from_description(&pipelines::transcode_video(asked), true)
                .map_err(|_| PipelineError::new("parse", 0))?;
            pipeline
                .add(&bin)
                .map_err(|_| PipelineError::new("pipeline", 0))?;
            link(bin.upcast_ref(), &mux)?;
            Some(bin)
        }
        None => None,
    };
    let (audio_bin, mixer) = match audio {
        Some((asked, encoder)) => {
            let bin = gst::parse::bin_from_description(
                &pipelines::transcode_audio(&asked, encoder),
                true,
            )
            .map_err(|_| PipelineError::new("parse", 0))?;
            pipeline
                .add(&bin)
                .map_err(|_| PipelineError::new("pipeline", 0))?;
            link(bin.upcast_ref(), &mux)?;
            let mixer = match mix {
                Some(_) => {
                    let mixer = make("audiomixer")?;
                    pipeline
                        .add(&mixer)
                        .map_err(|_| PipelineError::new("pipeline", 0))?;
                    link(&mixer, bin.upcast_ref())?;
                    Some(mixer)
                }
                None => None,
            };
            (Some(bin), mixer)
        }
        None => (None, None),
    };
    let missing = Arc::new(AtomicBool::new(false));
    let audio_seen = Arc::new(AtomicUsize::new(0));
    let video_linked = Arc::new(AtomicBool::new(false));
    let mixed = Arc::new(AtomicUsize::new(0));
    let wanted: Vec<usize> = mix.map(<[usize]>::to_vec).unwrap_or_default();
    let wanted_count = wanted.len();
    {
        let weak = pipeline.downgrade();
        let (video_bin, audio_bin, mixer) = (video_bin.clone(), audio_bin.clone(), mixer.clone());
        let (audio_seen, video_linked, mixed) = (
            Arc::clone(&audio_seen),
            Arc::clone(&video_linked),
            Arc::clone(&mixed),
        );
        decode.connect_pad_added(move |_, pad| {
            let Some(pipeline) = weak.upgrade() else {
                return;
            };
            let kind = pad
                .current_caps()
                .or_else(|| Some(pad.query_caps(None)))
                .and_then(|caps| caps.structure(0).map(|s| s.name().to_string()))
                .unwrap_or_default();
            let target: Option<gst::Pad> = if kind.starts_with("video/") {
                match &video_bin {
                    Some(bin) if !video_linked.swap(true, Ordering::SeqCst) => {
                        bin.static_pad("sink")
                    }
                    _ => None,
                }
            } else if kind.starts_with("audio/") {
                let index = audio_seen.fetch_add(1, Ordering::SeqCst);
                match (&mixer, &audio_bin) {
                    (Some(mixer), _) if wanted.contains(&index) => {
                        mixed.fetch_add(1, Ordering::SeqCst);
                        mixer.request_pad_simple("sink_%u")
                    }
                    (None, Some(bin)) if index == 0 => bin.static_pad("sink"),
                    _ => None,
                }
            } else {
                None
            };
            match target {
                Some(target) => {
                    if pad.link(&target).is_err() {
                        crate::fc_info!("preparing: a stream would not link");
                    }
                }
                None => {
                    // A stream nobody asked for goes nowhere, quietly.
                    if let Ok(fake) = gst::ElementFactory::make("fakesink")
                        .property("sync", false)
                        .property("async", false)
                        .build()
                        && pipeline.add(&fake).is_ok()
                    {
                        let _ = fake.sync_state_with_parent();
                        if let Some(target) = fake.static_pad("sink") {
                            let _ = pad.link(&target);
                        }
                    }
                }
            }
        });
    }
    {
        let missing = Arc::clone(&missing);
        let (video_bin, audio_bin) = (video_bin, audio_bin);
        decode.connect_no_more_pads(move |_| {
            // A branch nothing feeds would hold the muxer for ever: it ends now, and the attempt
            // is judged by what came.
            let unfed = |bin: &Option<gst::Bin>| {
                bin.as_ref()
                    .and_then(|bin| bin.static_pad("sink"))
                    .filter(|pad| !pad.is_linked())
            };
            if let Some(pad) = unfed(&video_bin) {
                missing.store(true, Ordering::SeqCst);
                pad.send_event(gst::event::Eos::new());
            }
            if mixer.is_none()
                && let Some(pad) = unfed(&audio_bin)
            {
                missing.store(true, Ordering::SeqCst);
                pad.send_event(gst::event::Eos::new());
            }
            if mixer.is_some() && mixed.load(Ordering::SeqCst) < wanted_count {
                missing.store(true, Ordering::SeqCst);
            }
        });
    }
    Ok((pipeline, missing))
}

/// What became of a picked file.
#[derive(Debug, Clone, PartialEq)]
pub enum Picked {
    /// Ready to stage.
    Staged(Box<StagedMedia>),
    /// Refused (too large, unreadable) — the composer says which.
    Refused(PrepFailure),
    /// Not a video, a sound or an animated picture: the composer's own path (a photo, a file).
    NotMine,
    /// An animated picture that could not be made into the animation: sent exactly as before #96
    /// (a GIF or WebP as a file, an APNG down the photo route) — the composer's own path.
    AsBefore,
}

/// A picked file made ready to stage — the preparer P1's composer hands to
/// `ComposerStaging::ingest`. `mime` is what the desktop says the file is; `name` its name (never
/// logged). `Err` only for the person's cancel.
pub async fn prepare_picked(
    file: &Path,
    mime: &str,
    name: Option<&str>,
    cancel: &Cancel,
) -> Result<Picked, OperationCanceled> {
    prepare_picked_told(file, mime, name, cancel, &mut |line| {
        crate::fc_info!("preparing: {line}");
    })
    .await
}

/// [`prepare_picked`], telling `log` what happened (sizes, counts, rates, codecs — never a name).
pub async fn prepare_picked_told(
    file: &Path,
    mime: &str,
    name: Option<&str>,
    cancel: &Cancel,
    log: &mut dyn FnMut(&str),
) -> Result<Picked, OperationCanceled> {
    let shell = Shell::get();
    let preparing = GstMediaPreparing::new(shell.paths.cache_dir());
    let clock = shell.clock.clone();
    let Some(size) = std::fs::metadata(file).ok().map(|meta| meta.len()) else {
        return Ok(Picked::Refused(PrepFailure::Unreadable));
    };
    let head = read_head(file);
    let mut log = |line: &str| log(line);
    if mime.starts_with("image/") {
        if !super::animation::is_animated(&head, file) {
            return Ok(Picked::NotMine);
        }
        let Ok(bytes) = std::fs::read(file) else {
            return Ok(Picked::Refused(PrepFailure::Unreadable));
        };
        return Ok(
            match super::animation::prepare(
                Rc::from(bytes.as_slice()),
                clock.as_ref(),
                &mut log,
                cancel,
            )
            .await?
            {
                Some(staged) => Picked::Staged(Box::new(staged)),
                None => Picked::AsBefore,
            },
        );
    }
    let duration = duration_ms(file).await;
    if cancel.is_cancelled() {
        return Err(OperationCanceled);
    }
    if mime.starts_with("video/") {
        let container = if mime == "video/quicktime" {
            "video/quicktime"
        } else {
            "video/mp4"
        };
        let job = VideoJob {
            file,
            container,
            size,
            head: &head,
            duration_ms: duration,
        };
        let prepared =
            logic::prepare_video(&job, &preparing, clock.as_ref(), &mut log, cancel).await?;
        return Ok(staged_video(prepared, file, container, name, duration).await);
    }
    let audio_type = media_encoding::picked_audio_type(mime, name.unwrap_or_default());
    let Some(container) = audio_type else {
        return Ok(Picked::NotMine);
    };
    let job = SoundJob {
        file,
        container: &container,
        as_audio: mime.starts_with("audio/"),
        size,
        head: &head,
        duration_ms: duration,
    };
    let prepared = logic::prepare_sound(&job, &preparing, clock.as_ref(), &mut log, cancel).await?;
    Ok(match prepared {
        Prepared::Original => match std::fs::read(file) {
            Ok(bytes) => Picked::Staged(Box::new(StagedMedia {
                duration_ms: duration.and_then(|ms| i32::try_from(ms).ok()),
                name: name.and_then(media::sanitized_name),
                ..StagedMedia::new("audio", &container, bytes)
            })),
            Err(_) => Picked::Refused(PrepFailure::Unreadable),
        },
        Prepared::AsFile => match std::fs::read(file) {
            Ok(bytes) => Picked::Staged(Box::new(StagedMedia {
                name: name.and_then(media::sanitized_name),
                ..StagedMedia::new("file", mime, bytes)
            })),
            Err(_) => Picked::Refused(PrepFailure::Unreadable),
        },
        Prepared::TooLarge => Picked::Refused(PrepFailure::TooLarge),
        Prepared::Made(made) => Picked::Staged(Box::new(StagedMedia {
            duration_ms: duration.and_then(|ms| i32::try_from(ms).ok()),
            name: media_encoding::m4a_name(name),
            ..StagedMedia::new("audio", made.mime, made.bytes)
        })),
    })
}

/// A prepared video as the outbox stages it: its bytes, its displayed size, its length and its
/// poster (a frame of what is sent).
async fn staged_video(
    prepared: Prepared,
    file: &Path,
    container: &str,
    name: Option<&str>,
    duration: Option<i64>,
) -> Picked {
    let (bytes, mime, sides) = match prepared {
        Prepared::TooLarge => return Picked::Refused(PrepFailure::TooLarge),
        Prepared::Original | Prepared::AsFile => match std::fs::read(file) {
            Ok(bytes) => (bytes, container.to_string(), displayed_sides(file).await),
            Err(_) => return Picked::Refused(PrepFailure::Unreadable),
        },
        Prepared::Made(made) => {
            let sides = made.width.zip(made.height);
            (made.bytes, made.mime.to_string(), sides)
        }
    };
    let preview = poster(file, sides, media::PREVIEW_EDGE).await;
    Picked::Staged(Box::new(StagedMedia {
        width: sides.and_then(|(w, _)| i32::try_from(w).ok()),
        height: sides.and_then(|(_, h)| i32::try_from(h).ok()),
        duration_ms: duration.and_then(|ms| i32::try_from(ms).ok()),
        name: name.and_then(media::sanitized_name),
        preview,
        ..StagedMedia::new("video", &mime, bytes)
    }))
}

/// A video's displayed sides (a quarter turn swaps them).
async fn displayed_sides(file: &Path) -> Option<(u32, u32)> {
    let video = read(file).await?.video?;
    let turn = media_encoding::rotation(video.rotation, None);
    let (width, height) =
        media_encoding::displayed(i64::from(video.width), i64::from(video.height), turn);
    Some((u32::try_from(width).ok()?, u32::try_from(height).ok()?))
}

/// A file's first bytes, for the magic-number checks.
fn read_head(file: &Path) -> Vec<u8> {
    use std::io::Read;
    let mut head = vec![0u8; 64];
    let read = std::fs::File::open(file)
        .and_then(|mut opened| opened.read(&mut head))
        .unwrap_or(0);
    head.truncate(read);
    head
}
