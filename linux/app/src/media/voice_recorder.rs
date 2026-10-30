//! The microphone (shell-design §10.2; fc-logic's `voice_notes::VoiceRecorder`/`VoiceRecording`,
//! `voice_look::VoiceMeter` and `VoiceDecoder` — each method's contract is its rustdoc there).
//!
//! **One pipeline, one microphone**: `pulsesrc` (PipeWire's Pulse server's default source; the
//! self-test's tone in a `selftest` build) into a `tee` — the AAC file (`mp4mux faststart=true`, a
//! `0600` temporary under `$XDG_CACHE_HOME/family-connect/tmp/`) and the `level` meter, whose
//! peaks the row draws and forgets.
//!
//! **However it ends the microphone is let go of**: stopped (EOS, the muxer's `moov` written, then
//! `Null`), abandoned, dropped. A bus error while it records is raised once on
//! [`VoiceRecording::failed`] — the microphone pulled out, the disk full. Logs carry the element's
//! factory and the error's domain and code — never its message, which can name the device.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fc_core::rt::{Cancel, Event, LocalBoxFuture};
use fc_logic::media_encoding::{self, AacEncoding};
use fc_logic::voice_look::{Pcm, VoiceDecoder, VoiceMeter};
use fc_logic::voice_notes::{
    CapabilityAccess, Recorded, RecordingFailure, VoiceRecorder, VoiceRecording,
};
use futures_util::StreamExt;
use futures_util::future::{Either, select};
use gst::prelude::*;
use gst_pbutils::prelude::*;

use super::gst_util::{self, PipelineError};
use super::pipelines::{self, AacEncoder};

/// How long a stop waits for the muxer to finish the file.
const STOP_LIMIT: Duration = Duration::from_secs(10);

/// fc-logic's `VoiceRecorder` on GStreamer.
#[derive(Debug)]
pub struct GstVoiceRecorder {
    tmp: PathBuf,
    /// The recording the last start made, concretely — what quitting stops synchronously
    /// (`GstVoiceRecording::stop_now`); taken by the composer that started it.
    last: Rc<RefCell<Option<Rc<GstVoiceRecording>>>>,
}

impl GstVoiceRecorder {
    /// Temporaries go under `tmp` (`Paths::tmp_dir`, swept at launch).
    pub fn new(tmp: PathBuf) -> Self {
        GstVoiceRecorder {
            tmp,
            last: Rc::new(RefCell::new(None)),
        }
    }

    /// The recording the last start made, once.
    pub fn take_last(&self) -> Option<Rc<GstVoiceRecording>> {
        self.last.borrow_mut().take()
    }
}

impl VoiceRecorder for GstVoiceRecorder {
    fn start(
        &self,
        encodings: Vec<AacEncoding>,
        speak_first: Option<LocalBoxFuture<'static, ()>>,
    ) -> LocalBoxFuture<'static, Result<Rc<dyn VoiceRecording>, RecordingFailure>> {
        let tmp = self.tmp.clone();
        let last = Rc::clone(&self.last);
        last.borrow_mut().take();
        Box::pin(async move {
            let mut speak_first = speak_first;
            for asked in encodings {
                for encoder in AacEncoder::ORDER {
                    if !gst_util::has_element(encoder.factory()) {
                        continue;
                    }
                    let opened = match Opening::open(&tmp, &asked, encoder) {
                        Ok(opened) => opened,
                        Err(error) => {
                            crate::fc_warn!(
                                "voice: {} would not open ({error})",
                                encoder.factory()
                            );
                            continue;
                        }
                    };
                    if let Some(speak) = speak_first.take() {
                        // The microphone is open (PAUSED); "Recording" is said and a second passes
                        // before anything is recorded (S6).
                        speak.await;
                    }
                    return match opened.play(asked) {
                        Ok(recording) => {
                            *last.borrow_mut() = Some(Rc::clone(&recording));
                            Ok(recording as Rc<dyn VoiceRecording>)
                        }
                        Err(error) => {
                            crate::fc_warn!("voice: the recording would not start ({error})");
                            Err(RecordingFailure::CouldNotStart)
                        }
                    };
                }
            }
            crate::fc_warn!("voice: no AAC encoder could record");
            Err(RecordingFailure::CouldNotStart)
        })
    }

    fn microphone_access(&self) -> CapabilityAccess {
        // A host app is never asked: PipeWire gives the default source (§10.2).
        CapabilityAccess::Allowed
    }
}

/// A pipeline opened to PAUSED, not yet recording. Dropped before [`play`](Opening::play) — the
/// start future dropped during the lead-in — it lets go of the microphone and the file.
struct Opening {
    pipeline: Option<gst::Pipeline>,
    file: PathBuf,
}

impl Opening {
    fn open(
        tmp: &Path,
        asked: &AacEncoding,
        encoder: AacEncoder,
    ) -> Result<Opening, PipelineError> {
        let file = gst_util::temp_file(tmp, "voice", "m4a")
            .map_err(|_| PipelineError::new("temporary", 0))?;
        let mut opening = Opening {
            pipeline: None,
            file,
        };
        let pipeline = gst_util::launch(&pipelines::voice_recording(
            super::microphone(),
            asked,
            encoder,
        ))?;
        gst_util::set_location(&pipeline, "out", &opening.file);
        opening.pipeline = Some(pipeline.clone());
        if pipeline.set_state(gst::State::Paused).is_err() {
            let error = pipeline
                .bus()
                .and_then(|bus| gst_util::first_error(&bus))
                .unwrap_or_else(|| PipelineError::new("state-change", 0));
            return Err(error);
        }
        Ok(opening)
    }

    fn play(mut self, asked: AacEncoding) -> Result<Rc<GstVoiceRecording>, PipelineError> {
        let pipeline = self
            .pipeline
            .take()
            .ok_or(PipelineError::new("pipeline", 0))?;
        let file = std::mem::take(&mut self.file);
        let recording = Rc::new(GstVoiceRecording {
            pipeline: pipeline.clone(),
            file,
            asked,
            started: Cell::new(Instant::now()),
            frozen: Cell::new(None),
            peak: Cell::new(None),
            failed: Event::new(),
            failed_raised: Cell::new(false),
            phase: Cell::new(Phase::Running),
            watch: RefCell::new(None),
        });
        if let Some(bus) = pipeline.bus() {
            let weak = Rc::downgrade(&recording);
            if let Ok(guard) = bus.add_watch_local(move |_, message| match weak.upgrade() {
                Some(recording) => {
                    recording.hear(message);
                    glib::ControlFlow::Continue
                }
                None => glib::ControlFlow::Break,
            }) {
                *recording.watch.borrow_mut() = Some(guard);
            }
        }
        if pipeline.set_state(gst::State::Playing).is_err() {
            let error = pipeline
                .bus()
                .and_then(|bus| gst_util::first_error(&bus))
                .unwrap_or_else(|| PipelineError::new("state-change", 0));
            recording.abandon();
            return Err(error);
        }
        // The note's monotonic clock starts here (S2.9).
        recording.started.set(Instant::now());
        Ok(recording)
    }
}

impl Drop for Opening {
    fn drop(&mut self) {
        if let Some(pipeline) = self.pipeline.take() {
            let _ = pipeline.set_state(gst::State::Null);
        }
        if !self.file.as_os_str().is_empty() {
            gst_util::remove_file(&self.file);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    Stopping,
    Done,
}

/// A voice note being recorded (fc-logic's `VoiceRecording` and `VoiceMeter`).
pub struct GstVoiceRecording {
    pipeline: gst::Pipeline,
    file: PathBuf,
    asked: AacEncoding,
    started: Cell<Instant>,
    frozen: Cell<Option<Duration>>,
    peak: Cell<Option<f64>>,
    failed: Event<()>,
    failed_raised: Cell<bool>,
    phase: Cell<Phase>,
    watch: RefCell<Option<gst::bus::BusWatchGuard>>,
}

impl std::fmt::Debug for GstVoiceRecording {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GstVoiceRecording")
            .field("phase", &self.phase.get())
            .finish_non_exhaustive()
    }
}

impl GstVoiceRecording {
    /// QUITTING (§3.8): stop and hand over SYNCHRONOUSLY — EOS sent, the bus itself read until the
    /// muxer's EOS (or `limit`), `Null`, the file read and deleted. Blocks the main thread for at
    /// most `limit`; the streaming threads finish the file meanwhile.
    pub fn stop_now(&self, limit: Duration) -> Recorded {
        let elapsed = self.elapsed();
        self.frozen.set(Some(elapsed));
        if self.phase.replace(Phase::Done) != Phase::Running {
            return Recorded {
                bytes: None,
                elapsed,
            };
        }
        self.watch.borrow_mut().take();
        let finished = !self.failed_raised.get()
            && self.pipeline.bus().is_some_and(|bus| {
                self.pipeline.send_event(gst::event::Eos::new());
                let timeout = gst::ClockTime::from_mseconds(
                    u64::try_from(limit.as_millis()).unwrap_or(u64::MAX),
                );
                matches!(
                    bus.timed_pop_filtered(
                        timeout,
                        &[gst::MessageType::Eos, gst::MessageType::Error]
                    )
                    .as_ref()
                    .map(|message| message.type_()),
                    Some(gst::MessageType::Eos)
                )
            });
        let _ = self.pipeline.set_state(gst::State::Null);
        let bytes = if finished {
            std::fs::read(&self.file)
                .ok()
                .filter(|bytes| !bytes.is_empty())
        } else {
            None
        };
        gst_util::remove_file(&self.file);
        Recorded { bytes, elapsed }
    }

    fn hear(&self, message: &gst::Message) {
        match message.view() {
            gst::MessageView::Element(element) => {
                let Some(structure) = element.structure() else {
                    return;
                };
                if structure.name() != "level" {
                    return;
                }
                if let Some(peak) = loudest(structure) {
                    let kept = self.peak.get().map_or(peak, |held| held.max(peak));
                    self.peak.set(Some(kept));
                }
            }
            gst::MessageView::Error(error) => {
                let error = PipelineError::from_message(error);
                crate::fc_warn!("voice: the recording failed ({error})");
                if self.phase.get() == Phase::Running && !self.failed_raised.replace(true) {
                    self.frozen.set(Some(self.started.get().elapsed()));
                    self.failed.emit(&());
                }
            }
            _ => {}
        }
    }
}

/// The loudest channel's `peak` of a `level` message, in dBFS.
fn loudest(structure: &gst::StructureRef) -> Option<f64> {
    #[allow(
        deprecated,
        reason = "the level element posts a GValueArray; there is no other way to read it"
    )]
    let peaks = structure.get::<glib::ValueArray>("peak").ok()?;
    peaks
        .iter()
        .filter_map(|value| value.get::<f64>().ok())
        .reduce(f64::max)
}

impl VoiceMeter for GstVoiceRecording {
    fn take_peak_dbfs(&self) -> Option<f64> {
        self.peak.take()
    }
}

impl VoiceRecording for GstVoiceRecording {
    fn elapsed(&self) -> Duration {
        self.frozen
            .get()
            .unwrap_or_else(|| self.started.get().elapsed())
    }

    fn failed(&self) -> &Event<()> {
        &self.failed
    }

    fn stop(&self) -> LocalBoxFuture<'static, Recorded> {
        let elapsed = self.elapsed();
        self.frozen.set(Some(elapsed));
        if self.phase.get() != Phase::Running {
            return Box::pin(std::future::ready(Recorded {
                bytes: None,
                elapsed,
            }));
        }
        self.phase.set(Phase::Stopping);
        // The stop no longer needs this object: the watch goes, and the bus is read as a stream by
        // the future, which owns the pipeline and the file from here.
        self.watch.borrow_mut().take();
        let pipeline = self.pipeline.clone();
        let file = self.file.clone();
        let asked = self.asked;
        let failed = self.failed_raised.get();
        let messages = pipeline.bus().map(|bus| bus.stream());
        pipeline.send_event(gst::event::Eos::new());
        self.phase.set(Phase::Done);
        Box::pin(async move {
            let finished = match messages {
                Some(mut messages) if !failed => {
                    let waiting = async {
                        while let Some(message) = messages.next().await {
                            match message.view() {
                                gst::MessageView::Eos(_) => return true,
                                gst::MessageView::Error(error) => {
                                    crate::fc_warn!(
                                        "voice: the file could not be finished ({})",
                                        PipelineError::from_message(error)
                                    );
                                    return false;
                                }
                                _ => {}
                            }
                        }
                        false
                    };
                    match select(
                        Box::pin(waiting),
                        Box::pin(glib::timeout_future(STOP_LIMIT)),
                    )
                    .await
                    {
                        Either::Left((finished, _)) => finished,
                        Either::Right(_) => false,
                    }
                }
                _ => false,
            };
            let _ = pipeline.set_state(gst::State::Null);
            let bytes = if finished {
                std::fs::read(&file).ok().filter(|bytes| !bytes.is_empty())
            } else {
                None
            };
            if bytes.is_some() {
                check_back(&file, &asked).await;
            }
            gst_util::remove_file(&file);
            Recorded { bytes, elapsed }
        })
    }

    fn abandon(&self) {
        if self.phase.replace(Phase::Done) == Phase::Done {
            return;
        }
        self.frozen.set(Some(self.elapsed()));
        self.watch.borrow_mut().take();
        let _ = self.pipeline.set_state(gst::State::Null);
        gst_util::remove_file(&self.file);
    }
}

impl Drop for GstVoiceRecording {
    fn drop(&mut self) {
        // Dropped without a stop: abandoned. (A stop under way owns the pipeline and the file.)
        if self.phase.get() == Phase::Running {
            self.abandon();
        }
    }
}

/// The recording read back with the discoverer: a mismatch with what was asked is only LOGGED — a
/// recording cannot be sent as an original instead (the contract).
async fn check_back(file: &Path, asked: &AacEncoding) {
    let Some(info) = gst_util::discover(file, Duration::from_secs(10)).await else {
        crate::fc_warn!("voice: the recording could not be read back");
        return;
    };
    let audio = info.audio_streams();
    let Some(first) = audio.first() else {
        crate::fc_warn!("voice: the recording read back with no sound");
        return;
    };
    let codec = first
        .caps()
        .and_then(|caps| caps.structure(0).map(super::preparing::audio_codec))
        .unwrap_or_else(|| "unknown".to_string());
    let channels = i64::from(first.channels());
    let bitrate = i64::from(first.bitrate());
    if !media_encoding::aac_came_out(asked, &codec, Some(channels))
        || !media_encoding::audio_rate_came_out(asked.bitrate, Some(bitrate))
    {
        crate::fc_warn!(
            "voice: the recording came out {codec}, {channels} channel(s), {bitrate} bit/s; asked {asked}"
        );
    }
}

/// fc-logic's `VoiceDecoder`: the note decoded whole to mono 16 kHz PCM for its waveform.
#[derive(Debug, Clone)]
pub struct GstVoiceDecoder {
    tmp: PathBuf,
}

impl GstVoiceDecoder {
    pub fn new(tmp: PathBuf) -> Self {
        GstVoiceDecoder { tmp }
    }
}

impl VoiceDecoder for GstVoiceDecoder {
    fn decode(&self, m4a: Rc<[u8]>) -> LocalBoxFuture<'static, Option<Pcm>> {
        let tmp = self.tmp.clone();
        Box::pin(async move {
            let file = gst_util::temp_file(&tmp, "shape", "m4a").ok()?;
            let guard = TempGuard(file.clone());
            std::fs::write(&file, &m4a).ok()?;
            let pipeline = gst_util::launch(pipelines::WAVEFORM_DECODE).ok()?;
            let stopper = Stopper(pipeline.clone());
            gst_util::set_location(&pipeline, "src", &file);
            let sink = pipeline
                .by_name("pcm")?
                .downcast::<gst_app::AppSink>()
                .ok()?;
            let samples: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::new()));
            {
                let samples = Arc::clone(&samples);
                sink.set_callbacks(
                    gst_app::AppSinkCallbacks::builder()
                        .new_sample(move |sink| {
                            let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                            if let Some(buffer) = sample.buffer()
                                && let Ok(map) = buffer.map_readable()
                                && let Ok(mut held) = samples.lock()
                            {
                                held.extend(
                                    map.as_slice()
                                        .chunks_exact(2)
                                        .map(|pair| i16::from_le_bytes([pair[0], pair[1]])),
                                );
                            }
                            Ok(gst::FlowSuccess::Ok)
                        })
                        .build(),
                );
            }
            let ended =
                gst_util::run_to_end(&pipeline, &Cancel::new(), Duration::from_secs(15)).await;
            drop(stopper);
            drop(guard);
            match ended {
                Ok(gst_util::Ended::Eos) => {}
                Ok(_) => return None,
                Err(error) => {
                    crate::fc_warn!("voice: the waveform could not be measured ({error})");
                    return None;
                }
            }
            let samples = samples.lock().ok()?.clone();
            (!samples.is_empty()).then_some(Pcm {
                rate: 16_000,
                channels: 1,
                samples,
            })
        })
    }
}

/// Sets a one-shot pipeline to `Null` however its future ends (dropped included).
pub struct Stopper(pub gst::Pipeline);

impl Drop for Stopper {
    fn drop(&mut self) {
        let _ = self.0.set_state(gst::State::Null);
    }
}

/// Deletes a temporary however its future ends.
pub struct TempGuard(pub PathBuf);

impl Drop for TempGuard {
    fn drop(&mut self) {
        gst_util::remove_file(&self.0);
    }
}
