//! The call's pipeline as text — the pieces [`GstCallMedia`](super::GstCallMedia) parses with
//! `gst::parse::bin_from_description` — and the choices an offer makes for them: which video codec
//! this device sends, and under which payload numbers (`fc_logic::calls`' contract, "The
//! pipeline"). Pure, so every string and every choice is tested without GStreamer.
//!
//! ```text
//! microphone ! audioconvert ! audioresample ! audio/x-raw,rate=48000
//!   ! webrtcdsp probe=<echo> echo-cancel=true noise-suppression=true gain-control=true
//!   ! volume name=mute                                     ── open()
//!   ! audioconvert ! opusenc ! rtpopuspay pt=<p> ! caps ! call.   ── linked once the role is known
//! camera ! videoconvert ! videoscale ! videorate ! video/x-raw,640×480,1/1,30/1,I420
//!   ! tee name=cam
//!   cam. ! queue leaky ! videoflip horizontal-flip ! videoconvert ! <video sink>   (self-view)
//!   cam. ! queue ! valve name=camera                       ── open()
//!   ! vp8enc … ! rtpvp8pay pt=<p>   (or x264enc … ! rtph264pay pt=<p>) ! caps ! call.
//! call. ! decodebin3
//!   audio ! audioconvert ! audioresample ! webrtcechoprobe ! audioconvert ! <speaker>
//!   video ! videoconvert ! <video sink>
//! ```
//!
//! **Why the send branches are linked late.** An answer must carry the offer's own payload numbers
//! (Firefox offers Opus 109 and VP8 120, libwebrtc 111 and 96) and the offer decides the video
//! codec, so the encoder tails are added when the role is known — before `create-offer`, or
//! before `set-remote-description` of an offer — while the pipeline is still `PAUSED` and its
//! live sources produce nothing (a source must be live: a non-live one would push into the
//! unlinked tail and stop with `not-linked`).

use fc_logic::calls::sdp;

/// The payload number Opus is offered under — libwebrtc's, which every peer takes (the phase-0
/// spike).
pub const OPUS_PT: u32 = 111;

/// The payload number VP8 is offered under — libwebrtc's.
pub const VP8_PT: u32 = 96;

/// The payload number H.264 is offered under, when an offer carries it
/// ([`SendPlan::offering`]) — libwebrtc's first H.264.
pub const H264_PT: u32 = 102;

/// What a call's video is encoded with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoCodec {
    /// Every offer: libwebrtc on every peer and Chromium accept it.
    Vp8,
    /// Only to answer an offer whose video carries no VP8.
    H264,
}

impl VideoCodec {
    /// The SDP's and the RTP caps' encoding name.
    pub fn encoding_name(self) -> &'static str {
        match self {
            VideoCodec::Vp8 => "VP8",
            VideoCodec::H264 => "H264",
        }
    }
}

/// What this device sends and under which payload numbers — fixed when the role is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SendPlan {
    pub audio_pt: u32,
    pub video: VideoCodec,
    pub video_pt: u32,
}

impl SendPlan {
    /// Placing a call: Opus 111 and VP8 96.
    pub const OFFER: SendPlan = SendPlan {
        audio_pt: OPUS_PT,
        video: VideoCodec::Vp8,
        video_pt: VP8_PT,
    };

    /// Placing a call with `video` as its only video codec: [`OFFER`](Self::OFFER) for VP8 —
    /// what every call offers — and H.264 under [`H264_PT`] for the self-test's and the
    /// harness's H.264-only offer.
    pub fn offering(video: VideoCodec) -> SendPlan {
        match video {
            VideoCodec::Vp8 => SendPlan::OFFER,
            VideoCodec::H264 => SendPlan {
                video: VideoCodec::H264,
                video_pt: H264_PT,
                ..SendPlan::OFFER
            },
        }
    }

    /// Answering `offer`: its own payload numbers, and VP8 unless its video m-line has none —
    /// then H.264, under the offer's best H.264 payload for a constrained-baseline stream
    /// ([`h264_payload`]). An offer with no Opus (none seen) keeps 111.
    pub fn answering(offer: &str) -> SendPlan {
        let audio_pt = sdp::payload_type(offer, "audio", "OPUS").unwrap_or(OPUS_PT);
        if let Some(video_pt) = sdp::payload_type(offer, "video", "VP8") {
            return SendPlan {
                audio_pt,
                video: VideoCodec::Vp8,
                video_pt,
            };
        }
        match h264_payload(offer) {
            Some(video_pt) => SendPlan {
                audio_pt,
                video: VideoCodec::H264,
                video_pt,
            },
            None => SendPlan {
                audio_pt,
                ..SendPlan::OFFER
            },
        }
    }
}

/// Whether `sdp` has an m-line of this kind ("audio", "video").
pub fn has_media(sdp: &str, media: &str) -> bool {
    sdp.lines()
        .filter_map(|line| line.trim_end_matches('\r').strip_prefix("m="))
        .any(|line| line.split(' ').next() == Some(media))
}

/// The lines of the first m-section of this kind, the `m=` line first.
fn section<'a>(sdp: &'a str, media: &str) -> Vec<&'a str> {
    let mut lines = Vec::new();
    let mut inside = false;
    for line in sdp.lines().map(|line| line.trim_end_matches('\r')) {
        if let Some(m) = line.strip_prefix("m=") {
            if inside {
                break;
            }
            inside = m.split(' ').next() == Some(media);
        }
        if inside {
            lines.push(line);
        }
    }
    lines
}

/// The payload number of the offer's H.264 that a constrained-baseline stream with
/// `packetization-mode=1` (what `x264enc … ! rtph264pay` sends) fits best: constrained baseline
/// (`42e0…`/`42c0…`) first, then baseline (`4200…`, of which constrained baseline is a subset),
/// then any H.264 with mode 1, then the first H.264 at all; `None` when the video carries none.
pub fn h264_payload(sdp: &str) -> Option<u32> {
    let lines = section(sdp, "video");
    let payloads: Vec<u32> = lines
        .iter()
        .filter_map(|line| {
            let (payload, format) = line.strip_prefix("a=rtpmap:")?.split_once(' ')?;
            let name = format.split('/').next()?;
            name.eq_ignore_ascii_case("H264")
                .then(|| payload.trim().parse().ok())
                .flatten()
        })
        .collect();
    let fmtp = |payload: u32| -> String {
        let prefix = format!("a=fmtp:{payload} ");
        lines
            .iter()
            .find_map(|line| line.strip_prefix(prefix.as_str()))
            .unwrap_or_default()
            .to_ascii_lowercase()
    };
    let parameter = |payload: u32, name: &str| -> Option<String> {
        fmtp(payload).split(';').find_map(|pair| {
            let (key, value) = pair.trim().split_once('=')?;
            (key == name).then(|| value.trim().to_string())
        })
    };
    let mode_one =
        |payload: &u32| parameter(*payload, "packetization-mode").as_deref() == Some("1");
    let profile = |payload: &u32| parameter(*payload, "profile-level-id").unwrap_or_default();
    let constrained = |payload: &u32| {
        let id = profile(payload);
        id.starts_with("42e0") || id.starts_with("42c0")
    };
    let baseline = |payload: &u32| profile(payload).starts_with("4200");
    payloads
        .iter()
        .find(|payload| mode_one(payload) && constrained(payload))
        .or_else(|| {
            payloads
                .iter()
                .find(|payload| mode_one(payload) && baseline(payload))
        })
        .or_else(|| payloads.iter().find(|payload| mode_one(payload)))
        .or_else(|| payloads.first())
        .copied()
}

/// The microphone up to the mute: the source (PipeWire's default through `pulsesrc`, or a test
/// source), 10-ms-friendly raw audio at a rate the canceller takes, the canceller listening to
/// `probe`, and `volume name=mute` — after the canceller, before the encoder, so silence keeps
/// flowing when muted.
pub fn microphone(source: &str, probe: &str) -> String {
    format!(
        "{source} ! audioconvert ! audioresample ! audio/x-raw,rate=48000 \
         ! webrtcdsp name=dsp probe={probe} echo-cancel=true noise-suppression=true \
         gain-control=true ! volume name=mute"
    )
}

/// The Opus tail from the mute to `webrtcbin`, under `pt`. The caps are spelled out in full
/// because `webrtcbin` writes an OFFER from its sink pad's caps before any data has negotiated
/// them: with `encoding-name` and `payload` alone the offer said `a=rtpmap:111 OPUS/48000` — no
/// `/2`, which RFC 7587 requires — and Chromium answered the m-line rejected (port 0), Firefox
/// refused the offer (measured; Linux↔Linux passed, `webrtcbin` being lenient).
pub fn audio_encoder(pt: u32) -> String {
    format!(
        "audioconvert ! opusenc ! rtpopuspay pt={pt} \
         ! capsfilter caps=\"application/x-rtp,media=audio,encoding-name=OPUS,payload={pt},\
         clock-rate=48000,encoding-params=(string)2\""
    )
}

/// The camera up to the encoder valve, with the mirrored self-view on the side. The picture is
/// 640 × 480 with square pixels (a 16:9 camera is letterboxed rather than squashed — VP8 carries
/// no aspect ratio), pinned to I420 before any encoder (an `x264enc` left free negotiates High
/// 4:4:4 10-bit, which no browser or Apple decoder plays — shell-design §1.4). The valve starts
/// shut and opens when the tail is linked.
pub fn camera(video_sink: &str) -> String {
    format!(
        "videoconvert ! videoscale ! videorate \
         ! video/x-raw,width=640,height=480,pixel-aspect-ratio=1/1,framerate=30/1,format=I420 \
         ! tee name=cam allow-not-linked=true \
         cam. ! queue max-size-buffers=2 leaky=downstream ! videoflip method=horizontal-flip \
         ! videoconvert ! {video_sink} \
         cam. ! queue ! valve name=camera drop=true"
    )
}

/// The video tail from the camera valve to `webrtcbin`, its caps in full for the same reason as
/// [`audio_encoder`]'s (H.264's `packetization-mode=1` is what `rtph264pay` sends; its
/// `profile-level-id` comes from the stream and is not pinned).
pub fn video_encoder(codec: VideoCodec, pt: u32) -> String {
    match codec {
        VideoCodec::Vp8 => format!(
            "vp8enc deadline=1 keyframe-max-dist=30 ! rtpvp8pay pt={pt} \
             ! capsfilter caps=\"application/x-rtp,media=video,encoding-name=VP8,payload={pt},\
             clock-rate=90000\""
        ),
        VideoCodec::H264 => format!(
            "x264enc tune=zerolatency speed-preset=ultrafast key-int-max=30 \
             ! video/x-h264,profile=constrained-baseline \
             ! rtph264pay config-interval=-1 aggregate-mode=zero-latency pt={pt} \
             ! capsfilter caps=\"application/x-rtp,media=video,encoding-name=H264,payload={pt},\
             clock-rate=90000,packetization-mode=(string)1\""
        ),
    }
}

/// The caps of a `recvonly` video transceiver for this codec and payload number.
pub fn receive_only_caps(codec: VideoCodec, pt: u32) -> String {
    format!(
        "application/x-rtp,media=video,encoding-name={},payload={pt},clock-rate=90000",
        codec.encoding_name()
    )
}

/// The far side's sound, from the decoder to the echo probe.
pub const SPEAKER_HEAD: &str = "audioconvert ! audioresample";

/// The far side's sound, from the echo probe to the speaker.
pub fn speaker_tail(speaker: &str) -> String {
    format!("audioconvert ! {speaker}")
}

/// The far side's picture, from the decoder.
pub fn picture(video_sink: &str) -> String {
    format!("videoconvert ! {video_sink}")
}

/// The ring tones' pipeline: an `appsrc` named `tone` looping one cycle, into `sink`.
pub fn ringer(sink: &str, rate: u32) -> String {
    format!(
        "appsrc name=tone is-live=true format=time \
         caps=audio/x-raw,format=S16LE,layout=interleaved,rate={rate},channels=1 \
         ! audioconvert ! audioresample ! {sink}"
    )
}

/// The echo probe's element name: process-unique, because `webrtcdsp` finds its probe by name
/// among every probe in the process.
pub fn echo_probe_name(serial: u64) -> String {
    format!("fc-call-echo-{serial}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chromium's audio + video offer, trimmed to the lines that matter here.
    const CHROMIUM: &str = "v=0\r\n\
        m=audio 9 UDP/TLS/RTP/SAVPF 111 63 9 0 8 13 110 126\r\n\
        a=mid:0\r\n\
        a=rtpmap:111 opus/48000/2\r\n\
        a=fmtp:111 minptime=10;useinbandfec=1\r\n\
        a=rtpmap:63 red/48000/2\r\n\
        m=video 9 UDP/TLS/RTP/SAVPF 96 97 102 103 104 105 106 107\r\n\
        a=mid:1\r\n\
        a=rtpmap:96 VP8/90000\r\n\
        a=rtpmap:97 rtx/90000\r\n\
        a=fmtp:97 apt=96\r\n\
        a=rtpmap:102 H264/90000\r\n\
        a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f\r\n\
        a=rtpmap:104 H264/90000\r\n\
        a=fmtp:104 level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42001f\r\n\
        a=rtpmap:106 H264/90000\r\n\
        a=fmtp:106 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n";

    /// Firefox's numbers: Opus 109, VP8 120, H.264 126 (mode 1) and 97 (mode 0).
    const FIREFOX: &str = "v=0\r\n\
        m=audio 9 UDP/TLS/RTP/SAVPF 109 9 0 8 101\r\n\
        a=mid:0\r\n\
        a=rtpmap:109 opus/48000/2\r\n\
        a=fmtp:109 maxplaybackrate=48000;stereo=1;useinbandfec=1\r\n\
        m=video 9 UDP/TLS/RTP/SAVPF 120 124 121 125 126 127 97 98\r\n\
        a=mid:1\r\n\
        a=rtpmap:120 VP8/90000\r\n\
        a=fmtp:120 max-fs=12288;max-fr=60\r\n\
        a=rtpmap:121 VP9/90000\r\n\
        a=rtpmap:126 H264/90000\r\n\
        a=fmtp:126 profile-level-id=42e01f;level-asymmetry-allowed=1;packetization-mode=1\r\n\
        a=rtpmap:97 H264/90000\r\n\
        a=fmtp:97 profile-level-id=42e01f;level-asymmetry-allowed=1\r\n";

    /// Chromium after `setCodecPreferences` kept H.264 alone.
    const H264_ONLY: &str = "v=0\r\n\
        m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
        a=rtpmap:111 opus/48000/2\r\n\
        m=video 9 UDP/TLS/RTP/SAVPF 102 104 106 108 112\r\n\
        a=rtpmap:102 H264/90000\r\n\
        a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f\r\n\
        a=rtpmap:104 H264/90000\r\n\
        a=fmtp:104 level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42001f\r\n\
        a=rtpmap:106 H264/90000\r\n\
        a=fmtp:106 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n\
        a=rtpmap:108 H264/90000\r\n\
        a=fmtp:108 level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42e01f\r\n\
        a=rtpmap:112 H264/90000\r\n\
        a=fmtp:112 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=64001f\r\n";

    #[test]
    fn an_offer_is_opus_111_and_vp8_96() {
        assert_eq!(
            SendPlan::OFFER,
            SendPlan {
                audio_pt: 111,
                video: VideoCodec::Vp8,
                video_pt: 96
            }
        );
    }

    #[test]
    fn an_h264_offer_keeps_opus_111_and_takes_102() {
        assert_eq!(SendPlan::offering(VideoCodec::Vp8), SendPlan::OFFER);
        assert_eq!(
            SendPlan::offering(VideoCodec::H264),
            SendPlan {
                audio_pt: 111,
                video: VideoCodec::H264,
                video_pt: 102
            }
        );
    }

    #[test]
    fn answering_libwebrtc_keeps_its_numbers_and_vp8() {
        assert_eq!(
            SendPlan::answering(CHROMIUM),
            SendPlan {
                audio_pt: 111,
                video: VideoCodec::Vp8,
                video_pt: 96
            }
        );
    }

    /// Firefox's offer answered under Firefox's own numbers — the answer must use the offer's.
    #[test]
    fn answering_firefox_takes_opus_109_and_vp8_120() {
        assert_eq!(
            SendPlan::answering(FIREFOX),
            SendPlan {
                audio_pt: 109,
                video: VideoCodec::Vp8,
                video_pt: 120
            }
        );
    }

    #[test]
    fn an_offer_without_vp8_is_answered_with_constrained_baseline_h264() {
        assert_eq!(
            SendPlan::answering(H264_ONLY),
            SendPlan {
                audio_pt: 111,
                video: VideoCodec::H264,
                video_pt: 106
            }
        );
    }

    #[test]
    fn the_h264_payload_falls_back_from_constrained_to_baseline_to_mode_one_to_any() {
        let baseline = "m=video 9 RTP/SAVPF 102 104\r\n\
            a=rtpmap:104 H264/90000\r\n\
            a=fmtp:104 packetization-mode=0;profile-level-id=42e01f\r\n\
            a=rtpmap:102 H264/90000\r\n\
            a=fmtp:102 packetization-mode=1;profile-level-id=42001f\r\n";
        assert_eq!(h264_payload(baseline), Some(102));
        let high = "m=video 9 RTP/SAVPF 112 108\r\n\
            a=rtpmap:108 H264/90000\r\n\
            a=fmtp:108 packetization-mode=0;profile-level-id=42e01f\r\n\
            a=rtpmap:112 H264/90000\r\n\
            a=fmtp:112 packetization-mode=1;profile-level-id=64001f\r\n";
        assert_eq!(h264_payload(high), Some(112));
        let bare = "m=video 9 RTP/SAVPF 98\r\na=rtpmap:98 h264/90000\r\n";
        assert_eq!(h264_payload(bare), Some(98));
        assert_eq!(
            h264_payload("m=video 9 RTP/SAVPF 96\r\na=rtpmap:96 VP8/90000\r\n"),
            None
        );
        // Firefox's mode-1 H.264 wins over its mode-0 one.
        assert_eq!(h264_payload(FIREFOX), Some(126));
    }

    /// An H.264 line in the AUDIO section (nonsense, but cheap to refuse) is not the video's.
    #[test]
    fn only_the_video_section_is_read_for_h264() {
        let sdp = "m=audio 9 RTP/SAVPF 102\r\na=rtpmap:102 H264/90000\r\n\
            m=video 9 RTP/SAVPF 96\r\na=rtpmap:96 VP8/90000\r\n";
        assert_eq!(h264_payload(sdp), None);
    }

    #[test]
    fn an_offer_with_neither_vp8_nor_h264_falls_back_to_the_offer_plan() {
        let vp9 = "m=audio 9 RTP/SAVPF 109\r\na=rtpmap:109 opus/48000/2\r\n\
            m=video 9 RTP/SAVPF 121\r\na=rtpmap:121 VP9/90000\r\n";
        assert_eq!(
            SendPlan::answering(vp9),
            SendPlan {
                audio_pt: 109,
                ..SendPlan::OFFER
            }
        );
    }

    #[test]
    fn media_kinds_are_read_from_the_m_lines() {
        assert!(has_media(CHROMIUM, "audio"));
        assert!(has_media(CHROMIUM, "video"));
        assert!(!has_media("m=audio 9 RTP/SAVPF 111\r\n", "video"));
        assert!(!has_media("a=video\r\n", "video"));
    }

    #[test]
    fn the_microphone_feeds_the_canceller_and_ends_at_the_mute() {
        let text = microphone("pulsesrc", "fc-call-echo-3");
        assert!(
            text.starts_with("pulsesrc ! audioconvert ! audioresample ! audio/x-raw,rate=48000")
        );
        assert!(text.contains("webrtcdsp name=dsp probe=fc-call-echo-3 echo-cancel=true"));
        assert!(text.contains("noise-suppression=true"));
        assert!(text.contains("gain-control=true"));
        assert!(text.ends_with("! volume name=mute"));
    }

    #[test]
    fn the_encoders_carry_the_payload_number_in_the_payloader_and_the_caps() {
        assert_eq!(
            audio_encoder(109),
            "audioconvert ! opusenc ! rtpopuspay pt=109 \
             ! capsfilter caps=\"application/x-rtp,media=audio,encoding-name=OPUS,payload=109,\
             clock-rate=48000,encoding-params=(string)2\""
        );
        let vp8 = video_encoder(VideoCodec::Vp8, 120);
        assert!(vp8.starts_with("vp8enc deadline=1 keyframe-max-dist=30 ! rtpvp8pay pt=120"));
        assert!(vp8.ends_with("encoding-name=VP8,payload=120,clock-rate=90000\""));
        let h264 = video_encoder(VideoCodec::H264, 106);
        assert!(h264.starts_with("x264enc tune=zerolatency speed-preset=ultrafast"));
        assert!(h264.contains("video/x-h264,profile=constrained-baseline"));
        assert!(h264.contains("rtph264pay config-interval=-1 aggregate-mode=zero-latency pt=106"));
        assert!(h264.ends_with(
            "encoding-name=H264,payload=106,clock-rate=90000,packetization-mode=(string)1\""
        ));
    }

    #[test]
    fn the_camera_pins_i420_and_starts_with_the_valve_shut() {
        let text = camera("gtk4paintablesink");
        assert!(text.contains("format=I420 ! tee name=cam allow-not-linked=true"));
        assert!(
            text.contains("videoflip method=horizontal-flip ! videoconvert ! gtk4paintablesink")
        );
        assert!(text.ends_with("cam. ! queue ! valve name=camera drop=true"));
    }

    #[test]
    fn a_receive_only_transceiver_names_the_codec_and_payload() {
        assert_eq!(
            receive_only_caps(VideoCodec::Vp8, 96),
            "application/x-rtp,media=video,encoding-name=VP8,payload=96,clock-rate=90000"
        );
        assert_eq!(
            receive_only_caps(VideoCodec::H264, 106),
            "application/x-rtp,media=video,encoding-name=H264,payload=106,clock-rate=90000"
        );
    }

    #[test]
    fn the_far_side_and_the_ringer_end_in_the_given_sinks() {
        assert_eq!(speaker_tail("pulsesink"), "audioconvert ! pulsesink");
        assert_eq!(picture("fakesink"), "videoconvert ! fakesink");
        let ring = ringer("pulsesink", 48_000);
        assert!(ring.starts_with("appsrc name=tone is-live=true format=time"));
        assert!(ring.contains("rate=48000,channels=1"));
        assert!(ring.ends_with("! audioconvert ! audioresample ! pulsesink"));
        assert_eq!(echo_probe_name(7), "fc-call-echo-7");
    }

    /// Every description parses with the real GStreamer and leaves exactly the pads the media
    /// links (`gst-launch` caps shorthand needs an element on BOTH sides, so a tail ending in
    /// caps must be a `capsfilter` — measured: "link has no sink").
    #[test]
    fn every_description_parses_with_gstreamer_and_ghosts_the_right_pads() {
        gst::init().unwrap();
        let pads = |description: &str| {
            use gst::prelude::*;
            let bin = gst::parse::bin_from_description(description, true)
                .unwrap_or_else(|error| panic!("{description}: {error}"));
            let mut names: Vec<String> = bin
                .pads()
                .iter()
                .map(|pad| pad.name().to_string())
                .collect();
            names.sort();
            names
        };
        let source = "audiotestsrc is-live=true";
        assert_eq!(pads(&microphone(source, "fc-call-echo-0")), ["src"]);
        assert_eq!(pads(&audio_encoder(111)), ["sink", "src"]);
        assert_eq!(pads(&video_encoder(VideoCodec::Vp8, 96)), ["sink", "src"]);
        assert_eq!(pads(&video_encoder(VideoCodec::H264, 106)), ["sink", "src"]);
        assert_eq!(pads(&camera("fakesink")), ["sink", "src"]);
        assert_eq!(pads(SPEAKER_HEAD), ["sink", "src"]);
        assert_eq!(pads(&speaker_tail("fakesink")), ["sink"]);
        assert_eq!(pads(&picture("fakesink")), ["sink"]);
        assert!(pads(&ringer("fakesink", RATE_FOR_TESTS)).is_empty());
        assert!(
            receive_only_caps(VideoCodec::H264, 106)
                .parse::<gst::Caps>()
                .is_ok()
        );
    }

    const RATE_FOR_TESTS: u32 = 48_000;
}
