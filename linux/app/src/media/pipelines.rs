//! Every GStreamer description the media package launches (shell-design §10; W.1 "Logic
//! placement"): pure string builders — no GStreamer here — so the profile's numbers and §1.4's
//! trap are pinned by tests that need no display.
//!
//! **The trap, once more** (§1.4, carry-forward): an `x264enc` with nothing after it, fed by a
//! source that can offer more, negotiated High 4:4:4 10-bit, which no browser and no Apple decoder
//! plays. Every encode here puts `video/x-raw,format=I420` before the encoder AND
//! `video/x-h264,profile=…` after it; every MP4 that is sent is `mp4mux faststart=true` (`moov`
//! first) — except an animation's, whose index must come LAST so fc-logic can rewrite its timing
//! (§8.4, `animation_preparing::AnimationEncoder`).
//!
//! File locations are never written into a description (a home folder may hold a space or a
//! quote): the file elements are named (`name=out`, `name=src`) and their `location` set on the
//! element after parsing.

use fc_logic::media_encoding::{AacEncoding, H264Profile, VideoEncoding};

/// The microphone: PipeWire's Pulse server's default source (§10.2). A host app is never asked.
pub const MICROPHONE: &str = "pulsesrc";

/// The self-test's microphone (§27): a live tone. Only a `selftest` build ever uses it
/// (`media::test_sources`).
pub const TEST_TONE: &str = "audiotestsrc is-live=true wave=sine freq=440 volume=0.4";

/// The self-test's camera: a moving picture at a laptop camera's 1280 × 720, 30 fps (a caps
/// filter must sit between two elements, so a `queue` ends it).
pub const TEST_CAMERA: &str = "videotestsrc is-live=true pattern=smpte ! video/x-raw,width=1280,height=720,framerate=30/1 ! queue";

/// An AAC encoder, in the order they are tried (§10.2: `avenc_aac`, then `voaacenc`; both take
/// 64 000 for mono as asked — measured, §1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AacEncoder {
    Avenc,
    Vo,
}

impl AacEncoder {
    /// The order to try them in.
    pub const ORDER: [AacEncoder; 2] = [AacEncoder::Avenc, AacEncoder::Vo];

    /// The element's factory name.
    pub fn factory(self) -> &'static str {
        match self {
            AacEncoder::Avenc => "avenc_aac",
            AacEncoder::Vo => "voaacenc",
        }
    }
}

/// The H.264 encoder an attempt's profile names (carry-forward: `x264enc` for High, `openh264enc`
/// for Main — its "High" is constrained-high).
pub fn h264_factory(profile: H264Profile) -> &'static str {
    match profile {
        H264Profile::High => "x264enc",
        H264Profile::Main => "openh264enc",
    }
}

/// `avenc_aac bitrate=64000`.
pub fn aac(encoder: AacEncoder, bitrate: u32) -> String {
    format!("{} bitrate={bitrate}", encoder.factory())
}

/// The raw audio an AAC encoding is fed: its rate and channels.
pub fn raw_audio(asked: &AacEncoding) -> String {
    format!(
        "audio/x-raw,rate={},channels={}",
        asked.sample_rate, asked.channels
    )
}

/// The raw video an encode is fed — the format ALWAYS pinned (§1.4), square pixels, and BT.709
/// where SDR was asked of an HDR source.
pub fn raw_video(width: u32, height: u32, numerator: u32, denominator: u32, sdr: bool) -> String {
    let mut caps = format!(
        "video/x-raw,format=I420,width={width},height={height},framerate={numerator}/{}",
        denominator.max(1)
    );
    caps.push_str(",pixel-aspect-ratio=1/1");
    if sdr {
        caps.push_str(",colorimetry=bt709");
    }
    caps
}

/// The encoder and its output caps for `profile`: `x264enc bitrate=<kbit/s> [key-int-max=…] !
/// video/x-h264,profile=high`, or `openh264enc bitrate=<bit/s> [gop-size=…] !
/// video/x-h264,profile=main`. `live`: a camera take (`speed-preset=veryfast tune=zerolatency`).
pub fn h264(profile: H264Profile, bitrate: u32, spacing: Option<u32>, live: bool) -> String {
    match profile {
        H264Profile::High => {
            let mut encoder = format!("x264enc bitrate={}", (bitrate / 1000).max(1));
            if let Some(spacing) = spacing {
                encoder.push_str(&format!(" key-int-max={spacing}"));
            }
            if live {
                encoder.push_str(" speed-preset=veryfast tune=zerolatency");
            }
            format!("{encoder} ! video/x-h264,profile=high")
        }
        H264Profile::Main => {
            let mut encoder = format!("openh264enc bitrate={}", bitrate.max(1));
            if let Some(spacing) = spacing {
                encoder.push_str(&format!(" gop-size={spacing}"));
            }
            format!("{encoder} ! video/x-h264,profile=main")
        }
    }
}

/// §10.2's voice note: ONE microphone, two branches — the AAC file and the `level` meter. The
/// `filesink` is named `out`.
pub fn voice_recording(source: &str, asked: &AacEncoding, encoder: AacEncoder) -> String {
    format!(
        "{source} ! audioconvert ! audioresample ! {raw} ! tee name=t \
         t. ! queue ! {aac} ! aacparse ! mp4mux faststart=true ! filesink name=out \
         t. ! queue leaky=downstream ! level interval=50000000 post-messages=true \
         ! fakesink sync=false async=false",
        raw = raw_audio(asked),
        aac = aac(encoder, asked.bitrate),
    )
}

/// What follows a camera source in the recorder's pipeline (§10.5): one stream, two branches — the
/// MIRRORED preview into `gtk4paintablesink` (named `preview`) and the frames for the take into an
/// `appsink` (named `frames`; the recording pipeline is fed from it, so a take starts and ends
/// without touching the preview). `decode`: the source may deliver MJPEG (a `v4l2src`).
pub fn camera_branches(decode: bool) -> String {
    let head = if decode {
        "decodebin ! videoconvert"
    } else {
        "videoconvert"
    };
    format!(
        "{head} ! video/x-raw,format=I420 ! tee name=t \
         t. ! queue leaky=downstream max-size-buffers=2 ! videoflip method=horizontal-flip \
         ! videoconvert ! gtk4paintablesink name=preview \
         t. ! queue leaky=downstream max-size-buffers=4 \
         ! appsink name=frames sync=false max-buffers=4 drop=true"
    )
}

/// A take's own pipeline (§10.5): the frames pushed from the camera's `appsink` into `appsrc
/// name=frames` (timestamped by this pipeline's clock), the centre square cut (`crop` = `videocrop`'s
/// left, right, top, bottom), scaled to 480 × 480 at the camera's own rate up to 30 — NOT mirrored,
/// upright — then H.264 and, when asked, the microphone's AAC, into `mp4mux faststart=true !
/// filesink name=out`. The audio source is named `mic` (its EOS ends the take with the video's).
pub fn take_recording(
    crop: (u32, u32, u32, u32),
    asked: &VideoEncoding,
    microphone: &str,
    aac_encoder: AacEncoder,
) -> String {
    let (left, right, top, bottom) = crop;
    let mut description = format!(
        "appsrc name=frames is-live=true do-timestamp=true format=time ! queue ! videoconvert \
         ! videocrop left={left} right={right} top={top} bottom={bottom} ! videoscale ! videorate \
         ! {raw} ! {h264} ! h264parse ! mux. ",
        raw = raw_video(
            asked.width,
            asked.height,
            asked.frame_rate_numerator,
            asked.frame_rate_denominator,
            false
        ),
        h264 = h264(asked.profile, asked.bitrate, asked.keyframe_spacing, true),
    );
    if let Some(audio) = asked.audio {
        description.push_str(&format!(
            "{microphone} name=mic ! audioconvert ! audioresample ! {raw} ! {aac} ! aacparse ! mux. ",
            raw = raw_audio(&audio),
            aac = aac(aac_encoder, audio.bitrate),
        ));
    }
    description.push_str("mp4mux name=mux faststart=true ! filesink name=out");
    description
}

/// The voice decoder's pipeline (`voice_look::VoiceDecoder`, §10.2's sender's waveform): the whole
/// note decoded to mono 16 kHz signed 16-bit into `appsink name=pcm`. The `filesrc` is `src`.
pub const WAVEFORM_DECODE: &str = "filesrc name=src ! decodebin ! audioconvert ! audioresample \
     ! audio/x-raw,format=S16LE,layout=interleaved,channels=1,rate=16000 \
     ! appsink name=pcm sync=false";

/// The sink bin of a frame grab — what `playbin3` draws a picked file's frame into: turned upright
/// by the stream's own orientation (the same for every file, so a source and its transcode are
/// compared alike — `frame_turn`), converted and scaled to `width × height` in `format`, square
/// pixels, into `appsink name=frame`. `None` for either side keeps the picture's own.
pub fn frame_sink(format: &str, width: Option<u32>, height: Option<u32>) -> String {
    let mut caps = format!("video/x-raw,format={format}");
    if let Some(width) = width {
        caps.push_str(&format!(",width={width}"));
    }
    if let Some(height) = height {
        caps.push_str(&format!(",height={height}"));
    }
    caps.push_str(",pixel-aspect-ratio=1/1");
    format!(
        "videoflip video-direction=auto ! videoconvert ! videoscale ! {caps} \
         ! appsink name=frame sync=false max-buffers=1"
    )
}

/// The video branch of a picked video's transcode (§10.7, `MediaPreparing::encode`): converted,
/// scaled to the STORED sides, the rate never raised, the format pinned, then the encoder — linked
/// from `decodebin`'s video pad into a bin whose sink is a ghost pad, ending in `h264parse`.
pub fn transcode_video(asked: &VideoEncoding) -> String {
    let convert = if asked.to_sdr {
        "videoconvert gamma-mode=remap primaries-mode=full"
    } else {
        "videoconvert"
    };
    format!(
        "queue ! {convert} ! videoscale ! videorate ! {raw} ! {h264} ! h264parse",
        raw = raw_video(
            asked.width,
            asked.height,
            asked.frame_rate_numerator,
            asked.frame_rate_denominator,
            asked.to_sdr
        ),
        h264 = h264(asked.profile, asked.bitrate, asked.keyframe_spacing, false),
    )
}

/// The audio branch of a transcode or a sound file's re-encode, after the decoder (or after an
/// `audiomixer` for a mix): to the asked rate and channels, then AAC, then `aacparse`.
pub fn transcode_audio(asked: &AacEncoding, encoder: AacEncoder) -> String {
    format!(
        "queue ! audioconvert ! audioresample ! {raw} ! {aac} ! aacparse",
        raw = raw_audio(asked),
        aac = aac(encoder, asked.bitrate),
    )
}

/// An animation's encode (§8.4, `AnimationEncoder`): NV12 frames pushed into `appsrc name=frames`
/// exactly as fc-logic made them (BT.709 limited — no `videoconvert` may touch them), H.264, NO
/// audio, and `mp4mux` WITHOUT `faststart`: `moov` must come last.
pub fn animation_encode(width: i32, height: i32, bitrate: u32, profile: H264Profile) -> String {
    format!(
        "appsrc name=frames format=time \
         caps=video/x-raw,format=NV12,width={width},height={height},framerate=30/1,\
pixel-aspect-ratio=1/1,colorimetry=bt709 \
         ! {h264} ! h264parse ! mp4mux ! filesink name=out",
        h264 = h264(profile, bitrate, Some(60), false),
    )
}

/// The headers souphttpsrc sends with every request of an authorised stream (§1.4): a
/// `GstStructure` description, the token quoted. Never logged.
pub fn bearer_headers(token: &str) -> String {
    let mut quoted = String::with_capacity(token.len() + 9);
    for c in format!("Bearer {token}").chars() {
        if matches!(c, '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    format!("headers, Authorization=(string)\"{quoted}\"")
}

/// A local file as the URI the players take (`file://`, every byte that needs it escaped).
pub fn file_uri(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(byte) {
            uri.push(char::from(*byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice() -> AacEncoding {
        fc_logic::media_encoding::voice_note_attempts()[0]
    }

    #[test]
    fn a_voice_note_is_one_microphone_with_a_meter_beside_the_file() {
        let description = voice_recording(MICROPHONE, &voice(), AacEncoder::Avenc);
        assert!(description.starts_with("pulsesrc ! "), "{description}");
        assert!(description.contains("audio/x-raw,rate=44100,channels=1"));
        assert!(description.contains("avenc_aac bitrate=64000 ! aacparse"));
        assert!(description.contains("mp4mux faststart=true ! filesink name=out"));
        assert!(description.contains("level interval=50000000 post-messages=true"));
        assert_eq!(description.matches("pulsesrc").count(), 1, "one microphone");
        assert!(
            voice_recording(TEST_TONE, &voice(), AacEncoder::Vo).contains("voaacenc bitrate=64000")
        );
    }

    #[test]
    fn every_h264_encode_pins_the_raw_format_and_the_profile() {
        let take = fc_logic::round_video_rules::encodes(30, 1, true);
        for asked in &take {
            let description =
                take_recording((280, 280, 0, 0), asked, MICROPHONE, AacEncoder::Avenc);
            assert!(
                description.contains("video/x-raw,format=I420,width=480,height=480,framerate=30/1")
            );
            let profile = match asked.profile {
                H264Profile::High => "video/x-h264,profile=high",
                H264Profile::Main => "video/x-h264,profile=main",
            };
            assert!(description.contains(profile), "{description}");
            assert!(description.contains("mp4mux name=mux faststart=true"));
            assert!(description.contains("videocrop left=280 right=280 top=0 bottom=0"));
            // Mirrored only in the preview.
            assert!(!description.contains("videoflip"));
        }
        assert!(take[0].keyframe_spacing.is_some());
        let first = take_recording((280, 280, 0, 0), &take[0], MICROPHONE, AacEncoder::Avenc);
        assert!(
            first.contains(
                "x264enc bitrate=500 key-int-max=60 speed-preset=veryfast tune=zerolatency"
            )
        );
        assert!(first.contains("pulsesrc name=mic"));
        let main = h264(H264Profile::Main, 500_000, Some(60), true);
        assert_eq!(
            main,
            "openh264enc bitrate=500000 gop-size=60 ! video/x-h264,profile=main"
        );
    }

    #[test]
    fn a_take_with_no_sound_asked_has_no_microphone() {
        let silent = fc_logic::round_video_rules::encodes(30, 1, false);
        let description =
            take_recording((0, 0, 120, 120), &silent[0], MICROPHONE, AacEncoder::Avenc);
        assert!(!description.contains("pulsesrc"));
        assert!(!description.contains("aacparse"));
    }

    #[test]
    fn the_preview_alone_is_mirrored() {
        let branches = camera_branches(false);
        assert!(branches.starts_with("videoconvert ! video/x-raw,format=I420 ! tee name=t"));
        assert!(branches.contains(
            "videoflip method=horizontal-flip ! videoconvert ! gtk4paintablesink name=preview"
        ));
        assert!(branches.contains("appsink name=frames"));
        assert!(camera_branches(true).starts_with("decodebin ! videoconvert"));
    }

    #[test]
    fn a_transcode_branch_pins_the_format_and_asks_sdr_where_told() {
        let target = fc_text::media_plan::VideoTarget {
            width: 1280,
            height: 720,
            frame_rate: 30.0,
            video_bitrate: 2_000_000,
            audio_bitrate: Some(128_000),
        };
        let attempts = fc_logic::media_encoding::video_attempts(
            &target,
            0,
            60,
            1,
            Some(2),
            Some(48_000),
            true,
        );
        let video = transcode_video(&attempts[0]);
        assert!(video.contains("videoconvert gamma-mode=remap primaries-mode=full"));
        assert!(video.contains(
            "video/x-raw,format=I420,width=1280,height=720,framerate=30/1,pixel-aspect-ratio=1/1,colorimetry=bt709"
        ));
        assert!(video.contains("x264enc bitrate=2000 ! video/x-h264,profile=high ! h264parse"));
        assert!(
            transcode_video(&attempts[1])
                .contains("openh264enc bitrate=2000000 ! video/x-h264,profile=main")
        );
        let audio = transcode_audio(&attempts[0].audio.unwrap(), AacEncoder::Avenc);
        assert_eq!(
            audio,
            "queue ! audioconvert ! audioresample ! audio/x-raw,rate=48000,channels=2 ! avenc_aac bitrate=128000 ! aacparse"
        );
    }

    #[test]
    fn an_animation_keeps_its_index_last_and_its_frames_untouched() {
        let description = animation_encode(320, 240, 800_000, H264Profile::High);
        assert!(description.contains("format=NV12,width=320,height=240,framerate=30/1"));
        assert!(
            description.contains("x264enc bitrate=800 key-int-max=60 ! video/x-h264,profile=high")
        );
        assert!(description.contains("! mp4mux ! filesink name=out"));
        assert!(!description.contains("faststart"));
        assert!(!description.contains("videoconvert"));
        assert!(!description.contains("audio"));
    }

    #[test]
    fn a_frame_grab_turns_upright_and_squashes_to_the_size_asked() {
        assert_eq!(
            frame_sink("BGRA", Some(16), Some(16)),
            "videoflip video-direction=auto ! videoconvert ! videoscale \
             ! video/x-raw,format=BGRA,width=16,height=16,pixel-aspect-ratio=1/1 \
             ! appsink name=frame sync=false max-buffers=1"
        );
    }

    #[test]
    fn the_bearer_header_is_quoted_and_a_file_is_a_file_uri() {
        assert_eq!(
            bearer_headers("abc\"d\\e"),
            "headers, Authorization=(string)\"Bearer abc\\\"d\\\\e\""
        );
        assert_eq!(
            file_uri(std::path::Path::new("/home/a b/blobs/12")),
            "file:///home/a%20b/blobs/12"
        );
        assert_eq!(
            file_uri(std::path::Path::new("/x/Привет")),
            "file:///x/%D0%9F%D1%80%D0%B8%D0%B2%D0%B5%D1%82"
        );
    }
}
