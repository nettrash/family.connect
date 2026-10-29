//! `DoorwayTests`: the door — which server, who you are, which family — and the one sentence each
//! refusal becomes.
//!
//! Where the C# formatted with `CultureInfo.GetCultureInfo("de-DE")` and the machine's zone, this
//! reads `DateCulture::named("de-DE")` (the same tables, measured) in UTC; a call's wording is
//! `fc_text::call_record`'s, so the Russian bubble sets `fc_text::i18n::use_lang` as the shell does.
//! One test is this port's own: `the_short_date_is_dot_nets_in_every_measured_culture` pins the `d`
//! format the row time needs against the .NET 10 oracle.

mod common;

use std::rc::Rc;

use fc_core::protocol::MemoryTokenStore;
use fc_core::protocol::{
    ApiError, AttachmentDto, CallRecordDto, MemberDto, MessageDto, error_codes,
};
use fc_core::rt::{Clock, Http, TransportError};
use fc_core::store::{ChatStore, Database, StoreResult};
use fc_core::testing::{FakeClock, drive};
use fc_core::text::string_catalog::{EnglishCatalog, StringCatalog};
use fc_logic::app_session::{AppSession, Gate};
use fc_logic::chat_list::{ChatListModel, RowTimeKind};
use fc_logic::conversation::Bubble;
use fc_logic::date_culture::DateCulture;
use fc_logic::doorway::{
    FamilyDoor, bubble_text, door_sentences, row_time_text, server_check, sign_in_form,
};
use time::macros::datetime;
use time::{PrimitiveDateTime, UtcOffset};

use common::server::Server;
use common::session::api_with;

const REFUSAL: &str = r#"{"error": {"code": "unauthorized", "message": "a token is required"}}"#;

const ME_PENDING: &str = r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"},
         "pending_join_request": {"family_id": 3, "family_name": "The Smiths"}}"#;

const ME_OWNER: &str = r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"},
         "family": {"id": 3, "name": "The Smiths"}, "role": "owner"}"#;

fn say() -> EnglishCatalog {
    common::english()
}

fn clock() -> FakeClock {
    FakeClock::at(datetime!(2026-09-12 10:00 UTC))
}

fn probe(server: &Server) -> bool {
    let clock = clock();
    let home = server_check::parse(Some("chat.example.com")).expect("a server");
    let http: Rc<dyn Http> = Rc::new(server.clone());
    let time: Rc<dyn Clock> = Rc::new(clock.clone());
    drive(&clock, server_check::answers(http, home, time))
}

/// "CONNECT" ASKS THE SERVER: the one refusal only this protocol writes — `GET /me` with no token,
/// answered 401 with a code the protocol names.
#[test]
fn a_familys_server_is_the_one_that_refuses_in_the_protocols_own_words() {
    let server = Server::new();
    server.on_status("/me", 401, Some(REFUSAL));

    assert!(probe(&server));
    assert_eq!(vec!["/api/v1/me".to_string()], server.asked());
}

/// Everything else that answers is not a family's server: a login page's 401 (HTML, so no code), a
/// 401 with a code no protocol names, a website that says 200 to anything, a 404, and a host that
/// never answered at all.
#[test]
fn a_login_page_or_a_router_is_not_a_familys_server() {
    let login = Server::new();
    login.on_status("/me", 401, Some("<html><body>Sign in</body></html>"));
    assert!(!probe(&login));
    let unknown = Server::new();
    unknown.on_status(
        "/me",
        401,
        Some(r#"{"error": {"code": "nope", "message": "who are you"}}"#),
    );
    assert!(!probe(&unknown));
    let router = Server::new();
    router.on("/me", "<html>router</html>");
    assert!(!probe(&router));
    let missing = Server::new();
    missing.on_status("/me", 404, None);
    assert!(!probe(&missing));
    let nowhere = Server::new();
    nowhere.on_async("/me", || {
        Box::pin(std::future::ready(Err(TransportError::Unreachable(
            "no route to host".to_string(),
        ))))
    });
    assert!(!probe(&nowhere));
}

#[test]
fn plain_http_is_allowed_and_said_and_a_bare_host_is_never_plain() {
    let parse = |typed: &str| server_check::parse(Some(typed)).expect("a server");
    assert!(server_check::is_plain_text(&parse(
        "http://192.168.1.10:8080"
    )));
    assert!(!server_check::is_plain_text(&parse("192.168.1.10:8080")));
    assert!(!server_check::is_plain_text(&parse(
        "https://chat.example.com/"
    )));
    assert_eq!(None, server_check::parse(Some("   ")));
}

/// A WRONG PASSWORD IS AN ANSWER TO THE FORM, and each other refusal says what it is — with the
/// server's own words only where they came with a code.
#[test]
fn signing_in_says_what_went_wrong() {
    let say = say();
    let said =
        |error: ApiError, registering: bool| door_sentences::sign_in(&error, registering, &say);

    assert_eq!(
        "Wrong username or password.",
        said(
            ApiError::new(error_codes::INVALID_CREDENTIALS, "wrong", 401),
            false
        )
    );
    assert_eq!(
        "The server rejected the request. Try again.",
        said(ApiError::new(error_codes::UNAUTHORIZED, "no", 401), true)
    );
    assert_eq!(
        "That username is taken.",
        said(
            ApiError::new(error_codes::USERNAME_TAKEN, "taken", 409),
            true
        )
    );
    assert_eq!(
        "Can't reach the server. Check your connection.",
        said(ApiError::transport("offline"), false)
    );
    assert_eq!(
        "Try again in a moment.",
        said(
            ApiError::new(error_codes::TOO_MANY_REQUESTS, "slow down", 429),
            false
        )
    );
    assert_eq!(
        "The server had a problem. Try again in a moment.",
        said(ApiError::new(error_codes::INTERNAL, "boom", 500), false)
    );
    assert_eq!(
        "password must be at least 8 characters",
        said(
            ApiError::new(
                error_codes::VALIDATION,
                "password must be at least 8 characters",
                400
            ),
            false
        )
    );
    // A proxy's reason phrase is not the server's words.
    assert_eq!(
        "The server rejected the request.",
        said(ApiError::new("", "Bad Request", 400), false)
    );
    assert_eq!(
        "Something went wrong. Try again.",
        said(ApiError::new(error_codes::NOT_IN_FAMILY, "no", 403), false)
    );
}

#[test]
fn joining_says_what_went_wrong() {
    let say = say();
    let said = |error: ApiError| door_sentences::join(&error, &say, 1, false);

    assert_eq!(
        "That code doesn't match any family. Check it and try again.",
        said(ApiError::new(
            error_codes::INVALID_INVITE_CODE,
            "no such code",
            404
        ))
    );
    assert_eq!(
        "That code doesn't work anymore.",
        said(ApiError::new(error_codes::USER_NOT_FOUND, "gone", 404))
    );
    assert_eq!(
        "You're already in a family.",
        said(ApiError::new(error_codes::ALREADY_IN_FAMILY, "", 409))
    );
    // From inside a family it means THAT family (docs/protocol.md, "Several families").
    assert_eq!(
        "You're already in that family.",
        door_sentences::join(
            &ApiError::new(error_codes::ALREADY_IN_FAMILY, "", 409),
            &say,
            1,
            true
        )
    );
    assert_eq!(
        "You already have a pending request.",
        said(ApiError::new(error_codes::JOIN_REQUEST_PENDING, "", 409))
    );
    assert_eq!(
        "That family is full right now. Ask them to make room, then try the code again.",
        said(ApiError::new(error_codes::FAMILY_FULL, "", 409))
    );
    assert_eq!(
        "invite code must not be empty",
        said(ApiError::new(
            error_codes::VALIDATION,
            "invite code must not be empty",
            400
        ))
    );
    assert_eq!(
        "The server rejected that code.",
        said(ApiError::new("", "Conflict", 409))
    );
    assert_eq!(
        "Can't reach the server. Try again.",
        said(ApiError::transport("offline"))
    );
}

#[test]
fn starting_a_family_says_what_went_wrong() {
    let say = say();
    let said = |error: ApiError| door_sentences::create(&error, &say, 1);

    assert_eq!(
        "This server doesn't take new families.",
        said(ApiError::new(
            error_codes::FAMILY_REGISTRATION_DISABLED,
            "closed",
            403
        ))
    );
    assert_eq!(
        "You're already in a family.",
        said(ApiError::new(error_codes::ALREADY_IN_FAMILY, "", 409))
    );
    assert_eq!(
        "name must be 1-64 characters",
        said(ApiError::new(
            error_codes::VALIDATION,
            "name must be 1-64 characters",
            400
        ))
    );
    assert_eq!(
        "The server rejected that name.",
        said(ApiError::new("", "Bad Request", 400))
    );
    assert_eq!(
        "Can't reach the server. Try again.",
        said(ApiError::transport("offline"))
    );
}

#[test]
fn the_sentences_read_in_the_readers_language() {
    let german = common::catalogues().for_language("de");
    assert_eq!(
        german.get("That code doesn't match any family. Check it and try again."),
        door_sentences::join(
            &ApiError::new(error_codes::INVALID_INVITE_CODE, "", 404),
            &german,
            1,
            false
        )
    );
    assert_ne!(
        "That code doesn't match any family. Check it and try again.",
        door_sentences::join(
            &ApiError::new(error_codes::INVALID_INVITE_CODE, "", 404),
            &german,
            1,
            false
        )
    );
}

struct Door {
    door: FamilyDoor,
    session: Rc<AppSession>,
    clock: FakeClock,
}

fn door(server: &Server) -> StoreResult<Door> {
    let clock = clock();
    let tokens = Rc::new(MemoryTokenStore::new(Some("t0ken")));
    let api = api_with(Rc::new(server.clone()), &clock, tokens.clone());
    let session = Rc::new(AppSession::new(
        api.clone(),
        tokens,
        Database::open_in_memory()?,
    ));
    Ok(Door {
        door: FamilyDoor::new(api, session.clone(), Rc::new(say())),
        session,
        clock,
    })
}

/// THE SCREEN IS DECIDED BY `GET /me`, NOT BY THE ANSWER: a join that came back `pending` is
/// followed by the read that puts the app on the waiting screen, and the code goes up trimmed.
#[test]
fn a_join_is_followed_by_the_read_that_decides_the_screen() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/families/join", r#"{"status": "pending"}"#)
        .on("/me", ME_PENDING);
    let rig = door(&server)?;

    assert_eq!(None, drive(&rig.clock, rig.door.join("  ABCD2345 "))?);

    assert_eq!(Gate::Pending, rig.session.state().gate);
    assert_eq!(
        Some("The Smiths".to_string()),
        rig.session.state().pending_family_name
    );
    assert_eq!(vec!["/api/v1/families/join", "/api/v1/me"], server.asked());
    assert!(server.bodies()[0].contains("\"ABCD2345\""));
    Ok(())
}

#[test]
fn a_refused_join_says_so_and_moves_nothing() -> StoreResult<()> {
    let server = Server::new();
    server.on_status(
        "/families/join",
        409,
        Some(r#"{"error": {"code": "family_full", "message": "full"}}"#),
    );
    let rig = door(&server)?;

    assert_eq!(
        Some(
            "That family is full right now. Ask them to make room, then try the code again."
                .to_string()
        ),
        drive(&rig.clock, rig.door.join("ABCD2345"))?
    );

    assert_eq!(Gate::SignedOut, rig.session.state().gate);
    assert_eq!(vec!["/api/v1/families/join"], server.asked());
    Ok(())
}

#[test]
fn starting_a_family_makes_its_owner_an_owner() -> StoreResult<()> {
    let server = Server::new();
    server
        .on_status(
            "/families",
            201,
            Some(r#"{"family": {"id": 3, "name": "The Smiths"}}"#),
        )
        .on("/me", ME_OWNER);
    let rig = door(&server)?;

    assert_eq!(None, drive(&rig.clock, rig.door.create(" The Smiths "))?);

    assert_eq!(Gate::Owner, rig.session.state().gate);
    assert!(server.bodies()[0].contains("\"The Smiths\""));
    Ok(())
}

#[test]
fn the_forms_send_nothing_they_cannot_send() {
    let limit = FamilyDoor::NAME_LIMIT;
    assert!(!FamilyDoor::may_join(Some("   ")));
    assert!(FamilyDoor::may_join(Some("ABCD2345")));
    assert!(!FamilyDoor::may_create(Some("  ")));
    assert!(FamilyDoor::may_create(Some(&"a".repeat(limit))));
    assert!(!FamilyDoor::may_create(Some(&"a".repeat(limit + 1))));
    // Surrounding space is not part of the name.
    assert!(FamilyDoor::may_create(Some(&format!(
        "  {}  ",
        "a".repeat(limit)
    ))));

    assert!(sign_in_form::may_submit(
        Some("anna"),
        Some("hunter22"),
        None,
        false
    ));
    assert!(!sign_in_form::may_submit(
        Some("anna"),
        Some(""),
        None,
        false
    ));
    assert!(!sign_in_form::may_submit(
        Some(" "),
        Some("hunter22"),
        None,
        false
    ));
    assert!(!sign_in_form::may_submit(
        Some("anna"),
        Some("hunter22"),
        Some(" "),
        true
    ));
    assert!(sign_in_form::may_submit(
        Some("anna"),
        Some("hunter22"),
        Some("Anna"),
        true
    ));
}

#[test]
fn a_rows_time_is_in_the_readers_language() {
    let at = datetime!(2026-09-13 14:05 UTC);
    let german = DateCulture::named("de-DE").expect("measured");
    let american = DateCulture::named("en-US").expect("measured");
    let zone = UtcOffset::UTC;
    let say = say();
    let wall = PrimitiveDateTime::new(at.date(), at.time());

    assert_eq!(
        german.format(wall, "t"),
        row_time_text::format(RowTimeKind::Clock, Some(at), &german, &zone, &say)
    );
    assert_ne!(
        row_time_text::format(RowTimeKind::Clock, Some(at), &american, &zone, &say),
        row_time_text::format(RowTimeKind::Clock, Some(at), &german, &zone, &say)
    );
    assert_eq!(
        "Sonntag",
        row_time_text::format(RowTimeKind::Weekday, Some(at), &german, &zone, &say)
    );
    assert_eq!(
        "13.09.2026",
        row_time_text::format(RowTimeKind::Date, Some(at), &german, &zone, &say)
    );
    let russian = common::catalogues().for_language("ru");
    assert_eq!(
        russian.get("Yesterday"),
        row_time_text::format(RowTimeKind::Yesterday, Some(at), &german, &zone, &russian)
    );
    assert_eq!(
        "",
        row_time_text::format(RowTimeKind::None, None, &german, &zone, &say)
    );
}

/// This port's own (the module's comment): the short date — the chat list's older rows — as .NET 10
/// writes it in each measured culture, printed by the oracle for 2026-09-13.
#[test]
fn the_short_date_is_dot_nets_in_every_measured_culture() {
    let at = datetime!(2026-09-13 14:05);
    for (name, expected) in [
        ("", "09/13/2026"),
        ("en-US", "9/13/2026"),
        ("en-GB", "13/09/2026"),
        ("de-DE", "13.09.2026"),
        ("es-ES", "13/9/2026"),
        ("fr-FR", "13/09/2026"),
        ("ja-JP", "2026/09/13"),
        ("ru-RU", "13.09.2026"),
        ("sr-Cyrl-RS", "13. 9. 2026."),
        ("sr-Latn-RS", "13. 9. 2026."),
        ("zh-CN", "2026/9/13"),
    ] {
        let culture = if name.is_empty() {
            DateCulture::invariant()
        } else {
            DateCulture::named(name).expect("measured")
        };
        assert_eq!(expected, culture.format(at, "d"), "{name}");
    }
}

#[test]
fn a_bubble_says_who_what_and_when() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let chats = Rc::new(ChatStore::new(&cache).with_me(|| 7));
    chats.replace_members(
        &[
            MemberDto::new(7, "anna", "Anna"),
            MemberDto::new(11, "bob", "Bob"),
        ],
        &[MemberDto {
            has_left: true,
            deleted: true,
            ..MemberDto::new(12, "carl", "Carl")
        }],
    )?;
    let say = say();
    let list = ChatListModel::new(chats.clone(), || 7, Rc::new(common::english()));
    const SENT: &str = "2026-09-13T14:05:00Z";
    let of = |sender: i64,
              body: &str,
              hidden: bool,
              revealed: bool,
              edited: Option<&str>,
              media: Option<Vec<AttachmentDto>>| {
        Bubble::new(
            MessageDto {
                edited_at: edited.map(str::to_string),
                attachments: media,
                ..MessageDto::new(1, 42, sender, None, body, SENT)
            },
            hidden,
            revealed,
            sender == 7,
        )
    };

    assert_eq!(
        "You",
        bubble_text::sender(&of(7, "hi", false, false, None, None), &chats, &say)?
    );
    assert_eq!(
        "Bob",
        bubble_text::sender(&of(11, "hi", false, false, None, None), &chats, &say)?
    );
    assert_eq!(
        "Deleted account",
        bubble_text::sender(&of(12, "hi", false, false, None, None), &chats, &say)?
    );

    assert_eq!(
        "Hidden — blocked member",
        bubble_text::words(&of(11, "hi", true, false, None, None), &list, &say)
    );
    assert_eq!(
        "line one\nline two",
        bubble_text::words(
            &of(11, "line one\nline two", false, false, None, None),
            &list,
            &say
        )
    );
    assert_eq!(
        "asked for",
        bubble_text::words(&of(11, "asked for", true, true, None, None), &list, &say)
    );
    assert_eq!(
        "Photo",
        bubble_text::words(
            &of(
                11,
                "",
                false,
                false,
                None,
                Some(vec![AttachmentDto::new(5, "photo")])
            ),
            &list,
            &say
        )
    );
    // A CALL READS AS A CALL: its body is the English placeholder the server writes for clients that
    // predate calls, and a bubble that drew it would be English on every translated screen.
    let call = Bubble::new(
        MessageDto {
            call: Some(CallRecordDto {
                outcome: "missed".to_string(),
                ..CallRecordDto::default()
            }),
            ..MessageDto::new(2, 42, 11, Some("call-1"), "Missed voice call", SENT)
        },
        false,
        false,
        false,
    );
    fc_text::i18n::use_lang(fc_text::i18n::Lang::Ru);
    let russian = Rc::new(common::catalogues().for_language("ru"));
    let in_russian = ChatListModel::new(chats.clone(), || 7, russian.clone());
    assert_ne!(
        "Missed voice call",
        bubble_text::words(&call, &in_russian, russian.as_ref())
    );
    assert_eq!(
        in_russian.preview(&call.message, false),
        bubble_text::words(&call, &in_russian, russian.as_ref())
    );
    fc_text::i18n::use_lang(fc_text::i18n::Lang::En);

    let german = DateCulture::named("de-DE").expect("measured");
    let zone = UtcOffset::UTC;
    let clock = german.format(datetime!(2026-09-13 14:05), "t");
    assert_eq!(
        clock,
        bubble_text::when(
            &of(11, "hi", false, false, None, None).message,
            &german,
            &zone,
            &say
        )
    );
    assert_eq!(
        format!("{clock} · edited"),
        bubble_text::when(
            &of(11, "hi", false, false, Some("2026-09-13T14:06:00Z"), None).message,
            &german,
            &zone,
            &say
        )
    );
    Ok(())
}
