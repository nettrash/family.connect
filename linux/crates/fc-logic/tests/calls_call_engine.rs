//! `CallEngineTests`: one call's signalling and state — web `calls.rs`'s own tests, and the ones
//! its browser made unnecessary: the engine's media is another component here, and its guards run
//! on a clock a test can move.
//!
//! Every rig action runs what it set going before it returns (`common::calls`), as the C#'s
//! continuations ran inline — so the assertion after it sees the result.

mod common;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use fc_core::protocol::IceCandidate;
use fc_core::rt::Clock;
use fc_logic::calls::{CallEngine, CallStage, CallState, MediaGrant, call_text};

use common::calls::{
    Rig, answer, end, ice, offer, refusal, refused, ringing, say, theirs, unreachable,
};

fn later(candidate: &str) -> IceCandidate {
    IceCandidate {
        candidate: candidate.to_string(),
        ..theirs()
    }
}

/// The protocol's order: the microphone, then the servers, then the offer the moment it is
/// described.
#[test]
fn placing_a_call_asks_for_the_microphone_then_the_servers_then_offers() {
    let rig = Rig::build();

    rig.place(42, 9, false);

    let call = rig.call();
    assert_eq!(
        (
            call.call_id.as_str(),
            call.chat_id,
            call.peer_user_id,
            call.outgoing,
            call.stage
        ),
        ("call-1", 42, 9, true, CallStage::Dialling)
    );
    assert_eq!(rig.media.log(), ["open", "connect 1", "offer"]);
    assert_eq!(rig.sent().len(), 1);
    let offer = rig.sent_frame(0);
    assert_eq!(
        (
            offer["type"].as_str(),
            offer["call_id"].as_str(),
            offer["chat_id"].as_i64(),
            offer["sdp"].as_str()
        ),
        (
            Some("call_offer"),
            Some("call-1"),
            Some(42),
            Some("v=offer")
        )
    );
    assert!(offer.get("video").is_none());
    assert_eq!(rig.asked.borrow().len(), 1);
    assert_eq!(rig.clock.waiting(CallEngine::OUTGOING_RING_GUARD), 1);
    assert!(rig.engine.busy());
    assert_eq!(
        call_text::status_line(&call, rig.clock.now(), &say()),
        "Calling…"
    );
}

/// A candidate found while the offer was being described follows the offer: the server knows no
/// call before it.
#[test]
fn a_candidate_found_before_the_offer_follows_it() {
    let rig = Rig::build();
    *rig.media.while_describing.borrow_mut() = Some("candidate:1".to_string());

    rig.place(42, 9, false);
    rig.gather("candidate:2");

    assert_eq!(rig.types(), ["call_offer", "call_ice", "call_ice"]);
    assert_eq!(rig.sent_frame(1)["candidate"]["candidate"], "candidate:1");
    assert_eq!(rig.sent_frame(2)["candidate"]["candidate"], "candidate:2");
}

/// The server reaching them is when the caller hears the tone — once, and for this call only.
#[test]
fn the_caller_hears_the_tone_when_it_rings_there() {
    let rig = Rig::build();
    rig.place(42, 9, false);

    rig.hear(ringing("theirs"));
    assert_eq!(rig.call().stage, CallStage::Dialling);

    rig.hear(ringing("call-1"));
    rig.hear(ringing("call-1"));

    assert_eq!(rig.call().stage, CallStage::Ringing);
    assert_eq!(
        rig.media
            .log()
            .iter()
            .filter(|entry| *entry == "ring Ringback")
            .count(),
        1
    );
    assert_eq!(
        call_text::status_line(&rig.call(), rig.clock.now(), &say()),
        "Ringing…"
    );
}

/// An answer connects; candidates held for it go in; and the clock counts from the media coming
/// UP.
#[test]
fn an_answer_connects_and_the_clock_starts_when_the_media_does() {
    let rig = Rig::build();
    rig.place(42, 9, false);
    rig.hear(ice("call-1", theirs()));
    assert!(rig.media.added().is_empty());

    rig.hear(answer("call-1", "v=answer"));

    assert!(
        rig.media
            .log()
            .contains(&"remote answer v=answer".to_string())
    );
    assert_eq!(rig.media.added(), [theirs()]);
    let call = rig.call();
    assert_eq!(
        (call.stage, call.taken, call.answered_at),
        (CallStage::Connecting, true, None)
    );
    assert_eq!(rig.clock.waiting(CallEngine::ANSWER_GUARD), 1);
    assert_eq!(rig.clock.waiting(CallEngine::OUTGOING_RING_GUARD), 0);

    rig.hear(ice("call-1", later("candidate:later")));
    assert_eq!(rig.media.added().len(), 2);

    rig.clock.forward(Duration::from_secs(5));
    rig.up();
    let started = rig.clock.now();
    assert_eq!(
        (rig.call().stage, rig.call().answered_at),
        (CallStage::Talking, Some(started))
    );
    assert_eq!(
        call_text::status_line(&rig.call(), started + Duration::from_millis(12_900), &say()),
        "0:12"
    );
    // A call that came up is not failed by the guard for one that never did.
    rig.pass(CallEngine::ANSWER_GUARD);
    assert_eq!(rig.call().stage, CallStage::Talking);

    // A second "up" changes nothing, and an answer for a call already taken is not applied again.
    rig.clock.forward(Duration::from_secs(3));
    rig.up();
    rig.hear(answer("call-1", "v=again"));
    assert_eq!(rig.call().answered_at, Some(started));
    assert!(
        !rig.media
            .log()
            .contains(&"remote answer v=again".to_string())
    );
}

/// A call nobody answers gives up at its guard — saying nothing: the server ended it long before —
/// and then goes.
#[test]
fn a_call_nobody_answers_gives_up_and_goes() {
    let rig = Rig::build();
    rig.place(42, 9, false);

    rig.pass(CallEngine::OUTGOING_RING_GUARD);

    let call = rig.call();
    assert_eq!(
        (call.stage, call.ended_reason.as_deref()),
        (CallStage::Ended, Some("timeout"))
    );
    assert_eq!(rig.types(), ["call_offer"]);
    assert_eq!(
        call_text::status_line(&call, rig.clock.now(), &say()),
        "No answer"
    );
    assert!(rig.media.log().contains(&"close".to_string()));
    assert!(!rig.engine.busy());

    rig.pass(CallEngine::ENDED_LINGER);
    assert!(rig.engine.call().is_none());
}

/// An answered call that never comes up fails — and this one says so.
#[test]
fn an_answered_call_that_never_comes_up_fails() {
    let rig = Rig::build();
    rig.place(42, 9, false);
    rig.hear(answer("call-1", "v=answer"));

    rig.pass(CallEngine::ANSWER_GUARD);

    assert_eq!(rig.call().ended_reason.as_deref(), Some("failed"));
    assert_eq!(rig.last_reason().as_deref(), Some("failed"));
    assert_eq!(
        call_text::status_line(&rig.call(), rig.clock.now(), &say()),
        "Call failed"
    );
}

/// The media dying ends the call as failed, once.
#[test]
fn media_that_dies_ends_the_call_once() {
    let rig = Rig::build();
    rig.place(42, 9, false);
    rig.hear(answer("call-1", "v=answer"));
    rig.up();

    rig.die();
    rig.die();
    // A candidate the dying connection still reports belongs to no call.
    rig.gather("candidate:late");

    assert_eq!(
        (rig.call().stage, rig.call().ended_reason.as_deref()),
        (CallStage::Ended, Some("failed"))
    );
    assert_eq!(rig.types(), ["call_offer", "call_end"]);
}

/// The reason follows the stage — and a call already over is not ended twice.
#[test]
fn the_reason_follows_the_stage() {
    let rig = Rig::build();
    rig.hear(offer("in", false));
    rig.end();
    assert_eq!(
        (
            rig.call().ended_reason.as_deref(),
            rig.last_reason().as_deref()
        ),
        (Some("decline"), Some("decline"))
    );
    rig.end();
    assert_eq!(rig.sent().len(), 1);
    assert_eq!(rig.call().ended_reason.as_deref(), Some("decline"));

    let rig = Rig::build();
    rig.place(42, 9, false);
    rig.end();
    assert_eq!(
        (
            rig.call().ended_reason.as_deref(),
            rig.last_reason().as_deref()
        ),
        (Some("cancel"), Some("cancel"))
    );

    let rig = Rig::build();
    rig.place(42, 9, false);
    rig.hear(answer("call-1", "v=answer"));
    rig.up();
    rig.end();
    assert_eq!(
        (
            rig.call().ended_reason.as_deref(),
            rig.last_reason().as_deref()
        ),
        (Some("hangup"), Some("hangup"))
    );
    assert_eq!(
        call_text::status_line(&rig.call(), rig.clock.now(), &say()),
        "Call ended"
    );

    assert_eq!(
        call_text::end_reason(&CallState::new(
            "c",
            42,
            9,
            false,
            false,
            CallStage::Ringing
        )),
        "decline"
    );
}

/// Answering: the microphone, the servers, the offer in with what was held for it, and the answer
/// out.
#[test]
fn answering_takes_the_offer_and_answers_it() {
    let rig = Rig::build();
    rig.hear(offer("in", false));
    // An answer frame is the caller's to hear: a callee holding the call ignores one.
    rig.hear(answer("in", "v=stray"));
    assert!(
        !rig.media
            .log()
            .contains(&"remote answer v=stray".to_string())
    );
    assert_eq!(
        (
            rig.call().stage,
            rig.call().outgoing,
            rig.call().peer_user_id
        ),
        (CallStage::Incoming, false, 9)
    );
    assert!(rig.media.log().contains(&"ring Incoming".to_string()));
    assert_eq!(rig.clock.waiting(CallEngine::INCOMING_RING_GUARD), 1);
    assert_eq!(
        call_text::status_line(&rig.call(), rig.clock.now(), &say()),
        "Incoming call"
    );
    rig.hear(ice("in", theirs()));
    rig.hear(ice("other", theirs()));
    rig.media.log.borrow_mut().clear();

    rig.answer();

    assert_eq!(
        rig.media.log(),
        [
            "open",
            "connect 1",
            "remote offer v=theirs",
            "answer",
            "quiet"
        ]
    );
    assert_eq!(rig.media.added(), [theirs()]);
    assert_eq!(rig.sent().len(), 1);
    let answer = rig.sent_frame(0);
    assert_eq!(
        (
            answer["type"].as_str(),
            answer["call_id"].as_str(),
            answer["sdp"].as_str()
        ),
        (Some("call_answer"), Some("in"), Some("v=answer"))
    );
    assert_eq!(
        (rig.call().stage, rig.call().taken),
        (CallStage::Connecting, true)
    );
    assert_eq!(rig.clock.waiting(CallEngine::ANSWER_GUARD), 1);
    assert_eq!(
        call_text::status_line(&rig.call(), rig.clock.now(), &say()),
        "Connecting…"
    );
}

/// The media may already be up by the time the answer is in: the call is then talking at once.
#[test]
fn media_already_up_is_talking_at_once() {
    let rig = Rig::build();
    rig.hear(offer("in", false));
    rig.media.connected.set(true);

    rig.answer();

    assert_eq!(rig.call().stage, CallStage::Talking);
    assert_eq!(rig.call().answered_at, Some(rig.clock.now()));
}

/// A candidate found before this device's answer went follows it: the server relays a callee's
/// only once it has answered.
#[test]
fn the_answer_is_tried_again_and_its_candidates_follow_it() {
    let rig = Rig::build();
    rig.hear(offer("in", false));
    rig.socket.refusals.set(2);
    *rig.media.while_describing.borrow_mut() = Some("candidate:early".to_string());

    rig.answer();
    rig.gather("candidate:meanwhile");
    assert!(rig.sent().is_empty());

    rig.pass(CallEngine::SAY_EVERY);
    assert!(rig.sent().is_empty());
    rig.pass(CallEngine::SAY_EVERY);

    assert_eq!(rig.types(), ["call_answer", "call_ice", "call_ice"]);
    rig.gather("candidate:after");
    assert_eq!(rig.sent().len(), 4);
}

/// Without a microphone, an answer is a refusal on the wire — and the reason on screen is the one
/// that can be acted on.
#[test]
fn without_a_microphone_an_answer_is_a_refusal() {
    let rig = Rig::build();
    rig.media.grant.set(MediaGrant::None);
    rig.hear(offer("in", false));

    rig.answer();

    assert_eq!(
        rig.call().ended_reason.as_deref(),
        Some("microphone_denied")
    );
    assert_eq!(rig.last_reason().as_deref(), Some("decline"));
    assert!(rig.asked.borrow().is_empty());
    assert_eq!(
        call_text::status_line(&rig.call(), rig.clock.now(), &say()),
        "Microphone access is needed for calls."
    );
}

/// Placing without a microphone says so, fetches nothing and sends nothing: the server never hears
/// of it.
#[test]
fn placing_without_a_microphone_sends_nothing() {
    let rig = Rig::build();
    rig.media.grant.set(MediaGrant::None);

    rig.place(42, 9, false);

    assert_eq!(
        rig.call().ended_reason.as_deref(),
        Some("microphone_denied")
    );
    assert!(rig.sent().is_empty());
    assert!(rig.asked.borrow().is_empty());
}

/// A server that refuses calls ends the placing in its own words; it never heard of the call.
#[test]
fn a_server_without_calls_ends_the_placing() {
    let rig = Rig::build_with(refused("calls_disabled"));

    rig.place(42, 9, false);

    assert_eq!(rig.call().ended_reason.as_deref(), Some("calls_disabled"));
    assert!(rig.sent().is_empty());
    assert_eq!(
        call_text::status_line(&rig.call(), rig.clock.now(), &say()),
        "Calls are off on this server."
    );
}

/// An answer whose servers could not be reached goes on with none; one the server refuses is
/// declined in its words.
#[test]
fn an_answer_goes_on_without_servers_it_could_not_reach() {
    let rig = Rig::build_with(unreachable());
    rig.hear(offer("in", false));
    rig.answer();
    assert!(rig.media.log().contains(&"connect 0".to_string()));
    assert_eq!(rig.call().stage, CallStage::Connecting);

    let rig = Rig::build_with(refused("calls_disabled"));
    rig.hear(offer("in", false));
    rig.answer();
    assert_eq!(
        (
            rig.call().ended_reason.as_deref(),
            rig.last_reason().as_deref()
        ),
        (Some("calls_disabled"), Some("decline"))
    );
    assert!(
        !rig.media
            .log()
            .iter()
            .any(|entry| entry.starts_with("connect"))
    );
}

/// A video call without a camera still connects — receiving the far side's picture — and says its
/// camera is off.
#[test]
fn a_video_call_without_a_camera_still_connects() {
    let rig = Rig::build();

    rig.place(42, 9, true);

    assert_eq!(
        rig.media.log(),
        ["open video", "connect 1 receive-only", "offer"]
    );
    assert!(!rig.call().camera);
    assert_eq!(rig.sent_frame(0)["video"], true);

    let rig = Rig::build();
    rig.media.grant.set(MediaGrant::MicrophoneAndCamera);
    rig.hear(offer("in", true));
    assert_eq!(
        call_text::status_line(&rig.call(), rig.clock.now(), &say()),
        "Incoming video call"
    );
    rig.answer();
    assert!(rig.media.log().contains(&"connect 1".to_string()));
    assert!(rig.call().camera);
}

/// Not in the C#: a mute or the camera turned off while the microphone is being asked for reaches
/// the media once it is connected — `GstCallMedia` has no pipeline to apply it to before `open`
/// resolves. Only what differs from a fresh pipeline is said again.
#[test]
fn a_toggle_during_set_up_reaches_the_media_once_it_is_connected() {
    let rig = Rig::build();
    let grant = rig.media.ask();
    let placing = rig.start(rig.engine.place(42, 9, true));
    rig.engine.toggle_mute();
    rig.engine.toggle_camera();
    rig.media.log.borrow_mut().clear();
    grant
        .send(MediaGrant::MicrophoneAndCamera)
        .expect("the question is open");
    rig.settle();
    assert!(placing.done());
    assert_eq!(
        rig.media.log(),
        ["connect 1", "muted true", "camera false", "offer"]
    );
    assert_eq!((rig.call().muted, rig.call().camera), (true, false));

    // Answering: the same; a camera turned off and on again is not said at all, and neither is a
    // camera the grant did not give.
    let rig = Rig::build();
    rig.hear(offer("in", true));
    let grant = rig.media.ask();
    let answering = rig.start(rig.engine.answer());
    rig.engine.toggle_mute();
    rig.engine.toggle_camera();
    rig.engine.toggle_camera();
    rig.media.log.borrow_mut().clear();
    grant
        .send(MediaGrant::MicrophoneAndCamera)
        .expect("the question is open");
    rig.settle();
    assert!(answering.done());
    let log = rig.media.log();
    assert_eq!(log[..2], ["connect 1", "muted true"]);
    assert!(!log.iter().any(|line| line.starts_with("camera")));

    let rig = Rig::build();
    rig.hear(offer("in", true));
    let grant = rig.media.ask();
    let answering = rig.start(rig.engine.answer());
    rig.engine.toggle_camera();
    rig.media.log.borrow_mut().clear();
    grant
        .send(MediaGrant::Microphone)
        .expect("the question is open");
    rig.settle();
    assert!(answering.done());
    assert!(
        !rig.media
            .log()
            .iter()
            .any(|line| line.starts_with("camera"))
    );
    assert!(!rig.media.log().contains(&"muted true".to_string()));
}

/// Mute and the camera: the state and the tracks both; the camera belongs to video calls alone.
#[test]
fn mute_and_camera_toggle_their_tracks() {
    let rig = Rig::build();
    rig.media.grant.set(MediaGrant::MicrophoneAndCamera);
    rig.place(42, 9, true);
    assert!(rig.call().camera);

    rig.engine.toggle_mute();
    rig.engine.toggle_camera();
    assert_eq!((rig.call().muted, rig.call().camera), (true, false));
    assert!(rig.media.log().contains(&"muted true".to_string()));
    assert!(rig.media.log().contains(&"camera false".to_string()));
    rig.engine.toggle_mute();
    assert!(!rig.call().muted);

    let rig = Rig::build();
    rig.place(42, 9, false);
    rig.engine.toggle_camera();
    assert!(!rig.call().camera);
    assert!(
        !rig.media
            .log()
            .iter()
            .any(|entry| entry.starts_with("camera"))
    );
}

/// THE SET-UP GUARD: a call cancelled while the microphone is being asked for is not placed, and
/// its microphone is given back.
#[test]
fn a_call_cancelled_during_set_up_is_not_placed() {
    let rig = Rig::build();
    let grant = rig.media.ask();
    let placing = rig.start(rig.engine.place(42, 9, false));

    rig.end();
    rig.media.log.borrow_mut().clear();
    grant
        .send(MediaGrant::Microphone)
        .expect("the question is open");
    rig.settle();
    assert!(placing.done());

    assert_eq!(rig.media.log(), ["close"]);
    assert!(!rig.types().contains(&"call_offer".to_string()));
    assert!(rig.asked.borrow().is_empty());
    assert_eq!(rig.call().ended_reason.as_deref(), Some("cancel"));
}

/// Every connection the callee has gets the offer: a second copy of the call held is the duplicate
/// it is.
#[test]
fn a_second_copy_of_an_offer_starts_nothing() {
    let rig = Rig::build();
    rig.hear(offer("in", false));
    rig.answer();

    rig.hear(offer("in", false));

    assert_eq!(rig.call().stage, CallStage::Connecting);
    assert_eq!(
        rig.media
            .log()
            .iter()
            .filter(|entry| *entry == "ring Incoming")
            .count(),
        1
    );
}

/// An offer that crossed its own end does not ring again; another call does, and takes an ended
/// one's place at once.
#[test]
fn an_ended_call_stays_ended_and_a_new_one_takes_its_place() {
    let rig = Rig::build();
    rig.hear(offer("one", false));
    rig.decline();
    assert_eq!(rig.call().stage, CallStage::Ended);

    rig.hear(offer("two", false));
    assert_eq!(
        (rig.call().call_id.as_str(), rig.call().stage),
        ("two", CallStage::Incoming)
    );
    rig.decline();
    rig.pass(CallEngine::ENDED_LINGER);
    assert!(rig.engine.call().is_none());

    rig.hear(offer("one", false));
    assert!(rig.engine.call().is_none());
    rig.hear(offer("three", false));
    assert_eq!(rig.call().call_id, "three");
}

/// Only the newest calls are remembered as over: past the memory, an old id rings like any other.
#[test]
fn only_the_newest_ended_calls_are_remembered() {
    let rig = Rig::build();
    for index in 0..=CallEngine::REMEMBERED {
        rig.hear(offer(&format!("call {index}"), false));
        rig.decline();
    }
    rig.pass(CallEngine::ENDED_LINGER);

    rig.hear(offer(&format!("call {}", CallEngine::REMEMBERED), false));
    assert!(rig.engine.call().is_none());
    rig.hear(offer("call 1", false));
    assert!(rig.engine.call().is_none());
    rig.hear(offer("call 0", false));
    assert_eq!(rig.call().call_id, "call 0");
}

/// A frame naming a call this device does not hold is ignored in silence.
#[test]
fn a_frame_for_another_call_is_ignored() {
    let rig = Rig::build();
    rig.place(42, 9, false);
    rig.hear(ringing("call-1"));

    rig.hear(answer("theirs", "v=0"));
    rig.hear(end("theirs", "hangup"));
    rig.hear(refusal("peer_busy", "busy", "theirs"));
    rig.hear(ice("theirs", theirs()));
    rig.hear(answer("call-1", "v=answer"));

    assert_eq!(
        (rig.call().call_id.as_str(), rig.call().stage),
        ("call-1", CallStage::Connecting)
    );
    assert!(rig.media.added().is_empty());
}

/// The server ends a call it cannot deliver BEFORE it answers the error, which then says better
/// what happened.
#[test]
fn an_error_refines_the_call_it_ends() {
    let rig = Rig::build();
    rig.place(42, 9, false);

    rig.hear(end("call-1", "cancel"));
    assert_eq!(rig.call().ended_reason.as_deref(), Some("cancel"));
    rig.hear(refusal("peer_unreachable", "nobody", "call-1"));

    assert_eq!(
        (rig.call().stage, rig.call().ended_reason.as_deref()),
        (CallStage::Ended, Some("unreachable"))
    );
    assert_eq!(
        call_text::status_line(&rig.call(), rig.clock.now(), &say()),
        "Unavailable"
    );
    assert_eq!(rig.types(), ["call_offer"]);
}

/// The buffer of candidates held for the remote description is capped where the server caps its
/// own.
#[test]
fn the_candidate_buffer_is_capped() {
    let rig = Rig::build();
    rig.hear(offer("in", false));
    for index in 0..80 {
        rig.hear(ice("in", later(&format!("candidate:{index}"))));
    }

    rig.answer();

    assert_eq!(rig.media.added().len(), CallEngine::HELD_CANDIDATES);
    assert_eq!(rig.media.added()[0].candidate, "candidate:0");
}

/// A callee left ringing gives up at its own guard, and the missed call is said the way the record
/// says it.
#[test]
fn an_unanswered_ring_gives_up_as_missed() {
    let rig = Rig::build();
    rig.hear(offer("in", true));

    rig.pass(CallEngine::INCOMING_RING_GUARD);

    assert_eq!(rig.call().ended_reason.as_deref(), Some("timeout"));
    assert!(rig.sent().is_empty());
    assert_eq!(
        call_text::status_line(&rig.call(), rig.clock.now(), &say()),
        "Missed video call"
    );
}

/// The app going says the reason its stage means, once, and gives the media back; a call already
/// over says nothing.
#[test]
fn leaving_says_what_the_stage_means() {
    let rig = Rig::build();
    rig.place(42, 9, false);
    rig.leave();
    assert_eq!(rig.last_reason().as_deref(), Some("cancel"));
    assert!(rig.media.log().contains(&"close".to_string()));

    let rig = Rig::build();
    rig.hear(offer("in", false));
    rig.leave();
    assert_eq!(rig.last_reason().as_deref(), Some("decline"));

    let rig = Rig::build();
    rig.hear(offer("in", false));
    rig.answer();
    rig.leave();
    assert_eq!(rig.last_reason().as_deref(), Some("hangup"));

    let rig = Rig::build();
    rig.hear(offer("in", false));
    rig.decline();
    rig.leave();
    assert_eq!(rig.sent().len(), 1);
}

/// Every change is announced, so a view redraws — a stream appearing included.
#[test]
fn every_change_is_announced() {
    let rig = Rig::build();
    let changes = Rc::new(Cell::new(0));
    let counted = changes.clone();
    rig.engine
        .on_changed()
        .subscribe(move |_| counted.set(counted.get() + 1));

    rig.place(42, 9, false);
    let placed = changes.get();
    rig.tracks();

    assert!(placed >= 1);
    assert_eq!(changes.get(), placed + 1);
}

/// The apps' numbers: guards past the server's 45 s, two seconds on screen, ten tries, sixteen
/// remembered, sixty-four held.
#[test]
fn the_guards_and_buffers_are_the_apps() {
    assert_eq!(CallEngine::OUTGOING_RING_GUARD, Duration::from_secs(90));
    assert_eq!(CallEngine::INCOMING_RING_GUARD, Duration::from_secs(60));
    assert_eq!(CallEngine::ANSWER_GUARD, Duration::from_secs(30));
    assert_eq!(CallEngine::ENDED_LINGER, Duration::from_secs(2));
    assert_eq!(CallEngine::SAY_EVERY, Duration::from_millis(500));
    assert_eq!(
        (
            CallEngine::SAY_ATTEMPTS,
            CallEngine::REMEMBERED,
            CallEngine::HELD_CANDIDATES
        ),
        (10, 16, 64)
    );
}

/// A call can be placed while the last one still says why it ended — and not while one is on.
#[test]
fn a_placed_call_takes_an_ended_ones_place_but_not_a_live_ones() {
    let rig = Rig::build();
    rig.hear(offer("in", false));
    rig.place(42, 9, false);
    assert_eq!(rig.call().call_id, "in");
    assert!(rig.sent().is_empty());

    rig.decline();
    rig.place(42, 9, false);

    assert_eq!(
        (rig.call().call_id.as_str(), rig.call().stage),
        ("call-1", CallStage::Dialling)
    );
    assert_eq!(rig.types(), ["call_end", "call_offer"]);
}

/// An offer the socket would not take is a call that visibly did not happen: failed, and not tried
/// again.
#[test]
fn an_offer_the_socket_would_not_take_fails() {
    let rig = Rig::build();
    rig.socket.refusals.set(1);

    rig.place(42, 9, false);

    assert_eq!(rig.call().ended_reason.as_deref(), Some("failed"));
    assert!(rig.sent().is_empty());
    assert_eq!(rig.clock.waiting(CallEngine::SAY_EVERY), 0);
}

/// An answer the media cannot take fails the call, and says so.
#[test]
fn an_answer_the_media_cannot_take_fails_the_call() {
    let rig = Rig::build();
    rig.place(42, 9, false);
    rig.media.accepts_remote.set(false);

    rig.hear(answer("call-1", "v=broken"));

    assert_eq!(
        (
            rig.call().ended_reason.as_deref(),
            rig.last_reason().as_deref()
        ),
        (Some("failed"), Some("failed"))
    );
}

/// A new call starts with none of the last one's candidates, held or waiting to follow an offer.
#[test]
fn a_new_call_starts_with_none_of_the_last_ones_candidates() {
    let rig = Rig::build();
    rig.hear(offer("one", false));
    rig.hear(ice("one", theirs()));
    rig.decline();
    rig.hear(offer("two", false));
    rig.answer();
    assert!(rig.media.added().is_empty());

    let rig = Rig::build();
    rig.place(42, 9, false);
    rig.end();
    // Reported by the connection on its way out: it belongs to no call, and must not follow the
    // next one's offer.
    rig.gather("candidate:stale");
    rig.socket.sent.borrow_mut().clear();
    *rig.media.while_describing.borrow_mut() = Some("candidate:new".to_string());
    rig.place(42, 9, false);

    assert_eq!(rig.types(), ["call_offer", "call_ice"]);
    assert_eq!(rig.sent_frame(1)["candidate"]["candidate"], "candidate:new");
}

/// An engine detached from the media hears none of it: the window's media is the next server's
/// engine's now.
#[test]
fn a_detached_engine_hears_nothing_from_the_media() {
    let rig = Rig::build();
    rig.place(42, 9, false);
    rig.hear(answer("call-1", "v=answer"));
    let changes = Rc::new(Cell::new(0));
    let counted = changes.clone();
    rig.engine
        .on_changed()
        .subscribe(move |_| counted.set(counted.get() + 1));

    rig.engine.detach();
    rig.gather("candidate:after");
    rig.up();
    rig.tracks();
    rig.die();

    assert_eq!(rig.call().stage, CallStage::Connecting);
    assert_eq!(changes.get(), 0);
    assert_eq!(rig.types(), ["call_offer"]);
}

/// What each refusal is called, and how each reason is said on either side of the call.
#[test]
fn a_refusal_is_named_and_said() {
    assert_eq!(call_text::refusal_reason("peer_busy"), "busy");
    assert_eq!(call_text::refusal_reason("call_busy"), "busy");
    assert_eq!(call_text::refusal_reason("peer_unreachable"), "unreachable");
    assert_eq!(call_text::refusal_reason("blocked"), "blocked");
    assert_eq!(
        call_text::refusal_reason("calls_disabled"),
        "calls_disabled"
    );
    assert_eq!(
        call_text::refusal_reason("video_calls_disabled"),
        "video_calls_disabled"
    );
    assert_eq!(call_text::refusal_reason("invalid_call"), "failed");

    let say = say();
    let call = CallState::new("c", 42, 9, true, false, CallStage::Ended);
    assert_eq!(call_text::ended_line("timeout", &call, &say), "No answer");
    assert_eq!(call_text::ended_line("decline", &call, &say), "Declined");
    assert_eq!(call_text::ended_line("hangup", &call, &say), "Call ended");
    assert_eq!(
        call_text::ended_line("answered_elsewhere", &call, &say),
        "Answered on another device"
    );
    assert_eq!(call_text::ended_line("busy", &call, &say), "Busy");
    assert_eq!(
        call_text::ended_line("unavailable", &call, &say),
        "Unavailable"
    );
    assert_eq!(
        call_text::ended_line("blocked", &call, &say),
        "You've blocked them."
    );
    assert_eq!(
        call_text::ended_line("video_calls_disabled", &call, &say),
        "Video calls are off on this server."
    );

    let callee = CallState {
        outgoing: false,
        ..call.clone()
    };
    assert_eq!(
        call_text::ended_line("timeout", &callee, &say),
        "Missed voice call"
    );
    assert_eq!(
        call_text::ended_line("decline", &callee, &say),
        "Call ended"
    );

    assert_eq!(
        call_text::unload_reason(&CallState {
            taken: true,
            ..callee.clone()
        }),
        "hangup"
    );
    assert_eq!(call_text::unload_reason(&callee), "decline");
    assert_eq!(call_text::unload_reason(&call), "cancel");
    assert_eq!(
        call_text::status_line(
            &CallState {
                stage: CallStage::Talking,
                ..call
            },
            time::OffsetDateTime::UNIX_EPOCH,
            &say
        ),
        "0:00"
    );
}
