//! Paid families (docs/protocol.md, "Paid families", 2026-10-30; issue #89) on a client that cannot
//! sell: the family door's decision, the unpaid banner, the allowance in Family Statistics, the
//! refusals' sentences, the reader's days — and the session holding what `GET /me` and the
//! `family_plan` frame say, live.

mod common;

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use fc_core::protocol::{
    AiErrorReason, AllowanceDto, AllowanceUseDto, ApiError, ChatDto, ChatRowDto, FamilyDto,
    FrameRouter, FrameSender, MemberDto, MemoryTokenStore, Resync, SendPipeline, ServerFrame,
    SubscriptionDto, error_codes,
};
use fc_core::store::{BoardStore, ChatStore, Database, OutboxStore, StoreResult};
use fc_core::testing::{FakeClock, drive};
use fc_core::text::catalogues::Catalogues;
use fc_core::text::media_text;
use fc_logic::app_session::{AppSession, Gate, SessionState};
use fc_logic::assistant_answers::AssistantAnswers;
use fc_logic::date_culture::DateCulture;
use fc_logic::doorway::door_sentences;
use fc_logic::live_connection::{LiveConnection, LiveParts};
use fc_logic::note_draft::note_sheet_text;
use fc_logic::pack::pack_text;
use fc_logic::paid_families::{self, CreateDoor, Days};
use fc_logic::transcripts::transcript_rules;
use time::macros::datetime;
use time::{OffsetDateTime, UtcOffset};

use common::server::Server;
use common::session::{Pool, Wire, api_with};

fn culture(name: &str) -> DateCulture {
    DateCulture::named(name).expect("a measured culture")
}

fn days_in<'a>(culture: &'a DateCulture, zone: &'a UtcOffset, now: OffsetDateTime) -> Days<'a> {
    Days::new(culture, zone, now)
}

/// The Linux client's own tables (`linux/data/i18n`), as `services::catalogue` loads them.
fn linux(language: &str) -> fc_core::text::catalogues::JsonCatalog {
    let tables = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/i18n");
    Catalogues::from_dir(&tables)
        .expect("the Linux tables load (python3 linux/i18n/generate.py .)")
        .json(language)
        .unwrap_or_else(|| panic!("{language} ships on Linux"))
}

fn family(delete_at: Option<&str>) -> FamilyDto {
    FamilyDto {
        plan: Some("paid".into()),
        delete_at: delete_at.map(str::to_string),
        ..FamilyDto::new(3, "Ivanovs")
    }
}

fn state(gate: Gate, family: FamilyDto) -> SessionState {
    SessionState {
        family: Some(family.clone()),
        families: Some(vec![family]),
        current_family_id: Some(3),
        subscriptions_enabled: true,
        ..SessionState::new(gate)
    }
}

fn roster() -> Vec<MemberDto> {
    vec![
        MemberDto {
            role: Some("member".into()),
            ..MemberDto::new(7, "anna", "Anna")
        },
        MemberDto {
            role: Some("owner".into()),
            ..MemberDto::new(9, "boris", "Boris")
        },
    ]
}

fn active(active: bool) -> Option<SubscriptionDto> {
    Some(SubscriptionDto {
        product_id: "me.nettrash.FamilyConnect.family.monthly".into(),
        environment: "Production".into(),
        expires_at: Some("2026-11-30T00:00:00Z".into()),
        active,
        renews: None,
    })
}

// ---- the reader's days ---------------------------------------------------------------------------

/// A day this year is the day and the month in the culture's own order; a day in another year
/// carries its year — both judged on the READER's wall clock, so 23:30 UTC on the 13th is the 14th
/// three hours east.
#[test]
fn a_day_is_the_readers_with_the_year_only_when_it_is_not_this_one() {
    let now = datetime!(2026-10-30 12:00 UTC);
    let utc = UtcOffset::UTC;
    let moscow = UtcOffset::from_hms(3, 0, 0).expect("an offset");
    let gb = culture("en-GB");
    let us = culture("en-US");
    let de = culture("de-DE");
    let ru = culture("ru-RU");
    let ja = culture("ja-JP");
    let at = datetime!(2026-11-13 23:30 UTC);
    assert_eq!(days_in(&gb, &utc, now).day(at), "13 November");
    assert_eq!(days_in(&gb, &moscow, now).day(at), "14 November");
    assert_eq!(days_in(&us, &utc, now).day(at), "November 13");
    assert_eq!(days_in(&de, &utc, now).day(at), "13. November");
    // Genitive after the day, as the culture writes it.
    assert_eq!(days_in(&ru, &utc, now).day(at), "13 ноября");
    assert_eq!(days_in(&ja, &utc, now).day(at), "11月13日");
    let next_year = datetime!(2027-01-04 09:00 UTC);
    assert_eq!(days_in(&gb, &utc, now).day(next_year), "4 January 2027");
    assert_eq!(days_in(&us, &utc, now).day(next_year), "January 4, 2027");
    assert_eq!(days_in(&ru, &utc, now).day(next_year), "4 января 2027 г.");
    assert_eq!(days_in(&ja, &utc, now).day(next_year), "2027年1月4日");
    // New Year's Eve in Moscow is already next year there.
    let eve = datetime!(2026-12-31 22:00 UTC);
    assert_eq!(days_in(&gb, &utc, now).day(eve), "31 December");
    assert_eq!(days_in(&gb, &moscow, now).day(eve), "1 January 2027");
}

/// The long date without its weekday, in all eleven measured cultures — what .NET 10.0.13 writes
/// for `LongDatePattern` with the weekday taken off (printed by the oracle in this session).
#[test]
fn the_day_month_and_year_read_as_dotnet_writes_them() {
    let vectors: &[(&str, (i32, u8, u8), &str)] = &[
        ("", (2027, 1, 4), "04 January 2027"),
        ("", (2026, 3, 12), "12 March 2026"),
        ("en-US", (2027, 1, 4), "January 4, 2027"),
        ("en-GB", (2027, 1, 4), "4 January 2027"),
        ("de-DE", (2026, 3, 12), "12. März 2026"),
        ("es-ES", (2027, 1, 4), "4 de enero de 2027"),
        ("fr-FR", (2026, 3, 12), "12 mars 2026"),
        ("ja-JP", (2027, 1, 4), "2027年1月4日"),
        ("ru-RU", (2026, 3, 12), "12 марта 2026 г."),
        ("sr-Cyrl-RS", (2027, 1, 4), "4. јануар 2027."),
        ("sr-Latn-RS", (2026, 3, 12), "12. mart 2026."),
        ("zh-CN", (2026, 3, 12), "2026年3月12日"),
    ];
    for (name, (year, month, day), written) in vectors {
        let culture = if name.is_empty() {
            DateCulture::invariant()
        } else {
            culture(name)
        };
        let date = time::Date::from_calendar_date(
            *year,
            time::Month::try_from(*month).expect("a month"),
            *day,
        )
        .expect("a date");
        assert_eq!(
            culture.format(date.midnight(), &culture.day_month_year_pattern()),
            *written,
            "{name}"
        );
    }
}

/// The allowance's renewal is its UTC calendar day — always the 1st, west of Greenwich too, where
/// midnight UTC on 1 December is still 30 November on the wall (docs/protocol.md, "UTC calendar
/// day") — while the banner's `delete_at` is a moment, the day it falls on where the reader is.
#[test]
fn a_renewal_is_the_utc_day_and_a_deadline_the_readers() {
    let us = culture("en-US");
    let new_york = UtcOffset::from_hms(-5, 0, 0).expect("an offset");
    let tokyo = UtcOffset::from_hms(9, 0, 0).expect("an offset");
    let now = datetime!(2026-11-20 12:00 UTC);
    let west = days_in(&us, &new_york, now);
    let east = days_in(&us, &tokyo, now);
    let renews = datetime!(2026-12-01 00:00 UTC);
    assert_eq!(west.utc_day(renews), "December 1");
    assert_eq!(east.utc_day(renews), "December 1");
    assert_eq!(west.day(renews), "November 30");
    let used = AllowanceDto {
        renews_at: Some("2026-12-01T00:00:00Z".into()),
        ..AllowanceDto::default()
    };
    let say = common::english();
    // The statistics' line.
    assert_eq!(
        paid_families::allowance_renews(&used, &say, &west).as_deref(),
        Some("Renews on December 1")
    );
    // The `ai_error` frame's day, and the REST refusal's 1st of the next UTC month.
    let sentence =
        "This family has used this month’s assistant allowance. It renews on December 1.";
    assert_eq!(
        AssistantAnswers::sentence(
            Some(AiErrorReason::AllowanceUsed {
                renews_at: Some(renews)
            }),
            &say,
            &west
        ),
        sentence
    );
    assert_eq!(
        paid_families::refusal(
            &ApiError::new(error_codes::AI_ALLOWANCE_USED, "x", 403),
            &say,
            &west
        )
        .as_deref(),
        Some(sentence)
    );
    // December's renewal is in January, with its year — the reader's year decides that.
    let december = days_in(&us, &new_york, datetime!(2026-12-10 12:00 UTC));
    assert_eq!(
        december.utc_day(datetime!(2027-01-01 00:00 UTC)),
        "January 1, 2027"
    );
    // The banner's deadline stays the reader's day: 02:00 UTC on the 14th is the 13th in New York.
    let unpaid = family(Some("2026-11-14T02:00:00Z"));
    assert!(
        paid_families::banner(&state(Gate::Owner, unpaid), &[], &say, &west)
            .is_some_and(|text| text.contains("deleted on November 13 "))
    );
}

/// A month is the UTC calendar month: the allowance starts again on the 1st of the next one, at
/// midnight UTC — December's in January, and "now" read in UTC whatever offset it came with.
#[test]
fn the_allowance_renews_on_the_first_of_the_next_utc_month() {
    let gb = culture("en-GB");
    let utc = UtcOffset::UTC;
    let next = |now: OffsetDateTime| days_in(&gb, &utc, now).next_month();
    assert_eq!(
        next(datetime!(2026-10-30 12:00 UTC)),
        datetime!(2026-11-01 00:00 UTC)
    );
    assert_eq!(
        next(datetime!(2026-12-31 23:59 UTC)),
        datetime!(2027-01-01 00:00 UTC)
    );
    // 23:30 on 31 October in New York is already 1 November in UTC.
    assert_eq!(
        next(datetime!(2026-10-31 23:30 -4)),
        datetime!(2026-12-01 00:00 UTC)
    );
}

// ---- the family door -----------------------------------------------------------------------------

#[test]
fn the_door_needs_a_subscription_only_where_the_server_charges() {
    let open = SessionState::new(Gate::NoFamily);
    assert_eq!(paid_families::create_door(&open, false), CreateDoor::Open);
    let charging = SessionState {
        subscriptions_enabled: true,
        ..SessionState::new(Gate::NoFamily)
    };
    // No subscription, or one that ended: the sentence, for a first family and for another.
    assert_eq!(
        paid_families::create_door(&charging, false),
        CreateDoor::StartInAppleApp
    );
    assert_eq!(
        paid_families::create_door(&charging, true),
        CreateDoor::StartInAppleApp
    );
    let ended = SessionState {
        subscription: active(false),
        ..charging.clone()
    };
    assert_eq!(
        paid_families::create_door(&ended, false),
        CreateDoor::StartInAppleApp
    );
    // Bought in the Apple app: creating works as before, on this app too.
    let paid = SessionState {
        subscription: active(true),
        ..charging.clone()
    };
    assert_eq!(paid_families::create_door(&paid, false), CreateDoor::Open);
    assert_eq!(paid_families::create_door(&paid, true), CreateDoor::Open);
    // A subscription on a server that does not charge changes nothing either way.
    let free = SessionState {
        subscription: active(false),
        ..SessionState::new(Gate::NoFamily)
    };
    assert_eq!(paid_families::create_door(&free, false), CreateDoor::Open);
}

/// The operator's shut door keeps priority and its own words — and "New Family…" never answered
/// to it (it has its own switch, which decides whether the menu offers it at all).
#[test]
fn a_shut_door_stays_shut_before_any_talk_of_paying() {
    let shut = SessionState {
        family_registration_enabled: false,
        subscriptions_enabled: true,
        ..SessionState::new(Gate::NoFamily)
    };
    assert_eq!(paid_families::create_door(&shut, false), CreateDoor::Closed);
    assert_eq!(
        paid_families::create_door(&shut, true),
        CreateDoor::StartInAppleApp
    );
    let shut_and_free = SessionState {
        family_registration_enabled: false,
        ..SessionState::new(Gate::NoFamily)
    };
    assert_eq!(
        paid_families::create_door(&shut_and_free, false),
        CreateDoor::Closed
    );
    assert_eq!(
        paid_families::create_door(&shut_and_free, true),
        CreateDoor::Open
    );
    assert_eq!(
        paid_families::start_in_apple_app(&common::english()),
        "On this server, a new family is started in the Family Connect app on iPhone, iPad or Mac."
    );
}

// ---- the banner ----------------------------------------------------------------------------------

#[test]
fn the_banner_tells_a_member_who_can_renew_and_the_owner_where() {
    let say = common::english();
    let days = common::days();
    let unpaid = family(Some("2026-11-14T09:00:00Z"));
    assert_eq!(
        paid_families::banner(&state(Gate::Member, unpaid.clone()), &roster(), &say, &days)
            .as_deref(),
        Some(
            "“Ivanovs” will be deleted on November 14 because its subscription has ended. Only Boris, the owner, can renew it."
        )
    );
    assert_eq!(
        paid_families::banner(&state(Gate::Owner, unpaid.clone()), &roster(), &say, &days)
            .as_deref(),
        Some(
            "“Ivanovs” will be deleted on November 14 because your subscription has ended. Renew it in the Family Connect app on iPhone, iPad or Mac."
        )
    );
    // The owner's own banner needs no roster.
    assert!(paid_families::banner(&state(Gate::Owner, unpaid.clone()), &[], &say, &days).is_some());
}

#[test]
fn no_deadline_no_banner() {
    let say = common::english();
    let days = common::days();
    // Paid for, or free: nothing.
    assert_eq!(
        paid_families::banner(&state(Gate::Member, family(None)), &roster(), &say, &days),
        None
    );
    // A deadline that cannot be read is no deadline.
    assert_eq!(
        paid_families::banner(
            &state(Gate::Member, family(Some("next week"))),
            &roster(),
            &say,
            &days
        ),
        None
    );
    // No family shown.
    assert_eq!(
        paid_families::banner(&SessionState::new(Gate::NoFamily), &roster(), &say, &days),
        None
    );
}

/// A member's banner names the owner from the roster — never a former owner's row, and never a
/// sentence with a gap: with no owner in the roster yet, no banner until it arrives.
#[test]
fn a_members_banner_names_the_owner_still_there() {
    let say = common::english();
    let days = common::days();
    let unpaid = state(Gate::Member, family(Some("2026-11-14T09:00:00Z")));
    assert_eq!(
        paid_families::banner(&unpaid, &roster()[..1], &say, &days),
        None
    );
    let mut handed_on = roster();
    handed_on[1].has_left = true;
    handed_on.push(MemberDto {
        role: Some("owner".into()),
        ..MemberDto::new(11, "carl", "Carl")
    });
    assert_eq!(paid_families::owner_name(&handed_on), Some("Carl"));
    assert!(
        paid_families::banner(&unpaid, &handed_on, &say, &days)
            .is_some_and(|text| text.contains("Only Carl, the owner"))
    );
}

/// The banner's sentences in the reader's language, from the tables the Linux client ships — the
/// placeholders where each language's grammar wants them.
#[test]
fn the_banner_reads_in_the_readers_language() {
    let ru = culture("ru-RU");
    let utc = UtcOffset::UTC;
    let days = days_in(&ru, &utc, datetime!(2026-10-30 12:00 UTC));
    let unpaid = family(Some("2026-11-14T09:00:00Z"));
    assert_eq!(
        paid_families::banner(&state(Gate::Member, unpaid), &roster(), &linux("ru"), &days)
            .as_deref(),
        Some(
            "«Ivanovs» будет удалена 14 ноября: её подписка закончилась. Продлить её может только владелец — Boris."
        )
    );
}

// ---- the allowance -------------------------------------------------------------------------------

fn allowance() -> AllowanceDto {
    AllowanceDto {
        renews_at: Some("2026-12-01T00:00:00Z".into()),
        storage_bytes: Some(AllowanceUseDto {
            used: 3_221_225_472,
            limit: 10_737_418_240,
        }),
        ai_tokens: Some(AllowanceUseDto {
            used: 812_345,
            limit: 2_000_000,
        }),
        images: Some(AllowanceUseDto { used: 4, limit: 30 }),
        transcript_ms: Some(AllowanceUseDto {
            used: 600_001,
            limit: 7_200_000,
        }),
    }
}

#[test]
fn the_allowance_is_one_row_per_limit_in_order() {
    let say = common::english();
    let group = |n: i64| n.to_string();
    let rows = paid_families::allowance_rows(&allowance(), &say, ".", &group);
    let size = |bytes: i64| media_text::display_size(bytes, ".");
    assert_eq!(
        rows,
        vec![
            (
                "Storage".to_string(),
                format!("{} of {}", size(3_221_225_472), size(10_737_418_240))
            ),
            ("Assistant".to_string(), "40% used".to_string()),
            ("Pictures".to_string(), "4 of 30".to_string()),
            // 10 minutes and a millisecond is 11 minutes used.
            ("Transcripts".to_string(), "11 of 120 min".to_string()),
        ]
    );
    assert_eq!(
        paid_families::allowance_title(&say),
        "This Month’s Allowance"
    );
    assert_eq!(
        paid_families::allowance_renews(&allowance(), &say, &common::days()).as_deref(),
        Some("Renews on December 1")
    );
}

/// A limit the operator turned off is absent — or present with no limit, which is the same "off" —
/// and has no row; with none at all there is nothing but the renewal line, and with no readable
/// day not even that.
#[test]
fn a_limit_that_is_off_has_no_row() {
    let say = common::english();
    let group = |n: i64| n.to_string();
    let some = AllowanceDto {
        storage_bytes: None,
        ai_tokens: Some(AllowanceUseDto { used: 5, limit: 0 }),
        transcript_ms: None,
        ..allowance()
    };
    let rows = paid_families::allowance_rows(&some, &say, ".", &group);
    assert_eq!(rows, vec![("Pictures".to_string(), "4 of 30".to_string())]);
    let none = AllowanceDto::default();
    assert!(paid_families::allowance_rows(&none, &say, ".", &group).is_empty());
    assert_eq!(
        paid_families::allowance_renews(&none, &say, &common::days()),
        None
    );
}

/// The counts and sizes in the reader's own number format: the decimal separator for sizes and
/// the grouping for counts are the caller's.
#[test]
fn the_allowance_writes_numbers_the_readers_way() {
    let say = common::english();
    let group = |n: i64| {
        if n >= 1000 {
            format!("{}.{:03}", n / 1000, n % 1000)
        } else {
            n.to_string()
        }
    };
    let big = AllowanceDto {
        images: Some(AllowanceUseDto {
            used: 1500,
            limit: 3000,
        }),
        storage_bytes: Some(AllowanceUseDto {
            used: 1_610_612_736,
            limit: 10_737_418_240,
        }),
        ..AllowanceDto::default()
    };
    let rows = paid_families::allowance_rows(&big, &say, ",", &group);
    assert_eq!(
        rows[1],
        ("Pictures".to_string(), "1.500 of 3.000".to_string())
    );
    assert!(rows[0].1.contains(','), "{}", rows[0].1);
}

#[test]
fn the_assistants_share_is_whole_rounded_down_and_capped() {
    assert_eq!(paid_families::percent(812_345, 2_000_000), 40);
    assert_eq!(paid_families::percent(1_991_999, 2_000_000), 99);
    assert_eq!(paid_families::percent(2_000_000, 2_000_000), 100);
    // One answer may overshoot by its own size: still 100.
    assert_eq!(paid_families::percent(2_150_000, 2_000_000), 100);
    assert_eq!(paid_families::percent(0, 2_000_000), 0);
    assert_eq!(paid_families::percent(-5, 2_000_000), 0);
    assert_eq!(paid_families::percent(5, 0), 0);
    assert_eq!(paid_families::percent(i64::MAX, 1), 100);
}

#[test]
fn transcript_minutes_round_used_up_and_the_limit_down() {
    assert_eq!(paid_families::minutes_used(0), 0);
    assert_eq!(paid_families::minutes_used(1), 1);
    assert_eq!(paid_families::minutes_used(60_000), 1);
    assert_eq!(paid_families::minutes_used(60_001), 2);
    assert_eq!(paid_families::minutes_used(-1), 0);
    assert_eq!(paid_families::minutes_limit(7_200_000), 120);
    assert_eq!(paid_families::minutes_limit(7_259_999), 120);
    assert_eq!(paid_families::minutes_limit(59_999), 0);
}

/// The allowance in another language: Japanese puts the limit first, Russian the word first.
#[test]
fn the_allowance_reads_in_the_readers_language() {
    let group = |n: i64| n.to_string();
    let ja = paid_families::allowance_rows(&allowance(), &linux("ja"), ".", &group);
    assert_eq!(ja[2], ("画像".to_string(), "30 中 4".to_string()));
    assert_eq!(ja[3].1, "120 分中 11 分");
    let ru = paid_families::allowance_rows(&allowance(), &linux("ru"), ",", &group);
    assert_eq!(
        ru[1],
        ("Ассистент".to_string(), "Использовано 40%".to_string())
    );
    let de = culture("de-DE");
    let utc = UtcOffset::UTC;
    let days = days_in(&de, &utc, datetime!(2026-10-30 12:00 UTC));
    assert_eq!(
        paid_families::allowance_renews(&allowance(), &linux("de"), &days).as_deref(),
        Some("Erneuert sich am 1. Dezember")
    );
}

// ---- refusals ------------------------------------------------------------------------------------

#[test]
fn each_paid_refusal_has_its_sentence() {
    let say = common::english();
    let days = common::days();
    let refused = |code: &str| ApiError::new(code, "x", 403);
    assert_eq!(
        paid_families::refusal(&refused(error_codes::SUBSCRIPTION_REQUIRED), &say, &days)
            .as_deref(),
        Some("A new family on this server needs a subscription.")
    );
    assert_eq!(
        paid_families::refusal(&refused(error_codes::STORAGE_ALLOWANCE_USED), &say, &days)
            .as_deref(),
        Some("This family has used its storage allowance.")
    );
    // Over REST there is no day in the answer: the 1st of the next month, UTC.
    assert_eq!(
        paid_families::refusal(&refused(error_codes::AI_ALLOWANCE_USED), &say, &days).as_deref(),
        Some("This family has used this month’s assistant allowance. It renews on November 1.")
    );
    assert_eq!(
        paid_families::refusal(&refused(error_codes::STORAGE_FULL), &say, &days),
        None
    );
    assert_eq!(
        paid_families::upload_refusal(error_codes::STORAGE_ALLOWANCE_USED, &say).as_deref(),
        Some("This family has used its storage allowance.")
    );
    assert_eq!(
        paid_families::upload_refusal(error_codes::ATTACHMENT_TOO_LARGE, &say),
        None
    );
}

/// The assistant's failed row says the allowance is used and when it renews — the frame's day,
/// else the 1st of the next month — and remembers it the way it remembers a refusal.
#[test]
fn an_allowance_used_answer_says_when_it_renews() {
    let say = common::english();
    let days = common::days();
    assert_eq!(
        AssistantAnswers::sentence(
            Some(AiErrorReason::AllowanceUsed {
                renews_at: Some(datetime!(2026-12-01 00:00 UTC)),
            }),
            &say,
            &days
        ),
        "This family has used this month’s assistant allowance. It renews on December 1."
    );
    assert_eq!(
        AssistantAnswers::sentence(
            Some(AiErrorReason::AllowanceUsed { renews_at: None }),
            &say,
            &days
        ),
        "This family has used this month’s assistant allowance. It renews on November 1."
    );
    let answers = AssistantAnswers::new();
    let row = fc_core::protocol::MessageDto::new(1339, 42, 1, None, "", "2026-10-30T12:00:00Z");
    answers.stopped(
        42,
        1339,
        Some(AiErrorReason::AllowanceUsed {
            renews_at: Some(datetime!(2026-12-01 00:00 UTC)),
        }),
    );
    assert!(answers.failed(&row));
    for _ in 0..2 {
        assert_eq!(
            answers.failure_sentence(&row, &say, &days).as_deref(),
            Some("This family has used this month’s assistant allowance. It renews on December 1.")
        );
    }
}

/// The same sentence where the REST code arrives: a transcript, a board backdrop. Final — asking
/// again before the 1st gets the same answer.
#[test]
fn a_transcript_or_a_backdrop_past_the_allowance_says_so() {
    let say = common::english();
    let days = common::days();
    let used = ApiError::new(error_codes::AI_ALLOWANCE_USED, "x", 403);
    let sentence =
        "This family has used this month’s assistant allowance. It renews on November 1.";
    assert_eq!(
        transcript_rules::failure_sentence(&used, &say, &days),
        sentence
    );
    assert!(!transcript_rules::may_retry(&used));
    assert_eq!(
        note_sheet_text::backdrop_failure(&used, &say, &days),
        sentence
    );
}

/// Every upload door says the storage allowance the same way: a message's file (the failed row), a
/// board photo, a sticker. And the family door says `subscription_required` in its own line.
#[test]
fn every_upload_door_says_the_storage_allowance() {
    let say = common::english();
    let full = ApiError::new(error_codes::STORAGE_ALLOWANCE_USED, "x", 403);
    let sentence = "This family has used its storage allowance.";
    assert_eq!(note_sheet_text::failure(&full, &say), sentence);
    assert_eq!(pack_text::sentence(&full, &say), sentence);
    assert_eq!(
        door_sentences::create(
            &ApiError::new(error_codes::SUBSCRIPTION_REQUIRED, "x", 403),
            &say,
            1
        ),
        "A new family on this server needs a subscription."
    );
}

// ---- the cap picker ------------------------------------------------------------------------------

#[test]
fn the_cap_pickers_range_is_the_familys_ceiling_else_the_servers() {
    let ceiling = |member_ceiling: Option<i32>| SessionState {
        max_family_members: 50,
        family: Some(FamilyDto {
            member_ceiling,
            ..FamilyDto::new(3, "Ivanovs")
        }),
        ..SessionState::new(Gate::Owner)
    };
    assert_eq!(ceiling(Some(25)).member_ceiling(), 25);
    assert_eq!(ceiling(None).member_ceiling(), 50);
    assert_eq!(ceiling(Some(0)).member_ceiling(), 50);
    assert_eq!(
        SessionState {
            max_family_members: 12,
            ..SessionState::new(Gate::NoFamily)
        }
        .member_ceiling(),
        12
    );
}

// ---- the session ---------------------------------------------------------------------------------

const ME: &str = r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "blocked_user_ids": [],
    "member_families_enabled": true, "max_families_per_account": 5, "max_family_members": 50"#;

fn me(families: &str, paid: &str) -> String {
    format!(
        r#"{ME}, "families": [{families}], "pending_join_requests": [], "invitations": []{paid}}}"#
    )
}

const UNPAID: &str = r#"{"id": 3, "name": "Ivanovs", "role": "member", "plan": "paid", "member_ceiling": 25, "delete_at": "2026-11-14T09:00:00Z"}"#;
const COVERED: &str =
    r#"{"id": 3, "name": "Ivanovs", "role": "member", "plan": "paid", "member_ceiling": 25}"#;
const SMITHS: &str = r#"{"id": 5, "name": "The Smiths", "role": "owner"}"#;
const CHARGING: &str = r#", "subscriptions_enabled": true, "unpaid_days": 14,
    "paid_family_limits": {"members": 25, "images": 30},
    "subscription": {"product_id": "x", "environment": "Sandbox", "expires_at": "2026-12-01T00:00:00Z", "active": true}"#;

fn session_over(server: &Server, cache: &Rc<Database>) -> (Rc<AppSession>, FakeClock) {
    let clock = FakeClock::at(datetime!(2026-10-30 10:00 UTC));
    let tokens = Rc::new(MemoryTokenStore::new(Some("t0ken")));
    let api = api_with(Rc::new(server.clone()), &clock, tokens.clone());
    (Rc::new(AppSession::new(api, tokens, cache.clone())), clock)
}

#[test]
fn me_tells_the_session_what_this_server_charges() -> StoreResult<()> {
    let server = Server::new();
    server.on("/me", &me(UNPAID, CHARGING));
    let (session, clock) = session_over(&server, &Database::open_in_memory()?);
    assert_eq!(None, drive(&clock, session.refresh())?);
    let state = session.state();
    assert!(state.subscriptions_enabled);
    assert!(state.subscription_active());
    assert_eq!(state.unpaid_days, Some(14));
    assert_eq!(
        state.paid_family_limits.as_ref().and_then(|l| l.images),
        Some(30)
    );
    let shown = state.family.as_ref().expect("a family");
    assert_eq!(shown.delete_at.as_deref(), Some("2026-11-14T09:00:00Z"));
    assert_eq!(state.member_ceiling(), 25);
    // A server from before: nothing charged.
    let older = Server::new();
    older.on("/me", &me(COVERED, ""));
    let (session, clock) = session_over(&older, &Database::open_in_memory()?);
    drive(&clock, session.refresh())?;
    assert!(!session.state().subscriptions_enabled);
    assert!(!session.state().subscription_active());
    Ok(())
}

/// A deadline that went while no frame reached this device goes at the next `GET /me`, though the
/// session keeps the shown family's fuller document across reads.
#[test]
fn a_read_without_the_deadline_clears_it() -> StoreResult<()> {
    let server = Server::new();
    server.then(
        "/me",
        &[
            (200, Some(&me(UNPAID, CHARGING))),
            (200, Some(&me(COVERED, CHARGING))),
        ],
    );
    let (session, clock) = session_over(&server, &Database::open_in_memory()?);
    drive(&clock, session.refresh())?;
    assert!(session.state().family.and_then(|f| f.delete_at).is_some());
    drive(&clock, session.refresh())?;
    let state = session.state();
    assert_eq!(
        state.family.as_ref().and_then(|f| f.delete_at.clone()),
        None
    );
    assert_eq!(state.all_families()[0].delete_at, None);
    Ok(())
}

/// `family_plan` sets and clears the deadline of the family it names — in the list, and on screen
/// when it is the one shown — and changes nothing for a family this account is not in.
#[test]
fn a_family_plan_frame_is_applied_to_the_family_it_names() -> StoreResult<()> {
    let server = Server::new();
    server.on("/me", &me(&format!("{COVERED}, {SMITHS}"), CHARGING));
    let (session, clock) = session_over(&server, &Database::open_in_memory()?);
    drive(&clock, session.refresh())?;
    let published = Rc::new(Cell::new(0));
    let count = Rc::clone(&published);
    session
        .on_changed()
        .subscribe(move |_| count.set(count.get() + 1));

    session.apply_family_plan(3, Some("paid"), Some("2026-11-14T09:00:00Z"));
    let state = session.state();
    assert_eq!(
        state.family.as_ref().and_then(|f| f.delete_at.as_deref()),
        Some("2026-11-14T09:00:00Z")
    );
    assert_eq!(
        state.all_families()[0].delete_at.as_deref(),
        Some("2026-11-14T09:00:00Z")
    );
    assert_eq!(state.all_families()[1].delete_at, None);
    assert_eq!(published.get(), 1);
    // The same again changes nothing, and publishes nothing.
    session.apply_family_plan(3, Some("paid"), Some("2026-11-14T09:00:00Z"));
    assert_eq!(published.get(), 1);
    // The other family, not shown: listed, the shown one untouched.
    session.apply_family_plan(5, Some("paid"), Some("2026-11-20T00:00:00Z"));
    let state = session.state();
    assert_eq!(
        state.all_families()[1].delete_at.as_deref(),
        Some("2026-11-20T00:00:00Z")
    );
    assert_eq!(state.family.as_ref().map(|f| f.id), Some(3));
    // Covered again: no `delete_at` clears it; an absent plan keeps the held one.
    session.apply_family_plan(3, None, None);
    let state = session.state();
    assert_eq!(
        state.family.as_ref().and_then(|f| f.delete_at.clone()),
        None
    );
    assert_eq!(
        state.family.as_ref().and_then(|f| f.plan.as_deref()),
        Some("paid")
    );
    // A family this account is not in.
    let before = published.get();
    session.apply_family_plan(99, Some("paid"), Some("2026-11-14T09:00:00Z"));
    assert_eq!(published.get(), before);
    Ok(())
}

/// The live connection hands the frame to the session: the banner's deadline comes and goes with
/// no read at all.
#[test]
fn the_live_connection_applies_family_plan_frames() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let clock = FakeClock::at(datetime!(2026-10-30 10:00 UTC));
    let pool = Pool::new(&clock);
    let server = Server::new();
    server.on("/me", &me(COVERED, CHARGING));
    let tokens = Rc::new(MemoryTokenStore::new(Some("t0ken")));
    let api = api_with(Rc::new(server), &clock, tokens.clone());
    let session = Rc::new(AppSession::new(api.clone(), tokens, cache.clone()));
    pool.run(session.refresh())?;
    let chats = Rc::new(ChatStore::new(&cache).with_me(|| 7));
    let board = Rc::new(BoardStore::new(&cache));
    chats.replace(
        &[ChatRowDto::new(ChatDto::new(42, "family", "Ivanovs"))],
        None,
    )?;
    let socket = Wire::new();
    let sender: Rc<dyn FrameSender> = Rc::new(socket.clone());
    let sending = Rc::new(SendPipeline::from_api(
        sender,
        Rc::new(OutboxStore::new(&cache)),
        chats.clone(),
        api.clone(),
        Rc::new(clock.clone()),
    ));
    let _live = LiveConnection::new(
        session.clone(),
        LiveParts {
            socket: Rc::new(socket.clone()),
            sending,
            router: Rc::new(FrameRouter::new(chats.clone(), board.clone(), None)),
            resync: Rc::new(Resync::new(api, chats, board)),
        },
        pool.spawn(),
    );
    let deadline = || session.state().family.and_then(|family| family.delete_at);
    socket.hear(
        &ServerFrame::parse(
            r#"{"type": "family_plan", "family_id": 3, "plan": "paid", "delete_at": "2026-11-14T09:00:00Z"}"#,
        )
        .expect("a frame"),
    );
    assert_eq!(deadline().as_deref(), Some("2026-11-14T09:00:00Z"));
    socket.hear(
        &ServerFrame::parse(r#"{"type": "family_plan", "family_id": 3, "plan": "paid"}"#)
            .expect("a frame"),
    );
    assert_eq!(deadline(), None);
    Ok(())
}
