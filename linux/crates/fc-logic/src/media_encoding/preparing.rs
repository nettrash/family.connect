//! A picked video or sound file brought to the protocol's profile — the pure half of
//! `win/src/FamilyConnect.App/Services/MediaPreparing.cs`'s video and sound-file paths, moved here
//! so the shell implements only the [`MediaPreparing`] seam (GStreamer: the discoverer, one
//! pipeline per attempt, a frame grabber) and the rest is tested (shell-design §10.7).
//!
//! A VIDEO IS BROUGHT TO THE PROFILE, OR LEFT ALONE WHEN IT IS ALREADY THERE ([`prepare_video`]).
//! What the discoverer reads of it goes into the planner (`fc_text::media_plan`); a transcode is
//! asked one way after another (`media_encoding::video_attempts` — `x264enc` High, then
//! `openh264enc` Main — through `transcode_attempts::first_taken`, one ceiling for all); what comes
//! out is read back and CHECKED — its numbers, and a frame of it beside the same frame of its source
//! (`frame_turn`) — its index moved to the front where it is not (`faststart`), and thrown away when
//! it is bigger than what it came from (rule D).
//!
//! A TRANSCODE KEEPS THE SOUND (rule E, issue #98). The source's sound tracks are counted from its
//! own `moov` (`container_sound`), because a decoder does not list a track it cannot read and a
//! transcode then writes the picture alone. One the decoder does not show is left as it is; more
//! than one that plays is MIXED into the profile's one audio track (an `audiomixer` in the same
//! pipeline); and a result with no sound track from a source with one — or a mix with other than
//! one — is a failed transcode, however small.
//!
//! A PICKED SOUND FILE IS RE-ENCODED ONLY WHERE THE AUDIO RULES SAY ([`prepare_sound`]) — lossless,
//! Ogg, or lossy above 192 kbit/s — into an M4A. AIFF and FLAC, which the server does not take as
//! audio and which went as files before, reach those rules too, and go as audio when they come
//! out.
//!
//! NOTHING HERE MAY LOSE A SEND THAT WORKED IN 1.1 (rule C). Anything that fails — a stream the
//! discoverer cannot read, a codec without a decoder, an encoder missing, a pipeline that errors,
//! stalls or comes out wrong — sends what 1.1 sent: the original when it is within the ceiling,
//! refused or as a file otherwise. THE ONE THING THAT IS NOT A FAILURE IS BEING TOLD TO STOP: the
//! person's cancel comes out as `Err(OperationCanceled)`, never as rule C, which would stage the
//! very file the person has just taken back.
//!
//! What a bubble shows of the source — its poster, its displayed size, its length — and the photo,
//! animation and video-message paths are not here: the shell's, `animation_preparing` and the
//! round-video files. LOGS say sizes, counts, rates and codecs — never a name or a path.

use std::borrow::Cow;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

use fc_core::rt::{Cancel, Clock, LocalBoxFuture};
use fc_text::media;
use fc_text::media_plan::{self, AudioPlan, AudioSource, OnFailure, VideoPlan, VideoSource};

use super::{
    AacEncoding, Chosen, VideoEncoding, aac_came_out, audio_attempts, audio_rate_came_out, choose,
    displayed, first_known, frame_rate, frame_rate_came_out, hides_audio, is_hdr, matches,
    mix_attempts, rotation, square_pixels, transcode_ceiling, video_attempts,
};
use crate::container_sound::{self, DecodedTrack, SoundBefore, SoundTrack};
use crate::faststart;
use crate::frame_turn::{self, FrameVerdict};
use crate::transcode_attempts::{self, AttemptEnd, AttemptFailure, Attempted, OperationCanceled};

/// What the discoverer reads of one file — a picked source, or a transcode's result.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StreamReading {
    /// The first video stream it lists, or `None` (an image stream is not a video).
    pub video: Option<VideoStream>,
    /// Every audio stream it lists, in its order — `GstDiscovererInfo::audio_streams()`, which
    /// for an MP4 is `qtdemux`'s pad order, the `moov`'s track order. A stream with no decoder is
    /// still listed (its caps name it) — leave it out only when the discoverer does.
    pub audio: Vec<AudioStream>,
}

/// One video stream as the discoverer reads it (`GstDiscovererVideoInfo` and its caps).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VideoStream {
    /// The planner's name for the codec, from the caps: `video/x-h264` → `"h264"`, `video/x-h265` →
    /// `"hevc"`, `video/x-av1` → `"av1"`, `video/x-vp9` → `"vp9"`, anything else `"unknown"`.
    pub codec: String,
    /// The caps' `profile` field for H.264 as GStreamer spells it (`"high"`, `"main"`,
    /// `"constrained-baseline"`, `"high-10"`, `"high-4:4:4"`, …), or `None` when the caps say none.
    pub profile: Option<String>,
    /// The STORED sides (`get_width`/`get_height`), before any turn.
    pub width: u32,
    pub height: u32,
    /// The turn in degrees from the stream's `image-orientation` tag: `rotate-90` → 90,
    /// `rotate-180` → 180, `rotate-270` → 270, `rotate-0` → 0; `None` with no tag, and for a
    /// `flip-*` orientation (a mirror is no turn). `qtdemux` writes it from the `tkhd` matrix, and
    /// `mp4mux` writes it back (measured, shell-design §1.4).
    pub rotation: Option<i64>,
    /// `get_framerate_num`/`get_framerate_denom` (0/1 for a rate it will not name).
    pub frame_rate_numerator: u32,
    pub frame_rate_denominator: u32,
    /// The container's own rate — `fc_text::mp4_read::parse_moov(..)?.video()?.frame_rate()` — or
    /// `None`; asked only when the stream's ratio is 0 (`media_encoding::frame_rate`).
    pub container_frame_rate: Option<f64>,
    /// `get_par_num`/`get_par_denom`.
    pub pixel_aspect_numerator: u32,
    pub pixel_aspect_denominator: u32,
    /// The caps' colorimetry in ISO/IEC 23091-2 code points: `gst_video_transfer_function_to_iso`
    /// and `gst_video_color_primaries_to_iso` of `GstVideoColorimetry`; `None` when unknown.
    pub transfer: Option<i64>,
    pub primaries: Option<i64>,
    /// The average bit rate in bit/s (`get_bitrate`, else the `bitrate` tag), `None` when not
    /// stated.
    pub bitrate: Option<i64>,
}

/// One audio stream as the discoverer reads it (`GstDiscovererAudioInfo` and its caps).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AudioStream {
    /// The planner's name: `audio/mpeg` mpegversion 2 or 4 → `"aac"`; mpegversion 1 layer 3 →
    /// `"mp3"`; `audio/x-raw` (WAV, AIFF) → `"pcm"`; `audio/x-flac` → `"flac"`; `audio/x-alac` →
    /// `"alac"`; `audio/x-vorbis` → `"vorbis"`; `audio/x-opus` → `"opus"`; anything else
    /// `"unknown"`.
    pub codec: String,
    /// `get_channels`, `get_sample_rate` — `None` (or 0) when not stated.
    pub channels: Option<i64>,
    pub sample_rate: Option<i64>,
    /// The average bit rate in bit/s: `get_bitrate`, else the stream's `bitrate` tag, else its
    /// `nominal-bitrate` tag; `None` when none is stated. For a VBR MP3 this is the file's
    /// average, which is what "above 192 kbit/s" is about.
    pub bitrate: Option<i64>,
}

/// What one attempt asks the encoder for.
#[derive(Debug, Clone, PartialEq)]
pub enum EncodeAsk {
    /// A video at `encoding`'s numbers, into a new MP4. `mix`: `None` — the source's FIRST audio
    /// stream encoded alone (none at all when `encoding.audio` is `None`); `Some(positions)` — the
    /// audio streams at those positions in the discoverer's list, MIXED (Rule E).
    Video {
        encoding: VideoEncoding,
        mix: Option<Vec<usize>>,
    },
    /// A sound file's re-encode: an M4A of AAC-LC at these numbers, no video.
    Sound(AacEncoding),
}

/// The platform's half of preparing media — the shell implements it with GStreamer.
///
/// Every method runs on the main context and never blocks it: GStreamer runs its own threads, and
/// the futures await the bus. Dropping a future cancels it (a pipeline set to `NULL`). Nothing is
/// logged by the shell but an error's domain and code — never a path, a name or an error's words.
pub trait MediaPreparing {
    /// What the discoverer (`gst_pbutils::Discoverer`, a timeout of 10 s) reads of `file`: see
    /// [`StreamReading`]. `None` when it cannot read the file at all (an error, a timeout, no
    /// streams).
    fn read_streams<'a>(&'a self, file: &'a Path) -> LocalBoxFuture<'a, Option<StreamReading>>;

    /// The container's sound tracks: `container_sound::read_from(&mut std::fs::File::open(file)?)`
    /// — `None` for an error opening or reading it, and for what that answers `None`.
    fn sound_tracks<'a>(&'a self, file: &'a Path) -> LocalBoxFuture<'a, Option<Vec<SoundTrack>>>;

    /// One frame of `file` at `at_seconds` (the nearest frame), as `frame_turn::SIDE` ×
    /// `frame_turn::SIDE` BGRA (B, G, R, A, rows top to bottom, no padding — exactly
    /// `4 × SIDE × SIDE` bytes): `filesrc ! decodebin3 ! videoconvert ! videoscale !
    /// video/x-raw,format=BGRA,width=16,height=16,pixel-aspect-ratio=1/1 ! appsink`, a flushing
    /// accurate seek to `at_seconds`, the first sample. Squashed to the square on purpose, never
    /// letterboxed. Whatever it does about the stream's turn (a `videoflip video-direction=auto`
    /// or none) it must do the same for every file: a source and its result are compared through
    /// it. `None` when no frame can be had.
    fn frame<'a>(&'a self, file: &'a Path, at_seconds: f64) -> LocalBoxFuture<'a, Option<Vec<u8>>>;

    /// Run ONE attempt into a NEW temporary file of the shell's own (under
    /// `$XDG_CACHE_HOME/family-connect/prepared/`, a fresh name each time, `.mp4` for a video and
    /// `.m4a` for a sound — what an earlier run left there for over a day is swept when the folder
    /// is made), and answer its path, which [`remove`](Self::remove) is later given.
    ///
    /// The pipelines (shell-design §10.7; carry-forward):
    /// - [`EncodeAsk::Video`]: `filesrc ! decodebin3`; the video branch `! videoconvert !
    ///   videoscale ! videorate ! video/x-raw,format=I420,width=<encoding.width>,
    ///   height=<encoding.height>,framerate=<num>/<den>,pixel-aspect-ratio=1/1` — the STORED sides,
    ///   never turned: the source's `image-orientation` tag goes through to `mp4mux`, which writes
    ///   it as the track's matrix (a `taginject` of `image-orientation=rotate-<encoding.rotation>`
    ///   when the decoder does not pass it on) — then for [`H264Profile::High`](super::H264Profile::High) `x264enc
    ///   bitrate=<encoding.bitrate / 1000> ! video/x-h264,profile=high`, for [`H264Profile::Main`](super::H264Profile::Main)
    ///   `openh264enc bitrate=<encoding.bitrate> ! video/x-h264,profile=main`; `key-int-max` /
    ///   `gop-size` = `encoding.keyframe_spacing` when set; `h264parse`. The raw caps pin `I420`
    ///   ALWAYS (left free, `x264enc` negotiated High 4:4:4 10-bit — measured). With
    ///   `encoding.to_sdr` the raw caps also say `colorimetry=bt709` and the `videoconvert` has
    ///   `gamma-mode=remap primaries-mode=full`: there is no tone-mapper in Ubuntu's GStreamer, so
    ///   an HDR picture's highlights clip, as Media Foundation's do. The audio, when
    ///   `encoding.audio` is `Some(aac)`: the first audio stream (`mix: None`) or the listed ones
    ///   into one `audiomixer` (each at its own timestamps — its place in time; summed and clamped)
    ///   `! audioconvert ! audioresample ! audio/x-raw,rate=<aac.sample_rate>,
    ///   channels=<aac.channels> ! avenc_aac bitrate=<aac.bitrate>` (`voaacenc` when there is no
    ///   `avenc_aac`) `! aacparse`; with `encoding.keep_audio` the first audio stream's own
    ///   compressed buffers go straight to `aacparse` (never asked on Linux). No audio branch at
    ///   all when `encoding.audio` is `None`. Then `mp4mux faststart=true ! filesink`.
    /// - [`EncodeAsk::Sound`]: the first audio stream only (the picture is never decoded) `!
    ///   audioconvert ! audioresample ! audio/x-raw,rate=…,channels=… ! avenc_aac bitrate=… !
    ///   aacparse ! mp4mux faststart=true ! filesink`.
    ///
    /// Answers: `Ok(Attempted::Taken(path))` once the bus said EOS and the file is closed;
    /// `Ok(Attempted::Refused)` when this way cannot be built — an element missing (`x264enc`,
    /// `openh264enc`, both AAC encoders), caps refused at linking or negotiation — so the next way
    /// is tried; `Ok(Attempted::Wrong)` when the source no longer has what was read of it (fewer
    /// audio streams than a mix names); `Err(AttemptFailure)` for an error out of the running
    /// pipeline (its GLib error domain as `kind`, its code as `code` — never its message). A file
    /// it made before failing is deleted before it answers anything but `Taken`.
    ///
    /// `cancel` is the person's cancel or the ceiling: when raised, set the pipeline to `NULL`,
    /// delete the file, and answer at once — anything. Dropping the future does the same.
    fn encode<'a>(
        &'a self,
        source: &'a Path,
        ask: EncodeAsk,
        cancel: Cancel,
    ) -> LocalBoxFuture<'a, Result<Attempted<PathBuf>, AttemptFailure>>;

    /// A file's length in bytes, `None` when it cannot be asked.
    fn file_length(&self, file: &Path) -> Option<u64>;

    /// A file's whole contents, `None` when it cannot be read.
    fn read_file<'a>(&'a self, file: &'a Path) -> LocalBoxFuture<'a, Option<Vec<u8>>>;

    /// Delete a file [`encode`](Self::encode) made. Idempotent; a failure is logged (its kind
    /// only) and forgotten.
    fn remove(&self, file: &Path);
}

/// A picked video as the router let it through (`kind=video`).
#[derive(Debug, Clone, Copy)]
pub struct VideoJob<'a> {
    pub file: &'a Path,
    /// `"video/quicktime"` or `"video/mp4"` — what it would be uploaded as.
    pub container: &'a str,
    pub size: u64,
    /// Its first bytes — enough for `fc_text::media::matches_magic`.
    pub head: &'a [u8],
    /// Its length, as the shell read it for the bubble, or `None`.
    pub duration_ms: Option<i64>,
}

/// A picked sound file (`kind=audio`, or a sound file the router sent as a file —
/// `media_encoding::picked_audio_type`).
#[derive(Debug, Clone, Copy)]
pub struct SoundJob<'a> {
    pub file: &'a Path,
    /// The audio type it is judged as: one the server takes (`audio/mp4`, `audio/mpeg`,
    /// `audio/wav`, `audio/ogg`, …) or the desktop's own name for one it does not (`audio/x-aiff`,
    /// `audio/flac`).
    pub container: &'a str,
    /// Whether the router sent it as audio (else as a file, where 1.1 sent it as a file).
    pub as_audio: bool,
    pub size: u64,
    pub head: &'a [u8],
    pub duration_ms: Option<i64>,
}

/// What goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Prepared {
    /// The source, untouched, as its kind — rule A, rule C or rule D's original.
    Original,
    /// The source as a FILE (`kind=file`): rule C's "today's path" for a sound file the router
    /// sent as one.
    AsFile,
    /// Refused as too large, as 1.1 refused it.
    TooLarge,
    /// A transcode within the profile.
    Made(Made),
}

/// A transcode that is sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Made {
    pub bytes: Vec<u8>,
    /// `"video/mp4"` or `"audio/mp4"`.
    pub mime: &'static str,
    /// A video's DISPLAYED sides — the planner's target; `None` for a sound.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Whether its index is in front of its media data. A video that goes with it at the end is
    /// one whose source could not have gone at all.
    pub moov_first: bool,
}

/// Why an attempt's result is not taken, and the line that says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub end: AttemptEnd,
    pub line: String,
}

impl Verdict {
    fn wrong(line: impl Into<String>) -> Self {
        Verdict {
            end: AttemptEnd::Wrong,
            line: line.into(),
        }
    }

    fn refused(line: impl Into<String>) -> Self {
        Verdict {
            end: AttemptEnd::Refused,
            line: line.into(),
        }
    }
}

/// What the planner is given for a video, and what an attempt needs besides.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoInput {
    pub source: VideoSource,
    pub rotation: u32,
    pub frame_rate_numerator: u32,
    pub frame_rate_denominator: u32,
    pub audio_sample_rate: Option<i64>,
    pub hdr: bool,
}

/// The H.264 profiles every player plays (8-bit 4:2:0). A result in any other — `high-10`,
/// `high-4:2:2`, `high-4:4:4` — is the trap of an unpinned raw format (shell-design §1.4), and no
/// browser or Apple decoder plays it.
const PLAYABLE_PROFILES: [&str; 6] = [
    "constrained-baseline",
    "baseline",
    "main",
    "high",
    "progressive-high",
    "constrained-high",
];

// --- reading ------------------------------------------------------------------------------------

/// What the planner is given for a video, from the discoverer's reading (`ReadVideoAsync`); `Err`
/// with the log line for the sources whose transcode would come out WRONG with every number right
/// — no video stream, pixels that are not square, sound the decoder does not show — which go by
/// rule C. `container_sound_track` is the container's first sound track (the second witness of
/// `media_encoding::hides_audio`), `None` when it has none or cannot be walked.
pub fn video_input(
    reading: &StreamReading,
    container: &str,
    size: u64,
    duration_ms: Option<i64>,
    container_sound_track: Option<&SoundTrack>,
) -> Result<VideoInput, &'static str> {
    let Some(video) = &reading.video else {
        return Err("a video has no stream the discoverer reads as video");
    };
    if !square_pixels(video.pixel_aspect_numerator, video.pixel_aspect_denominator) {
        return Err(
            "a video's pixels are not square; its stored sides are not the size it is shown at, so it is left as it is",
        );
    }
    let audio = reading.audio.first();
    if hides_audio(
        audio.is_some(),
        container_sound_track.and_then(|track| track.channels.map(i64::from)),
        container_sound_track.and_then(|track| track.sample_rate.map(i64::from)),
        None,
    ) {
        return Err(
            "a video has sound its container lists and the discoverer does not; transcoding it would lose the sound, so it is left as it is",
        );
    }
    let rotation = rotation(video.rotation, None);
    let (width, height) = displayed(i64::from(video.width), i64::from(video.height), rotation);
    let unsigned = |value: Option<i64>| value.and_then(|value| u64::try_from(value).ok());
    let source = VideoSource {
        width: u32::try_from(width).unwrap_or(0),
        height: u32::try_from(height).unwrap_or(0),
        frame_rate: frame_rate(
            video.frame_rate_numerator,
            video.frame_rate_denominator,
            video.container_frame_rate,
        ),
        container: container.to_string(),
        video_codec: video.codec.clone(),
        audio_codec: audio.map(|audio| audio.codec.clone()),
        audio_channels: audio
            .and_then(|audio| first_known(&[audio.channels]))
            .and_then(|count| u32::try_from(count).ok()),
        video_bitrate: unsigned(first_known(&[video.bitrate])),
        audio_bitrate: unsigned(audio.and_then(|audio| first_known(&[audio.bitrate]))),
        size_bytes: size,
        duration_ms: unsigned(duration_ms),
    };
    Ok(VideoInput {
        source,
        rotation,
        frame_rate_numerator: video.frame_rate_numerator,
        frame_rate_denominator: video.frame_rate_denominator,
        audio_sample_rate: audio.and_then(|audio| first_known(&[audio.sample_rate])),
        hdr: is_hdr(video.transfer, video.primaries),
    })
}

/// What the planner is given for a sound file (`ReadAudioAsync`): the first audio stream's codec,
/// channels and bit rate — a stream the discoverer cannot read is still judged, as a codec nobody
/// can name: the rules leave that alone unless it is in an Ogg, and an Ogg this machine cannot
/// decode then fails its transcode — rule C either way.
pub fn sound_source(
    reading: Option<&StreamReading>,
    container: &str,
    size: u64,
    duration_ms: Option<i64>,
) -> AudioSource {
    let audio = reading.and_then(|reading| reading.audio.first());
    AudioSource {
        container: container.to_string(),
        codec: audio.map_or_else(|| "unknown".to_string(), |audio| audio.codec.clone()),
        channels: audio
            .and_then(|audio| first_known(&[audio.channels]))
            .and_then(|count| u32::try_from(count).ok()),
        bitrate: audio
            .and_then(|audio| first_known(&[audio.bitrate]))
            .and_then(|rate| u64::try_from(rate).ok()),
        size_bytes: size,
        duration_ms: duration_ms.and_then(|ms| u64::try_from(ms).ok()),
    }
}

// --- judging ------------------------------------------------------------------------------------

/// Whether a transcode made what was asked for, by its numbers (`VideoCameOutAsync` before its
/// frame check): H.264 in a profile every player plays, its audio kept or left out as asked, no
/// sound track lost (Rule E: `source_sound` and `result_sound` are the playing sound tracks the
/// container lists of each, `None` when nobody could count), the encoded sides and the SAME turn,
/// a frame rate no higher than asked, SDR where SDR was asked for, and the audio rate asked for.
/// Only what another way of asking may change is a refusal: a profile the next encoder may get
/// right, an audio rate, a dropped pass-through; everything else is Wrong.
pub fn judge_video(
    asked: &VideoEncoding,
    back: &StreamReading,
    source_sound: Option<usize>,
    result_sound: Option<usize>,
) -> Result<(), Verdict> {
    let Some(video) = back.video.as_ref().filter(|video| video.codec == "h264") else {
        return Err(Verdict::wrong(format!(
            "a transcode came out as {}, not H.264",
            back.video
                .as_ref()
                .map_or("no video", |video| video.codec.as_str())
        )));
    };
    profile_came_out(video, "a transcode")?;
    if back.audio.is_empty() == asked.audio.is_some() {
        // A track that was to be passed through and was dropped instead may still be encoded.
        return Err(lost(
            asked,
            "a transcode came out with its audio track added or lost",
        ));
    }
    if source_sound.is_some_and(|count| count > 0)
        && container_sound::lost_sound(source_sound, result_sound)
    {
        return Err(lost(
            asked,
            "a transcode came out with no sound track where its source had one",
        ));
    }
    geometry_came_out(asked, video, "a transcode")?;
    rate_and_colour_came_out(asked, video, "a transcode")?;
    if let (Some(aac), Some(audio)) = (asked.audio, back.audio.first())
        && !audio_rate_came_out(aac.bitrate, audio.bitrate)
    {
        return Err(Verdict::refused(format!(
            "a transcode's audio came out at {} bit/s; asked {}",
            number(audio.bitrate),
            aac.bitrate
        )));
    }
    Ok(())
}

/// Whether a MIX made what was asked, by its numbers (`MixCameOutAsync` before its frame check):
/// everything [`judge_video`] holds a transcode to, AAC in the channels asked, and ONE sound track
/// in the result's own track list (`result_tracks`: every `soun` track the container lists, `None`
/// when it cannot be walked). On Windows a composition might turn the pixels or pick a sound format
/// of its own, so those were refusals; on Linux a mix is the transcode's own pipeline with an
/// `audiomixer`, and what would be Wrong for a transcode is Wrong for a mix.
pub fn judge_mix(
    asked: &VideoEncoding,
    back: &StreamReading,
    result_tracks: Option<usize>,
) -> Result<(), Verdict> {
    let Some(video) = back.video.as_ref().filter(|video| video.codec == "h264") else {
        return Err(Verdict::wrong(format!(
            "a mix came out as {}, not H.264",
            back.video
                .as_ref()
                .map_or("no video", |video| video.codec.as_str())
        )));
    };
    profile_came_out(video, "a mix")?;
    let (Some(audio), Some(aac)) = (back.audio.first(), asked.audio) else {
        return Err(Verdict::wrong("a mix came out with no sound"));
    };
    if !aac_came_out(&aac, &audio.codec, audio.channels) {
        return Err(Verdict::refused(format!(
            "a mix's sound came out {}, {} channel(s); asked {aac}",
            audio.codec,
            number(audio.channels)
        )));
    }
    if !container_sound::mix_came_out(result_tracks) {
        return Err(Verdict::wrong(format!(
            "a mix came out with {} sound tracks, not one",
            result_tracks.map_or_else(|| "an unreadable number of".to_string(), |n| n.to_string())
        )));
    }
    geometry_came_out(asked, video, "a mix")?;
    rate_and_colour_came_out(asked, video, "a mix")?;
    if !audio_rate_came_out(aac.bitrate, audio.bitrate) {
        return Err(Verdict::refused(format!(
            "a mix's sound came out at {} bit/s; asked {}",
            number(audio.bitrate),
            aac.bitrate
        )));
    }
    Ok(())
}

/// Whether a sound file's re-encode made AAC with the channels asked for, at the rate asked for
/// (`AudioCameOutAsync`). One that came out at some other rate leaves the next way of asking to
/// try.
pub fn judge_sound(asked: &AacEncoding, back: &StreamReading) -> Result<(), Verdict> {
    let Some(audio) = back
        .audio
        .first()
        .filter(|audio| aac_came_out(asked, &audio.codec, audio.channels))
    else {
        return Err(Verdict::wrong(format!(
            "a re-encode came out as {}, {} channel(s)",
            back.audio
                .first()
                .map_or("no audio", |audio| audio.codec.as_str()),
            number(back.audio.first().and_then(|audio| audio.channels))
        )));
    };
    if !audio_rate_came_out(asked.bitrate, audio.bitrate) {
        return Err(Verdict::refused(format!(
            "a re-encode came out at {} bit/s; asked {}",
            number(audio.bitrate),
            asked.bitrate
        )));
    }
    Ok(())
}

/// A profile no player plays is the next encoder's to get right. A profile the caps do not state
/// is not judged.
fn profile_came_out(video: &VideoStream, what: &str) -> Result<(), Verdict> {
    match &video.profile {
        Some(profile) if !PLAYABLE_PROFILES.contains(&profile.as_str()) => Err(Verdict::refused(
            format!("{what} came out as H.264 {profile}, which not every player plays"),
        )),
        _ => Ok(()),
    }
}

fn geometry_came_out(
    asked: &VideoEncoding,
    video: &VideoStream,
    what: &str,
) -> Result<(), Verdict> {
    let turned = rotation(video.rotation, None);
    if matches(asked, video.width, video.height, turned) {
        Ok(())
    } else {
        Err(Verdict::wrong(format!(
            "{what} came out {}x{} turned {turned}; asked {}x{} turned {}",
            video.width, video.height, asked.width, asked.height, asked.rotation
        )))
    }
}

fn rate_and_colour_came_out(
    asked: &VideoEncoding,
    video: &VideoStream,
    what: &str,
) -> Result<(), Verdict> {
    // The result's own ratio only — not its container's average: what was written is judged.
    let rate = frame_rate(
        video.frame_rate_numerator,
        video.frame_rate_denominator,
        None,
    );
    if !frame_rate_came_out(asked, rate) {
        // The bitrate was worked out for the rate asked for: this many frames share it, and the
        // profile says "at most 30".
        return Err(Verdict::wrong(format!(
            "{what} came out at {} fps; asked {}/{}",
            rate.map_or_else(|| "unknown".to_string(), |rate| format!("{rate:.3}")),
            asked.frame_rate_numerator,
            asked.frame_rate_denominator
        )));
    }
    if asked.to_sdr && is_hdr(video.transfer, video.primaries) {
        return Err(Verdict::wrong(format!(
            "{what} of an HDR source still says it is HDR"
        )));
    }
    Ok(())
}

fn lost(asked: &VideoEncoding, line: &str) -> Verdict {
    if asked.keep_audio {
        Verdict::refused(line)
    } else {
        Verdict::wrong(line)
    }
}

fn number(value: Option<i64>) -> String {
    value.map_or_else(|| "unknown".to_string(), |value| value.to_string())
}

// --- preparing ----------------------------------------------------------------------------------

/// A picked video made ready to send (`VideoAsync`): `Ok(Prepared)` — what goes — or
/// `Err(OperationCanceled)` once `cancel` is raised. `log` is told what happened.
pub async fn prepare_video(
    job: &VideoJob<'_>,
    media: &dyn MediaPreparing,
    clock: &dyn Clock,
    log: &mut dyn FnMut(&str),
    cancel: &Cancel,
) -> Result<Prepared, OperationCanceled> {
    let ceiling = media::SIZE_LIMIT;
    let sendable = media_plan::sendable(
        "video",
        job.container,
        media::matches_magic(job.container, job.head),
        job.size,
        ceiling,
    );
    let reading = media.read_streams(job.file).await;
    if cancel.is_cancelled() {
        return Err(OperationCanceled);
    }
    // The container's tracks are a second witness only for a reading with no audio in it.
    let mut tracks: Option<Option<Vec<SoundTrack>>> = None;
    if reading
        .as_ref()
        .is_some_and(|reading| reading.audio.is_empty())
    {
        tracks = Some(media.sound_tracks(job.file).await);
    }
    let input = match &reading {
        None => {
            log("a video's streams could not be read");
            None
        }
        Some(reading) => match video_input(
            reading,
            job.container,
            job.size,
            job.duration_ms,
            tracks.as_ref().and_then(|tracks| tracks.as_ref()?.first()),
        ) {
            Ok(input) => Some(input),
            Err(line) => {
                log(line);
                None
            }
        },
    };
    let plan = input.as_ref().map_or(VideoPlan::Fallback, |input| {
        media_plan::plan_video(&input.source)
    });
    if plan == VideoPlan::Keep {
        // Rule A: already within the profile. Re-encoding it would only cost it quality.
        return Ok(as_before(job.size, ceiling));
    }
    if let (Some(input), Some(reading), VideoPlan::Transcode(target)) = (&input, &reading, plan) {
        if cancel.is_cancelled() {
            return Err(OperationCanceled);
        }
        // Rule E (issue #98): the sound tracks are counted from the container's own track list —
        // what plays, not what the decoder shows — and only for a source about to be transcoded:
        // rule A sends the others untouched, every track and all.
        let tracks = match tracks {
            Some(tracks) => tracks,
            None => media.sound_tracks(job.file).await,
        };
        let sound = tracks
            .as_ref()
            .map(|tracks| container_sound::playing_indexes(tracks).len());
        let result = match container_sound::before_transcode(sound, !reading.audio.is_empty()) {
            SoundBefore::Unheard => {
                // Nothing here can decode the track, so nothing can keep it, and the original goes
                // (rule C).
                log(
                    "a video has a sound track the discoverer does not show; transcoding it would lose the sound, so it is left as it is",
                );
                None
            }
            SoundBefore::Mix => {
                let tracks = tracks.as_deref().unwrap_or_default();
                mix(
                    job, input, reading, &target, tracks, media, clock, log, cancel,
                )
                .await?
            }
            SoundBefore::Transcode => {
                // The audio asked for (or "none") and the count it was decided from: a silent
                // transcode is then something the log shows, not something a recipient finds out.
                log(&format!(
                    "transcoding a video to {}x{}, {:.3} fps, {} bit/s, audio {}, sound tracks {}",
                    target.width,
                    target.height,
                    target.frame_rate,
                    target.video_bitrate,
                    target
                        .audio_bitrate
                        .map_or_else(|| "none".to_string(), |rate| format!("{rate} bit/s")),
                    sound.map_or_else(|| "unknown".to_string(), |count| count.to_string())
                ));
                let attempts = video_attempts(
                    &target,
                    input.rotation,
                    input.frame_rate_numerator,
                    input.frame_rate_denominator,
                    input.source.audio_channels.map(i64::from),
                    input.audio_sample_rate,
                    input.hdr,
                );
                let said = RefCell::new(Vec::new());
                let made = transcode_attempts::first_taken(
                    &attempts,
                    |asked, linked| video_attempt(media, job, asked, None, sound, linked, &said),
                    transcode_ceiling(job.duration_ms),
                    clock,
                    |line| said.borrow_mut().push(line.to_string()),
                    cancel,
                )
                .await;
                flush(&said, log);
                made?
            }
        };
        if let Some(result) = result {
            return Ok(chosen(
                result,
                job.size,
                sendable,
                Some((target.width, target.height)),
                log,
            ));
        }
    }
    // Rule C: no size to scale to, a stream nothing can read, or a transcode that failed. For a
    // video the router let through, "the original" and "what 1.1 did" are the same thing:
    // untouched within the ceiling, refused over it.
    Ok(match media_plan::on_failure(sendable) {
        OnFailure::Original => Prepared::Original,
        OnFailure::TodaysPath => as_before(job.size, ceiling),
    })
}

/// A picked sound file made ready to send (`AudioAsync`).
pub async fn prepare_sound(
    job: &SoundJob<'_>,
    media: &dyn MediaPreparing,
    clock: &dyn Clock,
    log: &mut dyn FnMut(&str),
    cancel: &Cancel,
) -> Result<Prepared, OperationCanceled> {
    let ceiling = media::SIZE_LIMIT;
    let sendable = media_plan::sendable(
        "audio",
        job.container,
        media::matches_magic(job.container, job.head),
        job.size,
        ceiling,
    );
    let reading = media.read_streams(job.file).await;
    if cancel.is_cancelled() {
        return Err(OperationCanceled);
    }
    let source = sound_source(reading.as_ref(), job.container, job.size, job.duration_ms);
    if let AudioPlan::Transcode { bitrate } = media_plan::plan_audio(&source) {
        log(&format!(
            "re-encoding a sound file ({}) at {bitrate} bit/s",
            source.codec
        ));
        let sample_rate = reading
            .as_ref()
            .and_then(|reading| reading.audio.first())
            .and_then(|audio| first_known(&[audio.sample_rate]));
        let attempts = audio_attempts(
            i64::try_from(bitrate).unwrap_or(i64::MAX),
            source.channels.map(i64::from),
            sample_rate,
        );
        let said = RefCell::new(Vec::new());
        let made = transcode_attempts::first_taken(
            &attempts,
            |asked, linked| sound_attempt(media, job.file, asked, linked, &said),
            transcode_ceiling(job.duration_ms),
            clock,
            |line| said.borrow_mut().push(line.to_string()),
            cancel,
        )
        .await;
        flush(&said, log);
        if let Some(result) = made? {
            return Ok(chosen(result, job.size, sendable, None, log));
        }
        // Rule C. A sendable source is one the router already sent as audio, so this is what 1.1
        // did with it too.
        if media_plan::on_failure(sendable) == OnFailure::Original {
            return Ok(Prepared::Original);
        }
    }
    // Kept, or rule C's "today's path": what 1.1 did — as audio within the ceiling, as a file when
    // it was one.
    Ok(if !job.as_audio {
        Prepared::AsFile
    } else if job.size > ceiling {
        Prepared::TooLarge
    } else {
        Prepared::Original
    })
}

/// A transcode that ran and came out as asked: how long it is, its bytes when it is within the
/// ceiling (one over it is never sent — rule D picks the source or nothing — so it is never read
/// into memory either), and whether its index is in front.
#[derive(Debug)]
struct Transcoded {
    length: u64,
    bytes: Option<Vec<u8>>,
    moov_first: bool,
    mime: &'static str,
}

/// What 1.1 did with every video: untouched, and refused over the ceiling.
fn as_before(size: u64, ceiling: u64) -> Prepared {
    if size > ceiling {
        Prepared::TooLarge
    } else {
        Prepared::Original
    }
}

/// Rule D on a result (`ChosenAsync`). `shown` is a video's displayed target; `None` for a sound.
fn chosen(
    result: Transcoded,
    size: u64,
    sendable: bool,
    shown: Option<(u32, u32)>,
    log: &mut dyn FnMut(&str),
) -> Prepared {
    let video = shown.is_some();
    match choose(
        size,
        sendable,
        result.length,
        result.moov_first,
        video,
        media::SIZE_LIMIT,
    ) {
        Chosen::Original => {
            log(&if result.length > size {
                format!(
                    "a transcode came out bigger than its source ({} > {size} bytes); the original goes instead",
                    result.length
                )
            } else {
                "a transcode's index could not be moved to the front; the original goes instead"
                    .to_string()
            });
            Prepared::Original
        }
        // Within the ceiling is exactly when its bytes were read in.
        Chosen::Result if result.bytes.is_some() => {
            if !result.moov_first {
                log(if video {
                    "a transcode goes with its index at the end: the original could not have gone at all"
                } else {
                    "a re-encode goes with its index at the end: the profile asks for it in front of a video only"
                });
            }
            Prepared::Made(Made {
                bytes: result.bytes.unwrap_or_default(),
                mime: result.mime,
                width: shown.map(|(width, _)| width),
                height: shown.map(|(_, height)| height),
                moov_first: result.moov_first,
            })
        }
        _ => {
            log(&format!(
                "a transcode is still over the ceiling ({} bytes)",
                result.length
            ));
            Prepared::TooLarge
        }
    }
}

/// A video whose sound is on more than one track that plays, transcoded with them all MIXED into
/// the one AAC-LC track the profile allows (`MixVideoAsync`; Rule E). `Ok(None)` when the mix
/// cannot be made or did not come out right — rule C.
async fn mix(
    job: &VideoJob<'_>,
    input: &VideoInput,
    reading: &StreamReading,
    target: &media_plan::VideoTarget,
    tracks: &[SoundTrack],
    media: &dyn MediaPreparing,
    clock: &dyn Clock,
    log: &mut dyn FnMut(&str),
    cancel: &Cancel,
) -> Result<Option<Transcoded>, OperationCanceled> {
    let decoded: Vec<DecodedTrack> = reading.audio.iter().map(decoded).collect();
    let Some(mixed) = container_sound::mix_map(tracks, &decoded).filter(|mixed| mixed.len() > 1)
    else {
        // Fewer listed streams than sound tracks (one nothing here can decode), or ones that do not
        // line up: a mix of the wrong ones would keep fewer voices than the source has.
        log(&format!(
            "a video's {} sound tracks do not line up with the {} audio streams the discoverer reads; it is left as it is",
            tracks.len(),
            decoded.len()
        ));
        return Ok(None);
    };
    let sound = container_sound::mix_format(mixed.iter().map(|&at| (decoded[at], tracks[at])));
    let attempts = mix_attempts(
        target,
        input.rotation,
        input.frame_rate_numerator,
        input.frame_rate_denominator,
        &sound,
        input.hdr,
    );
    let aac = attempts.first().and_then(|attempt| attempt.audio);
    log(&format!(
        "mixing {} sound tracks (track ids {}) into a video at {}x{}, {:.3} fps, {} bit/s, audio {} bit/s, {} channel(s), {} Hz",
        mixed.len(),
        mixed
            .iter()
            .map(|&at| tracks[at].track_id.to_string())
            .collect::<Vec<_>>()
            .join(","),
        target.width,
        target.height,
        target.frame_rate,
        target.video_bitrate,
        aac.map_or_else(String::new, |aac| aac.bitrate.to_string()),
        aac.map_or_else(String::new, |aac| aac.channels.to_string()),
        aac.map_or_else(String::new, |aac| aac.sample_rate.to_string()),
    ));
    let said = RefCell::new(Vec::new());
    let made = transcode_attempts::first_taken(
        &attempts,
        |asked, linked| {
            video_attempt(
                media,
                job,
                asked,
                Some(mixed.as_slice()),
                None,
                linked,
                &said,
            )
        },
        transcode_ceiling(job.duration_ms),
        clock,
        |line| said.borrow_mut().push(line.to_string()),
        cancel,
    )
    .await;
    flush(&said, log);
    made
}

fn decoded(audio: &AudioStream) -> DecodedTrack {
    let narrow = |value: Option<i64>| value.and_then(|value| u32::try_from(value).ok());
    DecodedTrack {
        channels: narrow(audio.channels),
        sample_rate: narrow(audio.sample_rate),
        bitrate: narrow(audio.bitrate),
    }
}

/// One way of asking for a video — a transcode, or a mix of `mix`'s streams — into a file of its
/// own that is gone again when this returns, so what a failed attempt wrote is never underneath the
/// next one's result. `source_sound` is the source's playing sound tracks (a transcode's only).
async fn video_attempt(
    media: &dyn MediaPreparing,
    job: &VideoJob<'_>,
    asked: &VideoEncoding,
    mix: Option<&[usize]>,
    source_sound: Option<usize>,
    cancel: Cancel,
    said: &RefCell<Vec<String>>,
) -> Result<Attempted<Transcoded>, AttemptFailure> {
    let ask = EncodeAsk::Video {
        encoding: *asked,
        mix: mix.map(<[usize]>::to_vec),
    };
    let output = match media.encode(job.file, ask, cancel.clone()).await? {
        Attempted::Taken(output) => output,
        Attempted::Refused => {
            said.borrow_mut()
                .push(format!("a transcode profile was refused ({asked})"));
            return Ok(Attempted::Refused);
        }
        Attempted::Wrong => return Ok(Attempted::Wrong),
    };
    let _gone = Removed {
        media,
        file: &output,
    };
    let what = if mix.is_some() {
        "a mix"
    } else {
        "a transcode"
    };
    let Some(back) = media.read_streams(&output).await else {
        said.borrow_mut()
            .push(format!("{what} could not be read back ({asked})"));
        return Ok(Attempted::Refused);
    };
    let judged = if mix.is_some() {
        let tracks = media.sound_tracks(&output).await;
        judge_mix(asked, &back, tracks.map(|tracks| tracks.len()))
    } else {
        let result_sound = if source_sound.is_some_and(|count| count > 0) {
            media
                .sound_tracks(&output)
                .await
                .map(|tracks| container_sound::playing_indexes(&tracks).len())
        } else {
            None
        };
        judge_video(asked, &back, source_sound, result_sound)
    };
    if let Err(verdict) = judged {
        said.borrow_mut().push(verdict.line);
        return Ok(ended(verdict.end));
    }
    if !looks_like_its_source(
        media,
        job.file,
        &output,
        asked,
        job.duration_ms,
        &cancel,
        said,
    )
    .await
    {
        return Ok(Attempted::Wrong);
    }
    Ok(finished(media, &output, "video/mp4", said).await)
}

/// One way of asking for a sound file's re-encode, into a file of its own.
async fn sound_attempt(
    media: &dyn MediaPreparing,
    source: &Path,
    asked: &AacEncoding,
    cancel: Cancel,
    said: &RefCell<Vec<String>>,
) -> Result<Attempted<Transcoded>, AttemptFailure> {
    let output = match media
        .encode(source, EncodeAsk::Sound(*asked), cancel)
        .await?
    {
        Attempted::Taken(output) => output,
        Attempted::Refused => {
            said.borrow_mut()
                .push(format!("a re-encode was refused ({asked})"));
            return Ok(Attempted::Refused);
        }
        Attempted::Wrong => return Ok(Attempted::Wrong),
    };
    let _gone = Removed {
        media,
        file: &output,
    };
    let Some(back) = media.read_streams(&output).await else {
        said.borrow_mut()
            .push(format!("a re-encode could not be read back ({asked})"));
        return Ok(Attempted::Refused);
    };
    if let Err(verdict) = judge_sound(asked, &back) {
        said.borrow_mut().push(verdict.line);
        return Ok(ended(verdict.end));
    }
    Ok(finished(media, &output, "audio/mp4", said).await)
}

fn ended(end: AttemptEnd) -> Attempted<Transcoded> {
    match end {
        AttemptEnd::Wrong => Attempted::Wrong,
        _ => Attempted::Refused,
    }
}

/// Whether a frame of the result is the same picture as that frame of the source, the same way up
/// (`LooksLikeItsSourceAsync`) — what the numbers cannot say. Both are read by ONE reader, at the
/// moments a poster is looked for, until one says something. A source no frame can be read from
/// cannot be judged and is let through; a RESULT no frame can be read from, beside a source that
/// gave one, is not a video to send.
async fn looks_like_its_source(
    media: &dyn MediaPreparing,
    source: &Path,
    output: &Path,
    asked: &VideoEncoding,
    duration_ms: Option<i64>,
    cancel: &Cancel,
    said: &RefCell<Vec<String>>,
) -> bool {
    let mut frames = Vec::new();
    for seconds in media::POSTER_SEEK_SECONDS {
        if cancel.is_cancelled() {
            // The loop sees the cancel as soon as this answers.
            return false;
        }
        let at = match duration_ms {
            Some(duration) if duration > 0 => {
                seconds.min((duration as f64 / 1000.0 - 0.05).max(0.0))
            }
            _ => seconds,
        };
        let Some(before) = media
            .frame(source, at)
            .await
            .and_then(|bgra| frame_turn::grid(&bgra))
        else {
            frames.push(FrameVerdict::CannotTell);
            continue;
        };
        let verdict = media
            .frame(output, at)
            .await
            .and_then(|bgra| frame_turn::grid(&bgra))
            .map_or(FrameVerdict::Unlike, |after| {
                frame_turn::judge(&before, &after)
            });
        frames.push(verdict);
        if matches!(verdict, FrameVerdict::Same | FrameVerdict::Turned) {
            break;
        }
    }
    if frame_turn::looks_right(frames.iter().copied()) {
        if !frames.contains(&FrameVerdict::Same) {
            said.borrow_mut().push(
                "no frame of a transcode could be compared with its source; it goes on its numbers alone"
                    .to_string(),
            );
        }
        return true;
    }
    said.borrow_mut().push(format!(
        "a transcode does not look like its source ({}) ({asked})",
        frames
            .iter()
            .map(|frame| format!("{frame:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    false
}

/// A result that came out as asked, taken in (`FinishedAsync`): its length; its bytes, when it is
/// within the ceiling, with its index moved to the front where it is not; or Wrong when its bytes
/// are not the type they are sent as.
async fn finished(
    media: &dyn MediaPreparing,
    output: &Path,
    mime: &'static str,
    said: &RefCell<Vec<String>>,
) -> Attempted<Transcoded> {
    let Some(length) = media.file_length(output) else {
        said.borrow_mut()
            .push("a transcode's length could not be read".to_string());
        return Attempted::Refused;
    };
    if length > media::SIZE_LIMIT {
        // Where its index is, nobody needs to know: rule D sends the source, or nothing.
        return Attempted::Taken(Transcoded {
            length,
            bytes: None,
            moov_first: false,
            mime,
        });
    }
    let Some(bytes) = media.read_file(output).await else {
        said.borrow_mut()
            .push("a transcode could not be read in".to_string());
        return Attempted::Refused;
    };
    if !media::matches_magic(mime, &bytes) {
        said.borrow_mut()
            .push("a transcode's bytes are not the type they are sent as".to_string());
        return Attempted::Wrong;
    }
    // `mp4mux faststart=true` already put the index in front; a file that came out otherwise is
    // rearranged here, and kept as it is when that cannot be done safely.
    let arranged = faststart::moov_first(&bytes).map(|arranged| match arranged {
        Cow::Borrowed(_) => None,
        Cow::Owned(moved) => Some(moved),
    });
    let (bytes, moov_first) = match arranged {
        Some(Some(moved)) => (moved, true),
        Some(None) => (bytes, true),
        None => (bytes, false),
    };
    Attempted::Taken(Transcoded {
        length,
        bytes: Some(bytes),
        moov_first,
        mime,
    })
}

/// The lines an attempt and the loop said, in order, handed to the log once the loop is over.
fn flush(said: &RefCell<Vec<String>>, log: &mut dyn FnMut(&str)) {
    for line in said.take() {
        log(&line);
    }
}

/// A temporary file of the shell's, removed when this goes — whichever way the attempt ends,
/// dropped mid-way included.
struct Removed<'a> {
    media: &'a dyn MediaPreparing,
    file: &'a Path,
}

impl Drop for Removed<'_> {
    fn drop(&mut self) {
        self.media.remove(self.file);
    }
}
