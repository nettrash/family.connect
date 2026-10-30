//! The half of "Preparing media before upload" that needs no GStreamer (`MediaEncoding.cs`): what
//! an encoder is asked for a plan, and whether what came out is what was asked (docs/protocol.md;
//! the planner is `fc_text::media_plan`, the platform's half is the shell's — [`preparing`] runs the
//! attempts over its [`preparing::MediaPreparing`] seam).
//!
//! THE LINUX FORM. Windows' file is built around Media Foundation's AAC encoder, which takes four
//! bitrates (96, 128, 160 and 192 kbit/s) and no others, so every encode asked for the planner's
//! exact number and then for the nearest documented rate that kept rule B, and passed a thin AAC
//! track through untouched where no documented rate was left. GStreamer's `avenc_aac` takes every
//! rate the profile names — 64 000 mono included (measured, shell-design §1.4) — and `voaacenc`
//! does too, so none of that exists here (carry-forward): an audio encode is ONE attempt at the
//! planner's exact number, and a video is asked for twice — H.264 High through `x264enc`, then
//! Main through `openh264enc` — with the same audio both times. The pass-through attempt went with
//! the ladder it was the last step of: it existed only for the track rule B left no documented
//! rate for, and on Linux the exact rate is always there.
//!
//! A TRANSCODE IS CHECKED, NOT TRUSTED. Nothing here can see GStreamer run, so the result is read
//! back (the discoverer) and [`matches()`], [`frame_rate_came_out`], [`audio_rate_came_out`] and
//! [`aac_came_out`] decide whether it is what was asked for — and `frame_turn` whether it still
//! LOOKS like its source, which the numbers alone cannot say. A result that came out sideways, at
//! the wrong size or faster than asked is a failed transcode, and rule C sends the original — a
//! picture can only be lost by being sent wrong, never by being sent as it was.

pub mod preparing;

use std::fmt;
use std::time::Duration;

use fc_text::media;
use fc_text::media_plan::{self, Upload, VideoTarget};

use crate::container_sound::MixSound;

/// Which H.264 profile an encode asks for — High, and Main where an encoder offers nothing else.
/// On Linux each names its encoder too: High is `x264enc` with `video/x-h264,profile=high` after
/// it, Main is `openh264enc` with `profile=main` (its "High" is constrained-high). The same type
/// the animation encoder takes (`animation_preparing::H264Profile`), so one shell encoder serves
/// both.
pub use crate::animation_preparing::H264Profile;

/// AAC-LC as the encoder is asked for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AacEncoding {
    pub sample_rate: u32,
    pub channels: u32,
    pub bitrate: u32,
}

impl fmt::Display for AacEncoding {
    /// "AAC 48000 Hz 2 ch 128000 bit/s" — numbers only, for the diagnostics log.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "AAC {} Hz {} ch {} bit/s",
            self.sample_rate, self.channels, self.bitrate
        )
    }
}

/// One way of asking the encoder for the video the planner decided on — the numbers that go into
/// the pipeline, and nothing that needs GStreamer to hold them.
///
/// `width`/`height` are the ENCODED sides: the stored orientation, which for a phone's portrait clip
/// is the landscape frame; the turn rides along as `rotation` (degrees, 0, 90, 180 or 270), as
/// metadata, exactly as it did in the source. `to_sdr` asks for BT.709 SDR explicitly — set only
/// for a source that is HDR. `keep_audio`: the audio asked for is the SOURCE'S OWN TRACK, handed back
/// exactly as it was read so it is passed through rather than encoded again (`audio` then holds that
/// track's own numbers) — never set by this module on Linux (see the module's notes); a shell that
/// meets it passes the source's track through (`qtdemux` → `aacparse` → `mp4mux`) instead of
/// encoding it. `keyframe_spacing`: the most frames from one keyframe to the next (`x264enc
/// key-int-max`), or `None` to leave it to the encoder, as every planned video does — only a video
/// message asks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoEncoding {
    pub width: u32,
    pub height: u32,
    pub rotation: u32,
    pub bitrate: u32,
    pub frame_rate_numerator: u32,
    pub frame_rate_denominator: u32,
    pub profile: H264Profile,
    pub audio: Option<AacEncoding>,
    pub to_sdr: bool,
    pub keep_audio: bool,
    pub keyframe_spacing: Option<u32>,
}

impl fmt::Display for VideoEncoding {
    /// "H.264 High 1280x720 turned 90, 2000000 bit/s, 30/1 fps, AAC 48000 Hz 2 ch 128000 bit/s" —
    /// numbers only, for the diagnostics log (`transcode_attempts` names the attempt that failed).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "H.264 {} {}x{} turned {}, {} bit/s, {}/{} fps",
            self.profile,
            self.width,
            self.height,
            self.rotation,
            self.bitrate,
            self.frame_rate_numerator,
            self.frame_rate_denominator
        )?;
        match self.audio {
            Some(audio) if self.keep_audio => write!(f, ", its own {audio}")?,
            Some(audio) => write!(f, ", {audio}")?,
            None => f.write_str(", no audio")?,
        }
        if self.to_sdr {
            f.write_str(", to SDR")?;
        }
        if let Some(spacing) = self.keyframe_spacing {
            write!(f, ", a keyframe every {spacing} at most")?;
        }
        Ok(())
    }
}

/// What is sent once a transcode has run: rule D, and what faststart could not do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Chosen {
    /// The transcode's bytes.
    Result,
    /// The source, untouched — the result is thrown away.
    Original,
    /// Nothing: the source could not have gone, and the result is over the ceiling too.
    TooLarge,
}

/// A voice note's sample rate: 44.1 kHz, as the Apple and Android recorders use (the protocol
/// allows 48).
pub const VOICE_NOTE_SAMPLE_RATE: u32 = 44_100;

/// The ISO/IEC 23091-2 (H.273) code points HDR is told by — what GStreamer's
/// `gst_video_transfer_function_to_iso` and `gst_video_color_primaries_to_iso` answer for a
/// stream's colorimetry, and what an MP4's `colr nclx` box stores. (Windows read Media
/// Foundation's own numbers, in which PQ is 15 and HLG 16; in these, 15 is BT.2020's 12-bit SDR
/// transfer.)
pub const TRANSFER_PQ: i64 = 16;
pub const TRANSFER_HLG: i64 = 18;
pub const PRIMARIES_BT2020: i64 = 9;

// --- reading ------------------------------------------------------------------------------------

/// A count a reader handed over, or `None`: the first that is more than 0 (0 is what "not stated"
/// reads as).
pub fn first_known(values: &[Option<i64>]) -> Option<i64> {
    values.iter().flatten().copied().find(|&value| value > 0)
}

/// The turn, as degrees: the first reader's when it states one, the second's otherwise. On Linux
/// the first is the discoverer's — the `image-orientation` tag `rotate-90/180/270` (a `flip-*`
/// orientation is no turn and is passed as `None`) — because it is the pipeline's view of the
/// same stream. Windows' second was the shell's `System.Video.Orientation`; on Linux `qtdemux`'s
/// tag IS the container's `tkhd` matrix, so [`preparing`] asks no second reader (`None`). Either
/// way the same reader reads the result back, so the two are compared in one convention. Anything
/// but 0, 90, 180 or 270 is no turn.
pub fn rotation(stated: Option<i64>, second: Option<i64>) -> u32 {
    let turn = |value: Option<i64>| match value {
        Some(degrees @ (0 | 90 | 180 | 270)) => Some(degrees as u32),
        _ => None,
    };
    turn(stated).or(turn(second)).unwrap_or(0)
}

/// The size as it is DISPLAYED: a quarter turn either way swaps the sides.
pub fn displayed(width: i64, height: i64, rotation: u32) -> (i64, i64) {
    if matches!(rotation, 90 | 270) {
        (height, width)
    } else {
        (width, height)
    }
}

/// The frame rate: the stream's ratio when it has one (the discoverer's `framerate` fraction), else
/// the container's own — the samples over the time they cover, `fc_text::mp4_read`'s
/// `Track::frame_rate`, which is what a variable-rate clip averages to — else unknown. (Windows'
/// second witness was the shell's `System.Video.FrameRate`, frames per thousand seconds; a
/// GStreamer discoverer answers 0/1 for a rate it will not name, and the index is the one other
/// reader there is.)
pub fn frame_rate(numerator: u32, denominator: u32, container: Option<f64>) -> Option<f64> {
    if numerator > 0 && denominator > 0 {
        Some(f64::from(numerator) / f64::from(denominator))
    } else {
        media_plan::known_frame_rate(container)
    }
}

/// Whether a video is HDR by what its colorimetry says — PQ or HLG, or BT.2020's primaries — in
/// ISO/IEC 23091-2 code points ([`TRANSFER_PQ`], [`TRANSFER_HLG`], [`PRIMARIES_BT2020`]). A wide
/// gamut counts on its own (as on Windows; `fc_text::mp4_read`'s `Colour::is_hdr` asks the
/// transfer alone): a BT.2020 picture has to be converted to BT.709 to be the profile's SDR either
/// way.
pub fn is_hdr(transfer: Option<i64>, primaries: Option<i64>) -> bool {
    matches!(transfer, Some(TRANSFER_PQ | TRANSFER_HLG)) || primaries == Some(PRIMARIES_BT2020)
}

/// Whether a video has sound the reader did not show: a second reader states a channel count, a
/// sample rate or an audio bit rate for a file whose streams came back with no audio stream.
/// Transcoding that would send a SILENT video where 1.1 sent the sound, and the read-back could not
/// tell — no track was asked for, none came out — so it is a source this machine cannot transcode
/// (rule C). On Linux the second reader is the container's own first sound track (its sample
/// entry's channels and rate — `container_sound::SoundTrack`; it states no bit rate).
pub fn hides_audio(
    audio_track_read: bool,
    second_channels: Option<i64>,
    second_sample_rate: Option<i64>,
    second_bitrate: Option<i64>,
) -> bool {
    !audio_track_read
        && first_known(&[second_channels, second_sample_rate, second_bitrate]).is_some()
}

/// Whether a stream's pixels are square — or say nothing, which every phone's and every screen
/// recording's do (the discoverer's `par_num`/`par_denom`). The protocol's `W × H` is the
/// DISPLAYED size and only names the rotation; an anamorphic source (HDV's 1440×1080 shown 16:9)
/// is wider than its stored sides, and scaling those sides would squash it for good. It is left
/// alone.
pub fn square_pixels(numerator: u32, denominator: u32) -> bool {
    numerator == 0 || denominator == 0 || numerator == denominator
}

/// The type a picked sound file is judged as when the router sent it as a FILE: its declared type
/// when that is audio — AIFF and FLAC are, and the server takes neither — and FLAC by its extension
/// when the desktop named no type. `None` for anything else, which goes as the file it always went
/// as.
pub fn picked_audio_type(declared: &str, name: &str) -> Option<String> {
    if declared.starts_with("audio/") {
        Some(declared.to_string())
    } else if media::extension(name) == "flac" {
        Some("audio/flac".to_string())
    } else {
        None
    }
}

/// A re-encoded track keeps its name with the extension it now has; a name there never was stays
/// absent. Sanitised again, as the web client does (`fc_text::media::m4a_name` then
/// `sanitized_name`): a name with no extension grows by four, and the limit cuts the stem, never
/// the ".m4a".
pub fn m4a_name(name: Option<&str>) -> Option<String> {
    media::sanitized_name(&media::m4a_name(name?))
}

// --- the profile --------------------------------------------------------------------------------

/// The ENCODED sides for a displayed target: a quarter turn is undone, and carried as metadata
/// instead.
pub fn encoded(width: i64, height: i64, rotation: u32) -> (u32, u32) {
    let (encoded_width, encoded_height) = displayed(width, height, rotation);
    (side(encoded_width), side(encoded_height))
}

/// The frame rate as the ratio a pipeline's caps take, NEVER above `target`: the source's own ratio
/// when the target is the source's rate (30000/1001 stays exactly that), a whole number over one,
/// and otherwise the nearest thousandth that is not above it.
pub fn frame_rate_ratio(target: f64, source_numerator: u32, source_denominator: u32) -> (u32, u32) {
    if source_numerator > 0
        && source_denominator > 0
        && f64::from(source_numerator) / f64::from(source_denominator) == target
    {
        return (source_numerator, source_denominator);
    }
    if !target.is_finite() || target <= 0.0 {
        return (media_plan::MAX_FRAME_RATE as u32, 1);
    }
    if target == target.floor() && target <= f64::from(u32::MAX) {
        return (target as u32, 1);
    }
    // `Math.Round(…, MidpointRounding.AwayFromZero)` is Rust's `round`.
    let mut thousandths = (target * 1000.0).round();
    if thousandths / 1000.0 > target {
        thousandths -= 1.0;
    }
    if thousandths < 1.0 {
        // Under a thousandth of a frame a second: one frame in however many seconds keeps it at or
        // under.
        return (1, (1.0 / target).ceil().min(f64::from(u32::MAX)) as u32);
    }
    let numerator = thousandths.min(f64::from(u32::MAX)) as u32;
    let divisor = gcd(numerator, 1000);
    (numerator / divisor, 1000 / divisor)
}

/// 44.1 or 48 kHz: the source's own when it is one, 48 above 44.1, 44.1 otherwise — the two rates
/// the protocol's voice note names and every player plays. (Media Foundation's encoder takes these
/// two and nothing else; `avenc_aac` would take more, and a third rate buys nothing.)
pub fn aac_sample_rate(source: Option<i64>) -> u32 {
    if source.is_some_and(|rate| rate > 44_100) {
        48_000
    } else {
        44_100
    }
}

/// Mono stays mono; anything else is stereo — the profile has a stereo row and a mono row, and 5.1
/// is folded down.
pub fn aac_channels(source: Option<i64>) -> u32 {
    if source == Some(1) { 1 } else { 2 }
}

/// A sound's encodes, in the order to ask for them. On Linux there is ONE: the planner's exact
/// bitrate — `avenc_aac` takes every rate the profile names (measured, shell-design §1.4), so
/// Windows' second attempt at Media Foundation's nearest documented rate does not exist here. The
/// shell tries `avenc_aac`, then `voaacenc`, for the same encoding.
pub fn audio_attempts(
    bitrate: i64,
    channels: Option<i64>,
    sample_rate: Option<i64>,
) -> Vec<AacEncoding> {
    vec![AacEncoding {
        sample_rate: aac_sample_rate(sample_rate),
        channels: aac_channels(channels),
        bitrate: rate(bitrate),
    }]
}

/// A voice note's encodes: the protocol's AAC-LC mono 64 000 at 44.1 kHz — one attempt
/// ([`audio_attempts`]).
pub fn voice_note_attempts() -> Vec<AacEncoding> {
    audio_attempts(
        media_plan::VOICE_NOTE_BITRATE as i64,
        Some(1),
        Some(i64::from(VOICE_NOTE_SAMPLE_RATE)),
    )
}

/// A video's encodes, in the order to ask for them: H.264 High (`x264enc`), then Main
/// (`openh264enc`) — the protocol's "Main where an encoder offers nothing else" — each with the
/// planner's exact audio rate ([`audio_attempts`]) and the same size, frame rate, video bitrate and
/// turn. `audio_channels` and `audio_sample_rate` are the source track's, as the reader stated
/// them. (Windows' file also took the source track's bit rate and whether it is AAC, for its
/// encoder's nearest rate and the pass-through; neither exists on Linux.)
pub fn video_attempts(
    target: &VideoTarget,
    rotation: u32,
    source_frame_rate_numerator: u32,
    source_frame_rate_denominator: u32,
    audio_channels: Option<i64>,
    audio_sample_rate: Option<i64>,
    hdr: bool,
) -> Vec<VideoEncoding> {
    let (width, height) = encoded(i64::from(target.width), i64::from(target.height), rotation);
    let (numerator, denominator) = frame_rate_ratio(
        target.frame_rate,
        source_frame_rate_numerator,
        source_frame_rate_denominator,
    );
    let audio = target.audio_bitrate.and_then(|bitrate| {
        audio_attempts(wide(bitrate), audio_channels, audio_sample_rate)
            .into_iter()
            .next()
    });
    let first = VideoEncoding {
        width,
        height,
        rotation,
        bitrate: rate(wide(target.video_bitrate)),
        frame_rate_numerator: numerator,
        frame_rate_denominator: denominator,
        profile: H264Profile::High,
        audio,
        to_sdr: hdr,
        keep_audio: false,
        keyframe_spacing: None,
    };
    vec![
        first,
        VideoEncoding {
            profile: H264Profile::Main,
            ..first
        },
    ]
}

/// A MIX's encodes (Rule E, issue #98): the ways [`video_attempts`] would ask, with the audio worked
/// out for the mix instead of for one track — `fc_text::media_plan::target_audio_bitrate` of its
/// channels, capped by the tracks' bit rates added up, at the highest sample rate any of them has.
/// The planner's own numbers are not touched: the target's audio is replaced, not recomputed.
///
/// ONE RENDER, SO ONE SHAPE. Windows mixes through a `MediaComposition`, not its transcoder, and
/// could not tell whether a composition carries a source's turn as metadata or turns the pixels, so
/// it asked a quarter-turned source both ways. On Linux a mix is the same pipeline as a transcode
/// with an `audiomixer` before the AAC encoder — the picture's branch, and the turn carried as the
/// `image-orientation` tag, are exactly the transcode's — so it is asked exactly as a transcode is.
pub fn mix_attempts(
    target: &VideoTarget,
    rotation: u32,
    source_frame_rate_numerator: u32,
    source_frame_rate_denominator: u32,
    mix: &MixSound,
    hdr: bool,
) -> Vec<VideoEncoding> {
    let channels = aac_channels(mix.channels);
    let mixed = VideoTarget {
        audio_bitrate: Some(media_plan::target_audio_bitrate(
            Some(channels),
            mix.bitrate.and_then(|bitrate| u64::try_from(bitrate).ok()),
        )),
        ..*target
    };
    video_attempts(
        &mixed,
        rotation,
        source_frame_rate_numerator,
        source_frame_rate_denominator,
        Some(i64::from(channels)),
        mix.sample_rate,
        hdr,
    )
}

/// Whether what the encoder wrote is what was asked for: the encoded sides exactly, and the SAME
/// turn — read back by the same reader that read the source's. A result that differs came out
/// stretched, sideways or upside down, and is a failed transcode (rule C), never an upload.
pub fn matches(asked: &VideoEncoding, width: u32, height: u32, rotation: u32) -> bool {
    asked.width == width && asked.height == height && asked.rotation == rotation
}

/// Whether a result's frame rate is what was asked for: not above it by more than the planner's own
/// tolerance. A 60 fps source that came out at 60 was given a bitrate worked out for 30 — half the
/// bits a frame, and outside the profile's "at most 30" — so it is a failed transcode like a wrong
/// size is. A rate BELOW what was asked is not: a transcoder cannot invent frames the source never
/// had. One nobody can read is not judged.
pub fn frame_rate_came_out(asked: &VideoEncoding, rate: Option<f64>) -> bool {
    match rate {
        None => true,
        Some(_) if asked.frame_rate_denominator == 0 => true,
        Some(known) => {
            known
                <= f64::from(asked.frame_rate_numerator) / f64::from(asked.frame_rate_denominator)
                    + (media_plan::FRAME_RATE_TOLERANCE - media_plan::MAX_FRAME_RATE)
        }
    }
}

/// Whether a result's audio is at the rate asked for, within a tenth. An encoder that takes 64 000
/// and writes its own 96 000 has raised a bitrate over what rule B allowed without refusing
/// anything; that result is not taken. A rate the result does not state is not judged.
pub fn audio_rate_came_out(asked: u32, stated: Option<i64>) -> bool {
    match stated {
        Some(stated) if stated > 0 => i128::from(stated) * 10 <= i128::from(asked) * 11,
        _ => true,
    }
}

/// Whether a recording or a re-encode came out as the AAC it was asked for, in the channels asked
/// for. `codec` is the planner's name for what the reader saw (`"aac"`, `"mp3"`, `"unknown"`, …). A
/// count the result does not state is not judged.
pub fn aac_came_out(asked: &AacEncoding, codec: &str, channels: Option<i64>) -> bool {
    codec == "aac"
        && match channels {
            Some(count) if count > 0 => count == i64::from(asked.channels),
            _ => true,
        }
}

/// What goes once a transcode has come out as asked — rule D, and what faststart could not do. A
/// result bigger than a sendable source is thrown away; a VIDEO whose index could not be moved to
/// the front does not go where the source can go instead; and a result over the ceiling is refused,
/// as the source would have been. Otherwise the result is sent — the only thing that can be.
///
/// A SENDABLE SOURCE IS NEVER REFUSED HERE, whatever the result turned out to be: preparing media
/// may not turn a send that worked in 1.1 into one that does not (rule C).
///
/// THE INDEX MATTERS FOR A VIDEO ONLY (`needs_moov_first`). The protocol asks for `moov` before
/// `mdat` in the video container's row and not in "Audio alone", and an M4A a tenth the size of its
/// WAV is worth more with its index at the end than the WAV is — or than an Ogg original, which
/// does not play on iOS or macOS at all.
pub fn choose(
    source_bytes: u64,
    source_sendable: bool,
    result_bytes: u64,
    moov_first: bool,
    needs_moov_first: bool,
    ceiling: u64,
) -> Chosen {
    // A bigger result beside a sendable source (rule D), or a video whose index stayed at the end
    // where the source could go instead.
    let bigger =
        media_plan::keep_smaller(source_bytes, source_sendable, result_bytes) == Upload::Source;
    let index_at_end = needs_moov_first && !moov_first && source_sendable;
    if bigger || index_at_end {
        Chosen::Original
    } else if result_bytes > ceiling {
        Chosen::TooLarge
    } else {
        Chosen::Result
    }
}

/// How long a transcode may take before it counts as failed: two minutes plus three times the clip
/// — an encoder is many times faster than that, and a stuck one must not leave the composer
/// "Preparing…" for good. Ten minutes when the length is unknown. It is ONE ceiling for every way
/// of asking together (`transcode_attempts`), not one each. (The same rule as
/// `animation_preparing::transcode_ceiling`; a length too large to triple saturates.)
pub fn transcode_ceiling(duration_ms: Option<i64>) -> Duration {
    match duration_ms {
        Some(duration) if duration > 0 => Duration::from_secs(120)
            .saturating_add(Duration::from_millis((duration as u64).saturating_mul(3))),
        _ => Duration::from_secs(600),
    }
}

fn rate(bitrate: i64) -> u32 {
    bitrate.clamp(0, i64::from(u32::MAX)) as u32
}

fn wide(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn side(value: i64) -> u32 {
    value.clamp(0, i64::from(u32::MAX)) as u32
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}
