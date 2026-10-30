//! The realtime frames, read and written as the protocol writes them (docs/protocol.md,
//! "WebSocket protocol"). The JSON in these tests is copied from that document. A port of
//! `win/tests/FamilyConnect.Core.Tests/Protocol/FrameTests.cs`, test for test.

use fc_core::protocol::client_frames::{self, SendOptions};
use fc_core::protocol::{
    AiErrorReason, AttachmentDto, ChatDto, IceCandidate, MentionDto, MessageDto, ServerFrame,
    error_codes, wire,
};
use serde_json::Value;

fn parse(json: &str) -> Option<ServerFrame> {
    ServerFrame::parse(json)
}

fn message_of(frame: Option<ServerFrame>) -> MessageDto {
    match frame {
        Some(ServerFrame::Message(message)) => message,
        other => panic!("expected a message frame, got {other:?}"),
    }
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).expect("the frame is JSON")
}

#[test]
fn a_message_frame_carries_the_whole_message() {
    let message = message_of(parse(
        r#"
        {"type": "message", "message": {"id": 1338, "chat_id": 42, "sender_id": 7,
         "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
         "body": "Dinner at 7?", "created_at": "2026-09-11T18:00:00Z"}}
        "#,
    ));
    assert_eq!(message.id, 1338);
    assert_eq!(message.chat_id, 42);
    assert_eq!(message.body, "Dinner at 7?");
    assert!(message.media().is_empty());
    // Absent is not empty: a message with no reactions has no field at all, which is how a
    // client tells "no data" from "cleared".
    assert_eq!(message.reactions, None);
}

#[test]
fn an_ack_is_this_devices_own_send_coming_back() {
    let frame = parse(
        r#"
        {"type": "ack", "client_msg_id": "8f14e45f-…",
         "message": {"id": 1338, "chat_id": 42, "sender_id": 7, "client_msg_id": "8f14e45f-…",
                     "body": "Six works", "created_at": "2026-09-11T18:01:00Z"}}
        "#,
    );
    let Some(ServerFrame::Ack {
        client_msg_id,
        value,
    }) = frame
    else {
        panic!("expected an ack, got {frame:?}");
    };
    assert_eq!(client_msg_id, "8f14e45f-…");
    assert_eq!(value.id, 1338);
}

/// An edit is a SEPARATE type on purpose: `message` bumps unread counts and raises a
/// notification, and an edit must do neither.
#[test]
fn an_edit_is_not_a_message() {
    let frame = parse(
        r#"
        {"type": "message_edited",
         "message": {"id": 1338, "chat_id": 42, "sender_id": 7, "client_msg_id": null,
                     "body": "Dinner at 8?", "created_at": "2026-09-11T18:00:00Z",
                     "edited_at": "2026-09-11T18:05:00Z", "edit_seq": 88}}
        "#,
    );
    let Some(ServerFrame::MessageEdited(edited)) = &frame else {
        panic!("expected an edit, got {frame:?}");
    };
    assert_eq!(edited.edit_seq, Some(88));
    assert!(!matches!(frame, Some(ServerFrame::Message(_))));
}

#[test]
fn reactions_are_complete_state_and_never_a_delta() {
    let frame = parse(
        r#"
        {"type": "reaction", "chat_id": 42, "message_id": 1338, "reaction_seq": 124,
         "reactions": [{"user_id": 9, "emoji": "❤️"}]}
        "#,
    );
    let Some(ServerFrame::Reactions {
        reaction_seq,
        value,
        ..
    }) = frame
    else {
        panic!("expected reactions, got {frame:?}");
    };
    assert_eq!(reaction_seq, 124);
    assert_eq!(value.len(), 1);
    assert_eq!(value[0].user_id, 9);
    // Cleared: the field stays, empty. A parser that dropped the key would leave the old hearts
    // on screen for good.
    let cleared = parse(
        r#"
        {"type": "reaction", "chat_id": 42, "message_id": 1338, "reaction_seq": 125,
         "reactions": []}
        "#,
    );
    let Some(ServerFrame::Reactions { value, .. }) = cleared else {
        panic!("expected reactions, got {cleared:?}");
    };
    assert!(value.is_empty());
}

#[test]
fn a_board_note_arrives_whole_with_its_kinds_extra_fields() {
    let frame = parse(
        r#"
        {"type": "board_note", "note": {"id": 12, "author_id": 7, "kind": "tasks",
         "text": "Saturday", "color": "green", "size": "medium", "font": "plain",
         "x": 0.42, "y": 0.13, "created_at": "…", "updated_at": "…",
         "board_seq": 88, "content_seq": 84,
         "items": [{"id": 11, "text": "Milk", "done": true, "done_by": 9},
                   {"id": 12, "text": "Bread", "done": false}]}}
        "#,
    );
    let Some(ServerFrame::BoardNote(note)) = frame else {
        panic!("expected a note, got {frame:?}");
    };
    assert_eq!(note.kind.as_deref(), Some("tasks"));
    assert_eq!(note.content_seq, Some(84));
    assert_eq!(note.task_list().len(), 2);
    assert_eq!(note.task_list()[0].done_by, Some(9));
    // `done_by` is absent while a line is not done — not zero.
    assert_eq!(note.task_list()[1].done_by, None);
}

#[test]
fn an_event_note_counts_who_is_coming_and_knows_this_readers_answer() {
    let frame = parse(
        r#"
        {"type": "board_note", "note": {"id": 13, "author_id": 7, "kind": "event",
         "text": "Gran's birthday", "color": "blue", "size": "medium", "font": "plain",
         "x": 0.1, "y": 0.2, "board_seq": 90,
         "starts_at": "2026-12-24T17:00:00Z", "place": "Gran's",
         "rsvps": [{"user_id": 7, "answer": "going"}, {"user_id": 9, "answer": "going"},
                   {"user_id": 11, "answer": "maybe"}]}}
        "#,
    );
    let Some(ServerFrame::BoardNote(note)) = frame else {
        panic!("expected a note, got {frame:?}");
    };
    assert_eq!(note.count("going"), 2);
    assert_eq!(note.count("maybe"), 1);
    assert_eq!(note.count("no"), 0);
    assert_eq!(note.my_answer(11), Some("maybe"));
    assert_eq!(note.my_answer(99), None);
    // A list's `[]` is meaningful; an event that is not a list has no items at all.
    assert!(note.task_list().is_empty());
}

#[test]
fn a_photo_note_carries_its_picture_and_its_shape() {
    let frame = parse(
        r#"
        {"type": "board_note", "note": {"id": 14, "author_id": 7, "kind": "photo",
         "text": "", "color": "yellow", "size": "medium", "font": "plain",
         "x": 0.3, "y": 0.3, "board_seq": 91,
         "attachment": {"id": 34, "kind": "photo", "mime": "image/jpeg", "size": 182734,
                        "width": 600, "height": 1200, "has_preview": true}}}
        "#,
    );
    let Some(ServerFrame::BoardNote(note)) = frame else {
        panic!("expected a note, got {frame:?}");
    };
    let attachment = note
        .attachment
        .as_ref()
        .expect("a photo note has its picture");
    assert!(attachment.is_photo());
    assert_eq!(attachment.aspect_ratio(), Some(0.5));
    // A caption may be empty on a photo note — that is the bare picture.
    assert_eq!(note.text.as_deref(), Some(""));
}

#[test]
fn a_message_with_several_attachments_keeps_the_senders_order() {
    let message = message_of(parse(
        r#"
        {"type": "message", "message": {"id": 1340, "chat_id": 42, "sender_id": 7,
         "client_msg_id": "9d3f1e77-…", "body": "", "created_at": "…",
         "attachments": [{"id": 34, "kind": "photo"}, {"id": 35, "kind": "video"}],
         "attachment": {"id": 34, "kind": "photo"}}}
        "#,
    ));
    // The plural is what a modern client reads; the singular is the first of it, kept for
    // clients that predate plurality, and it is ignored here.
    let ids: Vec<i64> = message.media().iter().map(|media| media.id).collect();
    assert_eq!(ids, [34, 35]);
}

#[test]
fn an_older_servers_single_attachment_still_arrives() {
    let message = message_of(parse(
        r#"
        {"type": "message", "message": {"id": 1341, "chat_id": 42, "sender_id": 7,
         "client_msg_id": null, "body": "look", "created_at": "…",
         "attachment": {"id": 34, "kind": "photo"}}}
        "#,
    ));
    assert_eq!(message.media().len(), 1);
    assert_eq!(message.media()[0].id, 34);
}

#[test]
fn the_call_frames_read_as_the_protocol_has_them() {
    let offer = parse(
        r#"
        {"type": "call_offer", "call_id": "6a1f0c3e-…", "chat_id": 42, "from_user_id": 7,
         "sdp": "v=0\r\n…", "video": true}
        "#,
    );
    let Some(ServerFrame::CallOffer {
        video,
        from_user_id,
        ..
    }) = offer
    else {
        panic!("expected an offer, got {offer:?}");
    };
    assert!(video);
    assert_eq!(from_user_id, 7);
    // Absent `video` is a voice call, never false-because-missing.
    let voice = parse(
        r#"
        {"type": "call_offer", "call_id": "x", "chat_id": 42, "from_user_id": 7, "sdp": "v=0"}
        "#,
    );
    assert!(matches!(
        voice,
        Some(ServerFrame::CallOffer { video: false, .. })
    ));

    let ice = parse(
        r#"
        {"type": "call_ice", "call_id": "6a1f0c3e-…",
         "candidate": {"candidate": "candidate:…", "sdp_mid": "0", "sdp_mline_index": 0}}
        "#,
    );
    let Some(ServerFrame::CallIce { candidate, .. }) = ice else {
        panic!("expected a candidate, got {ice:?}");
    };
    assert_eq!(candidate.sdp_mid.as_deref(), Some("0"));
    assert_eq!(candidate.sdp_mline_index, Some(0));
    // Each is optional: a stack supplies one, the other, or both.
    let partial = parse(
        r#"
        {"type": "call_ice", "call_id": "x", "candidate": {"candidate": "candidate:…"}}
        "#,
    );
    let Some(ServerFrame::CallIce {
        candidate: half, ..
    }) = partial
    else {
        panic!("expected a candidate, got {partial:?}");
    };
    assert_eq!(half.sdp_mid, None);
    assert_eq!(half.sdp_mline_index, None);

    let end = parse(r#"{"type": "call_end", "call_id": "x", "reason": "declined"}"#);
    let Some(ServerFrame::CallEnd { reason, .. }) = end else {
        panic!("expected an end, got {end:?}");
    };
    assert_eq!(reason, "declined");
}

#[test]
fn an_error_says_which_request_it_answers_and_never_both() {
    let send = parse(
        r#"{"type": "error", "code": "not_chat_member", "message": "…", "client_msg_id": "8f14e45f-…"}"#,
    );
    let Some(ServerFrame::Error {
        code,
        client_msg_id,
        call_id,
        ..
    }) = send
    else {
        panic!("expected an error, got {send:?}");
    };
    assert_eq!(code, error_codes::NOT_CHAT_MEMBER);
    assert_eq!(client_msg_id.as_deref(), Some("8f14e45f-…"));
    assert_eq!(call_id, None);

    let call =
        parse(r#"{"type": "error", "code": "peer_busy", "message": "…", "call_id": "6a1f0c3e-…"}"#);
    let Some(ServerFrame::Error {
        client_msg_id,
        call_id,
        ..
    }) = call
    else {
        panic!("expected an error, got {call:?}");
    };
    assert_eq!(call_id.as_deref(), Some("6a1f0c3e-…"));
    assert_eq!(client_msg_id, None);
}

/// A frame this client has never heard of is IGNORED, not an error: a newer server may add a
/// type, and a client that threw would drop the connection over something it was free to skip
/// (docs/protocol.md, "Compatibility rules").
#[test]
fn an_unknown_frame_is_ignored_rather_than_failing() {
    assert_eq!(parse(r#"{"type": "fireworks", "colour": "green"}"#), None);
    assert_eq!(parse("not json at all"), None);
    assert_eq!(parse("[]"), None);
    assert_eq!(parse("{}"), None);
    // A known type whose payload is missing is not half-applied either.
    assert_eq!(parse(r#"{"type": "message"}"#), None);
    assert_eq!(parse(r#"{"type": "board_note"}"#), None);
}

#[test]
fn the_member_frames_carry_the_family_except_when_there_is_none() {
    let joined = parse(
        r#"
        {"type": "member_joined", "family_id": 3,
         "user": {"id": 11, "username": "junior", "display_name": "Junior", "avatar_version": 0}}
        "#,
    );
    let Some(ServerFrame::MemberJoined {
        family_id, user, ..
    }) = joined
    else {
        panic!("expected a join, got {joined:?}");
    };
    assert_eq!(family_id, Some(3));
    assert_eq!(user.display_name, "Junior");
    // `family_id` is absent when the deleted account belonged to no family.
    let deleted = parse(
        r#"
        {"type": "member_deleted",
         "member": {"id": 11, "username": "junior", "display_name": "Junior", "deleted": true}}
        "#,
    );
    let Some(ServerFrame::MemberDeleted { family_id, member }) = deleted else {
        panic!("expected a deletion, got {deleted:?}");
    };
    assert_eq!(family_id, None);
    assert!(member.deleted);
}

#[test]
fn the_assistants_deltas_and_its_failure_both_name() {
    let delta = parse(r#"{"type": "ai_delta", "chat_id": 42, "message_id": 1339, "text": "Sure"}"#);
    assert!(matches!(delta, Some(ServerFrame::AiDelta { ref text, .. }) if text == "Sure"));
    let error = parse(r#"{"type": "ai_error", "chat_id": 42, "message_id": 1339}"#);
    let Some(ServerFrame::AiError {
        message_id, reason, ..
    }) = error
    else {
        panic!("expected a failure, got {error:?}");
    };
    assert_eq!(message_id, 1339);
    // Absent is every failure that is not a refusal, exactly as before the field existed.
    assert_eq!(reason, None);
    assert_eq!(parse(r#"{"type": "pong"}"#), Some(ServerFrame::Pong));
}

/// `ai_error`'s optional `reason`: "refused" is the provider's own filter, and a value this
/// client does not know — or one that is not a string at all — is read as ABSENT, never guessed
/// at (docs/protocol.md).
#[test]
fn an_assistant_failure_says_why_only_in_words_it_knows() {
    let refused =
        parse(r#"{"type": "ai_error", "chat_id": 42, "message_id": 1339, "reason": "refused"}"#);
    assert_eq!(
        refused,
        Some(ServerFrame::AiError {
            chat_id: 42,
            message_id: 1339,
            reason: Some(AiErrorReason::Refused),
        })
    );

    for unknown in [
        r#""quota""#,
        r#""Refused""#,
        r#""""#,
        "null",
        "7",
        "true",
        "{}",
        r#"["refused"]"#,
    ] {
        let frame = parse(&format!(
            r#"{{"type": "ai_error", "chat_id": 42, "message_id": 1339, "reason": {unknown}}}"#
        ));
        assert_eq!(
            frame,
            Some(ServerFrame::AiError {
                chat_id: 42,
                message_id: 1339,
                reason: None,
            }),
            "{unknown}"
        );
    }
}

#[test]
fn what_this_client_sends_reads_as_the_protocol_writes_it() {
    let none = SendOptions::default();
    let send = client_frames::send(
        42,
        "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
        "Dinner at 7?",
        &none,
    );
    assert_eq!(
        send,
        r#"{"type":"send","chat_id":42,"client_msg_id":"8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01","body":"Dinner at 7?"}"#
    );
    // Optional fields are ABSENT when they do not apply, never null and never empty.
    assert!(!send.contains("reply_to_message_id"));
    assert!(!send.contains("attachment_ids"));
    assert!(!send.contains("poll"));
    assert!(!send.contains("mentions"));

    assert!(
        client_frames::send(
            42,
            "id",
            "Six works",
            &SendOptions {
                reply_to_message_id: Some(1337),
                ..SendOptions::default()
            }
        )
        .contains(r#""reply_to_message_id":1337"#)
    );
    assert!(
        client_frames::send(
            42,
            "id",
            "",
            &SendOptions {
                attachment_ids: &[34, 35, 36],
                ..SendOptions::default()
            }
        )
        .contains(r#""attachment_ids":[34,35,36]"#)
    );
    let options = ["Pizza".to_string(), "Pasta".to_string()];
    assert!(
        client_frames::send(
            42,
            "id",
            "Pizza or pasta?",
            &SendOptions {
                poll_options: &options,
                ..SendOptions::default()
            }
        )
        .contains(r#""poll":{"options":["Pizza","Pasta"]}"#)
    );
    let mentions = [MentionDto::new(9, "Anna")];
    assert!(
        client_frames::send(
            42,
            "id",
            "@Anna are you in?",
            &SendOptions {
                mentions: &mentions,
                ..SendOptions::default()
            }
        )
        .contains(r#""mentions":[{"user_id":9,"name":"Anna"}]"#)
    );

    assert_eq!(
        client_frames::read(42, 1337),
        r#"{"type":"read","chat_id":42,"last_read_message_id":1337}"#
    );
    assert_eq!(
        client_frames::typing(42),
        r#"{"type":"typing","chat_id":42}"#
    );
    assert_eq!(client_frames::ping(), r#"{"type":"ping"}"#);
    // A voice call sends no `video` at all; a video call sends true.
    assert!(!client_frames::call_offer("c", 42, "v=0", false).contains("video"));
    assert!(client_frames::call_offer("c", 42, "v=0", true).contains(r#""video":true"#));
    // And a candidate sends only the halves it was given.
    let ice = client_frames::call_ice("c", &IceCandidate::new("candidate:…", None, Some(0)));
    assert!(ice.contains(r#""sdp_mline_index":0"#));
    assert!(!ice.contains("sdp_mid"));
}

/// Every frame this client sends has to come back through its own parser unchanged — the
/// cheapest guard against a writer and a reader drifting apart.
#[test]
fn what_is_sent_survives_its_own_parser() {
    let candidate = IceCandidate::new("candidate:…", Some("0"), Some(0));
    assert_eq!(
        parse(&client_frames::call_ice("c", &candidate)),
        Some(ServerFrame::CallIce {
            call_id: "c".to_string(),
            candidate,
        })
    );
    assert_eq!(
        parse(&client_frames::call_end("c", "hangup")),
        Some(ServerFrame::CallEnd {
            call_id: "c".to_string(),
            reason: "hangup".to_string(),
        })
    );
    assert_eq!(
        parse(&client_frames::call_answer("c", "v=0")),
        Some(ServerFrame::CallAnswer {
            call_id: "c".to_string(),
            sdp: "v=0".to_string(),
        })
    );
}

// ---- the sticker pack (docs/protocol.md, "Sticker pack") ---------------------------------

#[test]
fn a_pack_item_frame_carries_a_live_item() {
    let frame = parse(
        r#"
        {"type": "pack_item", "item": {"id": 5, "added_by": 7,
         "attachment": {"id": 71, "kind": "photo", "mime": "image/webp", "size": 40960,
                        "width": 512, "height": 512, "has_preview": false},
         "created_at": "2026-09-13T10:00:00Z", "pack_seq": 12, "label": "party cat"}}
        "#,
    );
    let Some(ServerFrame::PackItem(item)) = frame else {
        panic!("expected a pack item, got {frame:?}");
    };
    assert_eq!(item.id, 5);
    assert_eq!(item.added_by, 7);
    assert_eq!(item.pack_seq, 12);
    assert_eq!(item.label.as_deref(), Some("party cat"));
    assert!(!item.deleted);
    let attachment = item
        .attachment
        .as_ref()
        .expect("a live item has its picture");
    assert_eq!(attachment.id, 71);
    assert_eq!(attachment.mime.as_deref(), Some("image/webp"));
    // A pack item's attachment NEVER carries the flag: that is a message's.
    assert!(!attachment.sticker);
}

/// A tombstone is the id, the flag and the seq — and nothing else.
#[test]
fn a_pack_item_frame_carries_a_tombstone() {
    let frame =
        parse(r#"{"type": "pack_item", "item": {"id": 5, "deleted": true, "pack_seq": 14}}"#);
    let Some(ServerFrame::PackItem(item)) = frame else {
        panic!("expected a pack item, got {frame:?}");
    };
    assert!(item.deleted);
    assert_eq!(item.id, 5);
    assert_eq!(item.pack_seq, 14);
    assert_eq!(item.attachment, None);
    assert_eq!(item.label, None);
    assert_eq!(item.added_by, 0);
}

#[test]
fn a_pack_item_frame_with_no_item_is_no_frame() {
    assert_eq!(parse(r#"{"type": "pack_item"}"#), None);
}

/// A message `With(body, media…)`, as the C# tests build one.
fn with(created_at: &str, body: &str, media: Vec<AttachmentDto>) -> MessageDto {
    MessageDto {
        attachments: Some(media),
        ..MessageDto::new(1, 42, 9, None, body, created_at)
    }
}

/// The flag is on the ATTACHMENT, present only when true — in `attachments` and in the legacy
/// singular — and it is what makes the message a sticker.
#[test]
fn a_sticker_message_is_one_flagged_photo_and_no_words() {
    let message = message_of(parse(
        r#"
        {"type": "message", "message": {"id": 1340, "chat_id": 42, "sender_id": 9,
         "client_msg_id": null, "body": "", "created_at": "2026-09-13T10:00:00Z",
         "attachments": [{"id": 90, "kind": "photo", "mime": "image/webp", "size": 40960,
                          "width": 512, "height": 512, "has_preview": false, "sticker": true}],
         "attachment": {"id": 90, "kind": "photo", "mime": "image/webp", "size": 40960,
                        "width": 512, "height": 512, "has_preview": false, "sticker": true}}}
        "#,
    ));
    assert_eq!(message.media().len(), 1);
    assert!(message.media()[0].sticker);
    assert!(message.attachment.as_ref().expect("the singular").sticker);
    assert_eq!(
        message.sticker_picture().map(|picture| picture.id),
        Some(90)
    );
}

/// OLD MESSAGES, AND OLD SERVERS: a photo without the flag is a photo — the field is absent,
/// never false — and nothing about it is drawn differently than it ever was.
#[test]
fn a_photo_without_the_flag_is_not_a_sticker() {
    let plain: MessageDto = wire::decode(
        r#"
        {"id": 1, "chat_id": 42, "sender_id": 9, "client_msg_id": null, "body": "",
         "created_at": "2026-09-13T10:00:00Z",
         "attachments": [{"id": 90, "kind": "photo", "mime": "image/webp", "size": 40960}]}
        "#,
    )
    .expect("a message");
    assert!(!plain.media()[0].sticker);
    assert_eq!(plain.sticker_picture(), None);

    let at = "2026-09-13T10:00:00Z";
    let flagged = AttachmentDto {
        mime: Some("image/webp".to_string()),
        size: Some(40960),
        sticker: true,
        ..AttachmentDto::new(90, "photo")
    };
    assert!(
        with(at, "", vec![flagged.clone()])
            .sticker_picture()
            .is_some()
    );
    // THE ONE TEST, the same on every client: exactly ONE attachment, kind photo, sticker: true.
    assert_eq!(
        with(at, "", vec![flagged.clone(), flagged.clone()]).sticker_picture(),
        None
    );
    let video = AttachmentDto {
        kind: "video".to_string(),
        ..flagged.clone()
    };
    assert_eq!(with(at, "", vec![video]).sticker_picture(), None);
    let unflagged = AttachmentDto {
        sticker: false,
        ..flagged.clone()
    };
    assert_eq!(with(at, "", vec![unflagged]).sticker_picture(), None);
    let other = AttachmentDto {
        id: 91,
        sticker: false,
        ..flagged.clone()
    };
    assert_eq!(
        with(at, "", vec![flagged.clone(), other]).sticker_picture(),
        None
    );
    assert_eq!(with(at, "", vec![]).sticker_picture(), None);
    // And NOTHING ELSE is asked. Words beside the flag are a shape the server refuses; were one
    // ever stored, every client must still agree on what it is, and the rule they share does not
    // read the body or the type.
    assert!(
        with(at, "look", vec![flagged.clone()])
            .sticker_picture()
            .is_some()
    );
    let jpeg = AttachmentDto {
        mime: Some("image/jpeg".to_string()),
        ..flagged
    };
    assert!(with(at, "", vec![jpeg]).sticker_picture().is_some());
}

#[test]
fn a_send_says_sticker_only_when_it_is_one() {
    let sticker = json(&client_frames::send(
        42,
        "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
        "",
        &SendOptions {
            attachment_ids: &[90],
            sticker: true,
            ..SendOptions::default()
        },
    ));
    assert_eq!(sticker["type"], "send");
    assert_eq!(sticker["sticker"], true);
    assert_eq!(sticker["body"], "");
    assert_eq!(sticker["attachment_ids"][0], 90);

    // Absent on an ordinary message — never false — so a server that predates stickers reads
    // exactly the frame it always read.
    let plain = json(&client_frames::send(
        42,
        "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
        "Dinner at 7?",
        &SendOptions {
            attachment_ids: &[90],
            ..SendOptions::default()
        },
    ));
    assert!(plain.get("sticker").is_none());
}

// ---- video messages (docs/protocol.md, "Video messages") -----------------------------------

/// The flag is on the ATTACHMENT, present only when true — in `attachments` and in the legacy
/// singular — and it is what makes the message a circle.
#[test]
fn a_video_message_is_one_flagged_video_and_no_words() {
    let message = message_of(parse(
        r#"
        {"type": "message", "message": {"id": 1341, "chat_id": 42, "sender_id": 9,
         "client_msg_id": null, "body": "", "created_at": "2026-10-05T10:00:00Z",
         "attachments": [{"id": 91, "kind": "video", "mime": "video/mp4", "size": 1649700, "width": 480,
                          "height": 480, "duration_ms": 23400, "has_preview": true, "round": true}],
         "attachment": {"id": 91, "kind": "video", "mime": "video/mp4", "size": 1649700, "width": 480,
                        "height": 480, "duration_ms": 23400, "has_preview": true, "round": true}}}
        "#,
    ));
    assert_eq!(message.media().len(), 1);
    let video = &message.media()[0];
    assert!(video.round);
    assert!(!video.sticker);
    assert!(message.attachment.as_ref().expect("the singular").round);
    let round = message.round_video().expect("a video message");
    assert_eq!(round.id, 91);
    assert_eq!(round.duration_ms, Some(23400));
    assert_eq!(message.sticker_picture(), None);
}

/// THE ONE TEST (S5.1), the same on every client: exactly ONE attachment, `kind=video`,
/// `round: true`, no body. A video without the flag — every video from before, and every one an
/// old server delivers — is a square video.
#[test]
fn only_one_flagged_video_is_a_video_message() {
    let plain: MessageDto = wire::decode(
        r#"
        {"id": 1, "chat_id": 42, "sender_id": 9, "client_msg_id": null, "body": "",
         "created_at": "2026-10-05T10:00:00Z",
         "attachments": [{"id": 91, "kind": "video", "mime": "video/mp4", "size": 1649700}]}
        "#,
    )
    .expect("a message");
    assert!(!plain.media()[0].round);
    assert_eq!(plain.round_video(), None);

    let at = "2026-10-05T10:00:00Z";
    let flagged = AttachmentDto {
        mime: Some("video/mp4".to_string()),
        size: Some(1649700),
        width: Some(480),
        height: Some(480),
        duration_ms: Some(23400),
        has_preview: true,
        round: true,
        ..AttachmentDto::new(91, "video")
    };
    let kind = |kind: &str| AttachmentDto {
        kind: kind.to_string(),
        ..flagged.clone()
    };
    assert!(with(at, "", vec![flagged.clone()]).round_video().is_some());
    let second = AttachmentDto {
        id: 92,
        ..flagged.clone()
    };
    assert_eq!(
        with(at, "", vec![flagged.clone(), second]).round_video(),
        None
    );
    assert_eq!(with(at, "", vec![kind("photo")]).round_video(), None);
    assert_eq!(with(at, "", vec![kind("audio")]).round_video(), None);
    let unflagged = AttachmentDto {
        round: false,
        ..flagged.clone()
    };
    assert_eq!(with(at, "", vec![unflagged]).round_video(), None);
    assert_eq!(with(at, "", vec![]).round_video(), None);
    // A body — any body, whitespace too, compared exactly (fc_text::record::is_round) — is an
    // ordinary message.
    assert_eq!(with(at, "look", vec![flagged.clone()]).round_video(), None);
    assert_eq!(with(at, " ", vec![flagged.clone()]).round_video(), None);
    assert_eq!(with(at, "\n", vec![flagged.clone()]).round_video(), None);
    assert_eq!(
        with(at, "\u{00A0}", vec![flagged.clone()]).round_video(),
        None
    );
    // And nothing else is asked — not the type, not the shape.
    let landscape = AttachmentDto {
        width: Some(640),
        height: Some(480),
        ..flagged.clone()
    };
    assert!(with(at, "", vec![landscape]).round_video().is_some());
    // A circle is not a sticker, and a sticker is not a circle.
    assert_eq!(with(at, "", vec![flagged]).sticker_picture(), None);
}

/// The legacy singular alone — a server that predates plurality — is read the same way.
#[test]
fn the_legacy_singular_carries_the_flag_too() {
    let legacy: MessageDto = wire::decode(
        r#"
        {"id": 1, "chat_id": 42, "sender_id": 9, "client_msg_id": null, "body": "",
         "created_at": "2026-10-05T10:00:00Z",
         "attachment": {"id": 91, "kind": "video", "mime": "video/mp4", "round": true}}
        "#,
    )
    .expect("a message");
    assert_eq!(legacy.round_video().map(|video| video.id), Some(91));
}

#[test]
fn a_send_says_round_only_when_it_is_a_video_message() {
    let round = json(&client_frames::send(
        42,
        "4f9e21c0-ceea-4e17-a91c-0d9f8e7b2a01",
        "",
        &SendOptions {
            attachment_ids: &[91],
            round: true,
            ..SendOptions::default()
        },
    ));
    assert_eq!(round["type"], "send");
    assert_eq!(round["round"], true);
    assert!(round.get("sticker").is_none());
    assert_eq!(round["attachment_ids"][0], 91);

    let plain = json(&client_frames::send(
        42,
        "4f9e21c0-ceea-4e17-a91c-0d9f8e7b2a01",
        "",
        &SendOptions {
            attachment_ids: &[91],
            ..SendOptions::default()
        },
    ));
    assert!(plain.get("round").is_none());
    let sticker = json(&client_frames::send(
        42,
        "4f9e21c0-ceea-4e17-a91c-0d9f8e7b2a01",
        "",
        &SendOptions {
            attachment_ids: &[90],
            sticker: true,
            ..SendOptions::default()
        },
    ));
    assert!(sticker.get("round").is_none());
}

// ---- animations (docs/protocol.md, "Animations") -------------------------------------------

/// The flag is on the ATTACHMENT, present only when true, read wherever `round` is — the plural
/// and the legacy singular — and an animation may carry a caption.
#[test]
fn an_animation_is_one_flagged_video_with_or_without_words() {
    let message = message_of(parse(
        r#"
        {"type": "message", "message": {"id": 1342, "chat_id": 42, "sender_id": 9,
         "client_msg_id": null, "body": "ha", "created_at": "2026-10-19T10:00:00Z",
         "attachments": [{"id": 93, "kind": "video", "mime": "video/mp4", "size": 182330, "width": 480,
                          "height": 270, "duration_ms": 2100, "has_preview": true, "animation": true}],
         "attachment": {"id": 93, "kind": "video", "mime": "video/mp4", "size": 182330, "width": 480,
                        "height": 270, "duration_ms": 2100, "has_preview": true, "animation": true}}}
        "#,
    ));
    assert_eq!(message.media().len(), 1);
    let video = &message.media()[0];
    assert!(video.animation);
    assert!(!video.round);
    assert!(!video.sticker);
    assert!(message.attachment.as_ref().expect("the singular").animation);
    let animation = message.animation_video().expect("an animation");
    assert_eq!(animation.id, 93);
    assert_eq!(animation.duration_ms, Some(2100));
    // Not a circle (that needs round: true and no body), and not a sticker.
    assert_eq!(message.round_video(), None);
    assert_eq!(message.sticker_picture(), None);
}

/// THE ONE TEST, the sticker's and the video message's: exactly ONE attachment, `kind=video`,
/// `animation: true`. The body is not asked — a caption is allowed — and a video without the flag
/// is an ordinary video.
#[test]
fn only_one_flagged_video_is_an_animation() {
    let plain: MessageDto = wire::decode(
        r#"
        {"id": 1, "chat_id": 42, "sender_id": 9, "client_msg_id": null, "body": "",
         "created_at": "2026-10-19T10:00:00Z",
         "attachments": [{"id": 93, "kind": "video", "mime": "video/mp4", "size": 182330}]}
        "#,
    )
    .expect("a message");
    assert!(!plain.media()[0].animation);
    assert_eq!(plain.animation_video(), None);

    let at = "2026-10-19T10:00:00Z";
    let flagged = AttachmentDto {
        mime: Some("video/mp4".to_string()),
        size: Some(182330),
        width: Some(480),
        height: Some(270),
        duration_ms: Some(2100),
        has_preview: true,
        animation: true,
        ..AttachmentDto::new(93, "video")
    };
    assert!(
        with(at, "", vec![flagged.clone()])
            .animation_video()
            .is_some()
    );
    assert!(
        with(at, "look at this", vec![flagged.clone()])
            .animation_video()
            .is_some()
    );
    let second = AttachmentDto {
        id: 94,
        ..flagged.clone()
    };
    assert_eq!(
        with(at, "", vec![flagged.clone(), second]).animation_video(),
        None
    );
    let photo = AttachmentDto {
        kind: "photo".to_string(),
        ..flagged.clone()
    };
    assert_eq!(with(at, "", vec![photo]).animation_video(), None);
    let unflagged = AttachmentDto {
        animation: false,
        ..flagged.clone()
    };
    assert_eq!(with(at, "", vec![unflagged]).animation_video(), None);
    assert_eq!(with(at, "", vec![]).animation_video(), None);
    assert_eq!(with(at, "", vec![flagged]).round_video(), None);

    let legacy: MessageDto = wire::decode(
        r#"
        {"id": 1, "chat_id": 42, "sender_id": 9, "client_msg_id": null, "body": "",
         "created_at": "2026-10-19T10:00:00Z",
         "attachment": {"id": 93, "kind": "video", "mime": "video/mp4", "animation": true}}
        "#,
    )
    .expect("a message");
    assert_eq!(legacy.animation_video().map(|video| video.id), Some(93));
}

/// What this device caches of an animation is read back an animation: the flag survives the
/// cache's own JSON.
#[test]
fn an_animation_survives_the_caches_own_encoding() {
    let media = vec![AttachmentDto {
        mime: Some("video/mp4".to_string()),
        size: Some(182330),
        width: Some(480),
        height: Some(270),
        duration_ms: Some(2100),
        has_preview: true,
        animation: true,
        ..AttachmentDto::new(93, "video")
    }];
    let again: Vec<AttachmentDto> = wire::decode(&wire::encode(&media)).expect("the cache's JSON");
    assert_eq!(again.len(), 1);
    assert!(again[0].animation);
}

#[test]
fn a_send_says_animation_only_when_it_is_one() {
    let animation = json(&client_frames::send(
        42,
        "4f9e21c0-ceea-4e17-a91c-0d9f8e7b2a03",
        "ha",
        &SendOptions {
            attachment_ids: &[93],
            animation: true,
            ..SendOptions::default()
        },
    ));
    assert_eq!(animation["type"], "send");
    assert_eq!(animation["animation"], true);
    assert_eq!(animation["body"], "ha");
    assert!(animation.get("round").is_none());
    assert!(animation.get("sticker").is_none());

    let plain = json(&client_frames::send(
        42,
        "4f9e21c0-ceea-4e17-a91c-0d9f8e7b2a03",
        "",
        &SendOptions {
            attachment_ids: &[93],
            ..SendOptions::default()
        },
    ));
    assert!(plain.get("animation").is_none());
    let round = json(&client_frames::send(
        42,
        "4f9e21c0-ceea-4e17-a91c-0d9f8e7b2a03",
        "",
        &SendOptions {
            attachment_ids: &[91],
            round: true,
            ..SendOptions::default()
        },
    ));
    assert!(round.get("animation").is_none());
}

/// The two group frames (docs/protocol.md, "Groups"): `chat_updated` carries the whole chat,
/// members and admin included, and `chat_removed` just the id. One with no chat, or no id, is no
/// frame at all.
#[test]
fn the_group_frames_carry_the_chat_and_the_id() {
    let updated = parse(
        r#"
        {"type": "chat_updated", "chat": {"id": 77, "kind": "group", "title": "Cousins", "peer_user_id": null,
         "member_ids": [7, 9, 11], "admin_user_id": 9}}
        "#,
    );
    let Some(ServerFrame::ChatUpdated(chat)) = updated else {
        panic!("expected a group, got {updated:?}");
    };
    assert_eq!(chat.id, 77);
    assert_eq!(chat.kind, "group");
    assert_eq!(chat.title, "Cousins");
    assert_eq!(chat.member_ids.as_deref(), Some(&[7, 9, 11][..]));
    assert_eq!(chat.admin_user_id, Some(9));

    assert_eq!(
        parse(r#"{"type": "chat_removed", "chat_id": 77}"#),
        Some(ServerFrame::ChatRemoved { chat_id: 77 })
    );

    assert_eq!(parse(r#"{"type": "chat_updated"}"#), None);
    assert_eq!(parse(r#"{"type": "chat_removed"}"#), None);
}

/// A chat that is not a group carries neither field, and decodes with both `None` — every chat
/// before groups.
#[test]
fn a_chat_that_is_not_a_group_has_no_members_or_admin() {
    let chat: ChatDto = wire::decode(
        r#"{"id": 42, "kind": "family", "title": "The Smiths", "peer_user_id": null}"#,
    )
    .expect("a chat");
    assert_eq!(chat.member_ids, None);
    assert_eq!(chat.admin_user_id, None);
}
