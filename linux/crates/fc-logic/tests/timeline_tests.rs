//! `TimelineTests`: the web client's timeline tests, ported — day pills, the sender caption, runs,
//! the divider and the opening anchor. Plus one that is not the C#'s: the day pill in all eleven
//! measured cultures, as .NET 10 writes it.

mod common;

use fc_core::protocol::MessageDto;
use fc_logic::conversation::Bubble;
use fc_logic::date_culture::DateCulture;
use fc_logic::timeline::{self, ANCHOR_CAP};
use time::macros::{date, offset};
use time::{Date, UtcOffset};

const ME: i64 = 7;
const ANNA: i64 = 9;
const BOB: i64 = 11;
const UTC: UtcOffset = UtcOffset::UTC;

fn at(day: u8, hour: u8) -> String {
    format!("2026-09-{day:02}T{hour:02}:00:00Z")
}

fn row(id: i64, sender: i64, at: &str) -> Bubble {
    row_hidden(id, sender, at, false)
}

fn row_hidden(id: i64, sender: i64, at: &str, hidden: bool) -> Bubble {
    Bubble::new(
        MessageDto::new(id, 42, sender, None, &format!("message {id}"), at),
        hidden,
        false,
        sender == ME,
    )
}

#[test]
fn each_local_day_starts_a_section_with_its_own_pill() {
    let rows = timeline::rows(
        vec![
            row(1, ANNA, &at(9, 10)),
            row(2, ANNA, &at(9, 11)),
            row(3, ANNA, &at(10, 9)),
        ],
        true,
        ME,
        None,
        &UTC,
    );

    assert_eq!(Some(date!(2026 - 09 - 09)), rows[0].day_above);
    assert_eq!(None, rows[1].day_above);
    assert_eq!(Some(date!(2026 - 09 - 10)), rows[2].day_above);
    // A run never crosses a day: the name shows again after the pill, and the day ends the run.
    assert_eq!(
        vec![true, false, true],
        rows.iter().map(|row| row.shows_sender).collect::<Vec<_>>()
    );
    assert!(rows[1].run_end);
    assert!(rows[2].run_start);
}

/// The day is the READER's: an evening message in New York is the next day's in UTC.
#[test]
fn the_day_is_the_readers_own() {
    let zone = offset!(-5);
    let rows = timeline::rows(
        vec![row(1, ANNA, &at(10, 2)), row(2, ANNA, &at(10, 6))],
        true,
        ME,
        None,
        &zone,
    );
    assert_eq!(Some(date!(2026 - 09 - 09)), rows[0].day_above);
    assert_eq!(Some(date!(2026 - 09 - 10)), rows[1].day_above);
}

/// Family chat only, never mine, never on a hidden row, and only where the sender changes.
#[test]
fn the_sender_shows_where_the_sender_changes_in_the_family_chat() {
    let bubbles = || {
        vec![
            row(1, ANNA, &at(9, 10)),
            row(2, ANNA, &at(9, 10)),
            row(3, ME, &at(9, 10)),
            row_hidden(4, BOB, &at(9, 10), true),
            row(5, ANNA, &at(9, 10)),
        ]
    };
    let rows = timeline::rows(bubbles(), true, ME, None, &UTC);
    assert_eq!(
        vec![true, false, false, false, true],
        rows.iter().map(|row| row.shows_sender).collect::<Vec<_>>()
    );
    assert!(
        !timeline::rows(bubbles(), false, ME, None, &UTC)
            .iter()
            .any(|row| row.shows_sender)
    );

    assert_eq!(
        vec![true, false, true, true, true],
        rows.iter().map(|row| row.run_start).collect::<Vec<_>>()
    );
    assert_eq!(
        vec![false, true, true, true, true],
        rows.iter().map(|row| row.run_end).collect::<Vec<_>>()
    );
}

/// A stamp that does not parse belongs to the day it sits at the end of — no pill, and the run goes
/// on.
#[test]
fn an_unparseable_stamp_joins_the_day_it_follows() {
    let rows = timeline::rows(
        vec![
            row(1, ANNA, &at(9, 10)),
            row(2, ANNA, "not a time"),
            row(3, ANNA, &at(9, 12)),
        ],
        true,
        ME,
        None,
        &UTC,
    );
    assert_eq!(
        vec![true, false, false],
        rows.iter()
            .map(|row| row.day_above.is_some())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        vec![true, false, false],
        rows.iter().map(|row| row.shows_sender).collect::<Vec<_>>()
    );
    assert_eq!(
        None,
        timeline::rows(vec![row(1, ANNA, "nope")], true, ME, None, &UTC)[0].day_above
    );
}

#[test]
fn there_is_at_most_one_divider() {
    let rows = timeline::rows(
        vec![
            row(1, ANNA, &at(9, 10)),
            row(2, ANNA, &at(9, 10)),
            row(2, ANNA, &at(9, 10)),
        ],
        true,
        ME,
        Some(2),
        &UTC,
    );
    assert_eq!(
        vec![false, true, false],
        rows.iter()
            .map(|row| row.unread_divider_above)
            .collect::<Vec<_>>()
    );
    assert!(
        !timeline::rows(vec![row(1, ANNA, &at(9, 10))], true, ME, None, &UTC)
            .iter()
            .any(|row| row.unread_divider_above)
    );
}

fn m(id: i64, sender: i64) -> MessageDto {
    MessageDto::new(id, 42, sender, None, "x", &at(9, 10))
}

/// With a marker: the oldest message from somebody else above it — or nowhere, if not every unread
/// one is held.
#[test]
fn an_opening_anchors_at_the_oldest_unread_above_the_marker() {
    let held = [
        m(1, ANNA),
        m(2, ANNA),
        m(3, ME),
        m(4, ANNA),
        m(5, BOB),
        m(0, ANNA),
    ];
    assert_eq!(None, timeline::open_anchor(&held, 0, 2, ME));
    assert_eq!(Some(4), timeline::open_anchor(&held, 2, 2, ME));
    assert_eq!(Some(4), timeline::open_anchor(&held, 1, 2, ME));
    assert_eq!(None, timeline::open_anchor(&held, 3, 2, ME));
}

/// With no marker: the unread count back from the newest message from somebody else, and nowhere on
/// a short count.
#[test]
fn with_no_marker_the_count_is_counted_back() {
    let held = [m(1, ANNA), m(2, ANNA), m(3, ME), m(4, BOB)];
    assert_eq!(Some(4), timeline::open_anchor(&held, 1, 0, ME));
    assert_eq!(Some(2), timeline::open_anchor(&held, 2, 0, ME));
    assert_eq!(Some(1), timeline::open_anchor(&held, 3, 0, ME));
    assert_eq!(None, timeline::open_anchor(&held, 4, 0, ME));
    // A row the server has not numbered is nobody's unread message, whoever it seems to be from.
    assert_eq!(
        Some(1),
        timeline::open_anchor(&[m(1, ANNA), m(0, ANNA)], 1, 0, ME)
    );
}

/// Never past the window: an anchor 301 rows back is no anchor, 300 is.
#[test]
fn an_anchor_beyond_the_cap_is_none() {
    let mut held = vec![m(1, ANNA)];
    held.extend((2..2 + i64::try_from(ANCHOR_CAP).unwrap()).map(|id| m(id, ME)));
    assert_eq!(Some(1), timeline::open_anchor(&held, 1, 0, ME));
    held.push(m(999, ME));
    assert_eq!(None, timeline::open_anchor(&held, 1, 0, ME));
}

#[test]
fn the_pill_says_today_yesterday_or_the_short_date() {
    let say = common::english();
    let today = date!(2026 - 09 - 17);
    let english = DateCulture::named("en-US").expect("measured");
    assert_eq!("Today", timeline::day_label(today, today, &english, &say));
    assert_eq!(
        "Yesterday",
        timeline::day_label(date!(2026 - 09 - 16), today, &english, &say)
    );
    assert_eq!(
        "Mon, Aug 17",
        timeline::day_label(date!(2026 - 08 - 17), today, &english, &say)
    );
    // Across a month and a year boundary the calendar, not 24 hours, decides.
    assert_eq!(
        "Yesterday",
        timeline::day_label(date!(2025 - 12 - 31), date!(2026 - 01 - 01), &english, &say)
    );
    assert!(
        timeline::day_label(
            date!(2026 - 08 - 17),
            today,
            &DateCulture::named("de-DE").expect("measured"),
            &say
        )
        .starts_with("Mo, 17.")
    );

    assert_eq!("1 new message", timeline::divider_text(1, &say));
    assert_eq!("3 new messages", timeline::divider_text(3, &say));
}

/// Not a C# test: the pill's date in every culture `DateCulture` measured, as .NET 10 (ICU) wrote
/// `$"{GetAbbreviatedDayName(day)}, {date.ToString(MonthDayPattern.Replace("MMMM", "MMM"))}"` — the
/// oracle in the session's scratchpad. "" is the invariant culture.
#[test]
fn the_pill_reads_as_dotnet_writes_it_in_every_culture() {
    const PRINTED: [(&str, Date, &str); 44] = [
        ("", date!(2026 - 08 - 17), "Mon, Aug 17"),
        ("", date!(2026 - 03 - 01), "Sun, Mar 01"),
        ("", date!(2026 - 12 - 24), "Thu, Dec 24"),
        ("", date!(2027 - 05 - 09), "Sun, May 09"),
        ("en-US", date!(2026 - 08 - 17), "Mon, Aug 17"),
        ("en-US", date!(2026 - 03 - 01), "Sun, Mar 1"),
        ("en-US", date!(2026 - 12 - 24), "Thu, Dec 24"),
        ("en-US", date!(2027 - 05 - 09), "Sun, May 9"),
        ("en-GB", date!(2026 - 08 - 17), "Mon, 17 Aug"),
        ("en-GB", date!(2026 - 03 - 01), "Sun, 1 Mar"),
        ("en-GB", date!(2026 - 12 - 24), "Thu, 24 Dec"),
        ("en-GB", date!(2027 - 05 - 09), "Sun, 9 May"),
        ("de-DE", date!(2026 - 08 - 17), "Mo, 17. Aug."),
        ("de-DE", date!(2026 - 03 - 01), "So, 1. März"),
        ("de-DE", date!(2026 - 12 - 24), "Do, 24. Dez."),
        ("de-DE", date!(2027 - 05 - 09), "So, 9. Mai"),
        ("es-ES", date!(2026 - 08 - 17), "lun, 17 de ago"),
        ("es-ES", date!(2026 - 03 - 01), "dom, 1 de mar"),
        ("es-ES", date!(2026 - 12 - 24), "jue, 24 de dic"),
        ("es-ES", date!(2027 - 05 - 09), "dom, 9 de may"),
        ("fr-FR", date!(2026 - 08 - 17), "lun., 17 août"),
        ("fr-FR", date!(2026 - 03 - 01), "dim., 1 mars"),
        ("fr-FR", date!(2026 - 12 - 24), "jeu., 24 déc."),
        ("fr-FR", date!(2027 - 05 - 09), "dim., 9 mai"),
        ("ja-JP", date!(2026 - 08 - 17), "月, 8月17日"),
        ("ja-JP", date!(2026 - 03 - 01), "日, 3月1日"),
        ("ja-JP", date!(2026 - 12 - 24), "木, 12月24日"),
        ("ja-JP", date!(2027 - 05 - 09), "日, 5月9日"),
        ("ru-RU", date!(2026 - 08 - 17), "Пн, 17 авг."),
        ("ru-RU", date!(2026 - 03 - 01), "Вс, 1 марта"),
        ("ru-RU", date!(2026 - 12 - 24), "Чт, 24 дек."),
        ("ru-RU", date!(2027 - 05 - 09), "Вс, 9 мая"),
        ("sr-Cyrl-RS", date!(2026 - 08 - 17), "пон, 17. авг"),
        ("sr-Cyrl-RS", date!(2026 - 03 - 01), "нед, 1. мар"),
        ("sr-Cyrl-RS", date!(2026 - 12 - 24), "чет, 24. дец"),
        ("sr-Cyrl-RS", date!(2027 - 05 - 09), "нед, 9. мај"),
        ("sr-Latn-RS", date!(2026 - 08 - 17), "pon, 17. avg"),
        ("sr-Latn-RS", date!(2026 - 03 - 01), "ned, 1. mar"),
        ("sr-Latn-RS", date!(2026 - 12 - 24), "čet, 24. dec"),
        ("sr-Latn-RS", date!(2027 - 05 - 09), "ned, 9. maj"),
        ("zh-CN", date!(2026 - 08 - 17), "周一, 8月17日"),
        ("zh-CN", date!(2026 - 03 - 01), "周日, 3月1日"),
        ("zh-CN", date!(2026 - 12 - 24), "周四, 12月24日"),
        ("zh-CN", date!(2027 - 05 - 09), "周日, 5月9日"),
    ];
    let say = common::english();
    // A "today" none of the days is, nor the day after.
    let today = date!(2030 - 01 - 15);
    for (name, day, printed) in PRINTED {
        let culture = if name.is_empty() {
            DateCulture::invariant()
        } else {
            DateCulture::named(name).expect("measured")
        };
        assert_eq!(
            printed,
            timeline::day_label(day, today, &culture, &say),
            "{name} {day}"
        );
    }
}
