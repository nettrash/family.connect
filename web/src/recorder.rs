//! A voice note, recorded in the browser (docs/protocol.md, "A browser is a
//! client too"): into MP4 (AAC) where the browser's recorder can write it,
//! and otherwise as raw samples made into a WAV (fc_text::wav) — never the
//! WebM most recorders default to, which the server refuses.
//!
//! Five minutes at most; the conversation stops it there. What comes out is
//! staged, like the Mac's, so a caption can be added and a recording made
//! by accident can still be thrown away.

use std::cell::RefCell;
use std::rc::Rc;

use fc_text::{media, wav};
use futures::channel::oneshot;
use futures::future::select;
use gloo_timers::future::TimeoutFuture;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioContext, AudioProcessingEvent, Blob, BlobEvent, BlobPropertyBag, MediaRecorder,
    MediaRecorderOptions, MediaStream, MediaStreamConstraints, ScriptProcessorNode,
};

/// Why recording did not start, with what to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    MicrophoneDenied,
    CouldNotStart,
}

impl Failure {
    pub fn message(self) -> &'static str {
        match self {
            Failure::MicrophoneDenied => {
                "Family needs permission to use your microphone. Allow it in your browser's settings for this site."
            }
            Failure::CouldNotStart => "Couldn't start recording.",
        }
    }
}

/// The MP4 types this browser's recorder may write, most wanted first. The
/// plain one is a second choice only: Chrome without an AAC encoder takes
/// it and writes OPUS into the MP4, which passes the server's container
/// check and plays on no phone — so what it actually chose is read back
/// after it starts, and Opus goes to the WAV path instead.
const MP4_TYPES: [&str; 2] = ["audio/mp4;codecs=mp4a.40.2", "audio/mp4"];

/// What a finished recording is.
pub struct Recorded {
    pub blob: Blob,
    /// What the upload declares — `audio/mp4` or `audio/wav`.
    pub mime: &'static str,
    pub duration_ms: i64,
}

enum Engine {
    /// The browser's recorder, writing MP4.
    Mp4 {
        recorder: MediaRecorder,
        chunks: Rc<RefCell<Vec<Blob>>>,
        waiting: oneshot::Receiver<()>,
        _on_data: Closure<dyn FnMut(BlobEvent)>,
        _on_stop: Closure<dyn FnMut()>,
    },
    /// Raw samples, off the microphone, for a WAV.
    Pcm {
        context: AudioContext,
        processor: ScriptProcessorNode,
        samples: Rc<RefCell<Vec<f32>>>,
        _on_audio: Closure<dyn FnMut(AudioProcessingEvent)>,
    },
}

impl Engine {
    /// Stop without keeping anything. The handlers go FIRST: an event
    /// arriving after its closure was dropped throws, every time, for as
    /// long as the thing keeps running.
    fn abandon(self) {
        match self {
            Engine::Mp4 { recorder, .. } => {
                recorder.set_ondataavailable(None);
                recorder.set_onstop(None);
                let _ = recorder.stop();
            }
            Engine::Pcm {
                context, processor, ..
            } => {
                processor.set_onaudioprocess(None);
                let _ = processor.disconnect();
                let _ = context.close();
            }
        }
    }
}

/// A recording in progress. However it ends — stopped, cancelled, or
/// simply dropped because whatever held it went away — the microphone is
/// let go of: a live microphone nothing can reach is a microphone left on.
pub struct Recording {
    stream: MediaStream,
    engine: Option<Engine>,
    started: f64,
}

impl Drop for Recording {
    fn drop(&mut self) {
        if let Some(engine) = self.engine.take() {
            engine.abandon();
        }
        stop_tracks(&self.stream);
    }
}

impl Recording {
    /// Ask for the microphone and start. The browser asks the person the
    /// first time; a refusal is said out loud, not left as a dead button.
    pub async fn start() -> Result<Recording, Failure> {
        let navigator = web_sys::window().ok_or(Failure::CouldNotStart)?.navigator();
        // Absent outside a secure context — an http:// server on a LAN.
        let devices = navigator
            .media_devices()
            .map_err(|_| Failure::CouldNotStart)?;
        let constraints = MediaStreamConstraints::new();
        constraints.set_audio(&JsValue::TRUE);
        let promise = devices
            .get_user_media_with_constraints(&constraints)
            .map_err(|_| Failure::CouldNotStart)?;
        let stream: MediaStream = match JsFuture::from(promise).await {
            Ok(stream) => stream.unchecked_into(),
            Err(error) => {
                let name = js_sys::Reflect::get(&error, &JsValue::from_str("name"))
                    .ok()
                    .and_then(|name| name.as_string())
                    .unwrap_or_default();
                return Err(if name == "NotAllowedError" || name == "SecurityError" {
                    Failure::MicrophoneDenied
                } else {
                    Failure::CouldNotStart
                });
            }
        };
        let engine = MP4_TYPES
            .into_iter()
            .filter(|mime| MediaRecorder::is_type_supported(mime))
            .find_map(|mime| mp4(&stream, mime))
            .or_else(|| pcm(&stream));
        match engine {
            Some(engine) => Ok(Recording {
                stream,
                engine: Some(engine),
                started: js_sys::Date::now(),
            }),
            None => {
                stop_tracks(&stream);
                Err(Failure::CouldNotStart)
            }
        }
    }

    /// How long it has been going, in milliseconds.
    pub fn elapsed_ms(&self) -> f64 {
        js_sys::Date::now() - self.started
    }

    /// Stop and hand over what was recorded — None when it is too short to
    /// be anything (ios AudioRecorder: 1024 bytes or less).
    pub async fn stop(mut self) -> Option<Recorded> {
        let duration_ms = self.elapsed_ms().round() as i64;
        let recorded = match self.engine.take()? {
            Engine::Mp4 {
                recorder,
                chunks,
                waiting,
                _on_data,
                _on_stop,
            } => {
                let _ = recorder.stop();
                // The last chunk comes before `stop`; a recorder that never
                // says stop is not waited on for ever.
                let _ = select(waiting, TimeoutFuture::new(5_000)).await;
                recorder.set_ondataavailable(None);
                recorder.set_onstop(None);
                let parts = js_sys::Array::new();
                for chunk in chunks.borrow().iter() {
                    parts.push(chunk);
                }
                let options = BlobPropertyBag::new();
                options.set_type("audio/mp4");
                let blob = Blob::new_with_blob_sequence_and_options(&parts, &options).ok()?;
                Recorded {
                    blob,
                    mime: "audio/mp4",
                    duration_ms,
                }
            }
            Engine::Pcm {
                context,
                processor,
                samples,
                _on_audio,
            } => {
                processor.set_onaudioprocess(None);
                let _ = processor.disconnect();
                let rate = context.sample_rate() as u32;
                let _ = context.close();
                let voice = wav::resample(&samples.borrow(), rate, wav::VOICE_RATE);
                let bytes = wav::encode(&voice, wav::VOICE_RATE);
                let array = js_sys::Uint8Array::from(bytes.as_slice());
                let parts = js_sys::Array::of1(&array);
                let options = BlobPropertyBag::new();
                options.set_type("audio/wav");
                let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &options).ok()?;
                Recorded {
                    blob,
                    mime: "audio/wav",
                    duration_ms: wav::duration_ms(voice.len(), wav::VOICE_RATE),
                }
            }
        };
        // The microphone goes with `self`, below.
        (recorded.blob.size() as u64 > media::VOICE_MIN_BYTES).then_some(recorded)
    }

    /// Give it up: nothing is kept, and the microphone is let go of.
    pub fn cancel(self) {
        drop(self);
    }
}

fn stop_tracks(stream: &MediaStream) {
    for track in stream.get_tracks().iter() {
        if let Ok(track) = track.dyn_into::<web_sys::MediaStreamTrack>() {
            track.stop();
        }
    }
}

fn mp4(stream: &MediaStream, mime: &str) -> Option<Engine> {
    let options = MediaRecorderOptions::new();
    options.set_mime_type(mime);
    let recorder =
        MediaRecorder::new_with_media_stream_and_media_recorder_options(stream, &options).ok()?;
    let chunks = Rc::new(RefCell::new(Vec::new()));
    let (sender, waiting) = oneshot::channel();
    let stopped = Rc::new(RefCell::new(Some(sender)));
    let on_data = {
        let chunks = chunks.clone();
        Closure::<dyn FnMut(BlobEvent)>::new(move |event: BlobEvent| {
            if let Some(data) = event.data() {
                chunks.borrow_mut().push(data);
            }
        })
    };
    let on_stop = {
        let stopped = stopped.clone();
        Closure::<dyn FnMut()>::new(move || {
            if let Some(sender) = stopped.borrow_mut().take() {
                let _ = sender.send(());
            }
        })
    };
    recorder.set_ondataavailable(Some(on_data.as_ref().unchecked_ref()));
    recorder.set_onstop(Some(on_stop.as_ref().unchecked_ref()));
    if recorder.start().is_err() || recorder.mime_type().to_ascii_lowercase().contains("opus") {
        recorder.set_ondataavailable(None);
        recorder.set_onstop(None);
        let _ = recorder.stop();
        return None;
    }
    Some(Engine::Mp4 {
        recorder,
        chunks,
        waiting,
        _on_data: on_data,
        _on_stop: on_stop,
    })
}

fn pcm(stream: &MediaStream) -> Option<Engine> {
    let context = AudioContext::new().ok()?;
    let source = context.create_media_stream_source(stream).ok()?;
    let processor = context
        .create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(4096, 1, 1)
        .ok()?;
    let samples = Rc::new(RefCell::new(Vec::new()));
    let on_audio = {
        let samples = samples.clone();
        Closure::<dyn FnMut(AudioProcessingEvent)>::new(move |event: AudioProcessingEvent| {
            if let Ok(buffer) = event.input_buffer() {
                if let Ok(channel) = buffer.get_channel_data(0) {
                    samples.borrow_mut().extend_from_slice(&channel);
                }
            }
        })
    };
    processor.set_onaudioprocess(Some(on_audio.as_ref().unchecked_ref()));
    source.connect_with_audio_node(&processor).ok()?;
    // Connected through to the speakers or some browsers never run it; its
    // output is silence, since nothing is ever written to it.
    processor
        .connect_with_audio_node(&context.destination())
        .ok()?;
    Some(Engine::Pcm {
        context,
        processor,
        samples,
        _on_audio: on_audio,
    })
}
