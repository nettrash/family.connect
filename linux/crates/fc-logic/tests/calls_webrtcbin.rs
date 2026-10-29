//! What Linux adds to `CallEngineTests`: the engine meeting `webrtcbin` half way — the ICE servers
//! as `webrtcbin`'s URIs (the TURN user and credential percent-encoded), an m-line index for every
//! far-side candidate and both halves on every one of ours, the few SDP lines the media reads —
//! and the four places Linux differs from the C# on purpose, each measured on .NET 10 with the C#
//! suite's own fakes (`fc_logic::calls`' documentation), plus one C# behaviour kept and pinned.

mod common;

use fc_core::protocol::{IceCandidate, IceServerDto, ServerFrame};
use fc_logic::calls::webrtcbin::{IceConfig, escape, ice_config};
use fc_logic::calls::{CallEngine, CallStage, MediaGrant, sdp};

use common::calls::{Rig, answer, end, ice, offer, refusal};

/// An offer as libwebrtc writes one: audio at mid 0, video at mid 1, bundled.
const OFFER: &str = "v=0\r\n\
o=- 4611731400430051336 2 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
a=group:BUNDLE 0 1\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111 63\r\n\
c=IN IP4 0.0.0.0\r\n\
a=mid:0\r\n\
a=sendrecv\r\n\
a=rtpmap:111 opus/48000/2\r\n\
a=rtpmap:63 red/48000/2\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96 102\r\n\
c=IN IP4 0.0.0.0\r\n\
a=mid:1\r\n\
a=sendrecv\r\n\
a=rtpmap:96 VP8/90000\r\n\
a=rtpmap:102 H264/90000\r\n";

/// An answer whose mids are names, as some stacks write them.
const ANSWER: &str = "v=0\n\
o=- 1 2 IN IP4 127.0.0.1\n\
s=-\n\
t=0 0\n\
m=audio 9 UDP/TLS/RTP/SAVPF 109\n\
a=mid:audio\n\
a=rtpmap:109 opus/48000/2\n\
m=video 9 UDP/TLS/RTP/SAVPF 120\n\
a=mid:video\n\
a=rtpmap:120 vp8/90000\n";

fn server(urls: &[&str], username: Option<&str>, credential: Option<&str>) -> IceServerDto {
    IceServerDto {
        urls: Some(urls.iter().map(|url| url.to_string()).collect()),
        username: username.map(str::to_string),
        credential: credential.map(str::to_string),
    }
}

// ---- the ICE servers, as webrtcbin takes them ------------------------------------------------

/// Coturn's REST credentials: the username is `<expiry>:<user_id>` and the credential base64 — a
/// colon in the user would end it, and `+`, `/` and `=` mean something in a URI. `libgstwebrtcnice`
/// splits the userinfo at the FIRST `:` and unescapes each half (measured on GStreamer 1.28.2).
#[test]
fn turn_credentials_are_percent_encoded_into_the_uri() {
    let config = ice_config(&[
        server(
            &[
                "stun:stun.l.google.com:19302",
                "stun:stun1.l.google.com:19302",
            ],
            None,
            None,
        ),
        server(
            &[
                "turn:turn.example.com:3478?transport=udp",
                "turns:turn.example.com:5349?transport=tcp",
            ],
            Some("1700000000:42"),
            Some("q+/x=="),
        ),
    ]);

    assert_eq!(
        config,
        IceConfig {
            stun_server: Some("stun://stun.l.google.com:19302".to_string()),
            turn_servers: vec![
                "turn://1700000000%3A42:q%2B%2Fx%3D%3D@turn.example.com:3478?transport=udp"
                    .to_string(),
                "turns://1700000000%3A42:q%2B%2Fx%3D%3D@turn.example.com:5349?transport=tcp"
                    .to_string(),
            ],
        }
    );
}

/// `webrtcbin` takes ONE STUN server: the first. A port left out is the scheme's default.
#[test]
fn the_first_stun_server_is_taken_and_a_missing_port_is_the_default() {
    let config = ice_config(&[
        server(&["STUN:a.example", "stun:b.example:19302"], None, None),
        server(
            &["turn:relay.example", "turns:relay.example"],
            Some("u"),
            Some("p"),
        ),
    ]);

    assert_eq!(config.stun_server.as_deref(), Some("stun://a.example:3478"));
    assert_eq!(
        config.turn_servers,
        [
            "turn://u:p@relay.example:3478",
            "turns://u:p@relay.example:5349"
        ]
    );
}

/// What `webrtcbin` cannot use is left out: STUN over TLS, other schemes, a URL with no host or a
/// port that is not one, and a relay with nothing to authenticate with.
#[test]
fn what_webrtcbin_cannot_use_is_left_out() {
    let config = ice_config(&[
        server(
            &["stuns:stun.example:5349", "https://stun.example", "stun:"],
            None,
            None,
        ),
        server(&["stun:stun.example:abc"], None, None),
        server(&["turn:relay.example:3478"], None, Some("p")),
        server(&["turn:relay.example:3478"], Some("u"), None),
        IceServerDto::default(),
    ]);

    assert_eq!(config, IceConfig::default());
}

/// A bracketed IPv6 literal keeps its brackets, port or no port.
#[test]
fn an_ipv6_literal_keeps_its_brackets() {
    let config = ice_config(&[
        server(&["stun:[2001:db8::1]:3478"], None, None),
        server(&["turn:[2001:db8::2]?transport=udp"], Some("u"), Some("p")),
    ]);

    assert_eq!(
        config.stun_server.as_deref(),
        Some("stun://[2001:db8::1]:3478")
    );
    assert_eq!(
        config.turn_servers,
        ["turn://u:p@[2001:db8::2]:3478?transport=udp"]
    );
}

/// Only RFC 3986's unreserved characters stay as they are; everything else is `%XX`, a byte at a
/// time.
#[test]
fn escaping_leaves_only_the_unreserved_characters() {
    assert_eq!(escape("aZ09-._~"), "aZ09-._~");
    assert_eq!(escape("a b:c@d/é%"), "a%20b%3Ac%40d%2F%C3%A9%25");
}

// ---- the SDP lines the engine and the media read ---------------------------------------------

#[test]
fn a_mid_and_an_mline_index_read_each_other() {
    assert_eq!(sdp::mline_of(OFFER, "0"), Some(0));
    assert_eq!(sdp::mline_of(OFFER, "1"), Some(1));
    assert_eq!(sdp::mline_of(OFFER, "2"), None);
    assert_eq!(sdp::mid_of(OFFER, 1).as_deref(), Some("1"));
    assert_eq!(sdp::mid_of(OFFER, 2), None);
    assert_eq!(sdp::mid_of(OFFER, -1), None);
    // Plain line feeds, and mids that are names.
    assert_eq!(sdp::mline_of(ANSWER, "video"), Some(1));
    assert_eq!(sdp::mid_of(ANSWER, 0).as_deref(), Some("audio"));
    assert_eq!(sdp::mline_of("v=answer", "0"), None);
}

/// Which codecs an offer carries, and under which payload numbers — how the media answers an
/// H.264-only offer, and Firefox's numbers rather than libwebrtc's.
#[test]
fn an_offer_says_which_codecs_it_carries_and_their_numbers() {
    assert_eq!(sdp::payload_type(OFFER, "audio", "OPUS"), Some(111));
    assert_eq!(sdp::payload_type(OFFER, "video", "VP8"), Some(96));
    assert_eq!(sdp::payload_type(OFFER, "video", "H264"), Some(102));
    assert_eq!(sdp::payload_type(OFFER, "audio", "VP8"), None);
    assert_eq!(sdp::payload_type(ANSWER, "audio", "opus"), Some(109));
    assert_eq!(sdp::payload_type(ANSWER, "video", "VP8"), Some(120));
    let h264_only = OFFER.replace("a=rtpmap:96 VP8/90000\r\n", "");
    assert_eq!(sdp::payload_type(&h264_only, "video", "VP8"), None);
    assert_eq!(sdp::payload_type(&h264_only, "video", "H264"), Some(102));
}

// ---- candidates ------------------------------------------------------------------------------

/// A far-side candidate that came with its mid alone gets its m-line index from the remote
/// description — held ones when it is set, later ones at once: `add-ice-candidate` takes the index.
#[test]
fn a_far_side_candidate_with_only_a_mid_gets_its_mline_index() {
    let rig = Rig::build();
    rig.place(42, 9, true);
    rig.hear(ice(
        "call-1",
        IceCandidate::new("candidate:held", Some("video"), None),
    ));

    rig.hear(answer("call-1", ANSWER));
    rig.hear(ice(
        "call-1",
        IceCandidate::new("candidate:later", Some("audio"), None),
    ));

    assert_eq!(
        rig.media.added(),
        [
            IceCandidate::new("candidate:held", Some("video"), Some(1)),
            IceCandidate::new("candidate:later", Some("audio"), Some(0)),
        ]
    );
}

/// A far-side candidate that names no m-line the remote description has — neither half, or a mid
/// it lacks — is not passed on; one with its index is passed as it came.
#[test]
fn a_far_side_candidate_naming_no_mline_is_not_passed_on() {
    let rig = Rig::build();
    rig.hear(ServerFrame::CallOffer {
        call_id: "in".to_string(),
        chat_id: 42,
        from_user_id: 9,
        sdp: OFFER.to_string(),
        video: true,
    });
    rig.hear(ice(
        "in",
        IceCandidate::new("candidate:neither", None, None),
    ));
    rig.hear(ice(
        "in",
        IceCandidate::new("candidate:stranger", Some("9"), None),
    ));
    rig.hear(ice(
        "in",
        IceCandidate::new("candidate:mid", Some("1"), None),
    ));
    rig.hear(ice(
        "in",
        IceCandidate::new("candidate:indexed", None, Some(0)),
    ));

    rig.answer();

    assert_eq!(
        rig.media.added(),
        [
            IceCandidate::new("candidate:mid", Some("1"), Some(1)),
            IceCandidate::new("candidate:indexed", None, Some(0)),
        ]
    );
}

/// `on-ice-candidate` gives the index alone: the candidate goes out with the mid of that m-line
/// from this device's own offer — the held ones and the later ones alike.
#[test]
fn our_candidate_with_only_an_index_goes_out_with_its_mid() {
    let rig = Rig::build();
    *rig.media.offer.borrow_mut() = Some(OFFER.to_string());
    *rig.media.gathered_halves.borrow_mut() = (None, Some(1));
    *rig.media.while_describing.borrow_mut() = Some("candidate:during".to_string());

    rig.place(42, 9, true);
    rig.gather("candidate:after");

    assert_eq!(rig.types(), ["call_offer", "call_ice", "call_ice"]);
    for n in [1, 2] {
        let candidate = &rig.sent_frame(n)["candidate"];
        assert_eq!(
            (
                candidate["sdp_mid"].as_str(),
                candidate["sdp_mline_index"].as_i64()
            ),
            (Some("1"), Some(1))
        );
    }
}

/// The callee's own candidates read their mid from its ANSWER; one that came with its mid alone
/// gets the index too.
#[test]
fn the_callees_candidates_read_its_answer() {
    let rig = Rig::build();
    rig.hear(offer("in", false));
    *rig.media.answer.borrow_mut() = Some(ANSWER.to_string());
    *rig.media.gathered_halves.borrow_mut() = (Some("video".to_string()), None);

    rig.answer();
    rig.gather("candidate:mine");

    assert_eq!(rig.types(), ["call_answer", "call_ice"]);
    let candidate = &rig.sent_frame(1)["candidate"];
    assert_eq!(
        (
            candidate["sdp_mid"].as_str(),
            candidate["sdp_mline_index"].as_i64()
        ),
        (Some("video"), Some(1))
    );
}

// ---- where Linux differs from the C# on purpose ----------------------------------------------

/// A placement cancelled while the microphone or the camera portal was being asked; an incoming
/// call rings by the time the answer comes. The C# closed the media under the ringing call (its
/// page's `close` stops the ring — measured: `[close]` with call "two" `Incoming`); here that call
/// keeps it.
#[test]
fn a_late_grant_leaves_the_call_now_ringing_its_media() {
    let rig = Rig::build();
    let grant = rig.media.ask();
    let placing = rig.start(rig.engine.place(42, 9, false));
    rig.end();
    rig.hear(offer("two", false));
    rig.media.log.borrow_mut().clear();

    grant
        .send(MediaGrant::Microphone)
        .expect("the question is open");
    rig.settle();

    assert!(placing.done());
    assert!(rig.media.log().is_empty());
    assert_eq!(
        (rig.call().call_id.as_str(), rig.call().stage),
        ("two", CallStage::Incoming)
    );
    assert_eq!(rig.types(), ["call_end"]);
}

/// A late `call_ice` for the call that just ended is not held for the next one: the C# held it and
/// put it into the NEXT placed call's connection when that one was answered (measured:
/// `[candidate:stale]` added to `call-2`).
#[test]
fn a_late_candidate_for_a_call_that_is_over_reaches_no_other() {
    let rig = Rig::build();
    rig.place(42, 9, false);
    rig.end();
    rig.hear(ice(
        "call-1",
        IceCandidate::new("candidate:stale", Some("0"), Some(0)),
    ));

    rig.place(42, 9, false);
    rig.hear(answer("call-2", "v=answer"));

    assert_eq!(rig.call().call_id, "call-2");
    assert!(rig.media.added().is_empty());
}

/// A call ended twice — `call_end`, then the `error` that refines it — takes one place in the
/// memory: the C# gave it two, and the oldest of sixteen ended calls was forgotten, ringing again
/// on a replayed offer (measured).
#[test]
fn a_call_ended_twice_is_remembered_once() {
    let rig = Rig::build();
    rig.hear(offer("a", false));
    rig.decline();
    rig.place(42, 9, false);
    rig.hear(end("call-1", "cancel"));
    rig.hear(refusal("peer_unreachable", "nobody", "call-1"));
    for index in 0..14 {
        rig.hear(offer(&format!("x{index}"), false));
        rig.decline();
    }
    rig.pass(CallEngine::ENDED_LINGER);

    // Sixteen calls ended, "a" the oldest: remembered.
    rig.hear(offer("a", false));

    assert!(rig.engine.call().is_none());
}

/// A media that reports its own `close` as a failure (the contract says it must not) finds the
/// call already over: the C# ended it again from inside its teardown — `call_end` said twice,
/// `hangup` then `failed` (measured with a failure reported once; reported every time, it recursed).
#[test]
fn a_close_reported_as_failed_does_not_end_the_call_again() {
    let rig = Rig::build();
    rig.place(42, 9, false);
    rig.hear(answer("call-1", "v=answer"));
    rig.up();
    rig.media.fails_on_close.set(true);

    rig.end();

    assert_eq!(rig.types(), ["call_offer", "call_end"]);
    assert_eq!(rig.last_reason().as_deref(), Some("hangup"));
    assert_eq!(rig.call().ended_reason.as_deref(), Some("hangup"));
}

// ---- kept as the C# has it -------------------------------------------------------------------

/// Declining a call the caller has already cancelled says `call_end` again, and the reason on
/// screen becomes the decline (measured on .NET 10 — kept: the server drops a `call_end` for a call
/// it no longer holds in silence, and a callee's "cancel" and "decline" read the same, "Call
/// ended").
#[test]
fn declining_a_call_already_over_says_call_end_again() {
    let rig = Rig::build();
    rig.hear(offer("in", false));
    rig.hear(end("in", "cancel"));

    rig.decline();

    assert_eq!(rig.types(), ["call_end"]);
    assert_eq!(rig.last_reason().as_deref(), Some("decline"));
    assert_eq!(rig.call().ended_reason.as_deref(), Some("decline"));
}
