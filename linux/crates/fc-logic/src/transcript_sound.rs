//! The sound track a device sends with a transcript request when the server cannot send its own
//! copy — a video, an Ogg file, an audio file over `transcribe_max_bytes` (docs/protocol.md,
//! "Transcripts on request", the multipart form) — a port of `TranscriptSound.cs`, the decisions,
//! and of the window's `SoundTracks.MakeAsync` and `MediaPreparing.SoundTrackAsync`, which run them
//! ([`sound_track`], over the shell's [`SoundTrackMaker`]).
//!
//! **PASS THROUGH WHAT IS ALREADY AAC; ENCODE THE REST AT 64 kbit/s MONO.** A phone's video carries
//! AAC, and copying that track into an M4A costs nothing and loses nothing. Anything else — Opus or
//! Vorbis in an Ogg, MP3, PCM, a track whose copy would be over the ceiling — is encoded as a voice
//! note is (`media_encoding::audio_attempts` at a voice note's numbers, mono), which holds about
//! fifty minutes in 25 MiB. No picture is ever asked for, so none is decoded.
//!
//! **ONE ENCODE ON LINUX.** Windows tried 64 000 and then Media Foundation's nearest documented
//! rate (96 000), because that encoder refuses 64 000 for mono; `avenc_aac` takes 64 000 as asked
//! (measured, linux/docs/shell-design.md §1.4), so the attempts here are the passthrough, where it
//! applies, and ONE re-encode (`media_encoding::audio_attempts` has no ladder).
//!
//! **NEVER HIDDEN FOR A REASON OF THE SOUND.** Whether a file can be made into sound is known only
//! once it is here and read, and downloading a video to decide whether to draw a button is the wrong
//! trade; and a hidden action cannot say why it is missing. So every voice note, audio file and
//! video the RULES allow is offered, and what the device then cannot do is said under it, final, in
//! the catalogue's own sentences — the ones iOS, Android and the web draw: a stated length whose
//! 64 kbit/s sound cannot fit ([`can_fit`], told at the press with nothing downloaded) or a result
//! still over the ceiling is "This recording is too long to turn into text." ([`too_long`]); no
//! track or a codec this machine lacks is "Couldn't read the sound in this file." ([`unreadable`]).
//! Neither is "Not available for this message.", which reads as a refusal.
//!
//! `fc_text::transcript_sound` is the web client's version of the same decision, built on its own
//! MP4 reader and writer (one `extraction` answer instead of attempts read back); this module keeps
//! Windows' attempt-and-judge shape because GStreamer, like Media Foundation, is asked and then read
//! back. Where the two share a rule it is called: [`fits`] is fc-text's.

use std::fmt;
use std::rc::Rc;

use fc_core::protocol::{ApiError, AttachmentDto, error_codes};
use fc_core::rt::{Cancel, Clock, LocalBoxFuture};
use fc_text::media_plan;

use crate::attachment_cache::AttachmentCache;
use crate::media_encoding::transcode_ceiling;
use crate::media_encoding::{self, AacEncoding};
use crate::transcode_attempts::{self, AttemptEnd, AttemptFailure, Attempted, OperationCanceled};

/// Which of the endpoint's two forms a recording's "Show text" uses (docs/protocol.md,
/// "Transcripts on request").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TranscriptForm {
    /// Neither: nothing is offered.
    None,
    /// No body — the server sends its own stored copy, and keeps the answer for whoever asks next.
    Stored,
    /// A multipart `audio` part this device made from the file it holds; the answer is never kept
    /// there.
    Supplied,
}

/// How one way of making the sound track goes about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SoundWay {
    /// The file's own AAC track, copied into an M4A untouched: nothing is decoded.
    Passthrough,
    /// The sound decoded and encoded again as mono AAC, at [`SoundAttempt::aac`]'s numbers.
    Reencode,
}

/// One way of asking the encoder for the sound track. `aac` is the encode's numbers; `None` for
/// [`SoundWay::Passthrough`], which keeps the track's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SoundAttempt {
    pub way: SoundWay,
    pub aac: Option<AacEncoding>,
}

impl SoundAttempt {
    /// The track copied as it is.
    pub const PASSTHROUGH: SoundAttempt = SoundAttempt {
        way: SoundWay::Passthrough,
        aac: None,
    };

    /// The sound encoded at these numbers.
    pub fn reencode(aac: AacEncoding) -> Self {
        SoundAttempt {
            way: SoundWay::Reencode,
            aac: Some(aac),
        }
    }
}

impl fmt::Display for SoundAttempt {
    /// What the diagnostics log says of an attempt — numbers, never a name or a path.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.aac {
            None => f.write_str("passthrough"),
            Some(aac) => write!(
                f,
                "re-encode AAC {} Hz, {} channel(s), {} bit/s",
                aac.sample_rate, aac.channels, aac.bitrate
            ),
        }
    }
}

/// The sound the device made for a transcript, or why it has none.
///
/// `Made`: an M4A of AAC, within the server's ceiling — exactly what the multipart part carries.
/// `Failed`: [`too_long`] when what it made, or would make, is over the ceiling; [`unreadable`] when
/// this device could not take the sound out at all; or whatever the download of the file failed
/// with (a lost connection is still "try again"). The C#'s record held both as nullable; its
/// "neither" (read as unreadable) cannot be written here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuppliedSound {
    Made(Vec<u8>),
    Failed(ApiError),
}

/// How the device makes a recording's sound: handed a [`Cancel`] raised when the session ends,
/// it answers the sound, or `Err` when it failed outright — what the C# caught as an exception,
/// said as [`unreadable`]. Once the cancel is raised, what it answers is not read.
pub type Make =
    Rc<dyn Fn(Cancel) -> LocalBoxFuture<'static, Result<SuppliedSound, AttemptFailure>>>;

/// Where a recording's supplied sound comes from: the ceiling it must fit, and the device's way of
/// making it. `duration_ms` is the attachment's stated length, when it states one: a length that
/// cannot fit `max_bytes` even at 64 kbit/s is [`too_long`] before `make` downloads anything.
#[derive(Clone)]
pub struct SoundSource {
    pub max_bytes: i64,
    pub make: Make,
    pub duration_ms: Option<i64>,
}

impl SoundSource {
    pub fn new(
        max_bytes: i64,
        make: impl Fn(Cancel) -> LocalBoxFuture<'static, Result<SuppliedSound, AttemptFailure>>
        + 'static,
        duration_ms: Option<i64>,
    ) -> Self {
        SoundSource {
            max_bytes,
            make: Rc::new(make),
            duration_ms,
        }
    }
}

impl fmt::Debug for SoundSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SoundSource")
            .field("max_bytes", &self.max_bytes)
            .field("duration_ms", &self.duration_ms)
            .finish_non_exhaustive()
    }
}

/// The encode: AAC-LC, mono, 64 kbit/s — a voice note's numbers.
pub const BITRATE: i64 = media_plan::VOICE_NOTE_BITRATE as i64;

/// The part's type: AAC in MPEG-4, which the server checks by its `ftyp` box.
pub const MIME: &str = "audio/mp4";

/// The part's name, as the server reads it.
pub const PART_NAME: &str = "audio";

/// The part's file name — what an M4A is called; the server reads only the bytes.
pub const FILE_NAME: &str = "audio.m4a";

/// What an MPEG-4 file costs beyond its sound, as a share of it (its sample tables) and a fixed
/// head.
const OVERHEAD_BYTES: i64 = 16 * 1024;

/// The code of [`too_long`]: this client's own, never the server's, and never on the wire.
pub const TOO_LONG_CODE: &str = "device_sound_too_long";

/// The code of [`unreadable`]: this client's own, never the server's, and never on the wire.
pub const UNREADABLE_CODE: &str = "device_sound_unreadable";

/// Nothing to ask with at all — no form for this recording: said as the server's
/// `not_transcribable` is, final.
pub fn unavailable() -> ApiError {
    ApiError::coded(
        error_codes::NOT_TRANSCRIBABLE,
        "this device could not make the recording's sound",
    )
}

/// The sound is longer than the ceiling holds even at 64 kbit/s — known from the stated length, or
/// measured once made. Final: the same file makes the same sound. A 4xx status, so it never reads
/// as a transport failure.
pub fn too_long() -> ApiError {
    ApiError::new(
        TOO_LONG_CODE,
        "the recording's sound is over the ceiling",
        400,
    )
}

/// This device could not take the sound out of the file: no sound track, a codec this machine
/// lacks, or what came out was not AAC in MPEG-4. Final on this device.
pub fn unreadable() -> ApiError {
    ApiError::new(
        UNREADABLE_CODE,
        "this device could not read the recording's sound",
        400,
    )
}

/// About how big a track of this rate and length comes out as an M4A: the sound, plus a fiftieth
/// for the sample tables, plus a fixed head. An over-estimate on purpose — it only ever decides
/// what is NOT tried.
pub fn estimated_bytes(bitrate: i64, duration_ms: i64) -> i64 {
    if bitrate <= 0 || duration_ms <= 0 {
        return 0;
    }
    // In floating point, so nonsense in saturates instead of overflowing; an estimate needs no
    // more.
    let sound = bitrate as f64 * duration_ms as f64 / 8000.0;
    let total = (sound + (sound / 50.0) + OVERHEAD_BYTES as f64).ceil();
    if total >= i64::MAX as f64 {
        i64::MAX
    } else {
        total as i64
    }
}

/// Whether a recording of this length could fit the ceiling at all once encoded at [`BITRATE`].
/// When it cannot, the press says [`too_long`] without downloading anything. A length nobody stated
/// may fit: the result is measured before it is sent.
pub fn can_fit(duration_ms: Option<i64>, max_bytes: i64) -> bool {
    match duration_ms {
        Some(duration) if duration > 0 => estimated_bytes(BITRATE, duration) <= max_bytes,
        _ => true,
    }
}

/// Whether made sound may go: something, and no more than the ceiling
/// (`fc_text::transcript_sound::fits`; a negative count or ceiling is nothing that fits).
pub fn fits(bytes: i64, max_bytes: i64) -> bool {
    match (u64::try_from(bytes), u64::try_from(max_bytes)) {
        (Ok(bytes), Ok(max_bytes)) => fc_text::transcript_sound::fits(bytes, max_bytes),
        _ => false,
    }
}

/// The ways to make the sound, in order: the track itself when it is AAC and its copy can fit, then
/// the voice-note encode when it can fit. EMPTY when there is no sound track — nothing to send.
///
/// `codec`: the track's codec as the shell names it ([`TrackReading::codec`]); `None` for no track
/// at all. `track_bitrate`, `sample_rate`: the track's own, when anything states them — the encode
/// keeps 48 kHz as 48. `duration_ms`: how long it runs, when known.
pub fn attempts(
    codec: Option<&str>,
    track_bitrate: Option<i64>,
    sample_rate: Option<i64>,
    duration_ms: Option<i64>,
    max_bytes: i64,
) -> Vec<SoundAttempt> {
    let Some(codec) = codec else {
        return Vec::new();
    };
    let mut attempts = Vec::new();
    if codec == "aac" && possible(track_bitrate, duration_ms, max_bytes) {
        attempts.push(SoundAttempt::PASSTHROUGH);
    }
    for aac in media_encoding::audio_attempts(BITRATE, Some(1), sample_rate) {
        if possible(Some(i64::from(aac.bitrate)), duration_ms, max_bytes) {
            attempts.push(SoundAttempt::reencode(aac));
        }
    }
    attempts
}

/// Whether what came out may go: AAC, no pictures, a re-encode at the rate it was asked for, and
/// within the ceiling. A copy that came out some other way, or too big, leaves the encode to try;
/// an encode that is not AAC, or that carries a picture, will not be put right by asking again.
///
/// `codec`: what the result's track reads as; `None` when it has none. `bitrate`: the result's
/// stated rate, when it states one. `bytes`: how big the result is. `has_video`: whether the result
/// carries a picture track, which it never may.
pub fn judge(
    attempt: &SoundAttempt,
    codec: Option<&str>,
    bitrate: Option<i64>,
    bytes: i64,
    has_video: bool,
    max_bytes: i64,
) -> AttemptEnd {
    if has_video {
        return AttemptEnd::Wrong;
    }
    if codec != Some("aac") {
        return if attempt.way == SoundWay::Passthrough {
            AttemptEnd::Refused
        } else {
            AttemptEnd::Wrong
        };
    }
    if let Some(asked) = attempt.aac
        && !media_encoding::audio_rate_came_out(asked.bitrate, bitrate)
    {
        return AttemptEnd::Refused;
    }
    if fits(bytes, max_bytes) {
        AttemptEnd::Taken
    } else {
        AttemptEnd::Refused
    }
}

/// The extension the held file is written out with for the reader: by the stored type first, the
/// name's own when the type says nothing, and `bin` otherwise. Never anything from the name but
/// ASCII letters and digits: it becomes part of a path. (Windows needed it because Media Foundation
/// picks its reader by the extension; GStreamer's typefind reads the bytes, so on Linux it only
/// names the temporary file the shell may write.)
pub fn temp_extension(mime: Option<&str>, name: Option<&str>) -> String {
    let by_type = match fc_text::media::essence(mime.unwrap_or("")).as_str() {
        "video/mp4" | "video/x-m4v" => Some("mp4"),
        "video/quicktime" => Some("mov"),
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => Some("m4a"),
        "audio/aac" | "audio/x-aac" => Some("aac"),
        "audio/mpeg" | "audio/mp3" => Some("mp3"),
        "audio/wav" | "audio/x-wav" | "audio/wave" => Some("wav"),
        "audio/ogg" | "application/ogg" => Some("ogg"),
        "audio/webm" | "video/webm" => Some("webm"),
        "audio/flac" => Some("flac"),
        _ => None,
    };
    if let Some(by_type) = by_type {
        return by_type.to_string();
    }
    let extension = name.map(fc_text::media::extension).unwrap_or_default();
    // An ASCII run, so its length in bytes is the C#'s length in UTF-16 units.
    if (1..=5).contains(&extension.len()) && extension.bytes().all(|b| b.is_ascii_alphanumeric()) {
        extension
    } else {
        "bin".to_string()
    }
}

fn possible(bitrate: Option<i64>, duration_ms: Option<i64>, max_bytes: i64) -> bool {
    match (bitrate, duration_ms) {
        (Some(bitrate), Some(duration)) if bitrate > 0 && duration > 0 => {
            estimated_bytes(bitrate, duration) <= max_bytes
        }
        _ => true,
    }
}

// ---- the shell's side: reading the held file and making one attempt ---------------------------

/// A held file's sound track, as the shell's reader found it (Windows: `MediaEncodingProfile.
/// CreateFromFileAsync(file).Audio`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrackReading {
    /// The codec, in the names [`attempts`] reads: `"aac"` (AAC-LC or HE-AAC, in any container or
    /// as ADTS), and for the rest `"mp3"`, `"pcm"` (integer or float), `"flac"`, `"alac"`,
    /// `"opus"`, `"vorbis"` — or `"unknown"`. Only `"aac"` is copied rather than encoded.
    pub codec: String,
    /// The track's own bit rate, bits per second, when anything states it (a `bitrate` or
    /// `nominal-bitrate` tag, or the container's index); `None` otherwise — never 0.
    pub bitrate: Option<i64>,
    /// Its sample rate, Hz, when stated; `None` otherwise — never 0.
    pub sample_rate: Option<i64>,
}

/// What one attempt made, read back by the shell's reader — never trusted as asked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MadeSound {
    /// The whole file the attempt wrote.
    pub bytes: Vec<u8>,
    /// Its sound track's codec as [`TrackReading::codec`] names it; `None` when it has none.
    pub codec: Option<String>,
    /// Its sound track's stated bit rate, when it states one.
    pub bitrate: Option<i64>,
    /// Whether it carries a picture track — which an attempt never may.
    pub has_video: bool,
}

/// The platform half of making a recording's sound (Windows' `MediaPreparing.SoundTrackAsync` over
/// Media Foundation): read the held file, and make ONE attempt of it. Everything else — which
/// attempts, in what order, under what ceiling, and whether what came out may go — is
/// [`sound_track`]'s. The Linux shell implements it with GStreamer; every method runs on the GLib
/// main context, GStreamer's own threads doing the work, and its future answering from the bus.
/// Nothing is logged here but numbers and an error's domain and code: the bytes are somebody's
/// voice, and a file name or a caps string can quote anything.
///
/// `bytes` is the whole held file (from the attachment cache) and `extension` its
/// [`temp_extension`]. The shell may hand the bytes to GStreamer from memory (`giostreamsrc` over a
/// seekable `gio::MemoryInputStream`) or write them to a temporary file named
/// `<uuid>.<extension>` under `$XDG_CACHE_HOME/family-connect/tmp/` and delete it before the
/// future answers; either way nothing it writes outlives the call.
pub trait SoundTrackMaker {
    /// Read the file's sound track with the discoverer (`gst_pbutils::Discoverer`, its first audio
    /// stream): `Ok(Some(reading))`, `Ok(None)` when the file has no sound track at all, `Err` when
    /// it cannot be read (no demuxer, no decoder for the codec, a damaged file — the error's GLib
    /// domain as `kind` and its code). Never decodes the picture.
    fn read<'a>(
        &'a self,
        bytes: &'a Rc<[u8]>,
        extension: &'a str,
    ) -> LocalBoxFuture<'a, Result<Option<TrackReading>, AttemptFailure>>;

    /// Make the sound as `attempt` says, into an M4A (`mp4mux faststart=true`, major brand `M4A `),
    /// and read the result back with the discoverer into a [`MadeSound`]:
    ///
    /// - [`SoundWay::Passthrough`]: the AAC track's frames COPIED, nothing decoded — `filesrc !
    ///   parsebin`, its `audio/mpeg, mpegversion=4` pad `! aacparse ! mp4mux faststart=true`; the
    ///   picture pad, if any, left unlinked (a `fakesink`).
    /// - [`SoundWay::Reencode`]: `filesrc ! decodebin3`, its audio pad only, `! audioconvert !
    ///   audioresample ! audio/x-raw,rate=<aac.sample_rate>,channels=<aac.channels> ! avenc_aac
    ///   bitrate=<aac.bitrate> ! aacparse ! mp4mux faststart=true` — `voaacenc bitrate=…` where
    ///   `avenc_aac` is missing; neither: `Err`.
    ///
    /// `cancel` is raised when the person's session ends or the ceiling runs out: set the pipeline
    /// to `Null` at once and answer anything (it is not read). Dropping the future is the same.
    /// `Err` is a pipeline that would not build or that posted an `Error` on its bus (the error's
    /// domain and code; never its message, which can quote a path); a pipeline that ran is
    /// `Ok` with what came out, however wrong — [`judge`] decides.
    fn make<'a>(
        &'a self,
        bytes: &'a Rc<[u8]>,
        extension: &'a str,
        attempt: &'a SoundAttempt,
        cancel: Cancel,
    ) -> LocalBoxFuture<'a, Result<MadeSound, AttemptFailure>>;
}

/// A recording's or a video's SOUND, as an M4A of AAC no bigger than `max_bytes`, for a transcript
/// the server cannot make from its own copy — Windows' `SoundTracks.MakeAsync` and
/// `MediaPreparing.SoundTrackAsync` in one: the file from the attachment cache (downloaded as it
/// would be to play it), its track read, [`attempts`] tried in order under ONE ceiling
/// (`transcode_ceiling`, [`transcode_attempts::first_taken`]), each result [`judge`]d and checked
/// to be MPEG-4 by its bytes.
///
/// Answers `Failed` with the download's own error (a lost connection is still "try again", a gone
/// attachment is not), with [`unreadable`] when this machine cannot read the sound — no sound
/// track, a codec it lacks, every way refused or wrong — and with [`too_long`] when no way of
/// making it can fit the ceiling, or what came out is still over it. `Err(OperationCanceled)` once
/// `cancel` is raised. `log` hears what happened, in numbers only.
pub async fn sound_track(
    cache: &AttachmentCache,
    attachment: &AttachmentDto,
    max_bytes: i64,
    maker: &dyn SoundTrackMaker,
    clock: &dyn Clock,
    cancel: &Cancel,
    mut log: impl FnMut(&str),
) -> Result<SuppliedSound, OperationCanceled> {
    let held: Rc<[u8]> = match cache.bytes(attachment, false).await {
        Ok(bytes) => bytes.into(),
        // The download's own failure: a lost connection is still "try again", a gone attachment
        // is not.
        Err(error) => return Ok(SuppliedSound::Failed(error)),
    };
    if cancel.is_cancelled() {
        return Err(OperationCanceled);
    }
    let extension = temp_extension(attachment.mime.as_deref(), attachment.name.as_deref());
    let track = match maker.read(&held, &extension).await {
        Ok(Some(track)) => track,
        Ok(None) => {
            log("a recording has no sound track the reader reads");
            return Ok(SuppliedSound::Failed(unreadable()));
        }
        Err(failure) => {
            log(&format!("reading a recording's sound track: {failure}"));
            return Ok(SuppliedSound::Failed(unreadable()));
        }
    };
    let duration_ms = attachment.duration_ms.map(i64::from);
    let ways = attempts(
        Some(&track.codec),
        track.bitrate.filter(|rate| *rate > 0),
        track.sample_rate.filter(|rate| *rate > 0),
        duration_ms,
        max_bytes,
    );
    if ways.is_empty() {
        log(&format!(
            "a recording's sound cannot be made to fit {max_bytes} bytes"
        ));
        return Ok(SuppliedSound::Failed(too_long()));
    }
    let made = transcode_attempts::first_taken(
        &ways,
        |attempt, cancel| {
            let held = &held;
            let extension = extension.as_str();
            async move {
                let made = maker.make(held, extension, attempt, cancel).await?;
                let length = i64::try_from(made.bytes.len()).unwrap_or(i64::MAX);
                Ok(
                    match judge(
                        attempt,
                        made.codec.as_deref(),
                        made.bitrate,
                        length,
                        made.has_video,
                        max_bytes,
                    ) {
                        AttemptEnd::Taken if fc_text::media::matches_magic(MIME, &made.bytes) => {
                            Attempted::Taken(made.bytes)
                        }
                        // Not MPEG-4 by its bytes: no other way of asking will make it so.
                        AttemptEnd::Taken | AttemptEnd::Wrong => Attempted::Wrong,
                        AttemptEnd::Refused => Attempted::Refused,
                    },
                )
            }
        },
        transcode_ceiling(duration_ms),
        clock,
        &mut log,
        cancel,
    )
    .await?;
    Ok(match made {
        None => SuppliedSound::Failed(unreadable()),
        Some(bytes) if fits(i64::try_from(bytes.len()).unwrap_or(i64::MAX), max_bytes) => {
            SuppliedSound::Made(bytes)
        }
        Some(_) => SuppliedSound::Failed(too_long()),
    })
}

/// The [`SoundSource`] a "Show text" hands the transcript model on a device that makes sound
/// (Windows' `ChatsView.AskTranscriptAsync`): `max_bytes` and the attachment's stated length, and
/// [`sound_track`] as the way of making it. A cancel answers `Err` — it is never read, because
/// the model has let go of the click by then.
pub fn device_source(
    cache: Rc<AttachmentCache>,
    attachment: AttachmentDto,
    max_bytes: i64,
    maker: Rc<dyn SoundTrackMaker>,
    clock: Rc<dyn Clock>,
    log: Rc<dyn Fn(&str)>,
) -> SoundSource {
    let duration_ms = attachment.duration_ms.map(i64::from);
    SoundSource::new(
        max_bytes,
        move |cancel| {
            let cache = Rc::clone(&cache);
            let attachment = attachment.clone();
            let maker = Rc::clone(&maker);
            let clock = Rc::clone(&clock);
            let log = Rc::clone(&log);
            Box::pin(async move {
                sound_track(
                    &cache,
                    &attachment,
                    max_bytes,
                    maker.as_ref(),
                    clock.as_ref(),
                    &cancel,
                    |line| log(line),
                )
                .await
                .map_err(|OperationCanceled| AttemptFailure::new("OperationCanceled", 0))
            })
        },
        duration_ms,
    )
}
