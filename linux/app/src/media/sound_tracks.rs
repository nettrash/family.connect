//! A recording's sound, made on this device for "Show text" (docs/protocol.md, "Transcripts on
//! request"): fc-logic's [`SoundTrackMaker`] on GStreamer. `transcript_sound::sound_track` decides
//! which attempts and judges what came out; this module reads a held file's sound track and makes
//! ONE attempt of it — the AAC frames copied into an M4A (`parsebin`, nothing decoded), or the sound
//! decoded and encoded again as a voice note is (`decodebin`, its audio pad only). A picture pad is
//! never decoded into anything: it goes to a `fakesink`.
//!
//! The held bytes are written to a temporary in `$XDG_CACHE_HOME/family-connect/tmp/` and deleted,
//! with what was made, before the future answers. Nothing is logged but an error's domain and code:
//! the bytes are somebody's voice.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use fc_core::rt::{Cancel, LocalBoxFuture};
use fc_logic::transcode_attempts::AttemptFailure;
use fc_logic::transcript_sound::{
    MadeSound, SoundAttempt, SoundTrackMaker, SoundWay, TrackReading,
};
use gst::prelude::*;

use super::gst_util::{self, Ended, PipelineError};
use super::pipelines::{self, AacEncoder};
use super::preparing;

/// The longest one attempt may run: an hour of sound copies or encodes in far less.
const LIMIT: Duration = Duration::from_secs(10 * 60);

/// fc-logic's `SoundTrackMaker` on GStreamer, its temporaries under `folder`.
#[derive(Debug, Clone)]
pub struct GstSoundTracks {
    folder: PathBuf,
}

impl GstSoundTracks {
    pub fn new(folder: PathBuf) -> Self {
        GstSoundTracks { folder }
    }

    /// Whether this machine can make a recording's sound at all: read a container, and write an
    /// M4A of AAC.
    pub fn available() -> bool {
        ["parsebin", "decodebin", "aacparse", "mp4mux"]
            .iter()
            .all(|factory| gst_util::has_element(factory))
            && aac_encoder().is_some()
    }

    /// The held bytes as a temporary file of their own.
    fn held(&self, bytes: &[u8], extension: &str) -> Result<Held, AttemptFailure> {
        let path = gst_util::temp_file(&self.folder, "sound", extension)
            .map_err(|_| AttemptFailure::new("temporary", 0))?;
        let held = Held(path);
        std::fs::write(&held.0, bytes).map_err(|_| AttemptFailure::new("temporary", 1))?;
        Ok(held)
    }
}

/// A temporary, deleted however the attempt ends.
struct Held(PathBuf);

impl Drop for Held {
    fn drop(&mut self) {
        gst_util::remove_file(&self.0);
    }
}

/// A pipeline set to `Null` however the attempt ends — the cancel, a dropped future included.
struct Stopping(gst::Pipeline);

impl Drop for Stopping {
    fn drop(&mut self) {
        let _ = self.0.set_state(gst::State::Null);
    }
}

fn aac_encoder() -> Option<AacEncoder> {
    AacEncoder::ORDER
        .into_iter()
        .find(|encoder| gst_util::has_element(encoder.factory()))
}

fn failure(error: &PipelineError) -> AttemptFailure {
    AttemptFailure::new(error.domain.clone(), error.code.unsigned_abs())
}

/// An element, a description or a link GStreamer refused while building.
fn refused<E>(_: E) -> AttemptFailure {
    AttemptFailure::new("pipeline", 0)
}

impl SoundTrackMaker for GstSoundTracks {
    fn read<'a>(
        &'a self,
        bytes: &'a Rc<[u8]>,
        extension: &'a str,
    ) -> LocalBoxFuture<'a, Result<Option<TrackReading>, AttemptFailure>> {
        Box::pin(async move {
            let held = self.held(bytes, extension)?;
            let Some(reading) = preparing::read(&held.0).await else {
                return Err(AttemptFailure::new("unreadable", 0));
            };
            Ok(reading.audio.first().map(|track| TrackReading {
                codec: track.codec.clone(),
                bitrate: track.bitrate,
                sample_rate: track.sample_rate,
            }))
        })
    }

    fn make<'a>(
        &'a self,
        bytes: &'a Rc<[u8]>,
        extension: &'a str,
        attempt: &'a SoundAttempt,
        cancel: Cancel,
    ) -> LocalBoxFuture<'a, Result<MadeSound, AttemptFailure>> {
        Box::pin(async move {
            let held = self.held(bytes, extension)?;
            let made = Held(
                gst_util::temp_file(&self.folder, "sound", "m4a")
                    .map_err(|_| AttemptFailure::new("temporary", 2))?,
            );
            let pipeline = build(&held.0, &made.0, attempt)?;
            let stopping = Stopping(pipeline.clone());
            let ended = gst_util::run_to_end(&pipeline, &cancel, LIMIT).await;
            drop(stopping);
            match ended {
                Ok(Ended::Eos) => {}
                Ok(Ended::Cancelled) => return Err(AttemptFailure::new("OperationCanceled", 0)),
                Ok(Ended::TimedOut) => return Err(AttemptFailure::new("timeout", 0)),
                Err(error) => return Err(failure(&error)),
            }
            let bytes = std::fs::read(&made.0).map_err(|_| AttemptFailure::new("read", 0))?;
            let reading = preparing::read(&made.0).await;
            let track = reading.as_ref().and_then(|reading| reading.audio.first());
            Ok(MadeSound {
                bytes,
                codec: track.map(|track| track.codec.clone()),
                bitrate: track.and_then(|track| track.bitrate),
                has_video: reading
                    .as_ref()
                    .is_some_and(|reading| reading.video.is_some()),
            })
        })
    }
}

/// `filesrc ! parsebin` (the AAC frames copied) or `filesrc ! decodebin` (the sound encoded again),
/// the sound pad into the M4A's tail and every other pad into a `fakesink`, linked as they appear.
fn build(
    source: &Path,
    output: &Path,
    attempt: &SoundAttempt,
) -> Result<gst::Pipeline, AttemptFailure> {
    let (front, tail) = match (attempt.way, attempt.aac) {
        (SoundWay::Passthrough, _) => (
            "parsebin",
            "aacparse ! mp4mux faststart=true ! filesink name=out".to_string(),
        ),
        (SoundWay::Reencode, Some(aac)) => {
            let encoder = aac_encoder().ok_or_else(|| AttemptFailure::new("encoder", 0))?;
            (
                "decodebin",
                format!(
                    "audioconvert ! audioresample ! {raw} ! {encode} ! aacparse \
                     ! mp4mux faststart=true ! filesink name=out",
                    raw = pipelines::raw_audio(&aac),
                    encode = pipelines::aac(encoder, aac.bitrate),
                ),
            )
        }
        (SoundWay::Reencode, None) => return Err(AttemptFailure::new("attempt", 0)),
    };
    let pipeline = gst::Pipeline::new();
    let file = gst::ElementFactory::make("filesrc")
        .property("location", source.to_string_lossy().as_ref())
        .build()
        .map_err(refused)?;
    let demux = gst::ElementFactory::make(front).build().map_err(refused)?;
    let tail = gst::parse::bin_from_description(&tail, true).map_err(refused)?;
    pipeline
        .add_many([&file, &demux, tail.upcast_ref()])
        .map_err(refused)?;
    file.link(&demux).map_err(refused)?;
    if !gst_util::set_location(&tail, "out", output) {
        return Err(AttemptFailure::new("pipeline", 1));
    }
    let passthrough = attempt.way == SoundWay::Passthrough;
    let sink_pad = tail.static_pad("sink");
    let weak = pipeline.downgrade();
    demux.connect_pad_added(move |_, pad| {
        let Some(pipeline) = weak.upgrade() else {
            return;
        };
        let caps = pad.current_caps().or_else(|| Some(pad.query_caps(None)));
        let wanted = caps
            .as_ref()
            .and_then(|caps| caps.structure(0))
            .is_some_and(|structure| {
                if passthrough {
                    preparing::audio_codec(structure) == "aac"
                } else {
                    structure.name().as_str() == "audio/x-raw"
                }
            });
        if wanted
            && let Some(sink) = sink_pad.as_ref()
            && !sink.is_linked()
            && pad.link(sink).is_ok()
        {
            return;
        }
        // A picture, a second sound, subtitles: drained, never decoded into anything.
        if let Ok(drain) = gst::ElementFactory::make("fakesink")
            .property("sync", false)
            .property("async", false)
            .build()
            && pipeline.add(&drain).is_ok()
        {
            let _ = drain.sync_state_with_parent();
            if let Some(sink) = drain.static_pad("sink") {
                let _ = pad.link(&sink);
            }
        }
    });
    Ok(pipeline)
}
