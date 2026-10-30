//! `GreetingWeatherTests`: today's weather in the daily greeting, from this client's side
//! (docs/protocol.md, "Today's weather, for places the owner chose"; issue #72) — who sees the
//! places field and when it may be edited, the fields as they are edited and how they follow the
//! server, the request a save makes — and the credit line under a greeting, drawn as a link that
//! opens.

mod common;

use std::rc::Rc;

use fc_core::protocol::{AssistantDto, FamilyDto, FamilyPatch, MessageDto, UserDto, wire};
use fc_core::store::{ChatStore, Database, StoreResult};
use fc_core::testing::drive;
use fc_logic::app_session::{Gate, SessionState};
use fc_logic::bubble_body::{BodyBlock, BodyRun, lay, openable};
use fc_logic::family::FamilyModel;
use fc_logic::greeting_weather::{PlacesDraft, editable, offered, saved, shown};
use fc_logic::lookups::may_preview;

use common::consent::{assistant, clock, strings};
use common::server::{Server, api};

const ME: i64 = 7;
const ASSISTANT_ID: i64 = 1;

fn weather(on: bool) -> AssistantDto {
    AssistantDto {
        processor: Some("Azure OpenAI".to_string()),
        greeting_weather: on,
        ..assistant(ASSISTANT_ID, "Assistant")
    }
}

fn smiths(greeting: bool, places: Option<&[&str]>) -> FamilyDto {
    FamilyDto {
        join_policy: Some("open".to_string()),
        ai_greeting: greeting,
        greeting_places: places.map(strings),
        ..FamilyDto::new(3, "The Smiths")
    }
}

fn state(gate: Gate, assistant: Option<AssistantDto>, family: Option<FamilyDto>) -> SessionState {
    SessionState {
        me: Some(UserDto::new(ME, "anna", "Anna")),
        family: Some(family.unwrap_or_else(|| smiths(true, None))),
        assistant,
        greetings_enabled: true,
        ..SessionState::new(gate)
    }
}

// ---- who sees it, and when it may be changed ----------------------------------------------------

/// The owner's field, on a server that says it can — like every other AI setting of the family,
/// which a member's screen does not draw at all. An older server sends no `greeting_weather`, which
/// reads as false.
#[test]
fn the_field_is_the_owners_and_only_where_the_server_offers_it() {
    assert!(shown(&state(Gate::Owner, Some(weather(true)), None)));
    assert!(!shown(&state(Gate::Member, Some(weather(true)), None)));
    assert!(!shown(&state(Gate::Owner, Some(weather(false)), None)));
    assert!(!shown(&state(Gate::Owner, None, None)));

    let older: AssistantDto =
        wire::decode(r#"{"user_id": 1, "display_name": "Assistant", "mention": "@ai"}"#)
            .expect("an assistant");
    assert!(!offered(Some(&older)));
    let newer: AssistantDto = wire::decode(
        r#"{"user_id": 1, "display_name": "Assistant", "mention": "@ai", "greeting_weather": true}"#,
    )
    .expect("an assistant");
    assert!(offered(Some(&newer)));
    // Tied to no family switch: lookups off, places still offered.
    let lookups_off = FamilyDto {
        ai_lookups: false,
        ..smiths(true, None)
    };
    assert!(shown(&state(
        Gate::Owner,
        Some(weather(true)),
        Some(lookups_off)
    )));
}

/// Editable with the greeting on or off — the owner may choose the places before turning it on, as
/// on iOS, Android and the web (protocol.md) — and only not while another change of the family's is
/// on its way.
#[test]
fn the_places_are_editable_with_the_greeting_on_or_off_while_nothing_else_is_saving() {
    assert!(editable(true));
    assert!(!editable(false));
}

#[test]
fn the_saved_list_is_the_servers_and_an_absent_one_is_none() {
    assert!(saved(None).is_empty());
    assert!(saved(Some(&smiths(true, None))).is_empty());
    assert_eq!(
        strings(&["Moscow", "Belgrade"]),
        saved(Some(&smiths(true, Some(&["Moscow", "Belgrade"]))))
    );
    let with_null: FamilyDto =
        wire::decode(r#"{"id": 3, "name": "The Smiths", "greeting_places": ["Moscow", null]}"#)
            .expect("a family");
    assert_eq!(strings(&["Moscow"]), saved(Some(&with_null)));
    let older: FamilyDto = wire::decode(r#"{"id": 3, "name": "The Smiths"}"#).expect("a family");
    assert_eq!(None, older.greeting_places);
}

// ---- the fields -------------------------------------------------------------------------------

#[test]
fn at_most_three_fields_are_listed() {
    let mut draft = PlacesDraft::new();
    assert!(draft.can_add());
    assert!(draft.add());
    assert!(draft.add());
    assert!(draft.add());
    assert!(!draft.can_add());
    assert!(!draft.add());
    assert_eq!(3, draft.fields().len());

    assert!(!draft.remove(3));
    assert!(!draft.remove(-1));
    assert!(draft.remove(0));
    assert!(draft.can_add());
}

/// Nothing is sent that changes nothing, a blank field is no place, and a field is typed into as the
/// server keeps it.
#[test]
fn only_a_real_change_is_sent() {
    let mut draft = PlacesDraft::new();
    draft.sync(&strings(&["Moscow"]));
    assert_eq!(strings(&["Moscow"]), draft.fields());
    assert_eq!(None, draft.pending());

    // An empty field added and left is not a change.
    draft.add();
    assert_eq!(None, draft.pending());
    assert_eq!("Bel grade", draft.set(1, "Bel\u{0001} grade"));
    draft.set(1, "  Belgrade  ");
    assert_eq!(Some(strings(&["Moscow", "Belgrade"])), draft.pending());
    // Whitespace only: the same list once folded.
    draft.set(0, "  Moscow ");
    draft.set(1, "");
    assert_eq!(None, draft.pending());

    // Clearing every field is a change: [] clears the list.
    draft.set(0, " ");
    assert_eq!(Some(Vec::new()), draft.pending());

    // A control character typed or pasted is taken out as it is typed; the field never holds one to
    // refuse.
    assert_eq!("Moscow", draft.set(0, "Mos\u{0000}cow"));
    assert_eq!(None, draft.pending());

    // A pasted name padded with whitespace the server folds away is not cut for it: the server keeps
    // "Paris".
    let padded = format!("{}Paris", " ".repeat(79));
    assert_eq!(padded, draft.set(0, &padded));
    assert_eq!(Some(strings(&["Paris"])), draft.pending());
}

/// A redraw is not a reset: the server's list replaces the fields only when it CHANGED and nothing
/// unsaved is in them, and an empty field the owner has just added survives the redraws in between.
#[test]
fn the_servers_list_follows_without_throwing_away_typing() {
    let mut draft = PlacesDraft::new();
    draft.sync(&strings(&["Moscow"]));
    draft.add();
    draft.sync(&strings(&["Moscow"]));
    assert_eq!(strings(&["Moscow", ""]), draft.fields());

    // Another device changed it, and nothing here is unsaved: it is taken — the added field still at
    // the end.
    draft.sync(&strings(&["Paris"]));
    assert_eq!(strings(&["Paris", ""]), draft.fields());

    // Somebody is typing: a new list from elsewhere does not overwrite them, and becomes what they
    // are measured against.
    draft.set(1, "Rome");
    draft.sync(&strings(&["Paris", "Oslo"]));
    assert_eq!(strings(&["Paris", "Rome"]), draft.fields());
    assert_eq!(strings(&["Paris", "Oslo"]), draft.basis());
    assert_eq!(Some(strings(&["Paris", "Rome"])), draft.pending());
}

/// After a save the fields are the list the server KEPT — shorter or respelt — unless the owner
/// changed them while the save was on its way, which is then the next save.
///
/// ADAPTED: the C# sends `["Moscow", "moscow"]` and the server drops the repeat; Linux follows
/// `fc_text::greeting_places::places`, which drops it before sending (linux/docs/carry-forward.md,
/// "Decided while porting") — so what is sent is `["Moscow"]`, and the server keeps the same list.
/// Everything after is the C#'s.
#[test]
fn the_fields_become_what_the_server_kept() {
    let mut draft = PlacesDraft::new();
    draft.add();
    draft.set(0, "Moscow");
    draft.add();
    draft.set(1, "moscow");
    let sent = draft.pending().expect("a change");
    assert_eq!(strings(&["Moscow"]), sent);
    draft.adopt(&sent, &strings(&["Moscow"]));
    assert_eq!(strings(&["Moscow"]), draft.fields());
    assert_eq!(None, draft.pending());

    // Typed into while the save was away: kept, and measured against what the server kept.
    draft.add();
    draft.set(1, "Oslo");
    let second = draft.pending().expect("a change");
    draft.set(1, "Oslo, Norway");
    draft.adopt(&second, &strings(&["Moscow", "Oslo"]));
    assert_eq!(strings(&["Moscow", "Oslo, Norway"]), draft.fields());
    assert_eq!(Some(strings(&["Moscow", "Oslo, Norway"])), draft.pending());

    // A server that predates the key keeps nothing, and the screen says so rather than showing what
    // was sent.
    let third = draft.pending().expect("a change");
    draft.adopt(&third, &[]);
    assert!(draft.fields().is_empty());
}

// ---- the request ------------------------------------------------------------------------------

/// A save is ONE key, the whole list, through the family's own PATCH; the answer's list is what was
/// kept.
#[test]
fn a_save_sends_the_whole_list_and_reads_back_what_was_kept() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let chats = Rc::new(ChatStore::new(&cache).with_me(|| ME));
    let server = Server::new();
    server.on(
        "/families/mine",
        r#"{"family": {"id": 3, "name": "The Smiths", "ai_greeting": true, "greeting_places": ["Moscow", "Belgrade"]}}"#,
    );
    let clock = clock();
    let family = FamilyModel::new(api(&server, &clock), chats);

    let mut draft = PlacesDraft::new();
    draft.sync(&[]);
    draft.add();
    draft.set(0, " Moscow ");
    draft.add();
    draft.set(1, "Belgrade");
    let request = draft.pending().expect("a change");
    let answered = drive(
        &clock,
        family.change(
            &smiths(true, None),
            &FamilyPatch {
                greeting_places: Some(request.clone()),
                ..FamilyPatch::default()
            },
        ),
    )
    .expect("kept");

    assert_eq!(
        vec!["{\"greeting_places\":[\"Moscow\",\"Belgrade\"]}"],
        server.bodies()
    );
    draft.adopt(&request, &saved(Some(&answered)));
    assert_eq!(strings(&["Moscow", "Belgrade"]), draft.fields());

    // Drawn as the server will apply it while the save is away: the list replaced, nothing else
    // touched.
    let applied = FamilyModel::as_applied(
        &smiths(true, Some(&["Paris"])),
        &FamilyPatch {
            greeting_places: Some(strings(&["Oslo"])),
            ..FamilyPatch::default()
        },
    );
    assert_eq!(Some(strings(&["Oslo"])), applied.greeting_places);
    assert!(applied.ai_greeting);
    assert_eq!(
        Some(strings(&["Paris"])),
        FamilyModel::as_applied(
            &smiths(true, Some(&["Paris"])),
            &FamilyPatch {
                ai_greeting: Some(false),
                ..FamilyPatch::default()
            }
        )
        .greeting_places
    );
    Ok(())
}

// ---- the greeting itself ----------------------------------------------------------------------

/// The credit under a greeting that used the weather is plain markdown the server writes: drawn as a
/// link that opens (through `bubble_body::openable`, as every bubble link is), in each of the nine
/// languages' words as the server writes them — and, being the lookups' credit line, it keeps the
/// greeting out of a preview card. (The C#'s theory, one case per language.)
#[test]
fn the_greetings_weather_credit_is_a_link_that_opens() {
    for words in [
        "Weather data by Open-Meteo.com",
        "Wetterdaten von Open-Meteo.com",
        "Datos meteorológicos de Open-Meteo.com",
        "Données météo par Open-Meteo.com",
        "気象データ: Open-Meteo.com",
        "Данные о погоде: Open-Meteo.com",
        "Подаци о времену: Open-Meteo.com",
        "Podaci o vremenu: Open-Meteo.com",
        "天气数据：Open-Meteo.com",
    ] {
        let greeting = format!(
            "Good morning! Moscow, Russia: sunny, 18 °C at most. ♌\n\n[{words}](https://open-meteo.com/)"
        );
        let blocks = lay(&greeting, None, &[], ME, &|_| false);
        let credits: Vec<&BodyRun> = blocks
            .iter()
            .filter_map(BodyBlock::runs)
            .flatten()
            .filter(|run| run.link.is_some())
            .collect();
        assert_eq!(1, credits.len(), "{words}");
        assert_eq!(words, credits[0].text);
        assert_eq!(
            Some("https://open-meteo.com/".to_string()),
            openable(credits[0].link.as_deref())
        );

        assert!(
            fc_core::text::sources_footer::ends_with_footer(Some(&greeting)),
            "{words}"
        );
        let message = MessageDto::new(42, 5, ASSISTANT_ID, None, &greeting, "2026-10-03T07:00:00Z");
        assert!(!may_preview(
            &message,
            &greeting,
            ME,
            false,
            Some(ASSISTANT_ID),
            false
        ));
    }
}
