//! TRANSIENT OR TERMINAL — every failure is one or the other, and the whole send pipeline hangs
//! on telling them apart (docs/protocol.md, "Error shape"). A port of
//! `win/tests/FamilyConnect.Core.Tests/Protocol/ApiErrorTests.cs`, test for test.

use std::collections::HashSet;

use fc_core::protocol::{ApiError, ErrorEnvelope, error_codes, wire};

fn error(code: &str, status: u16) -> ApiError {
    ApiError::new(code, "…", status)
}

#[test]
fn a_transient_failure_is_not_a_refusal() {
    // Nothing was read, so nothing was refused.
    assert!(ApiError::transport("connection reset").transient());
    assert!(error(error_codes::INTERNAL, 500).transient());
    assert!(error(error_codes::TOO_MANY_REQUESTS, 429).transient());
    assert!(error(error_codes::VALIDATION, 408).transient());
    assert!(error("", 502).transient());
    assert!(error("", 503).transient());
    // nginx answers its own rate limit with an HTML body, so a 429 may arrive with no code at
    // all — status alone has to be enough to classify it.
    assert!(ApiError::new("", "rate limited", 429).transient());
}

#[test]
fn a_terminal_failure_is_the_server_having_read_the_request_and_refused_it() {
    assert!(!error(error_codes::MESSAGE_TOO_LONG, 400).transient());
    assert!(!error(error_codes::NOT_CHAT_MEMBER, 403).transient());
    assert!(!error(error_codes::NOTE_NOT_FOUND, 404).transient());
    assert!(!error(error_codes::BOARD_FULL, 409).transient());
    assert!(!error(error_codes::INVALID_TASK, 422).transient());
}

/// `invalid_credentials` shares the 401 and not the meaning. A client that signs somebody out
/// for a mistyped current password has read the status and not the answer.
#[test]
fn a_wrong_password_is_not_an_expired_session() {
    assert!(error(error_codes::UNAUTHORIZED, 401).session_gone());
    assert!(!error(error_codes::INVALID_CREDENTIALS, 401).session_gone());
    assert!(!error(error_codes::VALIDATION, 400).session_gone());
    // And neither is a transport failure, whatever it looks like to the user.
    assert!(!ApiError::transport("offline").session_gone());
}

/// A refusal can arrive with NO STATUS AT ALL: a socket `error` frame carries a code and nothing
/// else. Reading the missing status as "it never got there" would retry a message the server has
/// already refused, and show the failure six delays late.
#[test]
fn a_socket_refusal_has_no_status_and_is_still_a_refusal() {
    assert!(!ApiError::coded(error_codes::MESSAGE_TOO_LONG, "…").transient());
    assert!(!ApiError::coded(error_codes::NOT_CHAT_MEMBER, "…").transient());
    assert!(!ApiError::coded(error_codes::PEER_BUSY, "…").transient());
    // Except the two codes that ARE transient wherever they come from.
    assert!(ApiError::coded(error_codes::INTERNAL, "…").transient());
    assert!(ApiError::coded(error_codes::TOO_MANY_REQUESTS, "…").transient());
    // And a code nobody knows, with no status, is the transport failure it looks like.
    assert!(ApiError::coded("", "…").transient());
    assert!(ApiError::coded("gremlins", "…").transient());
    // A NEWER server's code, arriving with a 4xx, is terminal — the status decides when the code
    // is not one we know.
    assert!(!error("family_hibernating", 409).transient());
    assert!(error("family_hibernating", 503).transient());
}

#[test]
fn the_error_body_decodes_as_the_protocol_writes_it() {
    let envelope: Option<ErrorEnvelope> = wire::decode(
        r#"{"error": {"code": "username_taken", "message": "username is already in use"}}"#,
    );
    let envelope = envelope.expect("the error shape");
    assert_eq!(envelope.error.code, error_codes::USERNAME_TAKEN);
    assert_eq!(envelope.error.message, "username is already in use");
    // A body that is not the shape at all does not throw — the status still classifies it.
    assert_eq!(
        wire::decode::<ErrorEnvelope>("<html>429 Too Many Requests</html>"),
        None
    );
}

/// The canonical list is kept WHOLE, retired codes included: a code that vanishes from the
/// document is a code somebody deletes from a client that is still talking to an old server.
#[test]
fn the_canonical_codes_are_all_here_and_all_distinct() {
    assert_eq!(error_codes::ALL.len(), 85);
    // Several families' six (docs/protocol.md, "Several families", 2026-10-22), every one a
    // refusal.
    for (code, wire, status) in [
        (error_codes::FAMILY_NOT_FOUND, "family_not_found", 404),
        (error_codes::TOO_MANY_FAMILIES, "too_many_families", 409),
        (
            error_codes::ALREADY_OWN_A_FAMILY,
            "already_own_a_family",
            409,
        ),
        (
            error_codes::MEMBER_FAMILIES_DISABLED,
            "member_families_disabled",
            403,
        ),
        (
            error_codes::MEMBER_IN_OTHER_FAMILIES,
            "member_in_other_families",
            403,
        ),
        (
            error_codes::INVITATION_NOT_PENDING,
            "invitation_not_pending",
            409,
        ),
    ] {
        assert_eq!(code, wire);
        assert!(error_codes::ALL.contains(&code));
        assert!(!ApiError::new(code, "x", status).transient());
    }
    // Paid families' six (docs/protocol.md, "Paid families", 2026-10-30), every one a refusal —
    // `storage_allowance_used` above all: an upload refused with it must FAIL, never be retried.
    for (code, wire, status) in [
        (
            error_codes::SUBSCRIPTION_REQUIRED,
            "subscription_required",
            403,
        ),
        (
            error_codes::SUBSCRIPTION_INVALID,
            "subscription_invalid",
            400,
        ),
        (error_codes::SUBSCRIPTION_TAKEN, "subscription_taken", 409),
        (
            error_codes::SUBSCRIPTIONS_DISABLED,
            "subscriptions_disabled",
            403,
        ),
        (
            error_codes::STORAGE_ALLOWANCE_USED,
            "storage_allowance_used",
            403,
        ),
        (error_codes::AI_ALLOWANCE_USED, "ai_allowance_used", 403),
    ] {
        assert_eq!(code, wire);
        assert!(error_codes::ALL.contains(&code));
        assert!(!ApiError::new(code, "x", status).transient());
        assert!(!ApiError::coded(code, "x").transient());
    }
    // Groups' two (docs/protocol.md, "Groups", 2026-10-22): refusals, with their status or
    // without it.
    for (code, status) in [
        (error_codes::NOT_GROUP_ADMIN, 403),
        (error_codes::TOO_MANY_GROUPS, 409),
    ] {
        assert!(error_codes::ALL.contains(&code));
        assert!(!ApiError::new(code, "x", status).transient());
        assert!(!ApiError::coded(code, "x").transient());
    }
    assert_eq!(error_codes::NOT_GROUP_ADMIN, "not_group_admin");
    assert_eq!(error_codes::TOO_MANY_GROUPS, "too_many_groups");
    // The sticker pack's four, as the document lists them.
    assert!(error_codes::ALL.contains(&error_codes::PACK_FULL));
    assert!(error_codes::ALL.contains(&error_codes::PACK_ITEM_TOO_LARGE));
    assert!(error_codes::ALL.contains(&error_codes::PACK_ITEM_NOT_FOUND));
    assert!(error_codes::ALL.contains(&error_codes::NOT_PACK_ITEM_AUTHOR));
    let distinct: HashSet<&str> = error_codes::ALL.iter().copied().collect();
    assert_eq!(distinct.len(), error_codes::ALL.len());
    assert!(error_codes::ALL.contains(&error_codes::OWNER_CANNOT_LEAVE));
    assert!(error_codes::ALL.contains(&error_codes::PICTURES_UNAVAILABLE));
    assert!(error_codes::ALL.contains(&error_codes::PICTURE_REFUSED));
    assert!(error_codes::ALL.contains(&error_codes::INVALID_TASK));
    // Missing from the document's list until 2026-10-02, and listed there now — so listed here
    // too. Terminal with or without its status: a socket frame carrying it has none, and it is
    // still a refusal.
    assert!(error_codes::ALL.contains(&error_codes::ASSISTANT_CONSENT_REQUIRED));
    assert!(!ApiError::new(error_codes::ASSISTANT_CONSENT_REQUIRED, "x", 403).transient());
    assert!(!ApiError::coded(error_codes::ASSISTANT_CONSENT_REQUIRED, "x").transient());
    // The transcript endpoint's four (docs/protocol.md, "Transcripts on request"), all refusals.
    for code in [
        error_codes::TRANSCRIPTS_UNAVAILABLE,
        error_codes::TRANSCRIPT_NOT_ALLOWED,
        error_codes::NOT_TRANSCRIBABLE,
        error_codes::TRANSCRIPT_REFUSED,
    ] {
        assert!(error_codes::ALL.contains(&code));
        assert!(!ApiError::new(code, "x", 400).transient());
    }
    // This client's own name for "it never got there" is NOT one of the protocol's.
    assert!(!error_codes::ALL.contains(&error_codes::TRANSPORT));
    assert!(error_codes::ALL.iter().all(|code| !code.starts_with('_')));
}
