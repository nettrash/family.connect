//! The assistant's one unprompted message a day (docs/protocol.md, "The
//! daily greeting").
//!
//! Everything else the assistant says is the second half of an exchange a
//! member started: a message in its own `ai` chat, or an `@ai` mention in the
//! family chat. This is the only thing it says with nobody's act behind it,
//! and that is what shapes every decision in this file.
//!
//! **It is not built on the mention path, and that is deliberate.** Reusing
//! [`crate::handlers_ai::answer`] would have been the obvious move and is
//! wrong three times over:
//!
//! - it binds a fresh `Uuid::new_v4()` as the message's `client_msg_id`, so
//!   two attempts — a restart, a redeploy, an overlapping tick, a second
//!   process against one database — insert two greetings;
//! - it derives the family for `ai_usage` from the asking user's row, and the
//!   assistant's `family_id` is NULL, so a greeting posted "as" the assistant
//!   would record no usage row at all and the daily bill would be invisible
//!   in Family Statistics;
//! - it writes an EMPTY message row first and streams into it, so a provider
//!   failure leaves a blank bubble that every client draws as an assistant
//!   still thinking — for ever, with no member's action to explain it and no
//!   way for anybody to remove it.
//!
//! So the shape here is [`crate::handlers_call::record_call`]'s instead:
//! compose the whole thing first, insert once with a DETERMINISTIC
//! `client_msg_id`, and deliver only what actually got written.
//!
//! **Two keys, and both must turn.** The operator's `[greetings]` section and
//! the family owner's `ai_greeting` column. One server can host several
//! families, so neither party may decide for the other.
//!
//! **It never pushes.** No timezone is stored for a family and none travels
//! on the wire, so the server cannot know whose night its configured hour
//! falls in; there are no quiet hours in this protocol; and the assistant
//! cannot be blocked or muted. A greeting that woke a phone would be an alarm
//! with no off switch.

use anyhow::Result;
use sqlx::Row;
use time::{Date, OffsetDateTime, Time};
use tracing::{info, warn};
use uuid::Uuid;

use crate::error::ApiError;
use crate::events;
use crate::models::Message;
use crate::state::AppState;

/// The ticker's period, and the reason it is a MINUTE and aligned to one.
///
/// The due window for a configured `hh:mm` is `[hh:mm:00, 24:00)` on that UTC
/// date and resets at midnight (`moment_passed`). A ticker with a fixed
/// five-minute period lands on the same seconds-of-day every day — 86 400 is
/// 288 × 300 — so a configured time inside the last five minutes of the day
/// had a window SHORTER than the period, and whether any tick ever fell into
/// it depended on the second the process happened to start. An operator in
/// UTC+9 who set 23:57 for a 08:57 morning could be greeted every day or
/// never, and a restart would silently flip which. Refusing those minutes in
/// `Config::validate` was the other fix; it is the wrong one, because
/// 23:56-23:59 UTC is an ordinary morning for a third of the world.
///
/// So the sweeper sleeps until the NEXT MINUTE BOUNDARY rather than for a
/// fixed period, and re-aligns every tick. A tick then lands at every
/// `hh:mm:00` (plus scheduler slack of milliseconds), which is at or after
/// every configured moment on the day it is configured for — including
/// `23:59`, whose window is exactly one period. Worst-case lateness is under
/// a minute, and it never drifts.
///
/// The cost is one clock read a minute and, after the hour has passed, one
/// indexed `NOT EXISTS` query a minute against a handful of families. That
/// is nothing.
const TICK: std::time::Duration = std::time::Duration::from_secs(60);

/// How long to sleep so the next tick lands just after the next minute
/// boundary. Its own function so the alignment can be pinned by a test.
fn until_next_minute(now: OffsetDateTime) -> std::time::Duration {
    let into_minute = u64::from(now.second()) * 1_000_000_000 + u64::from(now.nanosecond());
    let period = TICK.as_nanos() as u64;
    // Never zero: a tick that lands exactly on :00 would otherwise spin.
    let remaining = period - (into_minute % period);
    std::time::Duration::from_nanos(remaining) + std::time::Duration::from_millis(50)
}

/// The whole instruction, and every constraint in the protocol section
/// expressed as a sentence the model is actually given.
///
/// The negatives are not decoration. "No facts about the date" is the one
/// that matters most: the model has no retrieval of any kind, so an
/// on-this-day line is a daily opportunity for a confident falsehood posted
/// unattended into a family chat nobody reviews — and one that then feeds
/// back into the transcript every later mention reads.
///
/// "No season, no weather, no daylight" is the same rule from the other
/// side. An earlier draft invited the model to write about "the season, the
/// light" — and no timezone is stored for a family and no location either,
/// so a September greeting about autumn is simply wrong for every family in
/// the southern hemisphere, every day, said with confidence.
const GREETING_INSTRUCTION: &str = "\
You are writing the morning greeting for one family's private chat. Nobody asked for it; it \
simply appears, so it must be short and worth the interruption. Two or three sentences at most.

Write something warm about the day — the part of the week it is is all you know about it — and \
then, if star signs are named below, one light, kind line for each of them together. Keep the whole \
thing gentle and unhurried. No greeting, sign-off or signature line, no emoji spam, no questions, \
and never address anyone by name.

Do NOT state any fact about this date: no anniversaries, no events, no birthdays, no \
'on this day in history', no news. Do NOT name a season, describe the weather or the daylight, or \
assume a hemisphere: you do not know where this family lives. You have no way to look anything \
up, and this message is read by a family that will believe you. Say nothing that could be wrong.";

/// The signs, in calendar order, with the day each begins.
///
/// Tropical dates, the ones every newspaper column uses. They are fixed here
/// rather than computed because the family is reading a horoscope, not an
/// ephemeris — and because a table a reader can check against a newspaper is
/// worth more than an astronomically defensible one nobody can.
const SIGNS: [(&str, u8, u8); 12] = [
    ("Capricorn", 12, 22),
    ("Aquarius", 1, 20),
    ("Pisces", 2, 19),
    ("Aries", 3, 21),
    ("Taurus", 4, 20),
    ("Gemini", 5, 21),
    ("Cancer", 6, 21),
    ("Leo", 7, 23),
    ("Virgo", 8, 23),
    ("Libra", 9, 23),
    ("Scorpio", 10, 23),
    ("Sagittarius", 11, 22),
];

/// The sign a birthday falls in.
///
/// Takes the month and day the family already stores — never a year, because
/// there is not one to take (docs/protocol.md, "Birthdays"): a birthday here
/// is a day and a month precisely so that being wished a happy birthday never
/// means publishing your age.
pub fn sign_for(month: u8, day: u8) -> Option<&'static str> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // The sign that started most recently at or before this date. December's
    // Capricorn is the wrap-around: a January date is before every start in
    // the table except its own, and falls through to it.
    let mut best: Option<(&'static str, u8, u8)> = None;
    for &(name, start_month, start_day) in &SIGNS {
        let started = (start_month, start_day) <= (month, day);
        if started
            && best.is_none_or(|(_, best_month, best_day)| {
                (best_month, best_day) < (start_month, start_day)
            })
        {
            best = Some((name, start_month, start_day));
        }
    }
    Some(best.map_or("Capricorn", |(name, _, _)| name))
}

/// The `client_msg_id` a greeting carries, derived from the date alone.
///
/// This is what makes the whole job idempotent with no bookkeeping table and
/// no "last run" row to fall out of step with the messages themselves: the
/// send-deduplication rule the protocol already has — one message per
/// `(chat_id, sender_id, client_msg_id)` — turns a second attempt into a
/// no-op that inserts nothing and delivers nothing.
///
/// It does NOT depend on the chat, and does not need to: `chat_id` is already
/// part of the key, so one id per day is unique per family for free. It does
/// not depend on the family either, which is what lets the "who still needs
/// today's greeting" query bind a single value.
///
/// Built by hand rather than with `Uuid::new_v5`, which would mean adding the
/// `v5` feature and a SHA-1 implementation to hash four bytes that are not
/// secret. The bytes spell `fcgreet` and then the date, so the row can be
/// read straight out of the database with no tool.
///
/// A member cannot collide with it: `sender_id` is part of the key, and only
/// the assistant's row ever sends one of these.
pub fn client_msg_id_for(date: Date) -> Uuid {
    let mut bytes = [0u8; 16];
    bytes[..7].copy_from_slice(b"fcgreet");
    bytes[8..10].copy_from_slice(&(date.year() as i16).to_be_bytes());
    bytes[10] = u8::from(date.month());
    bytes[11] = date.day();
    Uuid::from_bytes(bytes)
}

/// Has the configured moment passed, on the given day?
///
/// The greeting CATCHES UP rather than being confined to a window. A server
/// that was down at 06:00 and comes back at 09:00 still posts that morning's
/// greeting, because a family that gets nothing has no way to tell a quiet
/// day from a broken server — and the deduplication above means catching up
/// can never post twice. It follows that a family switching `ai_greeting` on
/// in the afternoon is greeted that same afternoon, which is the right
/// answer to "turn it on": something happens.
fn moment_passed(now: OffsetDateTime, hour_utc: u8, minute: u8) -> bool {
    let Ok(configured) = Time::from_hms(hour_utc, minute, 0) else {
        // Refused by Config::validate at startup, so unreachable in a running
        // server; treated as "not yet" rather than panicking in a background
        // task nobody is watching.
        return false;
    };
    now.time() >= configured
}

/// One family that still needs today's greeting.
struct Candidate {
    family_id: i64,
    chat_id: i64,
    language: Option<String>,
}

/// Post the day's greetings. Returns how many were actually written.
///
/// A `pub async fn` over `&AppState` returning a count, exactly like the three
/// sweeps: the loop in `main.rs` is three lines and untestable, and the WORK
/// is a function an integration test can call directly.
pub async fn post_daily_greetings(state: &AppState) -> Result<u64, ApiError> {
    post_daily_greetings_at(state, OffsetDateTime::now_utc()).await
}

/// The job, with the clock handed in.
///
/// `now` is read ONCE, here, and threaded through everything that needs a
/// date — the due check, the dedup id and the line the model is told — so all
/// three name the same day even across midnight. It is also what makes the
/// job testable at any wall-clock time: an integration test hands it 05:59
/// and then 06:00 on a fixed date rather than skipping itself for an hour a
/// day, which is what the first version of that test did.
pub async fn post_daily_greetings_at(
    state: &AppState,
    now: OffsetDateTime,
) -> Result<u64, ApiError> {
    let cfg = &state.cfg;
    if !cfg.greetings.is_usable() || !cfg.ai.is_usable() {
        return Ok(0);
    }
    if !moment_passed(now, cfg.greetings.hour_utc, cfg.greetings.minute) {
        return Ok(0);
    }
    let Some(assistant_id) = crate::handlers_ai::assistant_user_id(state).await? else {
        return Ok(0);
    };
    let today = now.date();
    let client_msg_id = client_msg_id_for(today);

    // One query for "who still needs one", rather than a check per family per
    // tick. The NOT EXISTS rides the same `messages_dedup_uq` index the
    // insert below conflicts on.
    let rows = sqlx::query(
        "SELECT f.id AS family_id, f.language, c.id AS chat_id
           FROM families f
           JOIN chats c ON c.family_id = f.id AND c.kind = 'family'
          WHERE f.ai_greeting
            AND NOT EXISTS (
                SELECT 1 FROM messages m
                 WHERE m.chat_id = c.id
                   AND m.sender_id = $1
                   AND m.client_msg_id = $2)
          ORDER BY f.id",
    )
    .bind(assistant_id)
    .bind(client_msg_id)
    .fetch_all(&state.pool)
    .await?;

    let candidates: Vec<Candidate> = rows
        .iter()
        .map(|row| Candidate {
            family_id: row.get("family_id"),
            chat_id: row.get("chat_id"),
            language: row.get("language"),
        })
        .collect();

    let mut written = 0_u64;
    for candidate in candidates {
        match greet(state, assistant_id, &candidate, client_msg_id, today).await {
            // A family with no language and no operator default is skipped in
            // silence rather than greeted in a language nobody chose.
            Ok(false) => {}
            Ok(true) => written += 1,
            // One family's failure must not cost the others theirs.
            Err(error) => {
                warn!(
                    family_id = candidate.family_id,
                    ?error,
                    "could not post the daily greeting"
                );
            }
        }
    }
    Ok(written)
}

/// Compose and post one family's greeting. `false` means "deliberately not
/// greeted", which is not an error.
async fn greet(
    state: &AppState,
    assistant_id: i64,
    candidate: &Candidate,
    client_msg_id: Uuid,
    today: Date,
) -> Result<bool, ApiError> {
    // The family's language, then the operator's, then nothing at all. Never
    // English by default: docs/protocol.md, "The family's language" spends a
    // paragraph on why an unset language is not English, and a greeting has no
    // asking device to fall back to.
    let tag = candidate
        .language
        .as_deref()
        .or(state.cfg.greetings.language.as_deref());
    let Some(language) = tag.and_then(|tag| crate::handlers_ai::language_instruction(Some(tag)))
    else {
        return Ok(false);
    };

    let signs = signs_in_family(state, candidate.family_id).await?;
    // Signs only — no names, no birth dates, no roster. What travels is at
    // most twelve words and nothing that identifies anybody.
    let ask = if signs.is_empty() {
        format!("Today is {}. No star signs to mention.", today_line(today))
    } else {
        format!(
            "Today is {}. The star signs in this family are: {}.",
            today_line(today),
            signs.join(", ")
        )
    };

    // The language line goes LAST, as it does in every other prompt this
    // server composes: two adjacent instructions about language read as a
    // contradiction, and the later one wins.
    let system_prompt = format!("{GREETING_INSTRUCTION}\n\n{language}");
    let turns = [crate::ai::ChatTurn::user(ask)];

    let route = state.cfg.ai.text_route();
    // Composed in FULL before anything is written — the deltas go nowhere.
    // This is the difference from the mention path that keeps a provider
    // failure from leaving a permanent empty bubble in the family chat.
    let streamed =
        crate::ai::stream_reply(&state.http, &route, &system_prompt, &turns, &[], |_| {}).await?;
    let body = streamed.text.trim();
    if body.is_empty() {
        return Ok(false);
    }

    let inserted = sqlx::query(
        "INSERT INTO messages (chat_id, sender_id, client_msg_id, body)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (chat_id, sender_id, client_msg_id) DO NOTHING
         RETURNING id, chat_id, sender_id, client_msg_id, body, created_at",
    )
    .bind(candidate.chat_id)
    .bind(assistant_id)
    .bind(client_msg_id)
    .bind(body)
    .fetch_optional(&state.pool)
    .await?;

    // Another tick, another process, or a restart got there between the query
    // above and this insert. Nothing was written, so nothing is delivered.
    let Some(row) = inserted else {
        return Ok(false);
    };

    let message = Message::from_row(&row);
    // Fanned out, never pushed — see the module header.
    events::log_fanout_error(
        "delivering a daily greeting",
        events::deliver_message_without_push(state, &message, None).await,
    );

    // What it cost, attributed to the FAMILY and to the assistant's own row.
    // `handlers_ai::answer` derives the family from the asking user, which
    // cannot work here: the assistant belongs to no family, so deriving would
    // silently record nothing and hide the one recurring cost on the server.
    // Best effort, exactly as it is there: a greeting the family can already
    // read must not fail because a counter did not save.
    let message_id: i64 = row.get("id");
    if let Err(error) = sqlx::query(
        "INSERT INTO ai_usage
             (user_id, family_id, message_id, prompt_tokens, completion_tokens, images)
         VALUES ($1, $2, $3, $4, $5, 0)",
    )
    .bind(assistant_id)
    .bind(candidate.family_id)
    .bind(message_id)
    .bind(streamed.usage.prompt_tokens)
    .bind(streamed.usage.completion_tokens)
    .execute(&state.pool)
    .await
    {
        warn!(?error, "could not record the greeting's usage");
    }
    Ok(true)
}

/// The date, written out, for the one line the model is told — the SAME
/// date the dedup id was derived from, not a second reading of the clock.
fn today_line(today: Date) -> String {
    format!("{} {} {}", today.weekday(), today.day(), today.month())
}

/// The distinct signs among this family's birthdays, in calendar order.
///
/// Members who have not set a birthday are simply absent, with nothing said
/// about them and no special case: a family where nobody has set one produces
/// an empty set, and the note about the day stands alone.
async fn signs_in_family(state: &AppState, family_id: i64) -> Result<Vec<&'static str>, ApiError> {
    let rows = sqlx::query(
        "SELECT DISTINCT birthday_month, birthday_day
           FROM users
          WHERE family_id = $1
            AND birthday_month IS NOT NULL
            AND birthday_day IS NOT NULL",
    )
    .bind(family_id)
    .fetch_all(&state.pool)
    .await?;

    let mut found: Vec<&'static str> = Vec::new();
    for row in &rows {
        let month: Option<i16> = row.get("birthday_month");
        let day: Option<i16> = row.get("birthday_day");
        if let (Some(month), Some(day)) = (month, day)
            && let Ok(month) = u8::try_from(month)
            && let Ok(day) = u8::try_from(day)
            && let Some(sign) = sign_for(month, day)
            && !found.contains(&sign)
        {
            found.push(sign);
        }
    }
    // Calendar order, so the same family produces the same sentence every
    // day rather than one that reshuffles with whatever the database returned.
    found.sort_by_key(|name| SIGNS.iter().position(|(sign, _, _)| sign == name));
    Ok(found)
}

/// The ticker. Called once from `main.rs`, cancelled on shutdown, exactly as
/// [`crate::calls::spawn_sweeper`] is.
pub fn spawn_sweeper(state: AppState) {
    if !state.cfg.greetings.is_usable() {
        return;
    }
    let shutdown = state.registry.shutdown_token();
    tokio::spawn(async move {
        loop {
            match post_daily_greetings(&state).await {
                Ok(0) => {}
                Ok(count) => info!(count, "posted daily greetings"),
                Err(error) => warn!(?error, "posting daily greetings failed"),
            }
            tokio::select! {
                _ = tokio::time::sleep(until_next_minute(OffsetDateTime::now_utc())) => {}
                _ = shutdown.cancelled() => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime};

    /// Every boundary in the table, from both sides. A sign that starts on
    /// the 21st means the 20th belongs to the one before it, and an
    /// off-by-one here is a family told the wrong thing about themselves
    /// every day for a month.
    #[test]
    fn each_sign_starts_on_its_own_day_and_not_the_day_before() {
        let expected = [
            (1, 19, "Capricorn"),
            (1, 20, "Aquarius"),
            (2, 18, "Aquarius"),
            (2, 19, "Pisces"),
            (3, 20, "Pisces"),
            (3, 21, "Aries"),
            (4, 19, "Aries"),
            (4, 20, "Taurus"),
            (5, 20, "Taurus"),
            (5, 21, "Gemini"),
            (6, 20, "Gemini"),
            (6, 21, "Cancer"),
            (7, 22, "Cancer"),
            (7, 23, "Leo"),
            (8, 22, "Leo"),
            (8, 23, "Virgo"),
            (9, 22, "Virgo"),
            (9, 23, "Libra"),
            (10, 22, "Libra"),
            (10, 23, "Scorpio"),
            (11, 21, "Scorpio"),
            (11, 22, "Sagittarius"),
            (12, 21, "Sagittarius"),
            (12, 22, "Capricorn"),
        ];
        for (month, day, sign) in expected {
            assert_eq!(
                sign_for(month, day),
                Some(sign),
                "{month:02}-{day:02} should be {sign}"
            );
        }
    }

    /// The wrap-around is the one case the loop cannot answer by finding a
    /// start at or before the date: every start in the table is later in the
    /// year than the 1st of January.
    #[test]
    fn january_falls_through_to_the_december_sign() {
        assert_eq!(sign_for(1, 1), Some("Capricorn"));
        assert_eq!(sign_for(12, 31), Some("Capricorn"));
    }

    /// 29 February is a real birthday in this protocol — migration 0018's
    /// CHECK admits it, because a birthday here has no year to be a leap
    /// year in.
    #[test]
    fn the_twenty_ninth_of_february_has_a_sign() {
        assert_eq!(sign_for(2, 29), Some("Pisces"));
    }

    #[test]
    fn a_date_that_cannot_exist_has_no_sign() {
        assert_eq!(sign_for(0, 1), None);
        assert_eq!(sign_for(13, 1), None);
        assert_eq!(sign_for(1, 0), None);
        assert_eq!(sign_for(1, 32), None);
    }

    /// The whole idempotency guarantee rests on this being a pure function
    /// of the date: the same day must produce the same id on every process,
    /// every restart and every machine, or a family gets two greetings.
    #[test]
    fn the_id_is_the_same_every_time_for_one_day_and_different_across_days() {
        let a = client_msg_id_for(date!(2026 - 09 - 08));
        assert_eq!(a, client_msg_id_for(date!(2026 - 09 - 08)));
        assert_ne!(a, client_msg_id_for(date!(2026 - 09 - 09)));
        assert_ne!(a, client_msg_id_for(date!(2026 - 10 - 08)));
        assert_ne!(a, client_msg_id_for(date!(2025 - 09 - 08)));
        // Readable in the database without a tool, which is the reason it is
        // built by hand rather than hashed.
        assert!(
            a.simple().to_string().starts_with("6663677265657400"),
            "the id should still spell fcgreet: {a}"
        );
        // …and then the date, big-endian: 0x07ea is 2026, then 09, then 08.
        assert_eq!(a.to_string(), "66636772-6565-7400-07ea-090800000000");
    }

    /// Catching up is deliberate — a server that was down at the hour still
    /// posts when it comes back — so "passed" means at or after, all day.
    #[test]
    fn the_moment_passes_at_the_configured_time_and_stays_passed() {
        assert!(!moment_passed(datetime!(2026-09-08 05:59:59 UTC), 6, 0));
        assert!(moment_passed(datetime!(2026-09-08 06:00:00 UTC), 6, 0));
        assert!(moment_passed(datetime!(2026-09-08 23:59:59 UTC), 6, 0));
        // And the minute is honoured, which is why the ticker is not hourly.
        assert!(!moment_passed(datetime!(2026-09-08 06:29:00 UTC), 6, 30));
        assert!(moment_passed(datetime!(2026-09-08 06:30:00 UTC), 6, 30));
    }

    /// Midnight is a legal configured time and must not be treated as
    /// "unset" by an accidental zero test.
    #[test]
    fn midnight_is_a_configurable_hour() {
        assert!(moment_passed(datetime!(2026-09-08 00:00:00 UTC), 0, 0));
        assert!(moment_passed(datetime!(2026-09-08 12:00:00 UTC), 0, 0));
    }

    /// The alignment that closes the last-minutes-of-the-day gap: from any
    /// second, the next tick lands just past the next `:00`, never a full
    /// period away and never at zero.
    #[test]
    fn the_ticker_sleeps_to_the_next_minute_boundary() {
        let slack = std::time::Duration::from_millis(50);
        assert_eq!(
            until_next_minute(datetime!(2026-09-08 23:58:30 UTC)),
            std::time::Duration::from_secs(30) + slack
        );
        assert_eq!(
            until_next_minute(datetime!(2026-09-08 23:59:59.5 UTC)),
            std::time::Duration::from_millis(500) + slack
        );
        // Exactly on the boundary: a whole period, not zero — a zero sleep
        // would spin.
        assert_eq!(
            until_next_minute(datetime!(2026-09-08 23:59:00 UTC)),
            TICK + slack
        );
    }

    /// Every configured minute of the day is reachable by an aligned tick on
    /// the day it names: the tick at `hh:mm:00.05` satisfies `>= hh:mm:00`.
    #[test]
    fn every_configured_minute_gets_a_tick_inside_its_window() {
        for hour in 0..24u8 {
            for minute in 0..60u8 {
                let tick = datetime!(2026-09-08 00:00:00.05 UTC)
                    + time::Duration::hours(i64::from(hour))
                    + time::Duration::minutes(i64::from(minute));
                assert!(
                    moment_passed(tick, hour, minute),
                    "{hour:02}:{minute:02} has no tick inside its window"
                );
                // …and the tick one minute earlier does not fire it.
                assert!(
                    !moment_passed(tick - time::Duration::minutes(1), hour, minute)
                        || (hour == 0 && minute == 0),
                    "{hour:02}:{minute:02} fired a minute early"
                );
            }
        }
    }

    /// The date the model is told is the date the id was derived from.
    #[test]
    fn the_line_and_the_id_name_the_same_day() {
        let day = date!(2026 - 09 - 08);
        assert_eq!(today_line(day), "Tuesday 8 September");
    }
}
