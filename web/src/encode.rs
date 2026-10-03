//! Bringing sound and video to the protocol's profile in a browser
//! (docs/protocol.md, "Preparing media before upload").
//!
//! The DECISION is not made here: fc_text::media_plan decides, from what
//! fc_text::media_probe read of the file, and every port is held to the same
//! answers. This is the part only a browser can do — decode with its
//! decoders, draw with its canvas, encode with its encoders — around three
//! pieces of Rust that are tested natively: the MP4 reader (where each frame
//! of the source is), the transcode arithmetic (which frames are kept, which
//! way the picture is turned) and the MP4 writer (`moov` first).
//!
//! A VIDEO is read frame by frame through that reader into a `VideoDecoder`,
//! not played through a `<video>` element and captured. A played video runs
//! in real time at best — a ten-minute clip would take ten minutes — drops
//! frames whenever the tab is busy, stalls outright in a background tab, and
//! gives no frame its real timestamp. Fed from the index, every frame is
//! decoded exactly once, carries the time its own file gave it, and the
//! whole thing runs as fast as the machine can encode.
//!
//! Everything here answers `Option`: None is "this browser could not do it",
//! for whatever reason — a missing class, a codec it does not decode, a
//! file it could not read, an encoder that gave up half way — and the caller
//! then sends the file exactly as it went before (rule C). Nothing in this
//! module can turn a send that would have worked into one that does not.
//!
//! A transcode is minutes of work for a long clip, so whoever asks for one
//! holds a [`Job`]: it is told how far the work has got, and it can be told
//! to stop — by the person, or because the chat it was for has gone.
//!
//! The same reader, writer and AAC encoder take a RECORDING's sound out for
//! its text ([`sound_for_text`], docs/protocol.md, "Transcripts on
//! request"), where the server's stored copy will not do: a video's AAC
//! copied frame for frame into an M4A, anything else decoded and encoded
//! again at 64 kbit/s mono — fc_text::transcript_sound decides which. That
//! path answers why it could not, rather than None: there is no original to
//! fall back on, only a line under the player.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_text::media::SIZE_LIMIT;
use fc_text::media_plan::{self, AudioPlan, AudioSource, VideoPlan, VideoSource, VideoTarget};
use fc_text::media_probe::{self, AudioProbe};
use fc_text::mp4::{self, Brand, Media};
use fc_text::mp4_read::{self, Movie};
use fc_text::transcode::{self, AudioRoute, KEYFRAME_SECONDS};
use fc_text::transcript::Failure as TextFailure;
use fc_text::transcript_sound::{self, Extraction, Found, TEXT_BITRATE, TEXT_CHANNELS};
use futures::channel::mpsc;
use futures::future::{select, Either};
use futures::StreamExt;
use gloo_timers::future::TimeoutFuture;
use js_sys::{Array, Float32Array, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{Blob, BlobPropertyBag, CanvasRenderingContext2d, HtmlCanvasElement};

use crate::webcodecs::{
    self, aac_config, bytes_of, decoder_config_field, object, AudioData, AudioDecoder,
    AudioEncoder, EncodedAudioChunk, EncodedVideoChunk, FrameCanvas, Samples, VideoDecoder,
    VideoEncoder, VideoFrame,
};

/// Whether a picked video is transcoded at all. One switch, so that turning
/// the whole path off is one word and not a hunt: `false` sends every video
/// exactly as the client did before issue #74.
pub const VIDEO_TRANSCODE: bool = true;

/// How long a codec may say nothing before it is given up on. Not a bound on
/// the transcode — a long clip takes as long as it takes — but on the gap
/// between one frame and the next: a decoder that has hung never errors, and
/// a send must not wait on it for ever.
const STALL_MS: u32 = 30_000;

/// How often a loop waiting on a codec looks again without being told to.
const TICK_MS: u32 = 100;

/// How much of the file is read at once. Samples are read in the order the
/// decoder wants them, which is nearly the order they are stored in, so one
/// read serves hundreds of frames.
const WINDOW_BYTES: u64 = 4 * 1024 * 1024;

/// The longest index worth reading into memory. An hour of 240 fps video
/// indexes in a few megabytes; more than this is not an index.
const MAX_INDEX_BYTES: u64 = 64 * 1024 * 1024;

/// A file has a handful of top-level boxes; one with thousands is not being
/// walked one 16-byte read at a time.
const MAX_TOP_BOXES: usize = 4096;

/// How much of a sound file's start is read for its header.
const HEADER_BYTES: u64 = 64 * 1024;

/// The most decoded sound — 32-bit floats, every channel — a picked file may
/// come to. `decodeAudioData` decodes a whole file at once; past this a tab
/// is more likely to be killed than to finish, and a tab that dies takes the
/// message being written with it. Such a file goes as it went before. It is
/// twenty minutes of stereo — more than a WAV within the ceiling can hold.
const MAX_PCM_BYTES: u64 = 512 * 1024 * 1024;

/// Frames a video decoder may have been given and not yet handed back.
/// More than any H.264 or HEVC stream reorders across (16) — a decoder
/// holding frames back must never be starved of the input it is waiting
/// for, and one fed by how full its QUEUE looks is: it swallows eight
/// frames, says nothing, and waits for a ninth that never comes. Few enough
/// that the decoded frames waiting — each one a texture — stay a small,
/// fixed cost.
const FRAMES_AHEAD: usize = 24;

/// The same for sound, whose decoder hands back a frame for every frame —
/// less the one or two it keeps for itself.
const SOUND_AHEAD: usize = 64;

/// Work an encoder may have queued before it is given no more.
const QUEUE: u32 = 8;

/// Sound handed to the encoder at a time, in frames: a third of a second.
const SOUND_BLOCK: u32 = 16 * 1024;

// --- the job ------------------------------------------------------------------------------------

/// One file being prepared, as whoever asked for it holds it: a way to
/// STOP the work, and to hear how far it has got.
///
/// Before there was a profile, staging a video took a second — its size and
/// a poster. A transcode takes as long as the clip is heavy, and work that
/// long must be possible to call off: the person picked the wrong file, or
/// left the chat, or signed out. Without this it ran to the end regardless,
/// its whole output held in memory, to be thrown away.
///
/// Stopping is asked for, not forced: every loop here looks at the job each
/// time it comes round — at least every [`TICK_MS`] — and answers None,
/// which lets go of its codecs and everything they made. A stopped job is
/// NOT rule C: the caller asked for nothing to be sent, and must look at
/// [`Job::stopped`] before taking None to mean "send the original".
#[derive(Clone, Default)]
pub struct Job {
    stopped: Rc<Cell<bool>>,
    /// Told a percentage, each time it changes.
    progress: Option<Rc<dyn Fn(u32)>>,
    said: Rc<Cell<Option<u32>>>,
}

impl Job {
    /// A job whose progress is told to `progress`, in whole per cent.
    pub fn watched(progress: impl Fn(u32) + 'static) -> Self {
        Job {
            progress: Some(Rc::new(progress)),
            ..Job::default()
        }
    }

    /// Call it off. Whatever is being made is dropped the next time its
    /// loop comes round.
    pub fn stop(&self) {
        self.stopped.set(true);
    }

    pub fn stopped(&self) -> bool {
        self.stopped.get()
    }

    /// `done` of `all` — of the picture's frames, which are where the time
    /// goes. Said only when the whole per cent changes, and never once
    /// stopped: nobody is listening.
    fn progress(&self, done: usize, all: usize) {
        let Some(progress) = self.progress.as_ref() else {
            return;
        };
        if all == 0 || self.stopped() {
            return;
        }
        let percent = (done.min(all) * 100 / all) as u32;
        if self.said.replace(Some(percent)) != Some(percent) {
            progress(percent);
        }
    }
}

// --- reading ------------------------------------------------------------------------------------

/// Bytes `from..to` of `blob` — all of them, or None.
pub async fn bytes(blob: &Blob, from: u64, to: u64) -> Option<Uint8Array> {
    let slice = blob.slice_with_f64_and_f64(from as f64, to as f64).ok()?;
    let buffer = JsFuture::from(slice.array_buffer()).await.ok()?;
    let bytes = Uint8Array::new(&buffer);
    (u64::from(bytes.length()) == to.checked_sub(from)?).then_some(bytes)
}

/// A file's index, wherever it is: the top-level boxes are walked by their
/// headers — sixteen bytes each, so a gigabyte of `mdat` in front of a
/// camera's `moov` costs one small read — and the `moov` alone is read whole.
pub async fn movie(blob: &Blob) -> Option<Movie> {
    let size = blob.size() as u64;
    let mut at = 0u64;
    for _ in 0..MAX_TOP_BOXES {
        if at + 8 > size {
            return None;
        }
        let head = bytes(blob, at, (at + 16).min(size)).await?.to_vec();
        let header = mp4_read::box_header(&head)?;
        let length = header.size.unwrap_or(size - at);
        if length < header.header_len || at + length > size {
            return None;
        }
        if &header.kind == b"moov" {
            if length > MAX_INDEX_BYTES {
                return None;
            }
            let index = bytes(blob, at, at + length).await?.to_vec();
            return mp4_read::parse_moov(&index).ok();
        }
        at += length;
    }
    None
}

/// What a picked sound file is, as the planner needs it: from its index when
/// it is an MP4, and from its own header otherwise. A file that cannot be
/// read is "unknown", which the planner leaves alone.
pub async fn probe_audio(file: &Blob, container: &str) -> AudioProbe {
    let size = file.size() as u64;
    if container == "audio/mp4" {
        return match movie(file).await {
            Some(movie) => media_probe::audio_from_movie(&movie, container, size),
            None => media_probe::audio_from_header(container, &[], size),
        };
    }
    // An ID3 tag — megabytes of cover art, sometimes — comes before the
    // header of an MP3, and of a FLAC somebody tagged like one.
    let tag = match bytes(file, 0, size.min(10)).await {
        Some(start) => media_probe::id3_length(&start.to_vec()),
        None => 0,
    };
    let from = if tag < size { tag } else { 0 };
    let header = match bytes(file, from, size.min(from + HEADER_BYTES)).await {
        Some(header) => header.to_vec(),
        None => Vec::new(),
    };
    media_probe::audio_from_header(container, &header, size)
}

/// Samples read out of a file through one moving window.
struct Reader<'a> {
    blob: &'a Blob,
    size: u64,
    start: u64,
    window: Option<Uint8Array>,
}

impl<'a> Reader<'a> {
    fn new(blob: &'a Blob) -> Self {
        Reader {
            blob,
            size: blob.size() as u64,
            start: 0,
            window: None,
        }
    }

    /// `length` bytes at `offset`, as a view on the window they were read
    /// in — good until it is copied, which every user of it does at once.
    async fn read(&mut self, offset: u64, length: u32) -> Option<Uint8Array> {
        let end = offset.checked_add(u64::from(length))?;
        if end > self.size {
            return None;
        }
        let held = self.window.as_ref().is_some_and(|window| {
            offset >= self.start && end <= self.start + u64::from(window.length())
        });
        if !held {
            let to = (offset + WINDOW_BYTES.max(u64::from(length))).min(self.size);
            self.window = Some(bytes(self.blob, offset, to).await?);
            self.start = offset;
        }
        let window = self.window.as_ref()?;
        let begin = (offset - self.start) as u32;
        Some(window.subarray(begin, begin + length))
    }
}

/// `parts`, one after the other behind `header`, as a file of type `mime`.
/// The parts are the encoder's own buffers: the media is never copied into
/// this module's memory to be written out.
fn assemble(header: &[u8], parts: impl Iterator<Item = JsValue>, mime: &str) -> Option<Blob> {
    let all = Array::new();
    all.push(&Uint8Array::from(header));
    for part in parts {
        all.push(&part);
    }
    let options = BlobPropertyBag::new();
    options.set_type(mime);
    Blob::new_with_u8_array_sequence_and_options(&all, &options).ok()
}

// --- codecs -------------------------------------------------------------------------------------

/// A decoded frame, closed when it is let go of — a `VideoFrame` holds a
/// texture until it is, and the garbage collector is in no hurry.
struct Frame(VideoFrame);

impl Drop for Frame {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// Decoded sound, closed the same way.
struct Sound(AudioData);

impl Drop for Sound {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// What a codec's callbacks tell the loop that is driving it.
enum Event {
    Frame(Frame),
    Sound(Sound),
    /// An encoder put something out: whoever was waiting for room in its
    /// queue may look again.
    Progress,
    /// A decoder's `flush` settled — everything it had has been handed over
    /// (true), or it failed.
    Drained(bool),
    Failed,
}

type Wake = mpsc::UnboundedSender<Event>;

/// The next thing a codec says — or, after [`TICK_MS`] of silence, a
/// [`Event::Progress`] nobody sent, so that whoever is waiting looks again
/// at what it was waiting for. None once [`STALL_MS`] have gone by with
/// nothing said at all.
///
/// The tick is there because not everything a codec does is announced. One
/// that takes frames in and holds them — a decoder reordering, an encoder
/// looking ahead — empties its queue without a word, and a loop waiting
/// for that queue to have room would wait for ever on the silence.
async fn next(events: &mut mpsc::UnboundedReceiver<Event>, silence: &mut u32) -> Option<Event> {
    match select(events.next(), TimeoutFuture::new(TICK_MS)).await {
        Either::Left((event, _)) => {
            *silence = 0;
            event
        }
        Either::Right(_) => {
            *silence += TICK_MS;
            if *silence >= STALL_MS {
                log::warn!("A codec stalled; the file goes as it would have before");
                return None;
            }
            Some(Event::Progress)
        }
    }
}

/// Whether `promise` — a `flush` — resolved, within [`STALL_MS`].
async fn settled(promise: Result<Promise, JsValue>) -> bool {
    let Ok(promise) = promise else {
        return false;
    };
    match select(JsFuture::from(promise), TimeoutFuture::new(STALL_MS)).await {
        Either::Left((outcome, _)) => outcome.is_ok(),
        Either::Right(_) => false,
    }
}

/// Tell the loop when a decoder's `flush` has settled.
fn when_drained(promise: Result<Promise, JsValue>, wake: &Wake) {
    let wake = wake.clone();
    match promise {
        Ok(promise) => spawn_local(async move {
            let drained = JsFuture::from(promise).await.is_ok();
            let _ = wake.unbounded_send(Event::Drained(drained));
        }),
        Err(_) => {
            let _ = wake.unbounded_send(Event::Failed);
        }
    }
}

type Output = Closure<dyn FnMut(JsValue, JsValue)>;
type Failure = Closure<dyn FnMut(JsValue)>;

/// The `{output, error}` every WebCodecs class is constructed with, and the
/// two closures to keep alive for as long as it lives. An error is logged,
/// and wakes the loop with [`Event::Failed`].
fn callbacks(
    output: impl FnMut(JsValue, JsValue) + 'static,
    wake: &Wake,
) -> (Object, Output, Failure) {
    let output = Output::new(output);
    let wake = wake.clone();
    let failure = Failure::new(move |error: JsValue| {
        log::warn!("A codec failed: {error:?}");
        let _ = wake.unbounded_send(Event::Failed);
    });
    let init = object(&[
        ("output", output.as_ref().clone()),
        ("error", failure.as_ref().clone()),
    ]);
    (init, output, failure)
}

/// A codec and its callbacks, closed before the callbacks go: an event
/// arriving at a dropped closure throws, and a closed codec sends none.
struct Codec<T: Close> {
    codec: T,
    _output: Output,
    _failure: Failure,
}

trait Close {
    fn shut(&self);
}

macro_rules! closes {
    ($($class:ty),*) => {$(
        impl Close for $class {
            fn shut(&self) {
                // Closing twice, or after an error closed it, throws.
                let _ = self.close();
            }
        }
    )*};
}
closes!(AudioEncoder, AudioDecoder, VideoEncoder, VideoDecoder);

impl<T: Close> Drop for Codec<T> {
    fn drop(&mut self) {
        self.codec.shut();
    }
}

// --- AAC ----------------------------------------------------------------------------------------

/// Encoded AAC-LC: its frames, each still in the buffer the encoder gave.
pub struct Aac {
    frames: Vec<Uint8Array>,
    bytes: u64,
    /// The AudioSpecificConfig — the encoder's own, or the one that says
    /// what it was asked for when it hands back none.
    asc: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u32,
}

impl Aac {
    /// The track an MP4 carries it as: starting `skip` samples into itself,
    /// after `lead` milliseconds of nothing. Every frame is 1024 samples and
    /// a key frame; the rate the `esds` states is what the frames really
    /// come to.
    fn track(&self, skip: u32, lead: u32) -> mp4::Track {
        let ticks = self.frames.len() as u64 * u64::from(mp4::AAC_FRAME_SAMPLES);
        let average = (self.bytes * 8 * u64::from(self.sample_rate))
            .checked_div(ticks)
            .unwrap_or(0);
        mp4::Track {
            timescale: self.sample_rate,
            media: Media::Audio {
                sample_rate: self.sample_rate,
                channels: self.channels as u16,
                asc: self.asc.clone(),
                avg_bitrate: u32::try_from(average).unwrap_or(0),
                max_bitrate: 0,
            },
            samples: self
                .frames
                .iter()
                .map(|frame| mp4::Sample {
                    size: frame.length(),
                    duration: mp4::AAC_FRAME_SAMPLES,
                    composition_offset: 0,
                    sync: true,
                })
                .collect(),
            skip,
            lead,
        }
    }

    fn parts(&self) -> Vec<JsValue> {
        self.frames
            .iter()
            .map(|frame| frame.clone().into())
            .collect()
    }
}

#[derive(Default)]
struct AacSink {
    frames: Vec<Uint8Array>,
    bytes: u64,
    asc: Option<Vec<u8>>,
    failed: bool,
}

/// An `AudioEncoder` making AAC-LC, and what it has made so far.
struct AacEncoder {
    encoder: Codec<AudioEncoder>,
    sink: Rc<RefCell<AacSink>>,
    sample_rate: u32,
    channels: u32,
}

impl AacEncoder {
    /// Configured and ready — None if the browser refuses it. The caller
    /// has already asked `isConfigSupported`; this is the second opinion of
    /// the encoder itself.
    fn start(sample_rate: u32, channels: u32, bitrate: u64, wake: &Wake) -> Option<Self> {
        let sink = Rc::new(RefCell::new(AacSink::default()));
        let (init, output, failure) = {
            let sink = sink.clone();
            let waker = wake.clone();
            callbacks(
                move |chunk: JsValue, metadata: JsValue| {
                    let chunk: EncodedAudioChunk = chunk.unchecked_into();
                    let bytes = Uint8Array::new_with_length(chunk.byte_length());
                    let mut sink = sink.borrow_mut();
                    if chunk.copy_to(&bytes).is_err() {
                        sink.failed = true;
                    }
                    sink.bytes += u64::from(bytes.length());
                    sink.frames.push(bytes);
                    if sink.asc.is_none() {
                        sink.asc = decoder_config_field(&metadata, "description")
                            .and_then(|description| bytes_of(&description))
                            .filter(|asc| !asc.is_empty());
                    }
                    let _ = waker.unbounded_send(Event::Progress);
                },
                wake,
            )
        };
        let encoder = Codec {
            codec: AudioEncoder::new(&init).ok()?,
            _output: output,
            _failure: failure,
        };
        encoder
            .codec
            .configure(&aac_config(sample_rate, channels, bitrate))
            .ok()?;
        Some(AacEncoder {
            encoder,
            sink,
            sample_rate,
            channels,
        })
    }

    fn queued(&self) -> u32 {
        self.encoder.codec.encode_queue_size()
    }

    /// Everything encoded, once the encoder has let go of the last of it.
    /// `samples` is how much sound went in, per channel: what comes out has
    /// to be that, in frames of 1024, give or take the encoder's own priming
    /// and padding — an encoder that answers with anything else (frames of
    /// another length, half the sound) has not made what the index would
    /// claim, and is refused.
    async fn finish(self, samples: u64) -> Option<Aac> {
        if !settled(self.encoder.codec.flush()).await {
            return None;
        }
        let sink = std::mem::take(&mut *self.sink.borrow_mut());
        if sink.failed || sink.frames.is_empty() {
            return None;
        }
        let frame = u64::from(mp4::AAC_FRAME_SAMPLES);
        let made = sink.frames.len() as u64 * frame;
        if made + frame < samples || made > samples + 8 * frame {
            log::warn!("An AAC encoder made {made} samples of {samples}; not used");
            return None;
        }
        // What the encoder says it made wins over what it was asked for.
        let stated = sink.asc.as_deref().and_then(mp4_read::audio_config);
        let (sample_rate, channels) = match stated {
            Some(config) if config.object_type != mp4::AAC_LC => {
                log::warn!(
                    "An AAC encoder made object type {}; not used",
                    config.object_type
                );
                return None;
            }
            Some(config) if config.sample_rate > 0 && config.channels > 0 => {
                (config.sample_rate, u32::from(config.channels))
            }
            _ => (self.sample_rate, self.channels),
        };
        if sample_rate != self.sample_rate || channels != self.channels {
            return None;
        }
        let asc = sink.asc.unwrap_or_else(|| {
            mp4::audio_specific_config(mp4::AAC_LC, sample_rate, channels as u16)
        });
        Some(Aac {
            frames: sink.frames,
            bytes: sink.bytes,
            asc,
            sample_rate,
            channels,
        })
    }
}

/// Raw sound — 32-bit floats, a plane per channel, `block` by `block` until
/// it answers None — as AAC-LC at `bitrate`. Each block is
/// `(planes one after the other, frames in it)`.
pub async fn aac_from_pcm(
    mut block: impl FnMut() -> Option<(Float32Array, u32)>,
    sample_rate: u32,
    channels: u32,
    bitrate: u64,
) -> Option<Aac> {
    let (wake, mut events) = mpsc::unbounded();
    let encoder = AacEncoder::start(sample_rate, channels, bitrate, &wake)?;
    let mut samples = 0u64;
    let mut silence = 0u32;
    while let Some((data, frames)) = block() {
        if frames == 0 {
            continue;
        }
        let sound = AudioData::new(&object(&[
            ("format", JsValue::from_str("f32-planar")),
            ("sampleRate", JsValue::from(sample_rate)),
            ("numberOfFrames", JsValue::from(frames)),
            ("numberOfChannels", JsValue::from(channels)),
            (
                "timestamp",
                JsValue::from((samples as f64 * 1e6 / f64::from(sample_rate)).round()),
            ),
            ("data", data.into()),
        ]))
        .ok()?;
        let sent = encoder.encoder.codec.encode(&sound);
        sound.close();
        sent.ok()?;
        samples += u64::from(frames);
        while encoder.queued() > QUEUE {
            match next(&mut events, &mut silence).await {
                Some(Event::Failed) | None => return None,
                Some(_) => {}
            }
        }
    }
    encoder.finish(samples).await
}

/// AAC as an M4A file: `ftyp`, `moov`, then the frames.
pub fn m4a(aac: &Aac) -> Option<Blob> {
    let layout = mp4::layout(&[aac.track(0, 0)], Brand::M4a);
    let parts = aac.parts();
    let file = assemble(
        &layout.header,
        layout
            .order
            .iter()
            .map(|&(_, sample)| parts[sample].clone()),
        "audio/mp4",
    )?;
    (file.size() as u64 == layout.total_len).then_some(file)
}

// --- a picked sound file ------------------------------------------------------------------------

/// A sound file brought to the profile.
pub struct Track {
    pub blob: Blob,
    pub duration_ms: i64,
}

/// `file`, which the planner said to re-encode, as an M4A — decoded by the
/// browser's own `decodeAudioData`, which also resamples it to a rate the
/// encoder takes, and encoded at the planner's bitrate. None when this
/// browser cannot: it does not decode the format (AIFF outside Safari, ALAC
/// and Ogg in some), the sound has more than two channels, or it does not
/// encode AAC — and when `job` was stopped, which the caller tells apart by
/// asking the job.
pub async fn audio(file: &Blob, probe: &AudioProbe, job: &Job) -> Option<Track> {
    let sample_rate = transcode::audio_output_rate(probe.sample_rate);
    if !could_decode(&probe.source, sample_rate) {
        return None;
    }
    let buffer = JsFuture::from(file.array_buffer()).await.ok()?;
    if job.stopped() {
        return None;
    }
    // An offline context, because it is the one kind that can be made at a
    // chosen rate without a device, and that no autoplay rule suspends. Its
    // own length and channels do not matter: only its rate is used.
    let context =
        web_sys::OfflineAudioContext::new_with_number_of_channels_and_length_and_sample_rate(
            1,
            1,
            sample_rate as f32,
        )
        .ok()?;
    let decoding = context.decode_audio_data(buffer.unchecked_ref()).ok()?;
    let decoded: web_sys::AudioBuffer = JsFuture::from(decoding).await.ok()?.dyn_into().ok()?;
    // `decodeAudioData` cannot be interrupted; what follows it can.
    if job.stopped() {
        return None;
    }
    let channels = decoded.number_of_channels();
    let frames = decoded.length();
    let sample_rate = decoded.sample_rate().round() as u32;
    if !(1..=2).contains(&channels) || frames == 0 || sample_rate == 0 {
        return None;
    }
    // The planner's own rate for what was decoded — which says how many
    // channels there really are, where the header did not.
    let mut source = probe.source.clone();
    source.channels = Some(channels);
    let AudioPlan::Transcode { bitrate } = media_plan::plan_audio(&source) else {
        return None;
    };
    if !webcodecs::aac_supported(sample_rate, channels, bitrate).await {
        return None;
    }
    let samples: &Samples = decoded.unchecked_ref();
    let planes: Vec<Float32Array> = (0..channels)
        .map(|index| samples.channel(index).ok())
        .collect::<Option<_>>()?;
    let mut at = 0u32;
    let watch = job.clone();
    let blocks = move || {
        // A stopped job hands over no more — and what was encoded of the
        // part before is thrown away below, not written out as a file that
        // ends where the person pressed Cancel.
        if at >= frames || watch.stopped() {
            return None;
        }
        watch.progress(at as usize, frames as usize);
        let count = SOUND_BLOCK.min(frames - at);
        let data = Float32Array::new_with_length(count * channels);
        for (index, plane) in planes.iter().enumerate() {
            data.set(&plane.subarray(at, at + count), index as u32 * count);
        }
        at += count;
        Some((data, count))
    };
    let aac = aac_from_pcm(blocks, sample_rate, channels, bitrate).await?;
    if job.stopped() {
        return None;
    }
    Some(Track {
        blob: m4a(&aac)?,
        duration_ms: (f64::from(frames) * 1000.0 / f64::from(sample_rate)).round() as i64,
    })
}

/// Whether decoding `source` whole stays within [`MAX_PCM_BYTES`]: by its
/// length where the header says it, and otherwise by its size, at the most
/// a lossy file expands by — 128 kbit/s to 48 kHz stereo floats is 24 times.
fn could_decode(source: &AudioSource, sample_rate: u32) -> bool {
    let channels = u64::from(source.channels.unwrap_or(2).max(1));
    match source.duration_ms {
        Some(ms) => {
            let frames = ms.saturating_mul(u64::from(sample_rate)) / 1000;
            frames.saturating_mul(channels * 4) <= MAX_PCM_BYTES
        }
        None => source.size_bytes.saturating_mul(24) <= MAX_PCM_BYTES,
    }
}

// --- a picked video -----------------------------------------------------------------------------

/// What became of a picked video.
pub enum Planned {
    /// Already within the profile: the original goes, untouched (rule A).
    Keep,
    /// Transcoded to the profile: an MP4, its index first.
    Made(Blob),
}

/// `file` — sent as `container` — as the planner says it should go. None
/// when it cannot be read, planned or transcoded here (rule C) — and when
/// `job` was stopped, which is not rule C: the caller asks the job.
pub async fn video(file: &Blob, container: &str, job: &Job) -> Option<Planned> {
    if !VIDEO_TRANSCODE {
        return None;
    }
    let size = file.size() as u64;
    let movie = movie(file).await?;
    let source = media_probe::video_source(&movie, container, size)?;
    let target = match media_plan::plan_video(&source) {
        VideoPlan::Keep => return Some(Planned::Keep),
        VideoPlan::Fallback => return None,
        VideoPlan::Transcode(target) => target,
    };
    if !transcode::could_fit(size, SIZE_LIMIT, &target, source.duration_ms) {
        return None;
    }
    // Whether this browser can do the PICTURE at all is asked before a byte
    // of the media is read. A browser with no H.264 encoder, or no decoder
    // for a phone's HEVC, would otherwise decode and re-encode the whole
    // sound of a long clip — or copy every frame of it into memory — only
    // to find that out, on every such pick, before sending the original.
    let codecs = picture_codecs(&movie, &target).await?;
    // Then the sound, before the picture: it is seconds of work, and a clip
    // whose sound cannot be brought to the profile is found out before
    // minutes are spent on its picture.
    let sound = match (movie.audio(), target.audio_bitrate) {
        (Some(track), Some(_)) => Some(sound(file, &movie, track, &source, job).await?),
        (None, None) => None,
        _ => return None,
    };
    let (picture, frames) = picture(file, &movie, &source, &target, codecs, job).await?;
    let mut tracks = vec![picture];
    let mut parts = vec![frames];
    if let Some((track, frames)) = sound {
        tracks.push(track);
        parts.push(frames);
    }
    let layout = mp4::layout(&tracks, Brand::Mp4);
    let blob = assemble(
        &layout.header,
        layout
            .order
            .iter()
            .map(|&(track, sample)| parts[track][sample].clone()),
        "video/mp4",
    )?;
    (blob.size() as u64 == layout.total_len).then_some(Planned::Made(blob))
}

/// Microseconds — WebCodecs' unit — for `ticks` of `timescale`.
fn micros(ticks: i64, timescale: u32) -> f64 {
    (ticks as f64 * 1e6 / f64::from(timescale)).round()
}

/// The sound of a video, as the audio-in-video row has it: its AAC frames
/// copied where they already meet the row, decoded and re-encoded where
/// they do not.
async fn sound(
    file: &Blob,
    movie: &Movie,
    track: &mp4_read::Track,
    source: &VideoSource,
    job: &Job,
) -> Option<(mp4::Track, Vec<JsValue>)> {
    let entry = track.entry.as_ref()?;
    if !track.edit_supported() || track.timescale == 0 {
        return None;
    }
    // The planner's rate for it — asked again below, of what the decoder
    // actually hands out, where a header turns out to have miscounted.
    let bitrate = media_plan::target_audio_bitrate(source.audio_channels, source.audio_bitrate);
    // How long the source shows nothing before its sound starts. It is
    // written out again as it was read: dropped, the sound would start
    // that much early against the picture, for the whole clip.
    let lead = movie.lead_ms(track)?;
    // Only AAC is decoded here. Uncompressed sound in a QuickTime movie, or
    // MP3 in an old MP4, takes rule C.
    if mp4_read::audio_codec_name(entry) != "aac" {
        return None;
    }
    let config = mp4_read::audio_config(&entry.config)?;
    let route = transcode::audio_route(
        Some(config.object_type),
        source.audio_channels,
        source.audio_bitrate,
        bitrate,
    )?;
    // Where the presentation starts in the media: past the priming frames
    // of whatever encoder made it.
    let start = track.edit.map_or(0, |edit| edit.media_start.max(0));
    // And how much of it is presented, in the track's own ticks — None
    // without an edit list, which presents all of it.
    let presented = track
        .edit
        .and_then(|edit| edit.length)
        .filter(|_| movie.timescale > 0)
        .map(|length| length * u64::from(track.timescale) / u64::from(movie.timescale));
    let mut reader = Reader::new(file);
    match route {
        AudioRoute::Copy => {
            let mut parts = Vec::with_capacity(track.samples.len());
            for sample in &track.samples {
                if job.stopped() {
                    return None;
                }
                let bytes = reader.read(sample.offset, sample.size).await?;
                // A copy of its own: a view would keep its whole window.
                parts.push(bytes.slice(0, bytes.length()).into());
            }
            let copied = mp4::Track {
                timescale: track.timescale,
                media: Media::Audio {
                    sample_rate: config.sample_rate,
                    channels: source.audio_channels? as u16,
                    asc: entry.config.clone(),
                    avg_bitrate: source
                        .audio_bitrate
                        .and_then(|rate| u32::try_from(rate).ok())
                        .unwrap_or(0),
                    max_bitrate: 0,
                },
                samples: track
                    .samples
                    .iter()
                    .map(|sample| mp4::Sample {
                        size: sample.size,
                        duration: sample.duration,
                        composition_offset: 0,
                        sync: true,
                    })
                    .collect(),
                skip: u32::try_from(start).ok()?,
                lead,
            };
            Some((copied, parts))
        }
        AudioRoute::Encode { channels, .. } => {
            let decoder_config = object(&[
                (
                    "codec",
                    JsValue::from_str(&format!("mp4a.40.{}", config.object_type)),
                ),
                ("sampleRate", JsValue::from(config.sample_rate)),
                ("numberOfChannels", JsValue::from(channels)),
                (
                    "description",
                    Uint8Array::from(entry.config.as_slice()).into(),
                ),
            ]);
            if !webcodecs::supported("AudioDecoder", &decoder_config).await {
                return None;
            }
            let (wake, mut events) = mpsc::unbounded();
            let decoder = {
                let waker = wake.clone();
                let (init, output, failure) = callbacks(
                    move |data: JsValue, _| {
                        let _ = waker.unbounded_send(Event::Sound(Sound(data.unchecked_into())));
                    },
                    &wake,
                );
                Codec {
                    codec: AudioDecoder::new(&init).ok()?,
                    _output: output,
                    _failure: failure,
                }
            };
            decoder.codec.configure(&decoder_config).ok()?;
            let mut encoder: Option<AacEncoder> = None;
            let mut samples = 0u64;
            let (mut fed, mut heard) = (0usize, 0usize);
            let mut flushing = false;
            let mut silence = 0u32;
            loop {
                if job.stopped() {
                    return None;
                }
                while fed < track.samples.len()
                    && fed - heard < SOUND_AHEAD
                    && encoder.as_ref().map_or(0, AacEncoder::queued) < QUEUE
                {
                    let sample = &track.samples[fed];
                    let bytes = reader.read(sample.offset, sample.size).await?;
                    let chunk = EncodedAudioChunk::new(&object(&[
                        ("type", JsValue::from_str("key")),
                        (
                            "timestamp",
                            JsValue::from(micros(sample.pts, track.timescale)),
                        ),
                        (
                            "duration",
                            JsValue::from(micros(i64::from(sample.duration), track.timescale)),
                        ),
                        ("data", bytes.into()),
                    ]))
                    .ok()?;
                    decoder.codec.decode(&chunk).ok()?;
                    fed += 1;
                }
                if fed == track.samples.len() && !flushing {
                    flushing = true;
                    when_drained(decoder.codec.flush(), &wake);
                }
                match next(&mut events, &mut silence).await? {
                    Event::Sound(sound) => {
                        heard += 1;
                        let data = &sound.0;
                        let rate = data.sample_rate().round() as u32;
                        if encoder.is_none() {
                            // What the decoder actually hands out decides
                            // the encoder: HE-AAC decodes at twice the rate
                            // its index states, and a stream can hold one
                            // channel where its headers say two (or, with
                            // parametric stereo, two where they say one).
                            // The row is then the one for what the sound
                            // IS — 64 000 for one channel — not for what a
                            // header called it; more than two is rule C, as
                            // it is when the header says so.
                            let channels = data.number_of_channels();
                            if !(1..=2).contains(&channels) {
                                return None;
                            }
                            let bitrate = media_plan::target_audio_bitrate(
                                Some(channels),
                                source.audio_bitrate,
                            );
                            if !webcodecs::aac_supported(rate, channels, bitrate).await {
                                return None;
                            }
                            encoder = Some(AacEncoder::start(rate, channels, bitrate, &wake)?);
                        }
                        let encoder = encoder.as_ref()?;
                        if rate != encoder.sample_rate
                            || data.number_of_channels() != encoder.channels
                        {
                            return None;
                        }
                        // Past the end of what the source PRESENTS is its
                        // encoder's padding: silence that would only make
                        // the sound outlast the picture.
                        let end = presented.map(|ticks| {
                            (start as u128 + u128::from(ticks)) * u128::from(rate)
                                / u128::from(track.timescale)
                        });
                        if end.is_some_and(|end| u128::from(samples) >= end) {
                            continue;
                        }
                        encoder.encoder.codec.encode(data).ok()?;
                        samples += u64::from(data.number_of_frames());
                    }
                    Event::Drained(true) => break,
                    Event::Drained(false) | Event::Failed => return None,
                    Event::Frame(_) | Event::Progress => {}
                }
            }
            drop(decoder);
            let aac = encoder?.finish(samples).await?;
            // The source's priming was decoded with everything else, so it
            // is still at the front: the same skip, at the new rate.
            let skip = start as u128 * u128::from(aac.sample_rate) / u128::from(track.timescale);
            Some((aac.track(u32::try_from(skip).ok()?, lead), aac.parts()))
        }
    }
}

#[derive(Default)]
struct PictureSink {
    /// Bytes, presentation time in microseconds, key frame — in the order
    /// the encoder put them out, which is decode order.
    chunks: Vec<(Uint8Array, f64, bool)>,
    avcc: Option<Vec<u8>>,
    colour: Option<JsValue>,
    failed: bool,
}

/// The canvas each kept frame is drawn on: the target's size, with the
/// source's rotation drawn in.
struct Easel {
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
    width: f64,
    height: f64,
}

impl Easel {
    fn new(rotation: u16, target: &VideoTarget) -> Option<Self> {
        let document = web_sys::window()?.document()?;
        let canvas: HtmlCanvasElement = document.create_element("canvas").ok()?.dyn_into().ok()?;
        canvas.set_width(target.width);
        canvas.set_height(target.height);
        let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
        let _ = Reflect::set(
            &context,
            &JsValue::from_str("imageSmoothingQuality"),
            &JsValue::from_str("high"),
        );
        let placement = transcode::placement(rotation, target.width, target.height);
        let [a, b, c, d, e, f] = placement.transform;
        context.set_transform(a, b, c, d, e, f).ok()?;
        Some(Easel {
            canvas,
            context,
            width: placement.width,
            height: placement.height,
        })
    }

    /// `frame`, scaled and turned, as a new frame shown at `timestamp`.
    ///
    /// The canvas is 8-bit sRGB, so this is also where HDR ends: the
    /// browser tone-maps a PQ or HLG frame as it draws it, and what is read
    /// back is SDR whatever went in.
    fn paint(&self, frame: &VideoFrame, timestamp: f64) -> Option<VideoFrame> {
        let context: &FrameCanvas = self.context.unchecked_ref();
        context
            .draw_frame(frame, 0.0, 0.0, self.width, self.height)
            .ok()?;
        VideoFrame::new_from_image(
            self.canvas.as_ref(),
            &object(&[
                ("timestamp", JsValue::from(timestamp)),
                ("alpha", JsValue::from_str("discard")),
            ]),
        )
        .ok()
    }
}

/// What a picture's decoder and encoder are configured with — which this
/// browser has said it can do.
struct Codecs {
    decoder: Object,
    encoder: Object,
}

/// Whether this browser can transcode `movie`'s picture to `target` at all,
/// asked of the browser and answered from the index alone — nothing of the
/// media is read. None is rule C.
async fn picture_codecs(movie: &Movie, target: &VideoTarget) -> Option<Codecs> {
    let track = movie.video()?;
    let entry = track.entry.as_ref()?;
    track.rotation?;
    if !track.edit_supported() || track.timescale == 0 || track.timescale > 1_000_000 {
        return None;
    }
    u16::try_from(target.width).ok()?;
    u16::try_from(target.height).ok()?;
    let decoder = {
        let mut fields = vec![
            (
                "codec",
                JsValue::from_str(&mp4_read::video_codec_string(entry)?),
            ),
            ("codedWidth", JsValue::from(entry.width)),
            ("codedHeight", JsValue::from(entry.height)),
        ];
        if let Some(description) = mp4_read::video_description(entry) {
            fields.push(("description", Uint8Array::from(description).into()));
        }
        object(&fields)
    };
    if !webcodecs::supported("VideoDecoder", &decoder).await {
        return None;
    }
    let encoder = webcodecs::h264_config(
        target.width,
        target.height,
        target.frame_rate,
        target.video_bitrate,
    )
    .await?;
    Some(Codecs { decoder, encoder })
}

/// The picture of a video, at the planner's target: every frame decoded,
/// the ones the frame rate keeps drawn at the target's size the right way
/// up, and encoded as H.264 at the target's bitrate.
async fn picture(
    file: &Blob,
    movie: &Movie,
    source: &VideoSource,
    target: &VideoTarget,
    codecs: Codecs,
    job: &Job,
) -> Option<(mp4::Track, Vec<JsValue>)> {
    let track = movie.video()?;
    let rotation = track.rotation?;
    // Ticks a second of the output: the source's own, so that a frame's
    // time is written exactly as its file gave it.
    let timescale = track.timescale;
    let (width, height) = (
        u16::try_from(target.width).ok()?,
        u16::try_from(target.height).ok()?,
    );
    // How long the source shows nothing before its picture starts — a
    // phone whose camera came up after its microphone. Written out as it
    // was read, like the sound's.
    let lead = movie.lead_ms(track)?;
    let Codecs {
        decoder: decoder_config,
        encoder: encoder_config,
    } = codecs;
    let easel = Easel::new(rotation, target)?;
    let mut gate = transcode::frame_gate(source.frame_rate, target.frame_rate);

    let (wake, mut events) = mpsc::unbounded();
    let sink = Rc::new(RefCell::new(PictureSink::default()));
    let encoder = {
        let sink = sink.clone();
        let waker = wake.clone();
        let (init, output, failure) = callbacks(
            move |chunk: JsValue, metadata: JsValue| {
                let chunk: EncodedVideoChunk = chunk.unchecked_into();
                let bytes = Uint8Array::new_with_length(chunk.byte_length());
                let mut sink = sink.borrow_mut();
                if chunk.copy_to(&bytes).is_err() {
                    sink.failed = true;
                }
                sink.chunks
                    .push((bytes, chunk.timestamp(), chunk.kind() == "key"));
                if sink.avcc.is_none() {
                    sink.avcc = decoder_config_field(&metadata, "description")
                        .and_then(|description| bytes_of(&description))
                        .filter(|avcc| !avcc.is_empty());
                }
                if sink.colour.is_none() {
                    sink.colour = decoder_config_field(&metadata, "colorSpace");
                }
                let _ = waker.unbounded_send(Event::Progress);
            },
            &wake,
        );
        Codec {
            codec: VideoEncoder::new(&init).ok()?,
            _output: output,
            _failure: failure,
        }
    };
    encoder.codec.configure(&encoder_config).ok()?;
    let decoder = {
        let waker = wake.clone();
        let (init, output, failure) = callbacks(
            move |frame: JsValue, _| {
                let _ = waker.unbounded_send(Event::Frame(Frame(frame.unchecked_into())));
            },
            &wake,
        );
        Codec {
            codec: VideoDecoder::new(&init).ok()?,
            _output: output,
            _failure: failure,
        }
    };
    decoder.codec.configure(&decoder_config).ok()?;

    // The edit: where the presentation starts in the media (a reordering
    // encoder's delay), and where it ends.
    let start = track.edit.map_or(0, |edit| edit.media_start);
    let end = track
        .edit
        .and_then(|edit| edit.length)
        .filter(|_| movie.timescale > 0)
        .map(|length| length as f64 * 1e6 / f64::from(movie.timescale));
    let mut reader = Reader::new(file);
    let (mut fed, mut seen, mut kept) = (0usize, 0usize, 0usize);
    let mut last_key = f64::NEG_INFINITY;
    let mut flushing = false;
    let mut silence = 0u32;
    loop {
        if job.stopped() {
            return None;
        }
        job.progress(seen, track.samples.len());
        while fed < track.samples.len()
            && fed - seen < FRAMES_AHEAD
            && encoder.codec.encode_queue_size() < QUEUE
        {
            let sample = &track.samples[fed];
            let bytes = reader.read(sample.offset, sample.size).await?;
            let chunk = EncodedVideoChunk::new(&object(&[
                (
                    "type",
                    JsValue::from_str(if sample.sync { "key" } else { "delta" }),
                ),
                (
                    "timestamp",
                    JsValue::from(micros(sample.pts - start, timescale)),
                ),
                (
                    "duration",
                    JsValue::from(micros(i64::from(sample.duration), timescale)),
                ),
                ("data", bytes.into()),
            ]))
            .ok()?;
            decoder.codec.decode(&chunk).ok()?;
            fed += 1;
        }
        if fed == track.samples.len() && !flushing {
            flushing = true;
            when_drained(decoder.codec.flush(), &wake);
        }
        match next(&mut events, &mut silence).await? {
            Event::Frame(frame) => {
                seen += 1;
                let shown = frame.0.timestamp();
                // Before the edit starts, or after it ends: decoded, because
                // the frames around it need it, and never shown.
                if shown < 0.0 || end.is_some_and(|end| shown >= end) {
                    continue;
                }
                if let Some(gate) = gate.as_mut() {
                    // In the file's own ticks, not the decoder's rounded
                    // microseconds: a third of a microsecond is enough to
                    // make two frames of a 60 fps clip come to a hair
                    // under one frame of 30, and the gate to miss a beat.
                    let ticks = (shown * f64::from(timescale) / 1e6).round();
                    if !gate.keep(ticks / f64::from(timescale)) {
                        continue;
                    }
                }
                // The first frame is shown from the very start of the
                // track, as a player shows it: nothing written here says
                // otherwise. (Where the TRACK starts is its lead's to say.)
                let shown = if kept == 0 { 0.0 } else { shown };
                let key = shown - last_key >= KEYFRAME_SECONDS * 1e6;
                if key {
                    last_key = shown;
                }
                let drawn = Frame(easel.paint(&frame.0, shown)?);
                drop(frame);
                encoder
                    .codec
                    .encode(&drawn.0, &object(&[("keyFrame", JsValue::from(key))]))
                    .ok()?;
                kept += 1;
            }
            Event::Drained(true) => break,
            Event::Drained(false) | Event::Failed => return None,
            Event::Sound(_) | Event::Progress => {}
        }
    }
    drop(decoder);
    if !settled(encoder.codec.flush()).await {
        return None;
    }
    drop(encoder);
    let sink = std::mem::take(&mut *sink.borrow_mut());
    // Every frame that went in came out, the first of them a key frame, and
    // the encoder said how to decode them — or this is not a file.
    if sink.failed || kept == 0 || sink.chunks.len() != kept || !sink.chunks[0].2 {
        log::warn!(
            "A video encoder made {} frames of {kept}; not used",
            sink.chunks.len()
        );
        return None;
    }
    let avcc = sink.avcc?;
    let colour = match sink.colour.as_ref().map(colour) {
        // An encoder that says it wrote HDR did not write the profile.
        Some(Some(colour)) if matches!(colour.transfer, 16 | 18) => return None,
        Some(colour) => colour,
        None => None,
    };
    let pts: Vec<i64> = sink
        .chunks
        .iter()
        .map(|(_, shown, _)| (shown * f64::from(timescale) / 1e6).round() as i64)
        .collect();
    // The last frame lasts as long as one frame of the target's rate.
    let last = (f64::from(timescale) / target.frame_rate).round().max(1.0) as u32;
    let samples = mp4::decode_timeline(&pts, last)
        .into_iter()
        .zip(&sink.chunks)
        .map(
            |((duration, composition_offset), (bytes, _, key))| mp4::Sample {
                size: bytes.length(),
                duration,
                composition_offset,
                sync: *key,
            },
        )
        .collect();
    let picture = mp4::Track {
        timescale,
        media: Media::Video {
            width,
            height,
            avcc,
            colour,
        },
        samples,
        skip: 0,
        lead,
    };
    let parts = sink
        .chunks
        .into_iter()
        .map(|(bytes, _, _)| bytes.into())
        .collect();
    Some((picture, parts))
}

/// A `VideoColorSpace` — names — as the code points a `colr` box carries
/// (ISO/IEC 23091-2). None when the encoder named something this does not
/// know: no `colr` is written, and a player assumes BT.709, which is what a
/// canvas's SDR frames are encoded as.
fn colour(space: &JsValue) -> Option<mp4::Colour> {
    let named = |key: &str| {
        Reflect::get(space, &JsValue::from_str(key))
            .ok()
            .and_then(|value| value.as_string())
    };
    let primaries = match named("primaries")?.as_str() {
        "bt709" => 1,
        "bt470bg" => 5,
        "smpte170m" => 6,
        "bt2020" => 9,
        "smpte432" => 12,
        _ => return None,
    };
    let transfer = match named("transfer")?.as_str() {
        "bt709" => 1,
        "smpte170m" => 6,
        "linear" => 8,
        "iec61966-2-1" => 13,
        "pq" => 16,
        "hlg" => 18,
        _ => return None,
    };
    let matrix = match named("matrix")?.as_str() {
        "rgb" => 0,
        "bt709" => 1,
        "bt470bg" => 5,
        "smpte170m" => 6,
        "bt2020-ncl" => 9,
        _ => return None,
    };
    let full_range = Reflect::get(space, &JsValue::from_str("fullRange"))
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    Some(mp4::Colour {
        primaries,
        transfer,
        matrix,
        full_range,
    })
}

// --- a recording's sound, for its text ----------------------------------------------------------

/// The sound of `file` — a video or a sound file the chat holds, of type
/// `mime` where its metadata says — as an M4A of AAC to send for its text,
/// within `max_bytes` (docs/protocol.md, "Transcripts on request"). What
/// becomes of it is fc_text::transcript_sound's decision; this is the part
/// only a browser can do, with the same reader, writer and encoder a picked
/// file is prepared with. Nothing of a picture is ever decoded.
///
/// - An MP4 or QuickTime file is read by its index. AAC that fits is
///   COPIED: its frames read by the offsets the index gives and written
///   behind a new index, byte for byte.
/// - Anything else — AAC too big to copy, ALAC, an Ogg, MP3 or WAV file — is
///   decoded by the browser's own `decodeAudioData` (AAC from the frames
///   copied out first, so the video around them is never read), mixed down
///   to one channel, and encoded as 64 kbit/s AAC-LC.
///
/// The error is why there is nothing to send: [`TextFailure::TooLong`] or
/// [`TextFailure::Unreadable`], each terminal.
pub async fn sound_for_text(
    file: &Blob,
    mime: Option<&str>,
    max_bytes: u64,
) -> Result<Blob, TextFailure> {
    let index = movie(file).await;
    let (found, copy) = match &index {
        Some(index) => match transcript_sound::in_movie(index) {
            Some(sound) => (Some(sound.found), sound.copy),
            // A film with no sound track: nothing to hear.
            None => (None, None),
        },
        None => {
            let container = mime.map(fc_text::media::essence).unwrap_or_default();
            // A video is an MP4 or a QuickTime movie, or it is nothing
            // this client can read.
            if container.starts_with("video/") {
                (None, None)
            } else {
                let probe = probe_audio(file, &container).await;
                let found = Found {
                    copy_bytes: None,
                    duration_ms: probe.source.duration_ms,
                    sample_rate: probe.sample_rate,
                    channels: probe.source.channels,
                };
                (Some(found), None)
            }
        }
    };
    let mut plan = transcript_sound::extraction(found.as_ref(), max_bytes, true);
    if let Extraction::Encode { sample_rate } = plan {
        if !webcodecs::aac_supported(sample_rate, TEXT_CHANNELS, TEXT_BITRATE).await {
            plan = transcript_sound::extraction(found.as_ref(), max_bytes, false);
        }
    }
    let made = match plan {
        Extraction::TooLong => return Err(TextFailure::TooLong),
        Extraction::Unreadable => return Err(TextFailure::Unreadable),
        Extraction::Copy => {
            let (Some(index), Some(copy)) = (index.as_ref(), copy.as_ref()) else {
                return Err(TextFailure::Unreadable);
            };
            copied(file, index, copy)
                .await
                .ok_or(TextFailure::Unreadable)?
        }
        Extraction::Encode { sample_rate } => {
            // AAC is decoded from its own frames, copied out of the file:
            // the browser's decoder is handed a few megabytes of sound, not
            // the gigabyte of video around it.
            let source = match (index.as_ref(), copy.as_ref()) {
                (Some(index), Some(copy)) => copied(file, index, copy)
                    .await
                    .ok_or(TextFailure::Unreadable)?,
                _ => file.clone(),
            };
            let found = found.unwrap_or_default();
            let whole = AudioSource {
                container: String::new(),
                codec: String::new(),
                channels: found.channels,
                bitrate: None,
                size_bytes: source.size() as u64,
                duration_ms: found.duration_ms,
            };
            if !could_decode(&whole, sample_rate) {
                return Err(TextFailure::Unreadable);
            }
            one_voice(&source, sample_rate)
                .await
                .ok_or(TextFailure::Unreadable)?
        }
    };
    if transcript_sound::fits(made.size() as u64, max_bytes) {
        Ok(made)
    } else {
        Err(TextFailure::TooLong)
    }
}

/// `copy`'s frames, read out of `file` by the offsets `index` gives for
/// them, behind an M4A's own index — the sound as it was, and only the
/// sound.
async fn copied(file: &Blob, index: &Movie, copy: &mp4::Track) -> Option<Blob> {
    let track = index.audio()?;
    if track.samples.len() != copy.samples.len() {
        return None;
    }
    let mut reader = Reader::new(file);
    let mut parts = Vec::with_capacity(track.samples.len());
    for sample in &track.samples {
        let bytes = reader.read(sample.offset, sample.size).await?;
        // A copy of its own: a view would keep its whole window.
        parts.push(JsValue::from(bytes.slice(0, bytes.length())));
    }
    let layout = mp4::layout(std::slice::from_ref(copy), Brand::M4a);
    let blob = assemble(
        &layout.header,
        layout
            .order
            .iter()
            .map(|&(_, sample)| parts[sample].clone()),
        "audio/mp4",
    )?;
    (blob.size() as u64 == layout.total_len).then_some(blob)
}

/// `file` decoded whole by the browser at `sample_rate` — which also
/// resamples it, from an 8 kHz phone call or a 16 kHz voice note — its
/// channels mixed down to one, and encoded at [`TEXT_BITRATE`]. None when
/// the browser does not decode it or does not encode the result.
async fn one_voice(file: &Blob, sample_rate: u32) -> Option<Blob> {
    let buffer = JsFuture::from(file.array_buffer()).await.ok()?;
    let context =
        web_sys::OfflineAudioContext::new_with_number_of_channels_and_length_and_sample_rate(
            1,
            1,
            sample_rate as f32,
        )
        .ok()?;
    let decoding = context.decode_audio_data(buffer.unchecked_ref()).ok()?;
    let decoded: web_sys::AudioBuffer = JsFuture::from(decoding).await.ok()?.dyn_into().ok()?;
    let channels = decoded.number_of_channels();
    let frames = decoded.length();
    let rate = decoded.sample_rate().round() as u32;
    if channels == 0 || frames == 0 || rate == 0 {
        return None;
    }
    let samples: &Samples = decoded.unchecked_ref();
    let planes: Vec<Float32Array> = (0..channels)
        .map(|index| samples.channel(index).ok())
        .collect::<Option<_>>()?;
    let mut at = 0u32;
    let blocks = move || {
        if at >= frames {
            return None;
        }
        let count = SOUND_BLOCK.min(frames - at);
        let data = if planes.len() == 1 {
            planes[0].slice(at, at + count)
        } else {
            let mut mixed = vec![0f32; count as usize];
            for plane in &planes {
                for (sum, sample) in mixed
                    .iter_mut()
                    .zip(plane.subarray(at, at + count).to_vec())
                {
                    *sum += sample;
                }
            }
            let share = planes.len() as f32;
            mixed.iter_mut().for_each(|sum| *sum /= share);
            Float32Array::from(mixed.as_slice())
        };
        at += count;
        Some((data, count))
    };
    let aac = aac_from_pcm(blocks, rate, TEXT_CHANNELS, TEXT_BITRATE).await?;
    m4a(&aac)
}

/// What the tests of this module and of prep.rs make and read files with.
#[cfg(test)]
pub mod testing {
    use super::*;

    pub const QUICKTIME: &[u8] = include_bytes!("../text/fixtures/quicktime-h264-360p.mov");
    pub const WITHIN: &[u8] = include_bytes!("../text/fixtures/within-h264-720p30.mp4");
    pub const PORTRAIT: &[u8] = include_bytes!("../text/fixtures/portrait-hevc-hlg-4k60.mov");

    pub fn blob(bytes: &[u8], mime: &str) -> Blob {
        let options = BlobPropertyBag::new();
        options.set_type(mime);
        let parts = Array::of1(&Uint8Array::from(bytes));
        Blob::new_with_u8_array_sequence_and_options(&parts, &options).unwrap()
    }

    pub async fn whole(blob: &Blob) -> Vec<u8> {
        Uint8Array::new(&JsFuture::from(blob.array_buffer()).await.unwrap()).to_vec()
    }

    /// A file's top-level boxes in order, and its index.
    pub fn read(file: &[u8]) -> (Vec<[u8; 4]>, Movie) {
        let mut kinds = Vec::new();
        let mut movie = None;
        let mut at = 0usize;
        while let Some(header) = mp4_read::box_header(&file[at..]) {
            let size = header.size.unwrap_or((file.len() - at) as u64) as usize;
            if &header.kind == b"moov" {
                movie = Some(mp4_read::parse_moov(&file[at..at + size]).unwrap());
            }
            kinds.push(header.kind);
            at += size;
            if at >= file.len() {
                break;
            }
        }
        (kinds, movie.expect("a moov"))
    }

    /// A deterministic stand-in for a microphone: a tone with noise under
    /// it, so that an encoder's rate control has something to work on —
    /// a pure tone, like silence, encodes to almost nothing at any rate.
    pub fn noise(frames: u32, sample_rate: u32, seed: u32) -> Vec<f32> {
        let mut state = seed.max(1);
        (0..frames)
            .map(|index| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let hiss = (state as f32 / u32::MAX as f32) - 0.5;
                let phase = index as f32 * 440.0 * std::f32::consts::TAU / sample_rate as f32;
                0.4 * phase.sin() + 0.3 * hiss
            })
            .collect()
    }

    /// `seconds` of sound as the browser's decoder hears `file`.
    pub async fn heard(file: &Blob, sample_rate: u32) -> web_sys::AudioBuffer {
        let context =
            web_sys::OfflineAudioContext::new_with_number_of_channels_and_length_and_sample_rate(
                1,
                1,
                sample_rate as f32,
            )
            .unwrap();
        let bytes = JsFuture::from(file.array_buffer()).await.unwrap();
        let decoding = context.decode_audio_data(bytes.unchecked_ref()).unwrap();
        JsFuture::from(decoding)
            .await
            .expect("the browser's own decoder reads the file")
            .unchecked_into()
    }

    /// A source being put together: its tracks, and each one's samples —
    /// kept apart until [`Reel::bound`], so that a test can make the file
    /// say what a camera's would (a track that starts late, an edit that
    /// skips an encoder's priming, a header that miscounts its channels).
    pub struct Reel {
        pub tracks: Vec<mp4::Track>,
        pub parts: Vec<Vec<JsValue>>,
    }

    impl Reel {
        /// The file: an MP4, its index first.
        pub fn bound(&self) -> Blob {
            let layout = mp4::layout(&self.tracks, Brand::Mp4);
            assemble(
                &layout.header,
                layout
                    .order
                    .iter()
                    .map(|&(track, sample)| self.parts[track][sample].clone()),
                "video/mp4",
            )
            .unwrap()
        }
    }

    /// What a camera would have made: `seconds` of `width × height` video at
    /// `fps`, H.264 at `bitrate`, with stereo AAC at `sound_bitrate` when
    /// there is one.
    pub async fn film(
        width: u32,
        height: u32,
        fps: u32,
        seconds: u32,
        bitrate: u64,
        sound_bitrate: Option<u64>,
    ) -> Blob {
        // 600 ticks a second counts 24, 25, 30 and 60 fps exactly.
        let times: Vec<i64> = (0..fps * seconds)
            .map(|index| i64::from(index * (600 / fps)))
            .collect();
        let sound = sound_bitrate.map(|bitrate| (bitrate, 2));
        shot(width, height, 600, &times, 600 / fps, bitrate, sound)
            .await
            .bound()
    }

    /// The same, frame by frame: one frame shown at each of `times`, in
    /// ticks of `timescale` (the last of them for `last`), so that a frame
    /// rate can be 29.97 or no single rate at all; and `sound` as
    /// `(bit/s, channels)`. Moving, noisy content, because an encoder's rate
    /// control only shows itself on a picture that is hard to encode: a
    /// flat-colour clip comes out tiny at any bitrate asked for, and says
    /// nothing about whether the bitrate governs.
    pub async fn shot(
        width: u32,
        height: u32,
        timescale: u32,
        times: &[i64],
        last: u32,
        bitrate: u64,
        sound: Option<(u64, u32)>,
    ) -> Reel {
        let document = web_sys::window().unwrap().document().unwrap();
        let canvas: HtmlCanvasElement = document.create_element("canvas").unwrap().unchecked_into();
        canvas.set_width(width);
        canvas.set_height(height);
        let context: CanvasRenderingContext2d =
            canvas.get_context("2d").unwrap().unwrap().unchecked_into();
        let ticks = times.last().unwrap() + i64::from(last);
        let length = ticks as f64 / f64::from(timescale);
        let config = webcodecs::h264_config(width, height, times.len() as f64 / length, bitrate)
            .await
            .expect("the test browser encodes H.264");
        let (wake, mut events) = mpsc::unbounded();
        let sink = Rc::new(RefCell::new(PictureSink::default()));
        let encoder = {
            let sink = sink.clone();
            let waker = wake.clone();
            let (init, output, failure) = callbacks(
                move |chunk: JsValue, metadata: JsValue| {
                    let chunk: EncodedVideoChunk = chunk.unchecked_into();
                    let bytes = Uint8Array::new_with_length(chunk.byte_length());
                    chunk.copy_to(&bytes).unwrap();
                    let mut sink = sink.borrow_mut();
                    sink.chunks
                        .push((bytes, chunk.timestamp(), chunk.kind() == "key"));
                    if sink.avcc.is_none() {
                        sink.avcc = decoder_config_field(&metadata, "description")
                            .and_then(|description| bytes_of(&description));
                    }
                    let _ = waker.unbounded_send(Event::Progress);
                },
                &wake,
            );
            Codec {
                codec: VideoEncoder::new(&init).unwrap(),
                _output: output,
                _failure: failure,
            }
        };
        encoder.codec.configure(&config).unwrap();
        let mut state = 0x2545_F491u32;
        let mut random = move |below: u32| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state % below
        };
        // A scene: a backdrop that stays, things that move across it a
        // little every frame, and grain — what a camera sees. Not noise
        // from edge to edge, which no bitrate holds: with nothing in one
        // frame to predict the next from, an encoder is at its coarsest
        // quantiser and still over, and the test would measure that.
        let things: Vec<(u32, u32, u32, u32, String)> = (0..24)
            .map(|_| {
                (
                    random(width),
                    random(height),
                    random(7) + 1,
                    random(5) + 1,
                    format!("rgb({},{},{})", random(256), random(256), random(256)),
                )
            })
            .collect();
        let mut last_key = f64::NEG_INFINITY;
        for &time in times {
            let at = time as f64 / f64::from(timescale);
            context.set_fill_style_str("#2f4f4f");
            context.fill_rect(0.0, 0.0, f64::from(width), f64::from(height));
            for (row, colour) in ["#6b8e23", "#8b4513", "#4682b4"].iter().enumerate() {
                context.set_fill_style_str(colour);
                context.fill_rect(
                    0.0,
                    f64::from(height) * (0.25 + 0.25 * row as f64),
                    f64::from(width),
                    f64::from(height) * 0.1,
                );
            }
            // Per SECOND, so that the same scene is the same motion at any
            // frame rate.
            let moved = (at * 60.0).round() as u32;
            for (x, y, dx, dy, colour) in &things {
                context.set_fill_style_str(colour);
                context.fill_rect(
                    f64::from((x + dx * moved) % width),
                    f64::from((y + dy * moved) % height),
                    f64::from(width / 9),
                    f64::from(height / 7),
                );
            }
            for _ in 0..400 {
                context.set_fill_style_str(&format!(
                    "rgb({},{},{})",
                    random(256),
                    random(256),
                    random(256)
                ));
                context.fill_rect(
                    f64::from(random(width)),
                    f64::from(random(height)),
                    3.0,
                    3.0,
                );
            }
            let frame = VideoFrame::new_from_image(
                canvas.as_ref(),
                &object(&[("timestamp", JsValue::from(micros(time, timescale)))]),
            )
            .unwrap();
            // A key frame a second.
            let key = at - last_key >= 1.0;
            if key {
                last_key = at;
            }
            encoder
                .codec
                .encode(&frame, &object(&[("keyFrame", JsValue::from(key))]))
                .unwrap();
            frame.close();
            while encoder.codec.encode_queue_size() > QUEUE {
                let mut silence = 0;
                assert!(!matches!(
                    next(&mut events, &mut silence).await,
                    Some(Event::Failed) | None
                ));
            }
        }
        assert!(settled(encoder.codec.flush()).await);
        drop(encoder);
        let sink = std::mem::take(&mut *sink.borrow_mut());
        assert_eq!(sink.chunks.len(), times.len());
        let pts: Vec<i64> = sink
            .chunks
            .iter()
            .map(|(_, shown, _)| (shown * f64::from(timescale) / 1e6).round() as i64)
            .collect();
        let samples = mp4::decode_timeline(&pts, last)
            .into_iter()
            .zip(&sink.chunks)
            .map(
                |((duration, composition_offset), (bytes, _, key))| mp4::Sample {
                    size: bytes.length(),
                    duration,
                    composition_offset,
                    sync: *key,
                },
            )
            .collect();
        let mut tracks = vec![mp4::Track {
            timescale,
            media: Media::Video {
                width: width as u16,
                height: height as u16,
                avcc: sink.avcc.unwrap(),
                colour: None,
            },
            samples,
            skip: 0,
            lead: 0,
        }];
        let mut parts: Vec<Vec<JsValue>> = vec![sink
            .chunks
            .into_iter()
            .map(|(bytes, _, _)| bytes.into())
            .collect()];
        if let Some((sound_bitrate, channels)) = sound {
            let aac = voices((length * 48_000.0).round() as u32, channels, sound_bitrate).await;
            tracks.push(aac.track(0, 0));
            parts.push(aac.parts());
        }
        Reel { tracks, parts }
    }

    /// `frames` of 48 kHz sound — a different noise in each of `channels` —
    /// as AAC-LC at `bitrate`.
    pub async fn voices(frames: u32, channels: u32, bitrate: u64) -> Aac {
        let planes: Vec<Vec<f32>> = (0..channels)
            .map(|channel| noise(frames, 48_000, 7 + 4 * channel))
            .collect();
        let mut at = 0usize;
        aac_from_pcm(
            move || {
                if at >= frames as usize {
                    return None;
                }
                let count = (SOUND_BLOCK as usize).min(frames as usize - at);
                let data = Float32Array::new_with_length(channels * count as u32);
                for (index, plane) in planes.iter().enumerate() {
                    data.set(
                        &Float32Array::from(&plane[at..at + count]),
                        (index * count) as u32,
                    );
                }
                at += count;
                Some((data, count as u32))
            },
            48_000,
            channels,
            bitrate,
        )
        .await
        .expect("the test browser encodes AAC")
    }

    /// The picture of `file` — an MP4 or QuickTime movie — as a track to
    /// write again, frame for frame: its sizes, the order its encoder put
    /// it in, and when each frame is shown, counted from where its edit
    /// starts. It is how a test gets a source with B-frames in it, which a
    /// browser's own encoder does not make to order.
    pub fn picture_of(file: &[u8]) -> (mp4::Track, Vec<JsValue>) {
        let (_, movie) = read(file);
        let track = movie.video().unwrap();
        let entry = track.entry.as_ref().unwrap();
        let start = track.edit.map_or(0, |edit| edit.media_start);
        let parts = track
            .samples
            .iter()
            .map(|sample| {
                let at = sample.offset as usize;
                Uint8Array::from(&file[at..at + sample.size as usize]).into()
            })
            .collect();
        let picture = mp4::Track {
            timescale: track.timescale,
            media: Media::Video {
                width: entry.width,
                height: entry.height,
                avcc: entry.config.clone(),
                colour: None,
            },
            samples: track
                .samples
                .iter()
                .map(|sample| mp4::Sample {
                    size: sample.size,
                    duration: sample.duration,
                    composition_offset: (sample.pts - start - sample.dts) as i32,
                    sync: sample.sync,
                })
                .collect(),
            skip: 0,
            lead: 0,
        };
        (picture, parts)
    }

    /// How long the browser's own player says `file` is, in seconds.
    pub async fn length(file: &Blob) -> f64 {
        let document = web_sys::window().unwrap().document().unwrap();
        let video: web_sys::HtmlVideoElement =
            document.create_element("video").unwrap().unchecked_into();
        let url = web_sys::Url::create_object_url_with_blob(file).unwrap();
        video.set_muted(true);
        video.set_preload("auto");
        video.set_src(&url);
        assert!(
            crate::prep::wait_for(&video, "loadedmetadata", 10_000).await,
            "it plays"
        );
        let seconds = video.duration();
        video.set_src("");
        let _ = web_sys::Url::revoke_object_url(&url);
        seconds
    }

    /// The colour in the middle of each half of the first frame a player
    /// shows of `file`: `(left, right)`, each `[r, g, b]`.
    pub async fn halves(file: &Blob) -> ([u8; 3], [u8; 3]) {
        let document = web_sys::window().unwrap().document().unwrap();
        let video: web_sys::HtmlVideoElement =
            document.create_element("video").unwrap().unchecked_into();
        let url = web_sys::Url::create_object_url_with_blob(file).unwrap();
        video.set_muted(true);
        video.set_preload("auto");
        video.set_src(&url);
        assert!(
            crate::prep::wait_for(&video, "loadeddata", 10_000).await,
            "it plays"
        );
        let (width, height) = (video.video_width(), video.video_height());
        let canvas: HtmlCanvasElement = document.create_element("canvas").unwrap().unchecked_into();
        canvas.set_width(width);
        canvas.set_height(height);
        let context: CanvasRenderingContext2d =
            canvas.get_context("2d").unwrap().unwrap().unchecked_into();
        context
            .draw_image_with_html_video_element(&video, 0.0, 0.0)
            .unwrap();
        let pixel = |x: u32| {
            let data = context
                .get_image_data(f64::from(x), f64::from(height / 2), 1.0, 1.0)
                .unwrap()
                .data();
            [data[0], data[1], data[2]]
        };
        let halves = (pixel(width / 4), pixel(3 * width / 4));
        video.set_src("");
        let _ = web_sys::Url::revoke_object_url(&url);
        halves
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use crate::webcodecs::testing::*;
    use wasm_bindgen_test::*;

    fn pcm_blocks(samples: Vec<f32>) -> impl FnMut() -> Option<(Float32Array, u32)> {
        let mut at = 0usize;
        move || {
            if at >= samples.len() {
                return None;
            }
            let count = 4096.min(samples.len() - at);
            let block = Float32Array::from(&samples[at..at + count]);
            at += count;
            Some((block, count as u32))
        }
    }

    /// The muxer's output, made from a real encoder's frames, is a file a
    /// real decoder reads: index first, one mono AAC-LC track at the rate
    /// asked for, as long as what went in.
    #[wasm_bindgen_test]
    async fn pcm_becomes_an_m4a_the_browser_itself_can_play() {
        let aac = aac_from_pcm(pcm_blocks(noise(48_000 * 4, 48_000, 3)), 48_000, 1, 64_000)
            .await
            .expect("AAC");
        let file = m4a(&aac).unwrap();
        assert_eq!(file.type_(), "audio/mp4");
        let bytes = whole(&file).await;
        assert_eq!(&bytes[4..12], b"ftypM4A ");
        let (kinds, movie) = read(&bytes);
        assert_eq!(
            kinds,
            vec![*b"ftyp", *b"moov", *b"mdat"],
            "the index comes first"
        );
        let track = movie.audio().unwrap();
        let entry = track.entry.as_ref().unwrap();
        assert_eq!(&entry.format, b"mp4a");
        let config = mp4_read::audio_config(&entry.config).unwrap();
        assert_eq!(
            (config.object_type, config.sample_rate, config.channels),
            (2, 48_000, 1),
            "AAC-LC, mono"
        );
        // The rate GOVERNS: noise is as hard as sound gets, and it still
        // comes to 64 000, not to whatever the encoder felt like.
        let rate = track.data_rate().unwrap();
        assert!((48_000..=72_000).contains(&rate), "{rate} bit/s");
        // Every frame is where the index says it is.
        for sample in &track.samples {
            assert!(sample.offset + u64::from(sample.size) <= bytes.len() as u64);
        }
        let decoded = heard(&file, 48_000).await;
        assert_eq!(decoded.number_of_channels(), 1);
        let seconds = decoded.duration();
        assert!((3.95..=4.15).contains(&seconds), "{seconds} s");
        let samples = decoded.get_channel_data(0).unwrap();
        let power = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
        assert!(power > 0.01, "it is the sound, not silence: {power}");
    }

    /// Every way a browser can lack the encoder is None — rule C — and
    /// never an exception.
    #[wasm_bindgen_test]
    async fn no_encoder_is_none_not_an_error() {
        let samples = noise(48_000, 48_000, 3);
        {
            let _gone = without("AudioEncoder");
            assert!(aac_from_pcm(pcm_blocks(samples.clone()), 48_000, 1, 64_000)
                .await
                .is_none());
        }
        {
            // There, and throwing when it is configured.
            let throws = js_sys::Function::new_no_args(
                "const C = function() {}; \
                 C.prototype.configure = () => { throw new Error('no'); }; \
                 C.prototype.close = () => {}; return C;",
            )
            .call0(&JsValue::NULL)
            .unwrap();
            let _broken = Stand::in_for("AudioEncoder", &throws);
            assert!(aac_from_pcm(pcm_blocks(samples.clone()), 48_000, 1, 64_000)
                .await
                .is_none());
        }
        {
            // There, configured, and reporting an error on the first frame.
            let fails = js_sys::Function::new_no_args(
                "const C = function(init) { this.init = init; this.encodeQueueSize = 0; }; \
                 C.prototype.configure = () => {}; \
                 C.prototype.encode = function() { this.init.error(new Error('no')); }; \
                 C.prototype.flush = () => Promise.reject(new Error('no')); \
                 C.prototype.close = () => {}; return C;",
            )
            .call0(&JsValue::NULL)
            .unwrap();
            let _failing = Stand::in_for("AudioEncoder", &fails);
            assert!(aac_from_pcm(pcm_blocks(samples.clone()), 48_000, 1, 64_000)
                .await
                .is_none());
        }
        {
            // An encoder that answers with half the sound is refused: the
            // index would claim frames that are not there.
            let short = js_sys::Function::new_no_args(
                "const C = function(init) { this.init = init; this.encodeQueueSize = 0; }; \
                 C.prototype.configure = () => {}; \
                 C.prototype.encode = () => {}; \
                 C.prototype.flush = function() { \
                     this.init.output({byteLength: 4, copyTo: () => {}}, undefined); \
                     return Promise.resolve(); }; \
                 C.prototype.close = () => {}; return C;",
            )
            .call0(&JsValue::NULL)
            .unwrap();
            let _short = Stand::in_for("AudioEncoder", &short);
            assert!(aac_from_pcm(pcm_blocks(samples), 48_000, 1, 64_000)
                .await
                .is_none());
        }
    }

    /// The index is found wherever it is, and a file that has none — or is
    /// not an MP4 at all — is None.
    #[wasm_bindgen_test]
    async fn the_index_is_found_at_the_front_or_the_back() {
        let back = movie(&blob(WITHIN, "video/mp4")).await.expect("moov last");
        assert_eq!(
            back.video().unwrap().samples.len(),
            read(WITHIN).1.video().unwrap().samples.len()
        );
        assert!(movie(&blob(QUICKTIME, "video/quicktime")).await.is_some());
        assert!(
            movie(&blob(b"not a movie at all, not even close", "video/mp4"))
                .await
                .is_none()
        );
        assert!(movie(&blob(&WITHIN[..WITHIN.len() / 2], "video/mp4"))
            .await
            .is_none());
        assert!(movie(&blob(&[], "video/mp4")).await.is_none());
    }

    /// Rule A: a clip already within the profile is not touched.
    #[wasm_bindgen_test]
    async fn a_clip_within_the_profile_is_kept() {
        let kept = video(&blob(WITHIN, "video/mp4"), "video/mp4", &Job::default()).await;
        assert!(matches!(kept, Some(Planned::Keep)));
    }

    /// The whole path on a camera-shaped clip: 720p at 60 fps and 8 Mbit/s
    /// with 256 kbit/s stereo sound becomes the profile — every other frame,
    /// at the planner's bitrate, its sound re-encoded at 128 000, `moov`
    /// first — and the browser's own player plays it.
    #[wasm_bindgen_test]
    async fn a_720p60_clip_comes_out_at_30_frames_and_the_planners_bitrate() {
        let source = film(1280, 720, 60, 3, 8_000_000, Some(256_000)).await;
        let source_bytes = whole(&source).await;
        let (_, before) = read(&source_bytes);
        let probed =
            media_probe::video_source(&before, "video/mp4", source_bytes.len() as u64).unwrap();
        assert_eq!(probed.frame_rate, Some(60.0));
        let VideoPlan::Transcode(target) = media_plan::plan_video(&probed) else {
            panic!("the fixture is outside the profile: {probed:?}");
        };
        assert_eq!(
            (target.width, target.height, target.frame_rate),
            (1280, 720, 30.0)
        );
        assert_eq!(target.video_bitrate, 2_000_000);
        assert_eq!(target.audio_bitrate, Some(128_000));
        // The fixture itself is what it claims — or the test proves nothing.
        let source_rate = before.video().unwrap().data_rate().unwrap();
        assert!(
            source_rate > 4_000_000,
            "the source runs at {source_rate} bit/s"
        );

        let Some(Planned::Made(made)) = video(&source, "video/mp4", &Job::default()).await else {
            panic!("not transcoded");
        };
        let bytes = whole(&made).await;
        assert_eq!(made.type_(), "video/mp4");
        let (kinds, after) = read(&bytes);
        assert_eq!(
            kinds,
            vec![*b"ftyp", *b"moov", *b"mdat"],
            "the index comes first"
        );
        let picture = after.video().unwrap();
        let entry = picture.entry.as_ref().unwrap();
        assert_eq!(&entry.format, b"avc1");
        assert_eq!((entry.width, entry.height), (1280, 720));
        assert_eq!(picture.rotation, Some(0));
        // High profile, as asked.
        assert_eq!(entry.config[1], 0x64, "{:02x?}", &entry.config[..4]);
        assert_eq!(picture.samples.len(), 90, "every other frame of 180");
        let rate = picture.frame_rate().unwrap();
        assert!((rate - 30.0).abs() < 0.01, "{rate} fps");
        assert!(picture.samples[0].sync);
        // A key frame at least every two seconds.
        let keys: Vec<i64> = picture
            .samples
            .iter()
            .filter(|s| s.sync)
            .map(|s| s.pts)
            .collect();
        assert!(
            keys.windows(2).all(|pair| pair[1] - pair[0] <= 2 * 600),
            "{keys:?}"
        );
        // THE BITRATE GOVERNS. The content is as hard as video gets, the
        // source ran at four times the target, and what comes out is the
        // target — within what a real encoder's rate control does over
        // three seconds — not the source's rate, and not "whatever".
        let made_rate = picture.data_rate().unwrap();
        assert!(
            made_rate <= target.video_bitrate * 5 / 4,
            "{made_rate} bit/s of video against a target of {}",
            target.video_bitrate
        );
        assert!(
            made_rate >= target.video_bitrate / 4,
            "{made_rate} bit/s is not an encode"
        );
        assert!(
            bytes.len() < source_bytes.len() / 2,
            "{} of {}",
            bytes.len(),
            source_bytes.len()
        );
        // Its sound: AAC-LC stereo, re-encoded at the row's rate.
        let sound = after.audio().unwrap();
        let config = mp4_read::audio_config(&sound.entry.as_ref().unwrap().config).unwrap();
        assert_eq!(
            (config.object_type, config.sample_rate, config.channels),
            (2, 48_000, 2)
        );
        let sound_rate = sound.data_rate().unwrap();
        assert!(
            (96_000..=144_000).contains(&sound_rate),
            "{sound_rate} bit/s of sound"
        );
        // As long as it was, both halves.
        let length = after.presented_ms(picture).unwrap();
        assert!((2_950..=3_050).contains(&length), "{length} ms");
        let heard = heard(&made, 48_000).await;
        assert!(
            (2.9..=3.2).contains(&heard.duration()),
            "{} s of sound",
            heard.duration()
        );
        for track in &after.tracks {
            for sample in &track.samples {
                assert!(sample.offset + u64::from(sample.size) <= bytes.len() as u64);
            }
        }
        // And it plays.
        halves(&made).await;
    }

    /// 1080p comes down to 720 on its short side, in proportion; its frame
    /// rate, already 30, loses nothing.
    #[wasm_bindgen_test]
    async fn a_1080p_clip_is_scaled_to_720_and_keeps_every_frame() {
        let source = film(1920, 1080, 30, 2, 6_000_000, None).await;
        let Some(Planned::Made(made)) = video(&source, "video/mp4", &Job::default()).await else {
            panic!("not transcoded");
        };
        let bytes = whole(&made).await;
        let (_, after) = read(&bytes);
        let picture = after.video().unwrap();
        let entry = picture.entry.as_ref().unwrap();
        assert_eq!((entry.width, entry.height), (1280, 720));
        assert_eq!(picture.samples.len(), 60, "no frame dropped");
        assert!(after.audio().is_none(), "no sound in, none out");
        let made_rate = picture.data_rate().unwrap();
        assert!(made_rate <= 2_500_000, "{made_rate} bit/s");
    }

    /// A QuickTime movie holding exactly the profile's codecs is outside it
    /// only by its container: its picture is re-encoded at its OWN size —
    /// never scaled up — and its sound, already on the row, is copied frame
    /// for frame.
    #[wasm_bindgen_test]
    async fn a_quicktime_movie_becomes_an_mp4_at_its_own_size_with_its_sound_copied() {
        let (_, before) = read(QUICKTIME);
        let Some(Planned::Made(made)) = video(
            &blob(QUICKTIME, "video/quicktime"),
            "video/quicktime",
            &Job::default(),
        )
        .await
        else {
            panic!("not transcoded");
        };
        let bytes = whole(&made).await;
        assert_eq!(&bytes[4..12], b"ftypisom");
        let (kinds, after) = read(&bytes);
        assert_eq!(kinds, vec![*b"ftyp", *b"moov", *b"mdat"]);
        let picture = after.video().unwrap();
        let entry = picture.entry.as_ref().unwrap();
        assert_eq!((entry.width, entry.height), (640, 360), "never upscaled");
        assert_eq!(picture.samples.len(), before.video().unwrap().samples.len());
        let (was, is) = (before.audio().unwrap(), after.audio().unwrap());
        assert_eq!(is.samples.len(), was.samples.len());
        for (copied, original) in is.samples.iter().zip(&was.samples) {
            let at = |sample: &mp4_read::Sample| {
                sample.offset as usize..(sample.offset as usize + sample.size as usize)
            };
            assert_eq!(bytes[at(copied)], QUICKTIME[at(original)], "byte for byte");
        }
        assert_eq!(
            is.entry.as_ref().unwrap().config,
            was.entry.as_ref().unwrap().config
        );
        // Its priming is still skipped, as the source's edit list skipped it.
        assert_eq!(
            is.edit.map(|edit| edit.media_start),
            was.edit.map(|edit| edit.media_start)
        );
        heard(&made, 44_100).await;
    }

    /// A phone's portrait 4K HDR clip: a landscape HEVC frame with a
    /// quarter turn in its matrix, in HLG, at 60 fps. Where this browser
    /// decodes HEVC it becomes 720 × 1280 at 30, upright with no matrix,
    /// SDR; where it does not, it is None and goes as it went (rule C).
    #[wasm_bindgen_test]
    async fn a_turned_4k60_hdr_clip_becomes_upright_720_by_1280_sdr_at_30() {
        let (_, before) = read(PORTRAIT);
        let entry = before.video().unwrap().entry.as_ref().unwrap();
        let decodes = webcodecs::supported(
            "VideoDecoder",
            &object(&[
                (
                    "codec",
                    JsValue::from_str(&mp4_read::video_codec_string(entry).unwrap()),
                ),
                ("codedWidth", JsValue::from(entry.width)),
                ("codedHeight", JsValue::from(entry.height)),
            ]),
        )
        .await;
        let made = video(
            &blob(PORTRAIT, "video/quicktime"),
            "video/quicktime",
            &Job::default(),
        )
        .await;
        if !decodes {
            console_log!("This browser does not decode HEVC: the clip takes rule C.");
            assert!(made.is_none());
            return;
        }
        let Some(Planned::Made(made)) = made else {
            panic!("not transcoded");
        };
        let bytes = whole(&made).await;
        let (_, after) = read(&bytes);
        let picture = after.video().unwrap();
        let entry = picture.entry.as_ref().unwrap();
        assert_eq!((entry.width, entry.height), (720, 1280));
        assert_eq!(picture.rotation, Some(0), "the turn is in the pixels");
        let frames = before.video().unwrap().samples.len();
        assert_eq!(
            picture.samples.len(),
            frames.div_ceil(2),
            "every other frame"
        );
        assert!(
            !entry.colour.is_some_and(|colour| colour.is_hdr()),
            "{:?}",
            entry.colour
        );
        // 8-bit 4:2:0 High — not the source's 10 bits.
        assert_eq!(entry.config[1], 0x64);
        // The stored frame's TOP half is red and its bottom blue; turned a
        // quarter clockwise, the top is on the RIGHT.
        let (left, right) = halves(&made).await;
        assert!(left[2] > 120 && left[0] < 90, "left is blue: {left:?}");
        assert!(right[0] > 120 && right[2] < 90, "right is red: {right:?}");
    }

    /// Rule C, every way a browser can fall short: no encoder, an encoder
    /// that refuses the configuration, no decoder — and a file whose media
    /// is not what its index says. Each is None; none is an exception.
    #[wasm_bindgen_test]
    async fn what_this_browser_cannot_transcode_is_none() {
        let movie = blob(QUICKTIME, "video/quicktime");
        {
            let _gone = without("VideoEncoder");
            assert!(video(&movie, "video/quicktime", &Job::default())
                .await
                .is_none());
        }
        {
            let _refuses = refusing("VideoEncoder");
            assert!(video(&movie, "video/quicktime", &Job::default())
                .await
                .is_none());
        }
        {
            let _gone = without("VideoDecoder");
            assert!(video(&movie, "video/quicktime", &Job::default())
                .await
                .is_none());
        }
        {
            // The index intact, every byte of the picture garbage.
            let (_, index) = read(QUICKTIME);
            let mut broken = QUICKTIME.to_vec();
            for sample in &index.video().unwrap().samples {
                let at = sample.offset as usize;
                broken[at..at + sample.size as usize].fill(0x5A);
            }
            let broken = blob(&broken, "video/quicktime");
            assert!(video(&broken, "video/quicktime", &Job::default())
                .await
                .is_none());
        }
        // And with all of it back, the same file transcodes.
        assert!(matches!(
            video(&movie, "video/quicktime", &Job::default()).await,
            Some(Planned::Made(_))
        ));
    }

    /// `count` frame times `ticks` apart.
    fn steady(count: u32, ticks: u32) -> Vec<i64> {
        (0..count).map(|index| i64::from(index * ticks)).collect()
    }

    /// A PICTURE that starts after its sound — a phone whose camera came up
    /// a third of a second after its microphone says so with an empty edit
    /// — still starts after it, by as much. Dropped, the transcode would
    /// be a valid, smaller file, out of sync from its first frame to its
    /// last, and rules C and D would send it.
    #[wasm_bindgen_test]
    async fn a_picture_that_starts_late_still_starts_late() {
        let mut reel = shot(
            640,
            360,
            600,
            &steady(120, 10),
            10,
            3_000_000,
            Some((256_000, 2)),
        )
        .await;
        reel.tracks[0].lead = 300;
        let source = reel.bound();
        let source_bytes = whole(&source).await;
        let (_, before) = read(&source_bytes);
        // The fixture says what it is meant to — and is read as saying it.
        let late = before.video().unwrap();
        assert_eq!(late.edit.map(|edit| edit.lead), Some(300));
        assert!(late.edit_supported());
        assert_eq!(before.lead_ms(late), Some(300));

        let Some(Planned::Made(made)) = video(&source, "video/mp4", &Job::default()).await else {
            panic!("not transcoded");
        };
        let bytes = whole(&made).await;
        let (_, after) = read(&bytes);
        let picture = after.video().unwrap();
        assert_eq!(picture.samples.len(), 60, "every other frame of 120");
        assert_eq!(after.lead_ms(picture), Some(300), "still 300 ms late");
        assert_eq!(picture.edit.map(|edit| edit.media_start), Some(0));
        let sound = after.audio().unwrap();
        assert_eq!(after.lead_ms(sound), Some(0), "the sound starts at once");
        // The picture plays for its two seconds AFTER its lead.
        let shown = after.presented_ms(picture).unwrap();
        assert!((1_950..=2_050).contains(&shown), "{shown} ms");
        let whole_length = after.duration_ms().unwrap();
        assert!((2_250..=2_400).contains(&whole_length), "{whole_length} ms");
        // And the browser's own player reads it that way too.
        let played = length(&made).await;
        assert!((2.25..=2.45).contains(&played), "{played} s");
    }

    /// The same for a SOUND that starts late, on both of its routes: copied
    /// frame for frame, and decoded and re-encoded. Its lead and the
    /// priming its edit skips are two different things, and both are kept.
    #[wasm_bindgen_test]
    async fn a_sound_that_starts_late_still_starts_late_copied_or_re_encoded() {
        for (sound_bitrate, copied) in [(96_000, true), (256_000, false)] {
            let mut reel = shot(
                640,
                360,
                600,
                &steady(30, 20),
                20,
                1_000_000,
                Some((sound_bitrate, 2)),
            )
            .await;
            reel.tracks[1].lead = 250;
            reel.tracks[1].skip = 2048;
            let source = reel.bound();
            let source_bytes = whole(&source).await;
            let (_, before) = read(&source_bytes);
            let was = before.audio().unwrap();
            assert_eq!(before.lead_ms(was), Some(250));

            // A QuickTime movie is outside the profile whatever it holds.
            let Some(Planned::Made(made)) =
                video(&source, "video/quicktime", &Job::default()).await
            else {
                panic!("not transcoded");
            };
            let bytes = whole(&made).await;
            let (_, after) = read(&bytes);
            let is = after.audio().unwrap();
            assert_eq!(after.lead_ms(is), Some(250), "still 250 ms late");
            assert_eq!(
                is.edit.map(|edit| edit.media_start),
                Some(2048),
                "and its priming still skipped"
            );
            assert_eq!(after.lead_ms(after.video().unwrap()), Some(0));
            assert_eq!(
                is.samples.len() == was.samples.len()
                    && is.samples.iter().zip(&was.samples).all(|(new, old)| {
                        let at = |sample: &mp4_read::Sample| {
                            sample.offset as usize..(sample.offset as usize + sample.size as usize)
                        };
                        bytes[at(new)] == source_bytes[at(old)]
                    }),
                copied,
                "{sound_bitrate} bit/s"
            );
            let rate = is.data_rate().unwrap();
            assert!(rate <= 144_000, "{rate} bit/s of sound");
            heard(&made, 48_000).await;
        }
    }

    /// A camera's own picture, through the route the fixtures never took:
    /// B-frames — shown in another order than they are stored — beside
    /// sound over the row's rate with an edit that skips its priming, which
    /// is decoded and re-encoded rather than copied.
    #[wasm_bindgen_test]
    async fn reordered_frames_and_an_edit_survive_beside_re_encoded_sound() {
        let (picture, frames) = picture_of(WITHIN);
        // Stored in another order than they are shown: the distance from
        // when a frame is decoded to when it is shown is not one distance.
        let first = picture.samples[0].composition_offset;
        assert!(
            picture
                .samples
                .iter()
                .any(|sample| sample.composition_offset != first),
            "the fixture has B-frames"
        );
        let ticks: u64 = picture
            .samples
            .iter()
            .map(|sample| u64::from(sample.duration))
            .sum();
        let seconds = ticks as f64 / f64::from(picture.timescale);
        let sound = voices((seconds * 48_000.0) as u32, 2, 256_000).await;
        let reel = Reel {
            tracks: vec![picture, sound.track(2112, 0)],
            parts: vec![frames, sound.parts()],
        };
        let source = reel.bound();
        let source_bytes = whole(&source).await;
        let (_, before) = read(&source_bytes);
        let was = before.video().unwrap();

        let Some(Planned::Made(made)) = video(&source, "video/quicktime", &Job::default()).await
        else {
            panic!("not transcoded");
        };
        let bytes = whole(&made).await;
        let (_, after) = read(&bytes);
        // Every frame, each shown when its file said.
        let is = after.video().unwrap();
        assert_eq!(is.timescale, was.timescale);
        let shown = |track: &mp4_read::Track| {
            let mut times: Vec<i64> = track.samples.iter().map(|s| s.pts).collect();
            times.sort_unstable();
            times
        };
        assert_eq!(shown(is), shown(was));
        assert_eq!(is.samples.len(), 15);
        assert_eq!(after.lead_ms(is), Some(0));
        // Its sound: re-encoded to the row, its own priming still skipped.
        let sound = after.audio().unwrap();
        assert_ne!(
            sound.samples.iter().map(|s| s.size).collect::<Vec<_>>(),
            before
                .audio()
                .unwrap()
                .samples
                .iter()
                .map(|s| s.size)
                .collect::<Vec<_>>(),
            "not copied"
        );
        let config = mp4_read::audio_config(&sound.entry.as_ref().unwrap().config).unwrap();
        assert_eq!((config.object_type, config.channels), (2, 2));
        let rate = sound.data_rate().unwrap();
        assert!((64_000..=144_000).contains(&rate), "{rate} bit/s");
        assert_eq!(sound.edit.map(|edit| edit.media_start), Some(2112));
        // And it plays, as long as it was.
        let played = length(&made).await;
        assert!(
            (seconds - 0.05..=seconds + 0.15).contains(&played),
            "{played} s of {seconds} s"
        );
        halves(&made).await;
    }

    /// 29.97 frames a second is within the profile's 30 and is KEPT: every
    /// frame, at its own time to the tick — not squeezed through a gate
    /// meant for 60.
    #[wasm_bindgen_test]
    async fn a_29_97_clip_keeps_every_frame_at_its_own_time() {
        let times = steady(45, 1001);
        let source = shot(640, 360, 30_000, &times, 1001, 2_000_000, None)
            .await
            .bound();
        let source_bytes = whole(&source).await;
        let (_, before) = read(&source_bytes);
        let probed =
            media_probe::video_source(&before, "video/mp4", source_bytes.len() as u64).unwrap();
        let VideoPlan::Transcode(target) = media_plan::plan_video(&probed) else {
            panic!("the fixture is outside the profile: {probed:?}");
        };
        assert!(
            (target.frame_rate - 30_000.0 / 1001.0).abs() < 1e-9,
            "{}",
            target.frame_rate
        );
        let Some(Planned::Made(made)) = video(&source, "video/mp4", &Job::default()).await else {
            panic!("not transcoded");
        };
        let (_, after) = read(&whole(&made).await);
        let picture = after.video().unwrap();
        assert_eq!(picture.timescale, 30_000);
        let mut shown: Vec<i64> = picture.samples.iter().map(|s| s.pts).collect();
        shown.sort_unstable();
        assert_eq!(shown, times, "every frame, to the tick");
        assert!(picture.samples.iter().all(|s| s.duration == 1001));
    }

    /// A clip with no single frame rate — a phone's, in poor light — keeps
    /// each frame's own time where its average is within 30, and is held to
    /// 30 where it is not, without ever bunching two frames closer than
    /// that.
    #[wasm_bindgen_test]
    async fn a_variable_frame_rate_keeps_its_own_times_or_is_held_to_30() {
        // A thirtieth of a second, then a fifteenth: 20 a second on average.
        let mut slow = Vec::new();
        let mut at = 0i64;
        for index in 0..40 {
            slow.push(at);
            at += if index % 2 == 0 { 20 } else { 40 };
        }
        let source = shot(640, 360, 600, &slow, 40, 2_000_000, None)
            .await
            .bound();
        let Some(Planned::Made(made)) = video(&source, "video/mp4", &Job::default()).await else {
            panic!("not transcoded");
        };
        let (_, after) = read(&whole(&made).await);
        let picture = after.video().unwrap();
        let shown: Vec<i64> = picture.samples.iter().map(|s| s.pts).collect();
        assert_eq!(shown, slow, "every frame, when the source showed it");
        assert_eq!(
            picture
                .samples
                .iter()
                .take(4)
                .map(|s| s.duration)
                .collect::<Vec<_>>(),
            vec![20, 40, 20, 40],
            "its uneven pace is kept, not evened out"
        );

        // A sixtieth, then a thirtieth: 40 a second on average.
        let mut fast = Vec::new();
        let mut at = 0i64;
        for index in 0..80 {
            fast.push(at);
            at += if index % 2 == 0 { 10 } else { 20 };
        }
        let source = shot(640, 360, 600, &fast, 20, 3_000_000, None)
            .await
            .bound();
        let Some(Planned::Made(made)) = video(&source, "video/mp4", &Job::default()).await else {
            panic!("not transcoded");
        };
        let (_, after) = read(&whole(&made).await);
        let picture = after.video().unwrap();
        let mut shown: Vec<i64> = picture.samples.iter().map(|s| s.pts).collect();
        shown.sort_unstable();
        assert!(shown.len() < fast.len(), "frames were dropped");
        assert!(shown.len() >= fast.len() / 2, "{} kept", shown.len());
        assert!(
            shown.iter().all(|time| fast.contains(time)),
            "a kept frame is shown when the source showed it"
        );
        assert!(
            shown.windows(2).all(|pair| pair[1] - pair[0] >= 20),
            "never two frames closer than a thirtieth: {shown:?}"
        );
    }

    /// A mono stream under an entry that says two channels — the template
    /// value older muxers leave — is the mono row's: 64 000, one channel.
    /// Read by its entry it would be "stereo at 96 000", within the stereo
    /// row, and copied as it was.
    #[wasm_bindgen_test]
    async fn a_mono_stream_in_a_stereo_entry_is_brought_to_the_mono_row() {
        let mut reel = shot(
            640,
            360,
            600,
            &steady(30, 20),
            20,
            1_000_000,
            Some((96_000, 1)),
        )
        .await;
        let Media::Audio { channels, .. } = &mut reel.tracks[1].media else {
            panic!("the second track is the sound");
        };
        *channels = 2;
        let source = reel.bound();
        let source_bytes = whole(&source).await;
        let (_, before) = read(&source_bytes);
        assert_eq!(
            before.audio().unwrap().entry.as_ref().unwrap().channels,
            2,
            "the entry miscounts"
        );
        let Some(Planned::Made(made)) = video(&source, "video/quicktime", &Job::default()).await
        else {
            panic!("not transcoded");
        };
        let (_, after) = read(&whole(&made).await);
        let sound = after.audio().unwrap();
        let entry = sound.entry.as_ref().unwrap();
        let config = mp4_read::audio_config(&entry.config).unwrap();
        assert_eq!((config.object_type, config.channels), (2, 1));
        assert_eq!(entry.channels, 1, "and the entry now says so too");
        let rate = sound.data_rate().unwrap();
        assert!(rate <= 64_000 * 9 / 8, "{rate} bit/s");
        assert_eq!(heard(&made, 48_000).await.number_of_channels(), 1);
    }

    /// A class that answers "not supported" and remembers being asked —
    /// under `globalThis[flag]`.
    fn asked(flag: &str) -> JsValue {
        js_sys::Function::new_no_args(&format!(
            "globalThis.{flag} = false; \
             const C = function() {{ globalThis.{flag} = true; }}; \
             C.isConfigSupported = async () => {{ globalThis.{flag} = true; \
             return {{supported: false}}; }}; return C;"
        ))
        .call0(&JsValue::NULL)
        .unwrap()
    }

    fn was_asked(flag: &str) -> bool {
        Reflect::get(&js_sys::global(), &JsValue::from_str(flag))
            .unwrap()
            .as_bool()
            .unwrap()
    }

    /// A browser that cannot do the PICTURE is found out before anything is
    /// done about the sound: none of it is decoded, re-encoded or read.
    /// (The sound came first, and a browser with no H.264 encoder decoded
    /// and re-encoded all of a long clip's sound before falling to rule C.)
    #[wasm_bindgen_test]
    async fn the_sound_is_not_touched_when_the_picture_cannot_be_done() {
        // 256 kbit/s sound: the route that decodes and re-encodes.
        let source = film(640, 360, 60, 1, 3_000_000, Some(256_000)).await;
        let stands: [(Lack, &'static str); 3] = [
            (without, "VideoEncoder"),
            (refusing, "VideoEncoder"),
            (refusing, "VideoDecoder"),
        ];
        for (stand, class) in stands {
            let _no_picture = stand(class);
            let _decoder = Stand::in_for("AudioDecoder", &asked("fcSoundDecoder"));
            let _encoder = Stand::in_for("AudioEncoder", &asked("fcSoundEncoder"));
            assert!(video(&source, "video/mp4", &Job::default()).await.is_none());
            assert!(
                !was_asked("fcSoundDecoder"),
                "{class}: the sound was decoded"
            );
            assert!(
                !was_asked("fcSoundEncoder"),
                "{class}: the sound was encoded"
            );
        }
        // The stand-ins do notice: with the picture possible, the sound is
        // reached — and, refused, still ends in rule C.
        let _decoder = Stand::in_for("AudioDecoder", &asked("fcSoundDecoder"));
        assert!(video(&source, "video/mp4", &Job::default()).await.is_none());
        assert!(was_asked("fcSoundDecoder"));
    }

    /// A transcode says how far it has got, and stops when it is told to —
    /// at once, not at its end — leaving nothing behind: neither a result,
    /// nor the original in its place.
    #[wasm_bindgen_test]
    async fn a_transcode_reports_its_progress_and_stops_when_told() {
        let source = film(1280, 720, 60, 3, 8_000_000, Some(256_000)).await;
        let said = Rc::new(RefCell::new(Vec::new()));
        let watched = {
            let said = said.clone();
            Job::watched(move |percent| said.borrow_mut().push(percent))
        };
        let began = js_sys::Date::now();
        assert!(matches!(
            video(&source, "video/mp4", &watched).await,
            Some(Planned::Made(_))
        ));
        let whole_run = js_sys::Date::now() - began;
        let said = said.take();
        assert!(said.len() >= 5, "{said:?}");
        assert!(said.windows(2).all(|pair| pair[0] < pair[1]), "{said:?}");
        assert_eq!(said.last(), Some(&100), "{said:?}");

        // Stopped a third of the way in.
        let stopper: Rc<RefCell<Option<Job>>> = Rc::new(RefCell::new(None));
        let last = Rc::new(Cell::new(0));
        let job = {
            let stopper = stopper.clone();
            let last = last.clone();
            Job::watched(move |percent| {
                last.set(percent);
                if percent >= 30 {
                    if let Some(job) = stopper.borrow().as_ref() {
                        job.stop();
                    }
                }
            })
        };
        *stopper.borrow_mut() = Some(job.clone());
        let began = js_sys::Date::now();
        assert!(video(&source, "video/mp4", &job).await.is_none());
        let stopped_run = js_sys::Date::now() - began;
        assert!(job.stopped());
        assert!(
            (30..60).contains(&last.get()),
            "it went no further than {} %",
            last.get()
        );
        assert!(
            stopped_run < whole_run * 0.8,
            "{stopped_run} ms against {whole_run} ms for the whole clip"
        );

        // Stopped before it began: nothing is read at all.
        let never = Job::default();
        never.stop();
        let began = js_sys::Date::now();
        assert!(video(&source, "video/mp4", &never).await.is_none());
        assert!(js_sys::Date::now() - began < whole_run * 0.5);
    }

    /// A sound file too long to decode whole is left alone, by its stated
    /// length or — without one — by its size.
    #[wasm_bindgen_test]
    fn a_sound_file_too_big_to_decode_whole_is_left_alone() {
        let source = |duration_ms, size_bytes| AudioSource {
            container: "audio/wav".into(),
            codec: "pcm".into(),
            channels: Some(2),
            bitrate: None,
            size_bytes,
            duration_ms,
        };
        assert!(could_decode(&source(Some(10 * 60_000), 100 << 20), 48_000));
        assert!(!could_decode(&source(Some(60 * 60_000), 600 << 20), 48_000));
        assert!(could_decode(&source(None, 20 << 20), 48_000));
        assert!(!could_decode(&source(None, 100 << 20), 48_000));
    }

    // --- a recording's sound, for its text ------------------------------------------------------

    const MAX: u64 = 26_214_400;

    /// An M4A this client made for a transcript: index first, one sound
    /// track, and nothing of a picture.
    async fn sound_only(sound: &Blob) -> (Vec<u8>, Movie) {
        assert_eq!(sound.type_(), "audio/mp4");
        let bytes = whole(sound).await;
        assert!(fc_text::media::matches_magic("audio/mp4", &bytes[..12]));
        let (kinds, movie) = read(&bytes);
        assert_eq!(kinds, vec![*b"ftyp", *b"moov", *b"mdat"], "the index first");
        assert!(movie.video().is_none(), "nothing of the picture");
        assert_eq!(movie.tracks.len(), 1);
        (bytes, movie)
    }

    /// COPY, from a real QuickTime movie a phone's writer made: its AAC
    /// frames come out byte for byte, behind an M4A's own index, and the
    /// browser's decoder hears them — with or without an AAC encoder,
    /// because nothing is encoded.
    #[wasm_bindgen_test]
    async fn a_videos_aac_is_copied_out_byte_for_byte() {
        let (_, original) = read(QUICKTIME);
        let from = original.audio().expect("the fixture has sound").clone();
        for lack in [None, Some(without as Lack), Some(refusing as Lack)] {
            let _stand = lack.map(|lack| lack("AudioEncoder"));
            let source = blob(QUICKTIME, "video/quicktime");
            let sound = sound_for_text(&source, Some("video/quicktime"), MAX)
                .await
                .expect("its sound");
            let (bytes, movie) = sound_only(&sound).await;
            let track = movie.audio().unwrap();
            assert_eq!(track.samples.len(), from.samples.len());
            for (made, read) in track.samples.iter().zip(&from.samples) {
                let (at, was) = (made.offset as usize, read.offset as usize);
                assert_eq!(
                    &bytes[at..at + made.size as usize],
                    &QUICKTIME[was..was + read.size as usize]
                );
            }
            assert_eq!(
                track.entry.as_ref().unwrap().config,
                from.entry.as_ref().unwrap().config,
                "the same decoder configuration"
            );
        }
        let sound = sound_for_text(&blob(QUICKTIME, "video/quicktime"), None, MAX)
            .await
            .unwrap();
        let decoded = heard(&sound, 48_000).await;
        assert!(decoded.duration() > 0.2, "{} s", decoded.duration());
    }

    /// RE-ENCODE: AAC too big to copy within the ceiling is decoded from
    /// its own frames and encoded again as one channel at 64 kbit/s — the
    /// sound of a three-second clip, at under half its bytes — and a
    /// browser that cannot encode AAC says so instead.
    #[wasm_bindgen_test]
    async fn sound_too_big_to_copy_is_re_encoded_to_one_channel() {
        let clip = film(320, 180, 30, 3, 400_000, Some(128_000)).await;
        let (_, movie) = read(&whole(&clip).await);
        let copy = transcript_sound::copied_track(&movie).unwrap();
        let copy_bytes = transcript_sound::m4a_bytes(&copy);
        let max = copy_bytes - 1;
        let sound = sound_for_text(&clip, Some("video/mp4"), max)
            .await
            .expect("re-encoded");
        let made = sound.size() as u64;
        assert!(
            made <= max && made < copy_bytes * 2 / 3,
            "{made} of {copy_bytes}"
        );
        let (_, made) = sound_only(&sound).await;
        let track = made.audio().unwrap();
        let config = mp4_read::audio_config(&track.entry.as_ref().unwrap().config).unwrap();
        assert_eq!(
            (config.object_type, config.channels, config.sample_rate),
            (2, 1, 48_000),
            "AAC-LC, one channel"
        );
        let rate = track.data_rate().unwrap();
        assert!((40_000..=80_000).contains(&rate), "{rate} bit/s");
        let decoded = heard(&sound, 48_000).await;
        assert_eq!(decoded.number_of_channels(), 1);
        assert!(
            (2.9..=3.2).contains(&decoded.duration()),
            "{} s",
            decoded.duration()
        );

        for lack in [without as Lack, refusing as Lack] {
            let _stand = lack("AudioEncoder");
            assert_eq!(
                sound_for_text(&clip, Some("video/mp4"), max).await.err(),
                Some(TextFailure::Unreadable)
            );
            // …while one that fits is still copied.
            assert!(sound_for_text(&clip, Some("video/mp4"), copy_bytes)
                .await
                .is_ok());
        }
    }

    /// A sound file with no index — WAV here, and Ogg, MP3 and the rest the
    /// same way — is decoded whole by the browser, which also brings a
    /// 16 kHz voice up to a rate the encoder takes, and encoded at 64 kbit/s.
    /// Too long for the ceiling even then is said without the work — with
    /// an encoder or without one.
    #[wasm_bindgen_test]
    async fn a_sound_file_is_decoded_whole_and_too_long_is_said_first() {
        let voice = fc_text::wav::encode(&noise(16_000 * 3, 16_000, 5), 16_000);
        let file = blob(&voice, "audio/wav");
        let sound = sound_for_text(&file, Some("audio/wav"), 40_000)
            .await
            .expect("re-encoded");
        assert!(sound.size() as u64 <= 40_000);
        let (_, made) = sound_only(&sound).await;
        let config =
            mp4_read::audio_config(&made.audio().unwrap().entry.as_ref().unwrap().config).unwrap();
        assert_eq!((config.channels, config.sample_rate), (1, 48_000));
        let decoded = heard(&sound, 48_000).await;
        assert!(
            (2.9..=3.2).contains(&decoded.duration()),
            "{} s",
            decoded.duration()
        );

        // Three seconds is 24 000 bytes at 64 kbit/s.
        assert_eq!(
            sound_for_text(&file, Some("audio/wav"), 20_000).await.err(),
            Some(TextFailure::TooLong)
        );
        {
            let _gone = without("AudioEncoder");
            assert_eq!(
                sound_for_text(&file, Some("audio/wav"), 20_000).await.err(),
                Some(TextFailure::TooLong)
            );
            assert_eq!(
                sound_for_text(&file, Some("audio/wav"), 40_000).await.err(),
                Some(TextFailure::Unreadable)
            );
        }
    }

    /// Nothing to hear: a film with no sound track, a "video" that is no
    /// movie at all, and sound the browser cannot decode.
    #[wasm_bindgen_test]
    async fn no_sound_is_nothing_to_send() {
        let silent = film(160, 120, 30, 1, 200_000, None).await;
        assert_eq!(
            sound_for_text(&silent, Some("video/mp4"), MAX).await.err(),
            Some(TextFailure::Unreadable)
        );
        let junk = blob(b"not a movie, and not a sound either", "video/mp4");
        assert_eq!(
            sound_for_text(&junk, Some("video/mp4"), MAX).await.err(),
            Some(TextFailure::Unreadable)
        );
        let noise = blob(&[0x5Au8; 4_096], "audio/ogg");
        assert_eq!(
            sound_for_text(&noise, Some("audio/ogg"), MAX).await.err(),
            Some(TextFailure::Unreadable)
        );
    }
}
