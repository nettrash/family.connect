//! `PackSendingTests`: where the sticker button is offered, and what stands between one click in
//! the panel and the send (docs/protocol.md, "Sending one" and "Consenting to the assistant").
//!
//! The window only draws what these answer: the button's visibility is `pack_sending::offered` and
//! `pack_sending::offered_in_thread`, and the click goes through `pack_sending::gate_for` (the C#'s
//! `PackSending.For`) before anything is staged — so the rule is pinned here, where no window is
//! needed to run it.

use fc_logic::assistant_consent;
use fc_logic::pack::pack_sending::{self, Gate};

const PROCESSOR: &str = "Microsoft — Azure OpenAI";
const AGREED: &str = "2026-09-13T10:00:00Z";

/// THE BUTTON IS OFFERED IN EVERY CHAT A MESSAGE CAN BE SENT IN: the family chat, a one-to-one chat,
/// and the assistant's own chat — none of them is left out.
#[test]
fn the_button_is_offered_in_every_kind_of_chat() {
    for kind in ["family", "direct", "ai"] {
        assert!(pack_sending::offered(Some(kind), false, true), "{kind}");
    }
}

#[test]
fn the_button_is_not_offered_without_a_pack_a_chat_or_while_editing() {
    // A server that predates packs names no ceilings: no button, rather than a 404 on a click.
    assert!(!pack_sending::offered(Some("family"), false, false));
    assert!(!pack_sending::offered(Some("ai"), false, false));
    // No conversation open.
    assert!(!pack_sending::offered(None, false, true));
    // A sticker is its own message, and an edit is somebody else's.
    assert!(!pack_sending::offered(Some("family"), true, true));
    assert!(!pack_sending::offered(Some("ai"), true, true));
}

/// The THREAD's composer has the button wherever it can send — which needs a root to answer.
#[test]
fn the_thread_composer_offers_it_wherever_it_can_send() {
    assert!(pack_sending::offered_in_thread(true, true));
    assert!(!pack_sending::offered_in_thread(false, true));
    assert!(!pack_sending::offered_in_thread(true, false));
}

/// IN THE ASSISTANT'S CHAT A STICKER GOES THROUGH THE SAME QUESTION ANY MESSAGE THERE DOES — never
/// around it. Until this member has agreed the click ASKS; it sends only once the server's `/me`
/// carries the answer.
#[test]
fn in_the_assistants_chat_a_sticker_is_asked_about_like_any_message() {
    assert_eq!(
        Gate::Ask,
        pack_sending::gate_for(Some("ai"), true, Some(PROCESSOR), None)
    );
    assert_eq!(
        Gate::Ask,
        pack_sending::gate_for(Some("ai"), true, Some(PROCESSOR), Some("  "))
    );
    assert_eq!(
        Gate::Send,
        pack_sending::gate_for(Some("ai"), true, Some(PROCESSOR), Some(AGREED))
    );
}

/// The gate is the composer's own, word for word: what a message with no body is asked, a sticker
/// is.
#[test]
fn the_gate_is_the_one_an_empty_message_goes_through() {
    let cases: [(Option<&str>, bool, Option<&str>, Option<&str>); 9] = [
        (Some("ai"), true, Some(PROCESSOR), None),
        (Some("ai"), true, Some(PROCESSOR), Some(AGREED)),
        (Some("ai"), true, None, None),
        (Some("ai"), true, Some("  "), Some(AGREED)),
        (Some("ai"), false, None, None),
        (Some("family"), true, Some(PROCESSOR), None),
        (Some("family"), true, None, None),
        (Some("direct"), true, Some(PROCESSOR), None),
        (None, true, Some(PROCESSOR), None),
    ];
    for (kind, has_assistant, processor, agreed_at) in cases {
        let expected = if assistant_consent::is_required(kind, Some(""), processor, agreed_at) {
            Gate::Ask
        } else if assistant_consent::is_withheld_from_an_unnamed_assistant(
            kind,
            Some(""),
            has_assistant,
            processor,
        ) {
            Gate::Withheld
        } else {
            Gate::Send
        };

        assert_eq!(
            expected,
            pack_sending::gate_for(kind, has_assistant, processor, agreed_at),
            "{kind:?} {has_assistant} {processor:?} {agreed_at:?}"
        );
    }
}

/// An assistant whose owner this server will not name gets NOTHING, a sticker included: there is no
/// honest way to ask, so there is nothing to send.
#[test]
fn a_sticker_is_withheld_from_an_assistant_nobody_named() {
    assert_eq!(
        Gate::Withheld,
        pack_sending::gate_for(Some("ai"), true, None, None)
    );
    assert_eq!(
        Gate::Withheld,
        pack_sending::gate_for(Some("ai"), true, Some(" "), Some(AGREED))
    );
}

/// Between people nothing is asked: a sticker has no words, so in the family chat it cannot mention
/// the assistant, and a one-to-one chat never reaches the model at all. That holds in a thread too —
/// a thread's sticker is a reply in the same chat.
#[test]
fn between_people_a_sticker_is_simply_sent() {
    for kind in ["family", "direct"] {
        assert_eq!(
            Gate::Send,
            pack_sending::gate_for(Some(kind), true, Some(PROCESSOR), None)
        );
        assert_eq!(
            Gate::Send,
            pack_sending::gate_for(Some(kind), true, None, None)
        );
        assert_eq!(
            Gate::Send,
            pack_sending::gate_for(Some(kind), false, None, None)
        );
    }
}
