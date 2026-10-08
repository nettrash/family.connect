//! What a picked video or sound file becomes before it is uploaded
//! (docs/protocol.md, "Preparing media before upload"; the reasoning is
//! docs/media-upload-2026-09-28.md, issue #74).
//!
//! The server stores what it is given and never transcodes, so the size of a
//! family's history is decided on the sending device — by four codebases that
//! have to reach the same answer for the same file. iOS compressing to 1080p
//! while Android compressed to 720p is what happens when nothing writes the
//! target down. So this module is only the DECISION, as arithmetic: what the
//! source is, as a platform's reader saw it, goes in; what to do with it comes
//! out. Reading a file and encoding one are each platform's own business.
//!
//! The web client runs this module as it is. The Apple, Android and Windows
//! ports are held to it by the vectors `win/tools/board-oracle` prints from it
//! (`cargo run -- media-plan`), committed as `media-plan-vectors.json` beside
//! each port's tests — "an oracle, not four readings". So the names here are
//! plain on purpose, and every field is one a Swift struct, a Kotlin data
//! class and a C# record can carry under the same name.
//!
//! Integers everywhere the protocol says so, `f64` only where a frame rate
//! enters, and every function is total: a number a reader could not fill in
//! is "unknown", never a panic.

// --- the profile --------------------------------------------------------------------------------

/// The target's SHORT side at most — a 720p clip is indistinguishable from
/// 1080p in a chat bubble, and roughly half the bytes.
pub const MAX_SHORT_SIDE: u32 = 720;

/// The frame rate a transcode is capped at: 60 fps doubles the bits for
/// smoothness a family clip rarely needs.
pub const MAX_FRAME_RATE: f64 = 30.0;

/// The frame rate a source may have and still count as "at most 30". Not 30
/// itself, because readers report a nominal 30 as 30.0003 or a variable one
/// as 30.2, and re-encoding a 30 fps clip for that would only cost quality.
/// A rate ABOVE this becomes [`MAX_FRAME_RATE`]; one at or below it is kept
/// exactly as it is, 29.97, 25 and 24 included.
pub const FRAME_RATE_TOLERANCE: f64 = 30.5;

/// The video bitrate's floor and ceiling. 2 000 000 bit/s is the profile's
/// rate at 1280×720, 30 fps; below 250 000 a small clip turns to mush for
/// a saving nobody would notice.
pub const MIN_VIDEO_BITRATE: u64 = 250_000;
pub const MAX_VIDEO_BITRATE: u64 = 2_000_000;

/// AAC-LC for the audio in a video and for a picked file that is re-encoded:
/// transparent for speech and ambient sound. Mono gets half — the same
/// quality per channel.
pub const STEREO_AUDIO_BITRATE: u64 = 128_000;
pub const MONO_AUDIO_BITRATE: u64 = 64_000;

/// A voice note, which is recorded to the profile directly and never passes
/// through [`plan_audio`]: AAC-LC, mono, 44.1 or 48 kHz.
pub const VOICE_NOTE_BITRATE: u64 = 64_000;

/// MP3 or AAC at or below this is uploaded untouched. A second lossy
/// generation costs more than the few megabytes it saves — nettrash's 1.1
/// objection to re-encoding someone's music, kept where it is right.
pub const MAX_KEPT_LOSSY_AUDIO_BITRATE: u64 = 192_000;

// --- what a reader saw ------------------------------------------------------------------------------

/// A video the client is about to send as `kind=video`, as its reader saw it.
///
/// Codecs are lowercase names, not a platform's four-character codes, so four
/// readers can agree on them: `"h264"` (`avc1`/`avc3`), `"hevc"` (`hvc1`/`hev1`),
/// `"av1"`, `"vp9"`, and `"unknown"` for anything a reader cannot name. Only
/// `"h264"` and, for audio, `"aac"` (any AAC profile — `mp4a`) are ever asked
/// about; the rest are there so a vector reads plainly.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoSource {
    /// The size as it is DISPLAYED — after its rotation. A portrait phone
    /// clip is a landscape track with a 90° turn; this is 1080×1920, not
    /// 1920×1080. 0 when the reader could not tell.
    pub width: u32,
    pub height: u32,
    /// Frames per second, or None. A value that is not a finite number above
    /// zero is unknown too (see [`known_frame_rate`]).
    pub frame_rate: Option<f64>,
    /// The type the client would upload it as — `"video/mp4"` or
    /// `"video/quicktime"`, lowercase, no parameters ([`crate::media::essence`]).
    pub container: String,
    pub video_codec: String,
    /// None when the file has NO audio track. A track whose codec the reader
    /// cannot name is `Some("unknown")`, which is not "absent".
    pub audio_codec: Option<String>,
    /// The audio track's channel count, or None. Only exactly 1 is mono.
    pub audio_channels: Option<u32>,
    /// The bitrates the container STATES, in bit/s, or None (0 is None too:
    /// an MP4's `esds` writes 0 for a rate it does not state).
    pub video_bitrate: Option<u64>,
    pub audio_bitrate: Option<u64>,
    /// The whole file, in bytes.
    pub size_bytes: u64,
    /// Milliseconds, or None (and 0 is None).
    pub duration_ms: Option<u64>,
}

/// What a video is transcoded TO.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoTarget {
    /// Even, never larger than the source's, and in the source's orientation.
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    pub video_bitrate: u64,
    /// None when the source has no audio track — a transcode does not invent
    /// silence.
    pub audio_bitrate: Option<u64>,
}

/// What happens to a video.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VideoPlan {
    /// Rule A: it is already within the profile, and the ORIGINAL is uploaded.
    Keep,
    /// Rule 5: transcode to this target.
    Transcode(VideoTarget),
    /// Rule C: there is no size to scale to — the reader found none, or the
    /// short side is a single pixel and an even target would have none — so
    /// this platform "cannot transcode this source at all", and it goes the
    /// way it went before this section existed (see [`on_failure`]).
    Fallback,
}

/// A sound file the member PICKED — never a voice note, which is recorded to
/// the profile directly.
///
/// `codec` is lowercase: `"pcm"` (WAV and AIFF are both PCM), `"flac"`,
/// `"alac"`, `"aac"`, `"mp3"`, `"vorbis"`, `"opus"`, or `"unknown"`.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioSource {
    /// The type the file is, lowercase, no parameters: one the server
    /// accepts as audio (`"audio/mp4"`, `"audio/mpeg"`, `"audio/wav"`,
    /// `"audio/ogg"` — [`crate::media::audio_mime`]) or the platform's own
    /// name for one it does not (`"audio/aiff"`, `"audio/flac"`).
    pub container: String,
    pub codec: String,
    pub channels: Option<u32>,
    /// The bitrate the file STATES, in bit/s, or None (0 is None).
    pub bitrate: Option<u64>,
    pub size_bytes: u64,
    pub duration_ms: Option<u64>,
}

/// What happens to a picked sound file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioPlan {
    /// Uploaded untouched.
    Keep,
    /// Re-encoded as M4A (`audio/mp4`), AAC-LC, at this bitrate.
    Transcode { bitrate: u64 },
}

// --- unknowns ---------------------------------------------------------------------------------------

/// A count a reader handed over, or None when it could not. 0 is None as
/// well: it is what a container writes for a rate it does not state, and a
/// file of no duration has no rate to estimate. One rule for every port, so
/// none of them has to guess which of its platform's "unknown"s is which.
fn known(value: Option<u64>) -> Option<u64> {
    value.filter(|&value| value > 0)
}

/// A frame rate, or None when there is none worth believing: absent, zero,
/// negative, infinite or NaN. A reader reports all of these for a stream it
/// could not time, and every one of them means "unknown" to rule 2.
pub fn known_frame_rate(frame_rate: Option<f64>) -> Option<f64> {
    frame_rate.filter(|rate| rate.is_finite() && *rate > 0.0)
}

// --- the rules --------------------------------------------------------------------------------------

/// A bitrate the container does not state, estimated as the protocol says:
/// `size × 8 × 1000 ÷ duration_ms − audio_bitrate`, in integers (the division
/// truncates). None when it "cannot be estimated": no duration, or nothing
/// left once the audio is taken off — a stated audio rate larger than the
/// whole file's is a reader's mistake, not a video at a negative bitrate.
///
/// For a sound file on its own there is nothing to take off, and
/// `audio_bitrate` is 0.
///
/// It counts every byte of the file, so a container's overhead — or an
/// MP3's embedded cover art — reads as bitrate. That is the protocol's
/// formula as written; a stated rate always wins over it.
pub fn estimated_bitrate(
    size_bytes: u64,
    duration_ms: Option<u64>,
    audio_bitrate: u64,
) -> Option<u64> {
    let duration = known(duration_ms)?;
    // Saturating only so that nonsense in cannot panic: a file would have to
    // be two petabytes before the product overflowed.
    let whole = size_bytes.saturating_mul(8 * 1000) / duration;
    whole.checked_sub(audio_bitrate).filter(|&video| video > 0)
}

/// `V`: the video bitrate as the container states it, or else as estimated.
///
/// The estimate takes the AUDIO's bitrate off the whole, so it needs that
/// bitrate: with no audio track it is 0, but a track whose rate is not
/// stated leaves the formula with an unknown in it — and then `V` "cannot be
/// estimated either" and is unknown. (Which sends such a file past rule A to
/// a transcode, and rule D keeps the original if that came out bigger.)
pub fn source_video_bitrate(source: &VideoSource) -> Option<u64> {
    if let Some(stated) = known(source.video_bitrate) {
        return Some(stated);
    }
    let audio = match source.audio_codec {
        None => 0,
        Some(_) => known(source.audio_bitrate)?,
    };
    estimated_bitrate(source.size_bytes, source.duration_ms, audio)
}

/// Rule 1: the target size for a DISPLAYED `width × height`.
///
/// The short side becomes `min(720, short)` — never upscaled, so a 480p
/// clip stays 480p. The long side follows in proportion, rounded half up in
/// integers: `(2 × long × ts + short) ÷ (2 × short)`, which is
/// `⌊long × ts ÷ short + ½⌋` without a fraction in it. Then each side drops
/// to even if it is odd, because H.264 encoders work in 2×2 chroma blocks
/// and several refuse an odd size outright; dropping (not rounding up) is
/// what keeps the result from ever exceeding the source. The target keeps
/// the source's orientation: a portrait source gives a portrait target.
///
/// (0, 0) for a source with no size — there is nothing to scale.
pub fn target_size(width: u32, height: u32) -> (u32, u32) {
    let short = u64::from(width.min(height));
    let long = u64::from(width.max(height));
    if short == 0 {
        return (0, 0);
    }
    let target_short = short.min(u64::from(MAX_SHORT_SIDE));
    let target_long = (2 * long * target_short + short) / (2 * short);
    let target_short = (target_short - target_short % 2) as u32;
    let target_long = (target_long - target_long % 2) as u32;
    if width >= height {
        (target_long, target_short)
    } else {
        (target_short, target_long)
    }
}

/// Rule 2: 30 when the source's rate is above [`FRAME_RATE_TOLERANCE`] or
/// unknown, and the source's own rate otherwise — never raised, and never
/// "tidied": 29.97 stays 29.97, because resampling it to 30 would duplicate
/// a frame every 33 seconds for nothing.
pub fn target_frame_rate(frame_rate: Option<f64>) -> f64 {
    match known_frame_rate(frame_rate) {
        Some(rate) if rate <= FRAME_RATE_TOLERANCE => rate,
        _ => MAX_FRAME_RATE,
    }
}

/// Rule 3, before the source cap: `2 000 000 × (w × h ÷ 921 600) × (f ÷ 30)`,
/// clamped to [250 000, 2 000 000], rounded to the nearest 1 000, half up.
///
/// The EVALUATION ORDER is part of the rule, because the protocol's "the
/// rounding absorbs the last bit" is true everywhere except on an exact
/// half-thousand — and those exist at real sizes: 960×540 at 25 fps is
/// 937 500 exactly, 404×288 at 30 is 252 500. So: `2 000 000 ÷ (921 600 × 30)`
/// is exactly `1 ÷ 13 824`, and the rate in THOUSANDS is `w × h × f ÷ 13 824`,
/// computed as one `f64` product of the integer pixel count and the frame
/// rate, then one `f64` division. For a whole frame rate the product is
/// exact and the division correctly rounded, so a true half-thousand comes
/// out as exactly .5 and rounds up; evaluated left to right as the protocol
/// writes it, 3618×128 at 25 comes out 837.4999… and rounds the other way. Half up, too, and not a platform's default:
/// C#'s `Math.Round` rounds half to EVEN and would make 404×288 252 000.
///
/// Clamping before rounding, as written, is the same as after: both bounds
/// are whole thousands.
pub fn profile_video_bitrate(width: u32, height: u32, frame_rate: f64) -> u64 {
    let pixels = u64::from(width) * u64::from(height);
    let thousands = pixels as f64 * frame_rate / 13_824.0;
    let clamped = thousands.clamp(
        (MIN_VIDEO_BITRATE / 1000) as f64,
        (MAX_VIDEO_BITRATE / 1000) as f64,
    );
    // f64::round is half AWAY from zero, which for a positive number is half up.
    clamped.round() as u64 * 1000
}

/// Rule 3 whole: the profile's bitrate, and then — if `V` is known — no
/// higher than `V`. Rule B: a re-encode never raises a bitrate. The cap
/// comes AFTER the rounding, so a capped target is the source's exact rate,
/// not a whole thousand.
pub fn target_video_bitrate(
    width: u32,
    height: u32,
    frame_rate: f64,
    source_bitrate: Option<u64>,
) -> u64 {
    let profile = profile_video_bitrate(width, height, frame_rate);
    match known(source_bitrate) {
        Some(source) => profile.min(source),
        None => profile,
    }
}

/// The audio-in-video row, and the audio-alone row that repeats it: 128 000
/// for stereo, 64 000 for mono — never above the source's, when that is
/// known. Only a channel count of exactly 1 is mono. More than two channels
/// (5.1) take the stereo rate, and so does a count the reader could not
/// give: guessing mono would halve the bitrate of a stereo track.
pub fn target_audio_bitrate(channels: Option<u32>, source_bitrate: Option<u64>) -> u64 {
    let profile = if channels == Some(1) {
        MONO_AUDIO_BITRATE
    } else {
        STEREO_AUDIO_BITRATE
    };
    match known(source_bitrate) {
        Some(source) => profile.min(source),
        None => profile,
    }
}

/// Rules 1–3 for a source: the size, the frame rate and the two bitrates it
/// would be transcoded to. None when it has no size (a side of 0).
///
/// The audio's cap is the rate the container STATES for it. The protocol
/// gives no way to estimate an audio track's rate inside a video, so a
/// track that states none is capped only by the profile.
pub fn video_target(source: &VideoSource) -> Option<VideoTarget> {
    if source.width == 0 || source.height == 0 {
        return None;
    }
    let (width, height) = target_size(source.width, source.height);
    let frame_rate = target_frame_rate(source.frame_rate);
    let video_bitrate =
        target_video_bitrate(width, height, frame_rate, source_video_bitrate(source));
    let audio_bitrate = source
        .audio_codec
        .as_ref()
        .map(|_| target_audio_bitrate(source.audio_channels, source.audio_bitrate));
    Some(VideoTarget {
        width,
        height,
        frame_rate,
        video_bitrate,
        audio_bitrate,
    })
}

/// Rule A — leave it alone: the source is ALREADY within the profile, and
/// re-encoding it would only cost it quality. A 720p H.264 clip at 1.5 Mbit/s
/// re-encoded to "720p, 2 Mbit/s" comes out bigger and worse.
///
/// Every condition, exactly as listed: `video/mp4`; H.264; AAC or no audio;
/// the short side at most 720; `F` known and at most 30.5; `V` known and at
/// most 1.25 × step 3's bitrate for it. The last is `4 × V ≤ 5 × target`,
/// so it stays in integers. (`video/quicktime` is never within the profile,
/// even holding H.264: Firefox will not play the container.)
///
/// What it does NOT ask, because the protocol's list does not: the audio's
/// bitrate, the sides being even, or where the `moov` box is. A kept file is
/// the original, whatever those are.
pub fn within_profile(source: &VideoSource) -> bool {
    let (Some(target), Some(rate), Some(bitrate)) = (
        video_target(source),
        known_frame_rate(source.frame_rate),
        source_video_bitrate(source),
    ) else {
        return false;
    };
    source.container == "video/mp4"
        && source.video_codec == "h264"
        && matches!(source.audio_codec.as_deref(), None | Some("aac"))
        && source.width.min(source.height) <= MAX_SHORT_SIDE
        && rate <= FRAME_RATE_TOLERANCE
        && bitrate.saturating_mul(4) <= target.video_bitrate.saturating_mul(5)
}

/// Rules A, 5 and C for a video, in that order: kept when it is within the
/// profile (a one-pixel-wide clip can be — it is still the original that
/// goes); otherwise transcoded to [`video_target`]; and when there is no
/// size to transcode to, the fallback.
pub fn plan_video(source: &VideoSource) -> VideoPlan {
    let Some(target) = video_target(source) else {
        return VideoPlan::Fallback;
    };
    if within_profile(source) {
        return VideoPlan::Keep;
    }
    if target.width == 0 || target.height == 0 {
        return VideoPlan::Fallback;
    }
    VideoPlan::Transcode(target)
}

/// A sound file's bitrate: as the file states it, or else estimated from its
/// size and duration with nothing taken off — it is the only stream.
pub fn source_audio_bitrate(source: &AudioSource) -> Option<u64> {
    known(source.bitrate).or_else(|| estimated_bitrate(source.size_bytes, source.duration_ms, 0))
}

/// The audio rules, in order, for a picked sound file:
///
/// 1. Uncompressed or lossless — `pcm` (WAV, AIFF), `flac`, `alac` — is
///    re-encoded. That is where the saving is (about ten times) and there is
///    no first generation to lose.
/// 2. Anything in an Ogg container (`audio/ogg`: Vorbis or Opus) is
///    re-encoded, because AVFoundation has no Ogg reader and it would not
///    play on iOS or macOS. The protocol says "wherever the platform can
///    decode it": that is not this function's to know — a platform that
///    cannot decode it fails the transcode, and rule C sends it as before.
/// 3. MP3 or AAC above 192 000 bit/s is re-encoded; at or below, or with a
///    bitrate that can be neither read nor estimated, it is untouched.
///
/// Anything else — a lossy codec the rules do not name (AC-3, Opus in an
/// MP4), or a codec the reader could not name outside Ogg — is left
/// untouched, because no rule says to re-encode it.
///
/// The target is 128 000 stereo or 64 000 mono, never above the source's.
pub fn plan_audio(source: &AudioSource) -> AudioPlan {
    let bitrate = source_audio_bitrate(source);
    let transcode = match source.codec.as_str() {
        "pcm" | "flac" | "alac" => true,
        _ if source.container == "audio/ogg" => true,
        "mp3" | "aac" => bitrate.is_some_and(|rate| rate > MAX_KEPT_LOSSY_AUDIO_BITRATE),
        _ => false,
    };
    if transcode {
        AudioPlan::Transcode {
            bitrate: target_audio_bitrate(source.channels, bitrate),
        }
    } else {
        AudioPlan::Keep
    }
}

// --- when it goes wrong, or does not help ---------------------------------------------------------

/// The types the server takes as each kind (server `models.rs`,
/// `Attachment::ACCEPTED`; docs/protocol.md, "Audio" for the audio list).
const ACCEPTED_VIDEO: [&str; 2] = ["video/mp4", "video/quicktime"];
const ACCEPTED_AUDIO: [&str; 5] = [
    "audio/mp4",
    "audio/m4a",
    "audio/mpeg",
    "audio/wav",
    "audio/ogg",
];

/// Whether a file could be uploaded as `kind` (`"video"` or `"audio"`) just
/// as it is — the protocol's "sendable as that kind": an accepted type,
/// honest bytes (`honest`: the declared type matches the magic number, as
/// [`crate::media::matches_magic`] checks it), and within the ceiling.
/// Within means at most: the server refuses only a body LARGER than
/// `max_attachment_bytes`. Any other kind is not one these rules are about.
pub fn sendable(
    kind: &str,
    container: &str,
    honest: bool,
    size_bytes: u64,
    ceiling_bytes: u64,
) -> bool {
    let accepted = match kind {
        "video" => ACCEPTED_VIDEO.contains(&container),
        "audio" => ACCEPTED_AUDIO.contains(&container),
        _ => false,
    };
    accepted && honest && size_bytes <= ceiling_bytes
}

/// What rule C sends when a transcode fails, or the platform cannot do one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnFailure {
    /// The original, untouched, as its kind.
    Original,
    /// Whatever the client did before this section existed — as a `file`, or
    /// refused. Not a new rule: the old one, unchanged.
    TodaysPath,
}

/// Rule C — a failure sends what would have been sent without this section.
/// Preparing media is an optimisation, and it may never turn a send that
/// would have worked into one that does not: a sendable source goes up
/// untouched, and anything else takes the path it always took.
pub fn on_failure(source_sendable: bool) -> OnFailure {
    if source_sendable {
        OnFailure::Original
    } else {
        OnFailure::TodaysPath
    }
}

/// Which bytes rule D uploads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upload {
    Source,
    Result,
}

/// Rule D — a result BIGGER than its source is thrown away, and the source
/// uploaded instead, provided the source is itself sendable as that kind.
/// Otherwise the result is used, since it is the only thing that can be
/// sent. A result exactly the source's size is not bigger, so it is used:
/// it is in the profile, and the source may not be.
pub fn keep_smaller(source_bytes: u64, source_sendable: bool, result_bytes: u64) -> Upload {
    if result_bytes > source_bytes && source_sendable {
        Upload::Source
    } else {
        Upload::Result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1080p H.264 phone clip with stereo AAC, every number stated — the
    /// fixture each test changes one thing about.
    fn clip() -> VideoSource {
        VideoSource {
            width: 1920,
            height: 1080,
            frame_rate: Some(30.0),
            container: "video/mp4".into(),
            video_codec: "h264".into(),
            audio_codec: Some("aac".into()),
            audio_channels: Some(2),
            video_bitrate: Some(8_000_000),
            audio_bitrate: Some(128_000),
            size_bytes: 20_000_000,
            duration_ms: Some(19_000),
        }
    }

    /// A 720p clip already within the profile.
    fn within() -> VideoSource {
        VideoSource {
            width: 1280,
            height: 720,
            video_bitrate: Some(1_500_000),
            ..clip()
        }
    }

    fn sized(width: u32, height: u32) -> VideoSource {
        VideoSource {
            width,
            height,
            ..clip()
        }
    }

    fn song(container: &str, codec: &str, bitrate: Option<u64>) -> AudioSource {
        AudioSource {
            container: container.into(),
            codec: codec.into(),
            channels: Some(2),
            bitrate,
            size_bytes: 5_000_000,
            duration_ms: Some(180_000),
        }
    }

    fn target(source: &VideoSource) -> VideoTarget {
        video_target(source).expect("a source with a size has a target")
    }

    // --- rule 1 --------------------------------------------------------------------------------

    #[test]
    fn the_short_side_becomes_720_in_either_orientation() {
        assert_eq!(target_size(1920, 1080), (1280, 720));
        assert_eq!(
            target_size(1080, 1920),
            (720, 1280),
            "a portrait source stays portrait"
        );
        assert_eq!(target_size(3840, 2160), (1280, 720));
        assert_eq!(target_size(2160, 3840), (720, 1280));
        assert_eq!(target_size(1440, 1080), (960, 720), "4:3");
        assert_eq!(target_size(1080, 1080), (720, 720), "square");
        assert_eq!(
            target_size(2560, 1080),
            (1706, 720),
            "1706.67 rounds to 1707, then drops to even"
        );
    }

    #[test]
    fn a_small_source_is_never_upscaled() {
        assert_eq!(target_size(854, 480), (854, 480), "480p stays 480p");
        assert_eq!(target_size(480, 854), (480, 854));
        assert_eq!(target_size(640, 360), (640, 360));
        assert_eq!(target_size(176, 144), (176, 144));
        let source = sized(854, 480);
        assert_eq!((target(&source).width, target(&source).height), (854, 480));
    }

    #[test]
    fn a_short_side_of_exactly_720_is_kept_and_721_is_scaled() {
        assert_eq!(target_size(1280, 720), (1280, 720));
        assert_eq!(target_size(720, 1280), (720, 1280));
        // 1282 × 720 ÷ 721 = 1280.22…
        assert_eq!(target_size(1282, 721), (1280, 720));
        assert_eq!(target_size(721, 1282), (720, 1280));
        // 1281 × 720 ÷ 721 = 1279.22… → 1279 → 1278.
        assert_eq!(target_size(1281, 721), (1278, 720));
    }

    #[test]
    fn odd_sides_drop_to_even() {
        assert_eq!(
            target_size(853, 481),
            (852, 480),
            "both odd, both under the cap: each drops one"
        );
        assert_eq!(target_size(1279, 719), (1278, 718));
        assert_eq!(target_size(641, 361), (640, 360));
        assert_eq!(target_size(360, 641), (360, 640));
    }

    #[test]
    fn the_long_side_rounds_half_up() {
        // 1000 × 720 ÷ 800 = 900 exactly; 1001 × 720 ÷ 800 = 900.9 → 901 → 900;
        // 2025 × 720 ÷ 1080 = 1350 exactly.
        assert_eq!(target_size(1000, 800), (900, 720));
        assert_eq!(target_size(1001, 800), (900, 720));
        assert_eq!(target_size(2025, 1080), (1350, 720));
        // A half exactly: 1443 × 720 ÷ 1440 = 721.5 → 722, where half-down would give 721 → 720.
        assert_eq!(target_size(1443, 1440), (722, 720));
        // 1441 × 720 ÷ 1440 = 720.5 → 721, then drops to even.
        assert_eq!(target_size(1441, 1440), (720, 720));
    }

    #[test]
    fn a_source_with_no_size_has_no_target() {
        assert_eq!(target_size(0, 1080), (0, 0));
        assert_eq!(target_size(1920, 0), (0, 0));
        assert_eq!(video_target(&sized(0, 0)), None);
        assert_eq!(plan_video(&sized(0, 720)), VideoPlan::Fallback);
        // One pixel wide: the even target has no width at all.
        assert_eq!(target_size(1, 1080), (0, 1080));
        assert_eq!(plan_video(&sized(1, 1080)), VideoPlan::Fallback);
    }

    // --- rule 2 --------------------------------------------------------------------------------

    #[test]
    fn frame_rates_at_or_below_the_tolerance_are_kept_as_they_are() {
        for rate in [
            24.0,
            25.0,
            29.97,
            30_000.0 / 1001.0,
            24_000.0 / 1001.0,
            30.0,
            30.5,
            15.0,
        ] {
            assert_eq!(target_frame_rate(Some(rate)), rate, "{rate}");
        }
    }

    #[test]
    fn frame_rates_above_the_tolerance_or_unknown_become_30() {
        for rate in [30.51, 50.0, 59.94, 60_000.0 / 1001.0, 60.0, 120.0, 240.0] {
            assert_eq!(target_frame_rate(Some(rate)), 30.0, "{rate}");
        }
        assert_eq!(target_frame_rate(None), 30.0);
        for nonsense in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                target_frame_rate(Some(nonsense)),
                30.0,
                "{nonsense} is unknown"
            );
            assert_eq!(known_frame_rate(Some(nonsense)), None);
        }
    }

    // --- rule 3 --------------------------------------------------------------------------------

    #[test]
    fn the_profile_rate_is_two_megabits_at_720p30_and_scales_with_pixels_and_frames() {
        assert_eq!(profile_video_bitrate(1280, 720, 30.0), 2_000_000);
        assert_eq!(profile_video_bitrate(720, 1280, 30.0), 2_000_000);
        assert_eq!(profile_video_bitrate(1280, 720, 25.0), 1_667_000, "1666.67");
        assert_eq!(profile_video_bitrate(1280, 720, 24.0), 1_600_000);
        assert_eq!(profile_video_bitrate(1280, 720, 29.97), 1_998_000);
        assert_eq!(profile_video_bitrate(960, 720, 30.0), 1_500_000);
        assert_eq!(profile_video_bitrate(854, 480, 30.0), 890_000, "889.58");
        assert_eq!(profile_video_bitrate(640, 360, 30.0), 500_000);
        assert_eq!(profile_video_bitrate(720, 720, 30.0), 1_125_000);
    }

    #[test]
    fn the_profile_rate_is_clamped_to_its_floor_and_ceiling() {
        assert_eq!(
            profile_video_bitrate(320, 240, 15.0),
            250_000,
            "83.33 thousand, raised to the floor"
        );
        assert_eq!(profile_video_bitrate(176, 144, 30.0), 250_000);
        assert_eq!(
            profile_video_bitrate(1280, 720, 30.5),
            2_000_000,
            "2033.33 thousand, capped"
        );
        assert_eq!(
            profile_video_bitrate(720, 2560, 30.0),
            2_000_000,
            "a panorama"
        );
        assert_eq!(profile_video_bitrate(0, 0, 30.0), 250_000);
    }

    #[test]
    fn an_exact_half_thousand_rounds_up() {
        // w × h × f ÷ 13 824 is exactly 252.5, 937.5 and 302.5 thousand.
        // Half to even would say 252 000 and 302 000.
        assert_eq!(profile_video_bitrate(404, 288, 30.0), 253_000);
        assert_eq!(profile_video_bitrate(960, 540, 25.0), 938_000);
        assert_eq!(profile_video_bitrate(484, 360, 24.0), 303_000);
        // Left to right as the protocol writes it, this one is 837.4999…
        assert_eq!(profile_video_bitrate(3618, 128, 25.0), 838_000);
        let left_to_right = 2_000_000.0 * (3618.0 * 128.0 / 921_600.0) * (25.0 / 30.0);
        assert!(
            left_to_right < 837_500.0,
            "the order matters: {left_to_right}"
        );
    }

    #[test]
    fn the_target_rate_is_never_above_the_sources() {
        assert_eq!(
            target_video_bitrate(1280, 720, 30.0, Some(800_000)),
            800_000
        );
        assert_eq!(
            target_video_bitrate(1280, 720, 30.0, Some(1_234_567)),
            1_234_567,
            "the cap is after the rounding"
        );
        assert_eq!(
            target_video_bitrate(1280, 720, 30.0, Some(100_000)),
            100_000,
            "below the floor, still the source's"
        );
        assert_eq!(
            target_video_bitrate(1280, 720, 30.0, Some(9_000_000)),
            2_000_000
        );
        assert_eq!(target_video_bitrate(1280, 720, 30.0, None), 2_000_000);
        assert_eq!(
            target_video_bitrate(1280, 720, 30.0, Some(0)),
            2_000_000,
            "0 is unknown"
        );
        let hevc = VideoSource {
            video_codec: "hevc".into(),
            video_bitrate: Some(900_000),
            ..within()
        };
        assert_eq!(
            plan_video(&hevc),
            VideoPlan::Transcode(VideoTarget {
                width: 1280,
                height: 720,
                frame_rate: 30.0,
                video_bitrate: 900_000,
                audio_bitrate: Some(128_000),
            })
        );
    }

    // --- V -------------------------------------------------------------------------------------

    #[test]
    fn a_bitrate_is_estimated_from_size_and_duration_in_integers() {
        // 1 000 000 bytes over 3 000 ms: 2 666 666.67 → 2 666 666, less the audio.
        assert_eq!(
            estimated_bitrate(1_000_000, Some(3_000), 128_000),
            Some(2_538_666)
        );
        assert_eq!(
            estimated_bitrate(1_000_000, Some(3_000), 0),
            Some(2_666_666)
        );
        assert_eq!(estimated_bitrate(1_000_000, None, 0), None, "no duration");
        assert_eq!(
            estimated_bitrate(1_000_000, Some(0), 0),
            None,
            "a duration of 0 is none"
        );
        assert_eq!(
            estimated_bitrate(0, Some(1_000), 0),
            None,
            "nothing to estimate from"
        );
        assert_eq!(
            estimated_bitrate(16_000, Some(1_000), 128_000),
            None,
            "the audio is the whole file"
        );
        assert_eq!(estimated_bitrate(16_000, Some(1_000), 127_999), Some(1));
        assert_eq!(
            estimated_bitrate(16_000, Some(1_000), 200_000),
            None,
            "more audio than file"
        );
    }

    #[test]
    fn v_is_stated_else_estimated_else_unknown() {
        assert_eq!(
            source_video_bitrate(&clip()),
            Some(8_000_000),
            "stated wins"
        );
        let unstated = VideoSource {
            video_bitrate: None,
            size_bytes: 3_750_000,
            duration_ms: Some(10_000),
            ..clip()
        };
        assert_eq!(source_video_bitrate(&unstated), Some(3_000_000 - 128_000));
        let stated_zero = VideoSource {
            video_bitrate: Some(0),
            ..unstated.clone()
        };
        assert_eq!(
            source_video_bitrate(&stated_zero),
            Some(2_872_000),
            "0 is not stated"
        );
        let silent = VideoSource {
            audio_codec: None,
            audio_bitrate: None,
            ..unstated.clone()
        };
        assert_eq!(
            source_video_bitrate(&silent),
            Some(3_000_000),
            "no audio track: nothing to take off"
        );
        let unstated_audio = VideoSource {
            audio_bitrate: None,
            ..unstated.clone()
        };
        assert_eq!(
            source_video_bitrate(&unstated_audio),
            None,
            "an audio track of unknown rate leaves an unknown in it"
        );
        let no_duration = VideoSource {
            duration_ms: None,
            ..unstated
        };
        assert_eq!(source_video_bitrate(&no_duration), None);
    }

    // --- rule A --------------------------------------------------------------------------------

    #[test]
    fn a_clip_within_the_profile_is_left_alone() {
        assert!(within_profile(&within()));
        assert_eq!(plan_video(&within()), VideoPlan::Keep);
        let silent = VideoSource {
            audio_codec: None,
            audio_channels: None,
            audio_bitrate: None,
            ..within()
        };
        assert_eq!(plan_video(&silent), VideoPlan::Keep, "no audio is within");
        let small = VideoSource {
            video_bitrate: Some(300_000),
            ..sized(640, 360)
        };
        assert_eq!(plan_video(&small), VideoPlan::Keep);
        let odd = VideoSource {
            video_bitrate: Some(1_000_000),
            ..sized(1279, 719)
        };
        assert_eq!(
            plan_video(&odd),
            VideoPlan::Keep,
            "odd sides are not in rule A's list"
        );
        let loud = VideoSource {
            audio_bitrate: Some(320_000),
            ..within()
        };
        assert_eq!(
            plan_video(&loud),
            VideoPlan::Keep,
            "nor is the audio's bitrate"
        );
        let estimated = VideoSource {
            video_bitrate: None,
            size_bytes: 2_035_000,
            duration_ms: Some(10_000),
            ..within()
        };
        // 1 628 000 − 128 000 = 1 500 000.
        assert_eq!(
            plan_video(&estimated),
            VideoPlan::Keep,
            "an estimated V counts as known"
        );
    }

    #[test]
    fn rule_a_allows_a_quarter_over_the_target_and_not_a_bit_more() {
        let at = VideoSource {
            video_bitrate: Some(2_500_000),
            ..within()
        };
        assert_eq!(plan_video(&at), VideoPlan::Keep);
        let over = VideoSource {
            video_bitrate: Some(2_500_001),
            ..within()
        };
        assert_eq!(
            plan_video(&over),
            VideoPlan::Transcode(VideoTarget {
                width: 1280,
                height: 720,
                frame_rate: 30.0,
                video_bitrate: 2_000_000,
                audio_bitrate: Some(128_000),
            })
        );
        // 640×360 at 30: a target of 500 000, so 625 000 is the edge.
        assert_eq!(
            plan_video(&VideoSource {
                video_bitrate: Some(625_000),
                ..sized(640, 360)
            }),
            VideoPlan::Keep
        );
        assert!(matches!(
            plan_video(&VideoSource {
                video_bitrate: Some(625_001),
                ..sized(640, 360)
            }),
            VideoPlan::Transcode(_)
        ));
        // At 24 fps the target is 1 600 000, and the edge moves with it.
        let film = VideoSource {
            frame_rate: Some(24.0),
            video_bitrate: Some(2_000_000),
            ..within()
        };
        assert_eq!(plan_video(&film), VideoPlan::Keep);
        let film_over = VideoSource {
            video_bitrate: Some(2_000_001),
            ..film
        };
        assert!(matches!(plan_video(&film_over), VideoPlan::Transcode(_)));
    }

    #[test]
    fn quicktime_is_never_within_the_profile() {
        let mov = VideoSource {
            container: "video/quicktime".into(),
            ..within()
        };
        assert!(!within_profile(&mov));
        assert_eq!(
            plan_video(&mov),
            VideoPlan::Transcode(VideoTarget {
                width: 1280,
                height: 720,
                frame_rate: 30.0,
                video_bitrate: 1_500_000,
                audio_bitrate: Some(128_000),
            }),
            "the same numbers, re-wrapped; rule D keeps the original if it grew"
        );
    }

    #[test]
    fn every_other_condition_of_rule_a_sends_it_to_a_transcode() {
        let cases = [
            (
                "HEVC",
                VideoSource {
                    video_codec: "hevc".into(),
                    ..within()
                },
            ),
            (
                "an unnamed codec",
                VideoSource {
                    video_codec: "unknown".into(),
                    ..within()
                },
            ),
            (
                "MP3 audio",
                VideoSource {
                    audio_codec: Some("mp3".into()),
                    ..within()
                },
            ),
            (
                "unnamed audio",
                VideoSource {
                    audio_codec: Some("unknown".into()),
                    ..within()
                },
            ),
            (
                "short side 721",
                VideoSource {
                    width: 1282,
                    height: 721,
                    ..within()
                },
            ),
            (
                "above 30.5",
                VideoSource {
                    frame_rate: Some(30.51),
                    ..within()
                },
            ),
            (
                "60 fps",
                VideoSource {
                    frame_rate: Some(60.0),
                    ..within()
                },
            ),
            (
                "F unknown",
                VideoSource {
                    frame_rate: None,
                    ..within()
                },
            ),
            (
                "V unknown",
                VideoSource {
                    video_bitrate: None,
                    duration_ms: None,
                    ..within()
                },
            ),
        ];
        for (why, source) in cases {
            assert!(!within_profile(&source), "{why}");
            assert!(
                matches!(plan_video(&source), VideoPlan::Transcode(_)),
                "{why}"
            );
        }
        let at_tolerance = VideoSource {
            frame_rate: Some(30.5),
            ..within()
        };
        assert_eq!(
            plan_video(&at_tolerance),
            VideoPlan::Keep,
            "30.5 itself is within"
        );
    }

    // --- rule 5 --------------------------------------------------------------------------------

    #[test]
    fn a_4k60_portrait_clip_becomes_720p30_portrait() {
        let phone = VideoSource {
            width: 2160,
            height: 3840,
            frame_rate: Some(59.94),
            container: "video/quicktime".into(),
            video_codec: "hevc".into(),
            video_bitrate: Some(40_000_000),
            ..clip()
        };
        assert_eq!(
            plan_video(&phone),
            VideoPlan::Transcode(VideoTarget {
                width: 720,
                height: 1280,
                frame_rate: 30.0,
                video_bitrate: 2_000_000,
                audio_bitrate: Some(128_000),
            })
        );
    }

    #[test]
    fn a_kept_frame_rate_sets_the_bitrate_too() {
        let pal = VideoSource {
            frame_rate: Some(25.0),
            ..clip()
        };
        let kept = target(&pal);
        assert_eq!((kept.frame_rate, kept.video_bitrate), (25.0, 1_667_000));
        let ntsc = VideoSource {
            frame_rate: Some(30_000.0 / 1001.0),
            ..clip()
        };
        assert_eq!(target(&ntsc).video_bitrate, 1_998_000);
        let unknown = VideoSource {
            frame_rate: None,
            ..clip()
        };
        assert_eq!(target(&unknown).frame_rate, 30.0);
    }

    #[test]
    fn audio_in_a_video_is_128k_stereo_64k_mono_never_above_the_source() {
        let stereo = clip();
        assert_eq!(target(&stereo).audio_bitrate, Some(128_000));
        let mono = VideoSource {
            audio_channels: Some(1),
            ..clip()
        };
        assert_eq!(target(&mono).audio_bitrate, Some(64_000));
        let surround = VideoSource {
            audio_channels: Some(6),
            audio_bitrate: Some(384_000),
            ..clip()
        };
        assert_eq!(target(&surround).audio_bitrate, Some(128_000));
        let uncounted = VideoSource {
            audio_channels: None,
            ..clip()
        };
        assert_eq!(
            target(&uncounted).audio_bitrate,
            Some(128_000),
            "not known to be mono"
        );
        let thin = VideoSource {
            audio_bitrate: Some(96_000),
            ..clip()
        };
        assert_eq!(target(&thin).audio_bitrate, Some(96_000));
        let thin_mono = VideoSource {
            audio_channels: Some(1),
            audio_bitrate: Some(48_000),
            ..clip()
        };
        assert_eq!(target(&thin_mono).audio_bitrate, Some(48_000));
        let unstated = VideoSource {
            audio_bitrate: None,
            ..clip()
        };
        assert_eq!(
            target(&unstated).audio_bitrate,
            Some(128_000),
            "no stated rate: the profile's"
        );
        let silent = VideoSource {
            audio_codec: None,
            ..clip()
        };
        assert_eq!(target(&silent).audio_bitrate, None, "no track, no audio");
    }

    // --- audio alone ---------------------------------------------------------------------------

    #[test]
    fn mp3_and_aac_are_kept_up_to_192k() {
        for codec in ["mp3", "aac"] {
            let container = if codec == "mp3" {
                "audio/mpeg"
            } else {
                "audio/mp4"
            };
            assert_eq!(
                plan_audio(&song(container, codec, Some(128_000))),
                AudioPlan::Keep
            );
            assert_eq!(
                plan_audio(&song(container, codec, Some(192_000))),
                AudioPlan::Keep,
                "{codec} at 192k"
            );
            assert_eq!(
                plan_audio(&song(container, codec, Some(192_001))),
                AudioPlan::Transcode { bitrate: 128_000 },
                "{codec} at 192 001"
            );
            assert_eq!(
                plan_audio(&song(container, codec, Some(320_000))),
                AudioPlan::Transcode { bitrate: 128_000 }
            );
        }
        let mono = AudioSource {
            channels: Some(1),
            ..song("audio/mpeg", "mp3", Some(256_000))
        };
        assert_eq!(plan_audio(&mono), AudioPlan::Transcode { bitrate: 64_000 });
    }

    #[test]
    fn an_mp3_of_unknown_bitrate_is_left_alone_and_an_estimated_one_is_judged() {
        let unknown = AudioSource {
            duration_ms: None,
            ..song("audio/mpeg", "mp3", None)
        };
        assert_eq!(source_audio_bitrate(&unknown), None);
        assert_eq!(plan_audio(&unknown), AudioPlan::Keep);
        // 7 200 000 bytes over three minutes is 320 000 bit/s.
        let estimated_high = AudioSource {
            size_bytes: 7_200_000,
            ..song("audio/mpeg", "mp3", None)
        };
        assert_eq!(source_audio_bitrate(&estimated_high), Some(320_000));
        assert_eq!(
            plan_audio(&estimated_high),
            AudioPlan::Transcode { bitrate: 128_000 }
        );
        // 4 320 000 bytes is exactly 192 000.
        let estimated_edge = AudioSource {
            size_bytes: 4_320_000,
            ..song("audio/mpeg", "mp3", None)
        };
        assert_eq!(source_audio_bitrate(&estimated_edge), Some(192_000));
        assert_eq!(plan_audio(&estimated_edge), AudioPlan::Keep);
        let stated_wins = AudioSource {
            size_bytes: 7_200_000,
            ..song("audio/mpeg", "mp3", Some(128_000))
        };
        assert_eq!(plan_audio(&stated_wins), AudioPlan::Keep);
    }

    #[test]
    fn lossless_audio_is_re_encoded() {
        let wav = AudioSource {
            size_bytes: 31_752_000,
            ..song("audio/wav", "pcm", None)
        };
        assert_eq!(source_audio_bitrate(&wav), Some(1_411_200));
        assert_eq!(plan_audio(&wav), AudioPlan::Transcode { bitrate: 128_000 });
        let wav_mono = AudioSource {
            channels: Some(1),
            ..wav.clone()
        };
        assert_eq!(
            plan_audio(&wav_mono),
            AudioPlan::Transcode { bitrate: 64_000 }
        );
        let aiff = song("audio/aiff", "pcm", Some(1_411_200));
        assert_eq!(plan_audio(&aiff), AudioPlan::Transcode { bitrate: 128_000 });
        let flac = song("audio/flac", "flac", None);
        assert_eq!(plan_audio(&flac), AudioPlan::Transcode { bitrate: 128_000 });
        let alac = song("audio/mp4", "alac", Some(900_000));
        assert_eq!(plan_audio(&alac), AudioPlan::Transcode { bitrate: 128_000 });
        let unknowable = AudioSource {
            duration_ms: None,
            ..song("audio/flac", "flac", None)
        };
        assert_eq!(
            plan_audio(&unknowable),
            AudioPlan::Transcode { bitrate: 128_000 },
            "lossless needs no bitrate to be re-encoded; with none, nothing caps the profile's"
        );
    }

    #[test]
    fn ogg_is_re_encoded_and_never_raised() {
        assert_eq!(
            plan_audio(&song("audio/ogg", "vorbis", Some(160_000))),
            AudioPlan::Transcode { bitrate: 128_000 }
        );
        assert_eq!(
            plan_audio(&song("audio/ogg", "opus", Some(96_000))),
            AudioPlan::Transcode { bitrate: 96_000 },
            "Rule B: never above the source's"
        );
        let voice = AudioSource {
            channels: Some(1),
            ..song("audio/ogg", "opus", Some(32_000))
        };
        assert_eq!(plan_audio(&voice), AudioPlan::Transcode { bitrate: 32_000 });
        assert_eq!(
            plan_audio(&song("audio/ogg", "unknown", None)),
            AudioPlan::Transcode { bitrate: 128_000 },
            "it is the container iOS cannot read"
        );
        assert_eq!(
            plan_audio(&song("audio/ogg", "flac", None)),
            AudioPlan::Transcode { bitrate: 128_000 }
        );
    }

    #[test]
    fn other_audio_is_left_alone() {
        assert_eq!(
            plan_audio(&song("audio/mp4", "opus", Some(256_000))),
            AudioPlan::Keep,
            "Opus outside Ogg"
        );
        assert_eq!(
            plan_audio(&song("audio/mp4", "ac3", Some(448_000))),
            AudioPlan::Keep
        );
        assert_eq!(
            plan_audio(&song("audio/mp4", "unknown", Some(900_000))),
            AudioPlan::Keep
        );
        assert_eq!(
            plan_audio(&song("audio/mpeg", "vorbis", None)),
            AudioPlan::Keep
        );
    }

    // --- rules C and D, and what "sendable" is ------------------------------------------------

    #[test]
    fn sendable_is_an_accepted_type_honest_bytes_and_within_the_ceiling() {
        let ceiling = crate::media::SIZE_LIMIT;
        for container in ["video/mp4", "video/quicktime"] {
            assert!(sendable("video", container, true, 1_000, ceiling));
        }
        for container in [
            "audio/mp4",
            "audio/m4a",
            "audio/mpeg",
            "audio/wav",
            "audio/ogg",
        ] {
            assert!(sendable("audio", container, true, 1_000, ceiling));
        }
        assert!(
            !sendable("video", "audio/mp4", true, 1_000, ceiling),
            "a type of the other kind"
        );
        assert!(!sendable("audio", "audio/aiff", true, 1_000, ceiling));
        assert!(!sendable("audio", "audio/flac", true, 1_000, ceiling));
        assert!(!sendable("video", "video/webm", true, 1_000, ceiling));
        assert!(
            !sendable("video", "video/mp4", false, 1_000, ceiling),
            "the bytes say otherwise"
        );
        assert!(
            sendable("video", "video/mp4", true, ceiling, ceiling),
            "at the ceiling is within it"
        );
        assert!(!sendable("video", "video/mp4", true, ceiling + 1, ceiling));
        assert!(!sendable("file", "video/mp4", true, 1_000, ceiling));
        assert!(!sendable("photo", "image/jpeg", true, 1_000, ceiling));
    }

    #[test]
    fn a_failure_sends_the_original_when_it_could_be_sent() {
        assert_eq!(on_failure(true), OnFailure::Original);
        assert_eq!(on_failure(false), OnFailure::TodaysPath);
    }

    #[test]
    fn a_result_bigger_than_a_sendable_source_is_thrown_away() {
        assert_eq!(keep_smaller(10_000_000, true, 12_000_000), Upload::Source);
        assert_eq!(
            keep_smaller(10_000_000, false, 12_000_000),
            Upload::Result,
            "the only thing that can be sent"
        );
        assert_eq!(keep_smaller(10_000_000, true, 4_000_000), Upload::Result);
        assert_eq!(keep_smaller(10_000_000, false, 4_000_000), Upload::Result);
        assert_eq!(
            keep_smaller(10_000_000, true, 10_000_000),
            Upload::Result,
            "equal is not bigger"
        );
        assert_eq!(keep_smaller(10_000_000, true, 10_000_001), Upload::Source);
    }
}
