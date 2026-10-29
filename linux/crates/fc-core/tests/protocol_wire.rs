//! This port's own checks of the data model — not ports of a `win/` test, but of what `win/`
//! did without one: how System.Text.Json read and wrote these types (each case run on .NET 10
//! itself), the bodies `ApiClientTests` sends and reads checked at the model before there is a
//! client, and the .NET number and URI formatting the upload query leans on.

use fc_core::protocol::upload_queries::{self, escape_data_string, fixed};
use fc_core::protocol::{
    AssistantConsentRequest, AssistantLookupConsentResponse, AttachmentDto, ChatResponse,
    ErrorEnvelope, FamilyDto, FamilyOnlyResponse, FamilyPatch, FamilyResponse, MeResponse,
    MessageDto, NoteDto, NotePatch, NoteRequest, PollRequest, ReportDto, RoundVideoLimits,
    SendRequest, ServerFrame, StatsResponse, TaskLineRequest, TranscriptResponse, error_codes,
    wire,
};
use serde_json::Value;

fn json(text: &str) -> Value {
    serde_json::from_str(text).expect("JSON")
}

// ---- reading: what C#'s decoder did with what the protocol leaves out -----------------------

/// A payload that lacks the fields `win/` declares without a default reads them as C#'s decoder
/// filled them — zero, false, and (for a string, where C# had `null`) empty — rather than
/// failing the frame.
#[test]
fn a_payload_missing_its_fields_reads_as_the_cs_decoder_filled_it() {
    let frame = ServerFrame::parse(r#"{"type": "message", "message": {}}"#);
    let Some(ServerFrame::Message(message)) = frame else {
        panic!("expected a message, got {frame:?}");
    };
    assert_eq!(message, MessageDto::default());
    // A chat with no id is no group to upsert.
    assert_eq!(
        ServerFrame::parse(r#"{"type": "chat_updated", "chat": {"kind": "group"}}"#),
        None
    );
    // A board tombstone carries no author; a reported message's attachments carry no id.
    let tombstone: NoteDto =
        wire::decode(r#"{"id": 5, "deleted": true, "board_seq": 14}"#).expect("a tombstone");
    assert_eq!(tombstone.author_id, 0);
    assert!(tombstone.deleted);
    let report: ReportDto = wire::decode(
        r#"{"id": 1, "reporter": {"id": 1, "username": "a", "display_name": "A"},
            "reported": {"id": 2, "username": "b", "display_name": "B"}, "reason": "spam",
            "message_attachments": [{"kind": "photo", "name": null}]}"#,
    )
    .expect("a report");
    let attachments = report.message_attachments.expect("its attachments");
    assert_eq!(attachments[0].id, 0);
    assert_eq!(attachments[0].kind, "photo");
    assert_eq!(attachments[0].name, None);
    // A `null` string is read as C# read a missing one.
    let silent: MessageDto = wire::decode(r#"{"id": 1, "body": null}"#).expect("a message");
    assert_eq!(silent.body, "");
}

/// What C#'s decoder refused, this one refuses: a `null` where a number or a flag must be, a
/// number too big for its field, a fraction where an id goes, a trailing comma.
#[test]
fn what_the_cs_decoder_refused_is_refused() {
    assert_eq!(wire::decode::<MessageDto>(r#"{"id": null}"#), None);
    assert_eq!(
        wire::decode::<AttachmentDto>(r#"{"id": 1, "kind": "photo", "sticker": null}"#),
        None
    );
    assert_eq!(
        wire::decode::<AttachmentDto>(r#"{"id": 1, "kind": "photo", "width": 3000000000}"#),
        None
    );
    assert_eq!(
        wire::decode::<AttachmentDto>(r#"{"id": 1.0, "kind": "photo"}"#),
        None
    );
    assert_eq!(
        wire::decode::<AttachmentDto>(r#"{"id": 1, "kind": "photo",}"#),
        None
    );
}

/// An unknown field is a newer server, not an error.
#[test]
fn an_unknown_field_is_a_newer_server() {
    let attachment: AttachmentDto =
        wire::decode(r#"{"id": 1, "kind": "photo", "future": {"a": [1, null]}, "hdr": true}"#)
            .expect("an attachment");
    assert_eq!(attachment.id, 1);
}

/// A `null` inside the greeting's places or the lookup providers is dropped where it is read —
/// every reader of the C# array dropped it — and never fails the family's read.
#[test]
fn a_null_inside_a_list_of_names_is_dropped() {
    let family: FamilyDto =
        wire::decode(r#"{"id": 3, "name": "The Smiths", "greeting_places": ["Moscow", null]}"#)
            .expect("a family");
    assert_eq!(family.greeting_places, Some(vec!["Moscow".to_string()]));
    let older: FamilyDto = wire::decode(r#"{"id": 3, "name": "The Smiths"}"#).expect("a family");
    assert_eq!(older.greeting_places, None);
    let response: FamilyResponse = wire::decode(
        r#"{"family": {"id": 3, "name": "The Smiths"},
            "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai",
                          "lookups": [null, "Wikipedia"]}}"#,
    )
    .expect("the family's document");
    let assistant = response.assistant.expect("an assistant");
    assert_eq!(assistant.lookups, Some(vec!["Wikipedia".to_string()]));
}

/// `ai_history` is a switch with "a real default (true)" (docs/protocol.md, "Objects"), and the
/// Mac and the web read an absent key as true; `win/` read false. The protocol wins.
#[test]
fn an_absent_history_switch_is_the_protocols_true() {
    let older: FamilyDto = wire::decode(r#"{"id": 3, "name": "The Smiths"}"#).expect("a family");
    assert!(older.ai_history);
    assert!(!older.ai_vision);
    assert!(FamilyDto::new(3, "The Smiths").ai_history);
    let off: FamilyDto =
        wire::decode(r#"{"id": 3, "name": "The Smiths", "ai_history": false}"#).expect("a family");
    assert!(!off.ai_history);
    // And a server that predates the registration switch took new families.
    let me: MeResponse =
        wire::decode(r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"}}"#)
            .expect("GET /me");
    assert!(me.family_registration_enabled);
    assert!(MeResponse::default().family_registration_enabled);
}

/// An error body without its `error` object is no envelope — the status classifies it — where
/// C# handed back an envelope whose `Error` was null for `ApiClient` to dereference. A code with
/// no sentence is still a code.
#[test]
fn an_error_body_without_its_object_is_no_envelope() {
    assert_eq!(wire::decode::<ErrorEnvelope>("{}"), None);
    assert_eq!(
        wire::decode::<ErrorEnvelope>(r#"{"message": "Not Found"}"#),
        None
    );
    let terse: ErrorEnvelope =
        wire::decode(r#"{"error": {"code": "blocked"}}"#).expect("an envelope");
    assert_eq!(terse.error.code, error_codes::BLOCKED);
    assert_eq!(terse.error.message, "");
}

/// A field of the wrong JSON type in a frame reads as ABSENT, where `win/` threw and its socket
/// dropped the connection: the frame's own fallback applies, and one that cannot stand without
/// the field is no frame.
#[test]
fn a_frame_field_of_the_wrong_type_reads_as_absent() {
    assert_eq!(ServerFrame::parse(r#"{"type": 5}"#), None);
    assert_eq!(ServerFrame::parse(r#"{"type": null}"#), None);
    assert_eq!(
        ServerFrame::parse(r#"{"type": "member_blocked", "user_id": 9, "blocked": "yes"}"#),
        Some(ServerFrame::MemberBlocked {
            user_id: 9,
            blocked: false,
        })
    );
    assert_eq!(
        ServerFrame::parse(r#"{"type": "call_end", "call_id": 7}"#),
        None
    );
    assert_eq!(
        ServerFrame::parse(r#"{"type": "error", "code": 5}"#),
        Some(ServerFrame::Error {
            code: error_codes::INTERNAL.to_string(),
            detail: String::new(),
            client_msg_id: None,
            call_id: None,
        })
    );
    assert_eq!(
        ServerFrame::parse(r#"{"type": "ai_delta", "chat_id": 1, "message_id": 2, "text": 5}"#),
        Some(ServerFrame::AiDelta {
            chat_id: 1,
            message_id: 2,
            text: String::new(),
        })
    );
    // Numbers as `win/`'s `Number` read them: anything that is not a 64-bit integer is 0, and an
    // optional one is absent only when missing or null.
    assert_eq!(
        ServerFrame::parse(
            r#"{"type": "read", "chat_id": "42", "user_id": 42.0, "last_read_message_id": 9223372036854775808}"#
        ),
        Some(ServerFrame::Read {
            chat_id: 0,
            user_id: 0,
            last_read_message_id: 0,
        })
    );
    assert_eq!(
        ServerFrame::parse(r#"{"type": "member_left", "family_id": "3", "user_id": 1}"#),
        Some(ServerFrame::MemberLeft {
            family_id: Some(0),
            user_id: 1,
        })
    );
    assert_eq!(
        ServerFrame::parse(r#"{"type": "member_left", "family_id": null, "user_id": 1}"#),
        Some(ServerFrame::MemberLeft {
            family_id: None,
            user_id: 1,
        })
    );
    // Reactions that are not a list are none, as `?? []` made them.
    assert_eq!(
        ServerFrame::parse(
            r#"{"type": "reaction", "chat_id": 1, "message_id": 2, "reaction_seq": 3, "reactions": {"a": 1}}"#
        ),
        Some(ServerFrame::Reactions {
            chat_id: 1,
            message_id: 2,
            reaction_seq: 3,
            value: Vec::new(),
        })
    );
    // Trailing text is not JSON.
    assert_eq!(ServerFrame::parse(r#"{"type": "pong"} trailing"#), None);
}

// ---- writing --------------------------------------------------------------------------------

/// What the cache keeps: every `None` left out, every other field written. (C#'s writer also
/// wrote the record's computed properties — `is_photo`, `aspect_ratio`, a message's `media` —
/// which nothing read back; here only the wire's own fields are written.)
#[test]
fn the_caches_json_leaves_out_what_is_absent() {
    let attachment = AttachmentDto {
        mime: Some("video/mp4".to_string()),
        size: Some(182330),
        width: Some(480),
        height: Some(270),
        duration_ms: Some(2100),
        has_preview: true,
        animation: true,
        ..AttachmentDto::new(93, "video")
    };
    assert_eq!(
        wire::encode(&attachment),
        r#"{"id":93,"kind":"video","mime":"video/mp4","size":182330,"width":480,"height":270,"duration_ms":2100,"has_preview":true,"sticker":false,"round":false,"animation":true}"#
    );
    let message = MessageDto {
        attachments: Some(vec![attachment]),
        reactions: Some(Vec::new()),
        ..MessageDto::new(1, 42, 9, None, "Tom & <b>Привет</b>", "t")
    };
    let encoded = wire::encode(&message);
    assert!(!encoded.contains("client_msg_id"));
    assert!(encoded.contains(r#""reactions":[]"#));
    assert_eq!(wire::decode::<MessageDto>(&encoded), Some(message));
}

/// The REST send, as `ApiClientTests` holds it: only the fields that apply, each flag present
/// only when true.
#[test]
fn a_rest_send_writes_only_what_applies() {
    let sticker = SendRequest {
        client_msg_id: "k".to_string(),
        attachment_ids: Some(vec![90]),
        sticker: Some(true),
        ..SendRequest::default()
    };
    assert_eq!(
        wire::encode(&sticker),
        r#"{"client_msg_id":"k","body":"","attachment_ids":[90],"sticker":true}"#
    );
    let round = SendRequest {
        client_msg_id: "k".to_string(),
        reply_to_message_id: Some(41),
        attachment_ids: Some(vec![91]),
        round: Some(true),
        ..SendRequest::default()
    };
    assert_eq!(
        wire::encode(&round),
        r#"{"client_msg_id":"k","body":"","reply_to_message_id":41,"attachment_ids":[91],"round":true}"#
    );
    let animation = SendRequest {
        client_msg_id: "k".to_string(),
        attachment_ids: Some(vec![93]),
        animation: Some(true),
        ..SendRequest::default()
    };
    assert_eq!(
        wire::encode(&animation),
        r#"{"client_msg_id":"k","body":"","attachment_ids":[93],"animation":true}"#
    );
    let plain = wire::encode(&SendRequest {
        client_msg_id: "k".to_string(),
        body: "hi".to_string(),
        ..SendRequest::default()
    });
    assert!(plain.contains(r#""client_msg_id":"k""#));
    for absent in [
        "reply_to_message_id",
        "attachment_ids",
        "poll",
        "mentions",
        "sticker",
    ] {
        assert!(!plain.contains(absent), "{absent}");
    }
    let poll = wire::encode(&SendRequest {
        poll: Some(PollRequest {
            options: vec!["Pizza".to_string(), "Pasta".to_string()],
        }),
        ..SendRequest::default()
    });
    assert!(poll.contains(r#""poll":{"options":["Pizza","Pasta"]}"#));
}

/// A note's bodies, as `ApiClientTests` holds them: a patch sends only what changed, a new list
/// sends its lines without ids, a rewritten line keeps its id, and an event's end is taken off
/// with the one null.
#[test]
fn a_notes_bodies_write_only_what_changed() {
    let moved = NotePatch {
        x: Some(0.4),
        y: Some(0.6),
        ..NotePatch::default()
    };
    assert_eq!(wire::encode(&moved), r#"{"x":0.4,"y":0.6}"#);
    let list = NoteRequest {
        kind: Some("tasks".to_string()),
        items: Some(vec![
            TaskLineRequest::new("Milk", None),
            TaskLineRequest::new("Bread", None),
        ]),
        ..NoteRequest::new("Saturday", "green", 0.1, 0.1)
    };
    let body = wire::encode(&list);
    assert!(body.contains(r#""items":[{"text":"Milk"},{"text":"Bread"}]"#));
    assert!(!body.contains(r#""id""#));
    let rewritten = wire::encode(&NotePatch {
        items: Some(vec![
            TaskLineRequest::new("Milk and eggs", Some(11)),
            TaskLineRequest::new("Jam", None),
        ]),
        ..NotePatch::default()
    });
    assert!(rewritten.contains(r#"{"text":"Milk and eggs","id":11}"#));
    assert!(rewritten.contains(r#"{"text":"Jam"}"#));

    let cleared = json(&wire::encode(&NotePatch {
        place: Some(String::new()),
        clears_end: true,
        ..NotePatch::default()
    }));
    assert_eq!(cleared["ends_at"], Value::Null);
    assert!(cleared.get("ends_at").is_some());
    assert_eq!(cleared["place"], "");
    assert!(cleared.get("clears_end").is_none());
    assert!(cleared.get("text").is_none());
    let alone = json(&wire::encode(&NotePatch {
        place: Some("Park".to_string()),
        ..NotePatch::default()
    }));
    assert!(alone.get("ends_at").is_none());
    // The null wins over an end that was also set.
    let both = json(&wire::encode(&NotePatch {
        ends_at: Some("2026-12-24T19:00:00Z".to_string()),
        clears_end: true,
        ..NotePatch::default()
    }));
    assert_eq!(both["ends_at"], Value::Null);

    // What was sent reads back as the patch it was (BoardModelTests reads the body this way).
    let sent: NotePatch = wire::decode(&wire::encode(&moved)).expect("the patch");
    assert_eq!(sent.x, Some(0.4));
    assert_eq!(sent.text, None);
    assert!(NotePatch::default().is_empty());
    assert!(!moved.is_empty());
    assert!(
        !NotePatch {
            clears_end: true,
            ..NotePatch::default()
        }
        .is_empty()
    );
}

/// The family's patch, as `ApiClientTests` holds it: each switch its own key and only when in the
/// patch, the places one list that is never `null`, and the two fields a `null` clears.
#[test]
fn a_family_patch_writes_only_what_is_in_it() {
    let patch = |patch: FamilyPatch| wire::encode(&patch);
    assert_eq!(
        patch(FamilyPatch {
            ai_transcripts: Some(true),
            ..FamilyPatch::default()
        }),
        r#"{"ai_transcripts":true}"#
    );
    assert!(
        !patch(FamilyPatch {
            ai_vision: Some(true),
            ..FamilyPatch::default()
        })
        .contains("ai_transcripts")
    );
    assert_eq!(
        patch(FamilyPatch {
            ai_lookups: Some(false),
            ..FamilyPatch::default()
        }),
        r#"{"ai_lookups":false}"#
    );
    assert_eq!(
        patch(FamilyPatch {
            greeting_places: Some(vec!["Moscow".to_string(), "moscow".to_string()]),
            ..FamilyPatch::default()
        }),
        r#"{"greeting_places":["Moscow","moscow"]}"#
    );
    assert_eq!(
        patch(FamilyPatch {
            greeting_places: Some(Vec::new()),
            ..FamilyPatch::default()
        }),
        r#"{"greeting_places":[]}"#
    );
    assert!(
        !patch(FamilyPatch {
            ai_greeting: Some(true),
            ..FamilyPatch::default()
        })
        .contains("greeting_places")
    );
    // Any script: the body is JSON whatever the escaping, and it reads back as what was typed.
    let sent = json(&patch(FamilyPatch {
        greeting_places: Some(vec!["Белград".to_string()]),
        ..FamilyPatch::default()
    }));
    assert_eq!(sent["greeting_places"][0], "Белград");
    assert_eq!(sent.as_object().map(|object| object.len()), Some(1));
    // The two places on this wire where `null` means something a missing key does not.
    assert_eq!(
        patch(FamilyPatch {
            clears_cap: true,
            max_members: Some(8),
            ..FamilyPatch::default()
        }),
        r#"{"max_members":null}"#
    );
    assert_eq!(
        patch(FamilyPatch {
            max_members: Some(8),
            ..FamilyPatch::default()
        }),
        r#"{"max_members":8}"#
    );
    assert_eq!(
        patch(FamilyPatch {
            clears_language: true,
            ..FamilyPatch::default()
        }),
        r#"{"language":null}"#
    );
    assert_eq!(
        patch(FamilyPatch {
            join_policy: Some("approval".to_string()),
            language: Some("ru".to_string()),
            ai_history: Some(true),
            ..FamilyPatch::default()
        }),
        r#"{"join_policy":"approval","language":"ru","ai_history":true}"#
    );
    assert_eq!(patch(FamilyPatch::default()), "{}");
    assert_eq!(
        wire::encode(&AssistantConsentRequest { granted: true }),
        r#"{"granted":true}"#
    );
}

// ---- the answers `ApiClientTests` reads, at the model ----------------------------------------

#[test]
fn the_familys_document_reads_its_capabilities_and_marks() {
    let family: FamilyResponse = wire::decode(
        r#"
        {"family": {"id": 3, "name": "The Smiths", "join_policy": "open", "ai_history": true},
         "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}],
         "blocked_user_ids": [],
         "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai",
                       "draw": "/draw", "vision": true, "images": true}}
        "#,
    )
    .expect("the family's document");
    assert_eq!(family.family.name, "The Smiths");
    assert!(family.has_assistant());
    assert!(family.can_draw_pictures());
    assert_eq!(family.blocked_user_ids, Some(Vec::new()));
    assert_eq!(family.family.max_members, None);
    assert!(family.members.as_deref().expect("members")[0].owner());

    let bare: FamilyResponse =
        wire::decode(r#"{"family": {"id": 3, "name": "The Smiths"}}"#).expect("a document");
    assert!(!bare.has_assistant());
    assert!(!bare.can_draw_pictures());

    let packs: FamilyResponse = wire::decode(
        r#"{"family": {"id": 3, "name": "The Smiths"}, "members": [],
            "max_pack_seq": 14, "max_pack_items": 200, "max_pack_item_bytes": 524288,
            "max_round_video_ms": 60000, "max_round_video_bytes": 4194304}"#,
    )
    .expect("a document");
    assert_eq!(packs.max_pack_seq, Some(14));
    assert_eq!(packs.max_pack_items, Some(200));
    assert_eq!(packs.max_pack_item_bytes, Some(524_288));
    assert_eq!(
        RoundVideoLimits::of(&packs),
        Some(RoundVideoLimits {
            max_ms: 60_000,
            max_bytes: 4_194_304,
        })
    );
    assert_eq!(bare.max_pack_items, None);
    assert_eq!(RoundVideoLimits::of(&bare), None);

    // A document with no family at all is not this document.
    assert_eq!(wire::decode::<FamilyResponse>(r#"{"members": []}"#), None);
}

#[test]
fn the_assistants_newer_fields_arrive_tolerantly() {
    let newer: FamilyResponse = wire::decode(
        r#"
        {"family": {"id": 3, "name": "The Smiths", "ai_transcripts": true, "ai_lookups": true,
                    "ai_greeting": true, "greeting_places": ["Moscow", "Белград"]},
         "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai",
                       "processor": "Azure OpenAI", "transcribe": true, "transcribe_max_bytes": 26214400,
                       "lookups": ["Brave Search", "Open-Meteo", "Wikipedia"], "greeting_weather": true}}
        "#,
    )
    .expect("a document");
    assert!(newer.family.ai_transcripts);
    assert!(newer.family.ai_lookups);
    assert_eq!(
        newer.family.greeting_places,
        Some(vec!["Moscow".to_string(), "Белград".to_string()])
    );
    let assistant = newer.assistant.expect("an assistant");
    assert!(assistant.transcribe);
    assert_eq!(assistant.transcribe_max_bytes, Some(26_214_400));
    assert_eq!(
        assistant.lookups,
        Some(vec![
            "Brave Search".to_string(),
            "Open-Meteo".to_string(),
            "Wikipedia".to_string()
        ])
    );
    assert!(assistant.greeting_weather);

    let older: FamilyResponse = wire::decode(
        r#"{"family": {"id": 3, "name": "The Smiths"},
            "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai"}}"#,
    )
    .expect("a document");
    assert!(!older.family.ai_transcripts);
    assert!(!older.family.ai_lookups);
    assert_eq!(older.family.greeting_places, None);
    let assistant = older.assistant.expect("an assistant");
    assert!(!assistant.transcribe);
    assert_eq!(assistant.transcribe_max_bytes, None);
    assert_eq!(assistant.lookups, None);
    assert!(!assistant.greeting_weather);

    let me: MeResponse = wire::decode(
        r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"},
            "assistant_consent_at": "2026-09-19T08:12:04Z", "assistant_lookup_consent_at": "2026-10-03T09:30:00Z"}"#,
    )
    .expect("GET /me");
    assert_eq!(
        me.assistant_lookup_consent_at.as_deref(),
        Some("2026-10-03T09:30:00Z")
    );
    let withdrawn: AssistantLookupConsentResponse =
        wire::decode(r#"{"assistant_lookup_consent_at": null}"#).expect("an answer");
    assert_eq!(withdrawn.assistant_lookup_consent_at, None);
    let patched: FamilyOnlyResponse =
        wire::decode(r#"{"family": {"id": 3, "name": "The Smiths", "greeting_places": []}}"#)
            .expect("an answer");
    assert_eq!(patched.family.greeting_places, Some(Vec::new()));
}

#[test]
fn the_statistics_and_the_transcript_read_as_the_protocol_writes_them() {
    let stats: StatsResponse = wire::decode(
        r#"
        {"generated_at": "2026-10-03T10:00:00Z",
         "totals": {"members": 2, "messages": 10, "board_notes": 0,
                    "ai": {"questions": 3, "transcripts": 2, "transcript_duration_ms": 95000, "searches": 9}},
         "members": [{"user_id": 7, "display_name": "Anna", "messages": 6, "ai": {"questions": 2, "searches": 4}},
                     {"user_id": 11, "display_name": "Bob", "messages": 4, "ai": {"questions": 1}}]}
        "#,
    )
    .expect("the statistics");
    let totals = stats.totals.ai.expect("the totals' assistant");
    assert_eq!(totals.transcripts, 2);
    assert_eq!(totals.transcript_duration_ms, 95_000);
    assert_eq!(totals.searches, 9);
    let members = stats.members.expect("the rows");
    assert_eq!(members[0].ai.as_ref().expect("Anna's").searches, 4);
    assert_eq!(members[0].ai.as_ref().expect("Anna's").transcripts, 0);
    assert_eq!(members[1].ai.as_ref().expect("Bob's").searches, 0);

    let spoken: TranscriptResponse =
        wire::decode(r#"{"transcript": {"text": "Мы будем в шесть", "language": "ru"}}"#)
            .expect("a transcript");
    let spoken = spoken.transcript.expect("its transcript");
    assert_eq!(spoken.text.as_deref(), Some("Мы будем в шесть"));
    assert_eq!(spoken.language.as_deref(), Some("ru"));
    // SILENCE IS AN ANSWER: the empty text, and no language.
    let silence: TranscriptResponse =
        wire::decode(r#"{"transcript": {"text": ""}}"#).expect("a transcript");
    let silence = silence.transcript.expect("its transcript");
    assert_eq!(silence.text.as_deref(), Some(""));
    assert_eq!(silence.language, None);
    // A malformed one keeps its absence, for the reader to refuse.
    let malformed: TranscriptResponse =
        wire::decode(r#"{"transcript": {"text": null}}"#).expect("a transcript");
    assert_eq!(malformed.transcript.expect("its transcript").text, None);

    let group: ChatResponse = wire::decode(
        r#"{"chat": {"id": 77, "kind": "group", "title": "Cousins", "peer_user_id": null, "member_ids": [7, 9, 11], "admin_user_id": 7}}"#,
    )
    .expect("a group");
    assert_eq!(group.chat.member_ids, Some(vec![7, 9, 11]));
    assert_eq!(group.chat.admin_user_id, Some(7));
    assert_eq!(group.chat.peer_user_id, None);
}

// ---- the upload query and the .NET formatting it leans on ------------------------------------

/// `ToString("F7", InvariantCulture)`, each value run on .NET 10: an exact tie rounds half to
/// EVEN there as it does in Rust, a negative that rounds to zero keeps its sign, and the three
/// values that are not numbers have .NET's names.
#[test]
fn seven_places_are_dotnets_seven_places() {
    for (value, expected) in [
        (0.00390625, "0.0039062"),
        (-0.00390625, "-0.0039062"),
        (0.01171875, "0.0117188"),
        (55.75390625, "55.7539062"),
        (1.5, "1.5000000"),
        (0.125, "0.1250000"),
        (2.675, "2.6750000"),
        (55.7558, "55.7558000"),
        (-0.0, "-0.0000000"),
        (1e20, "100000000000000000000.0000000"),
        (180.0, "180.0000000"),
        (-0.00000001, "-0.0000000"),
        (12345.99999999, "12346.0000000"),
        (0.99999995, "0.9999999"),
        (89.99999995, "90.0000000"),
        (f64::NAN, "NaN"),
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
    ] {
        assert_eq!(fixed(value, 7), expected, "{value:?}");
    }
}

/// `Uri.EscapeDataString`: everything but the unreserved characters, as UTF-8, upper-case hex.
#[test]
fn a_name_is_escaped_as_dotnet_escapes_data() {
    assert_eq!(
        escape_data_string("Café & Co/1 ~x"),
        "Caf%C3%A9%20%26%20Co%2F1%20~x"
    );
    assert_eq!(escape_data_string("a+b=c?d#e"), "a%2Bb%3Dc%3Fd%23e");
    assert_eq!(
        escape_data_string("Привет 😀"),
        "%D0%9F%D1%80%D0%B8%D0%B2%D0%B5%D1%82%20%F0%9F%98%80"
    );
    assert_eq!(escape_data_string("AZaz09-._~"), "AZaz09-._~");
}

/// `UploadQueries.Location`, each case run on .NET 10.
#[test]
fn a_place_is_three_numbers_and_maybe_a_name() {
    for (latitude, longitude, accuracy, name, expected) in [
        (
            55.7558,
            37.6173,
            Some(12.5),
            Some("Café & Co/1 ~x"),
            "kind=location&latitude=55.7558000&longitude=37.6173000&accuracy_m=13&name=Caf%C3%A9%20%26%20Co%2F1%20~x",
        ),
        (
            0.00390625,
            -0.00390625,
            Some(-3.0),
            None,
            "kind=location&latitude=0.0039062&longitude=-0.0039062&accuracy_m=0",
        ),
        (
            1.0,
            2.0,
            Some(f64::NAN),
            Some(""),
            "kind=location&latitude=1.0000000&longitude=2.0000000",
        ),
        (
            1.0,
            2.0,
            Some(1e300),
            Some("a+b=c?d#e"),
            "kind=location&latitude=1.0000000&longitude=2.0000000&accuracy_m=9223372036854775807&name=a%2Bb%3Dc%3Fd%23e",
        ),
        (
            1.0,
            2.0,
            Some(2.5),
            Some("Привет 😀"),
            "kind=location&latitude=1.0000000&longitude=2.0000000&accuracy_m=3&name=%D0%9F%D1%80%D0%B8%D0%B2%D0%B5%D1%82%20%F0%9F%98%80",
        ),
        (
            1.0,
            2.0,
            Some(0.49999999999999994),
            None,
            "kind=location&latitude=1.0000000&longitude=2.0000000&accuracy_m=0",
        ),
        (
            1.0,
            2.0,
            Some(f64::INFINITY),
            None,
            "kind=location&latitude=1.0000000&longitude=2.0000000",
        ),
        (
            1.0,
            2.0,
            None,
            None,
            "kind=location&latitude=1.0000000&longitude=2.0000000",
        ),
    ] {
        assert_eq!(
            upload_queries::location(latitude, longitude, accuracy, name),
            expected
        );
    }
}
