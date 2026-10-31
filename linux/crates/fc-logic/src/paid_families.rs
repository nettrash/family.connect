//! Paid families on a client that cannot sell (docs/protocol.md, "Paid families", 2026-10-30;
//! docs/paid-families-2026-10-30.md §8): what the family door offers, the banner over an unpaid
//! family's chat list, the allowance in Family Statistics and the sentences of the refusals.
//!
//! NOTHING HERE APPLIES ON A SERVER THAT DOES NOT CHARGE. `subscriptions_enabled` is false on every
//! server from before (its absence reads false), and then the door is the door it always was, no
//! family carries `delete_at`, and the statistics carry no `allowance`.
//!
//! ONLY APPLE'S APPS SELL THE SUBSCRIPTION, so this one says where a family is started instead of
//! offering to start it — unless the account already holds an active subscription, bought there,
//! when creating works as before. A door the operator shut (`family_registration_enabled: false`)
//! keeps priority and its own words: nobody is sent to buy a subscription for a family they could
//! not have started anyway. "Join a family" is never touched: members never pay.
//!
//! THE BANNER IS THE SHOWN FAMILY'S. While the family on screen has `delete_at`, its members are
//! told on which day it will be deleted and who can renew it; the owner, that it is renewed in the
//! Apple app. It goes when `delete_at` does — a `family_plan` frame without one, or a read of the
//! family that no longer carries one.
//!
//! A DEADLINE IS THE READER'S DAY, A RENEWAL IS THE UTC DAY. `delete_at` is a moment, written as
//! the day it falls on in the reader's zone ([`Days::day`]). `renews_at` — the frame's, the
//! statistics', and the 1st of the next month a REST refusal implies — is the start of a UTC month
//! and is written as its UTC calendar day, always the 1st ([`Days::utc_day`]): "30 November" west of
//! Greenwich would name a month that has not ended (docs/protocol.md, "UTC calendar day"). Both in
//! the reader's culture, with the year only when it is not the reader's current one.

use fc_core::protocol::{
    AllowanceDto, AllowanceUseDto, ApiError, FamilyDto, MemberDto, error_codes,
};
use fc_core::store::times;
use fc_core::text::media_text;
use fc_core::text::string_catalog::StringCatalog;
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time};

use crate::app_session::SessionState;
use crate::date_culture::{self, DateCulture, TimeZone};

/// How the reader writes a day, and when "now" is: their culture, their zone and the clock.
#[derive(Clone, Copy)]
pub struct Days<'a> {
    culture: &'a DateCulture,
    zone: &'a dyn TimeZone,
    now: OffsetDateTime,
}

impl std::fmt::Debug for Days<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Days")
            .field("culture", &self.culture.name)
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

impl<'a> Days<'a> {
    pub fn new(culture: &'a DateCulture, zone: &'a dyn TimeZone, now: OffsetDateTime) -> Self {
        Days { culture, zone, now }
    }

    /// `at` as the reader's day: the day and the month in the culture's own order ("14 November",
    /// "November 14", "14. November", "11月14日"), and the year too when that day is not in the
    /// reader's current year ("4 January 2027") — both judged on the reader's wall clock.
    pub fn day(&self, at: OffsetDateTime) -> String {
        self.write(date_culture::local(at, self.zone).date())
    }

    /// `at`'s UTC calendar day, in the reader's culture — for a paid family's allowance, whose
    /// month is the UTC month (docs/protocol.md, "UTC calendar day"): midnight UTC on 1 December is
    /// "December 1" in New York too, never "November 30". The year is added as [`day`](Self::day)
    /// adds it, when it is not the reader's current year.
    pub fn utc_day(&self, at: OffsetDateTime) -> String {
        self.write(at.to_offset(time::UtcOffset::UTC).date())
    }

    /// A calendar day, with the year only when it is not this year on the reader's wall clock.
    fn write(&self, day: Date) -> String {
        let today = date_culture::local(self.now, self.zone);
        let pattern = if day.year() == today.year() {
            self.culture.month_day_pattern.clone()
        } else {
            self.culture.day_month_year_pattern()
        };
        self.culture.format(day.midnight(), &pattern)
    }

    /// The 1st of the next month, at midnight UTC: when a paid family's monthly allowance starts
    /// again (docs/protocol.md, "Paid families": "a month is the UTC calendar month") — the day a
    /// REST `ai_allowance_used` names, as its answer carries none.
    pub fn next_month(&self) -> OffsetDateTime {
        let now = self.now.to_offset(time::UtcOffset::UTC);
        let (year, month) = match now.month() {
            Month::December => (now.year() + 1, Month::January),
            month => (now.year(), month.next()),
        };
        Date::from_calendar_date(year, month, 1)
            .map(|first| PrimitiveDateTime::new(first, Time::MIDNIGHT).assume_utc())
            .unwrap_or(now)
    }
}

// ---- the family door ----------------------------------------------------------------------------

/// What the door offers where a family would be created — the family gate's "Create a family" and
/// "New Family…".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CreateDoor {
    /// The create form, as it always was.
    Open,
    /// The operator has closed this server to new families (`family_registration_enabled: false`):
    /// how to run a server of one's own instead. Only ever for a FIRST family — "New Family…" has
    /// its own switch, and is not offered at all while it is off.
    Closed,
    /// The server charges for a new family and this account holds no active subscription: the
    /// sentence saying where a family is started ([`start_in_apple_app`]), in place of the form.
    StartInAppleApp,
}

/// The door for this account: `another` is "New Family…" from inside a family, which answers to
/// `member_families` (the menu's own guard), never to `registration`.
pub fn create_door(state: &SessionState, another: bool) -> CreateDoor {
    if !another && !state.family_registration_enabled {
        return CreateDoor::Closed;
    }
    if state.subscriptions_enabled && !state.subscription_active() {
        return CreateDoor::StartInAppleApp;
    }
    CreateDoor::Open
}

/// Where a family on this server is started — what the door says instead of "Create Family".
pub fn start_in_apple_app(say: &dyn StringCatalog) -> String {
    say.get(
        "On this server, a new family is started in the Family Connect app on iPhone, iPad or Mac.",
    )
    .to_string()
}

// ---- the banner ---------------------------------------------------------------------------------

/// The banner over the SHOWN family's chat list while it has `delete_at`, or `None` while it has
/// none (or none that can be read). The owner reads that it is renewed in the Apple app; a member
/// reads who the owner is — the roster's, from `members` — and with no owner in the roster yet
/// (the first read still on its way) there is no banner rather than a sentence with a gap in it.
pub fn banner(
    state: &SessionState,
    members: &[MemberDto],
    say: &dyn StringCatalog,
    days: &Days<'_>,
) -> Option<String> {
    let family = state.family.as_ref()?;
    let at = deadline(family)?;
    let date = days.day(at);
    if state.is_owner() {
        return Some(say.format(
            "“%@” will be deleted on %@ because your subscription has ended. Renew it in the Family Connect app on iPhone, iPad or Mac.",
            &[&family.name, &date],
        ));
    }
    let owner = owner_name(members)?;
    Some(say.format(
        "“%@” will be deleted on %@ because its subscription has ended. Only %@, the owner, can renew it.",
        &[&family.name, &date, &owner],
    ))
}

/// When the server will delete this family, or `None` while it is paid for (or free).
pub fn deadline(family: &FamilyDto) -> Option<OffsetDateTime> {
    times::instant(family.delete_at.as_deref()).and_then(times::from_milliseconds)
}

/// The owner's display name from a roster: the member whose role is owner and who is still there.
pub fn owner_name(members: &[MemberDto]) -> Option<&str> {
    members
        .iter()
        .find(|member| member.owner() && !member.is_former())
        .map(|member| member.display_name.as_str())
        .filter(|name| !name.trim().is_empty())
}

// ---- the allowance in Family Statistics ---------------------------------------------------------

/// The allowance section's title.
pub fn allowance_title(say: &dyn StringCatalog) -> String {
    say.get("This Month’s Allowance").to_string()
}

/// The allowance section's rows, `(label, value)`, one per limit the block carries and in this
/// order: storage as sizes, the assistant as a share of its tokens, pictures as counts, and
/// transcripts in whole minutes. A limit that is absent — or present with no limit, which is the
/// operator's `0`, "off" — has no row. `decimal` is the reader's decimal separator (for sizes) and
/// `group` writes a count with the reader's digit grouping.
pub fn allowance_rows(
    allowance: &AllowanceDto,
    say: &dyn StringCatalog,
    decimal: &str,
    group: &dyn Fn(i64) -> String,
) -> Vec<(String, String)> {
    let on = |limit: Option<AllowanceUseDto>| limit.filter(|limit| limit.limit > 0);
    let mut rows = Vec::new();
    if let Some(storage) = on(allowance.storage_bytes) {
        let size = |bytes: i64| media_text::display_size(bytes.max(0), decimal);
        rows.push((
            say.get("Storage").to_string(),
            say.format("%@ of %@", &[&size(storage.used), &size(storage.limit)]),
        ));
    }
    if let Some(tokens) = on(allowance.ai_tokens) {
        rows.push((
            say.get("Assistant").to_string(),
            say.format("%lld%% used", &[&percent(tokens.used, tokens.limit)]),
        ));
    }
    if let Some(images) = on(allowance.images) {
        rows.push((
            say.get("Pictures").to_string(),
            say.format(
                "%@ of %@",
                &[&group(images.used.max(0)), &group(images.limit)],
            ),
        ));
    }
    if let Some(minutes) = on(allowance.transcript_ms) {
        rows.push((
            say.get("Transcripts").to_string(),
            say.format(
                "%@ of %@ min",
                &[
                    &group(minutes_used(minutes.used)),
                    &group(minutes_limit(minutes.limit)),
                ],
            ),
        ));
    }
    rows
}

/// The section's last line — when the monthly limits start again, as its UTC calendar day — or
/// `None` when the block does not say (or says it unreadably).
///
/// The key is `Renews on %1$@`, positional, and not `Renews on %@`: Apple's apps say "Renews on …"
/// of the SUBSCRIPTION too, in other words in four languages ("verlängert sich" — this one
/// "erneuert sich"), and the apps' catalogue would win over this port's own table for a key it
/// shares. English reads the same either way.
pub fn allowance_renews(
    allowance: &AllowanceDto,
    say: &dyn StringCatalog,
    days: &Days<'_>,
) -> Option<String> {
    let at = times::instant(allowance.renews_at.as_deref()).and_then(times::from_milliseconds)?;
    Some(say.format("Renews on %1$@", &[&days.utc_day(at)]))
}

/// The assistant's share of its tokens: used ÷ limit × 100, in whole per cent rounded DOWN — 99.6 %
/// is not yet "100% used" while the allowance still answers — and never past 100, however far one
/// answer overshot. 0 for no limit.
pub fn percent(used: i64, limit: i64) -> i64 {
    if limit <= 0 {
        return 0;
    }
    let share = i128::from(used.max(0)) * 100 / i128::from(limit);
    i64::try_from(share.min(100)).unwrap_or(100)
}

/// Transcript time USED, in whole minutes rounded UP: a minute begun is a minute used.
pub fn minutes_used(milliseconds: i64) -> i64 {
    let milliseconds = milliseconds.max(0);
    milliseconds / 60_000 + i64::from(milliseconds % 60_000 != 0)
}

/// The transcript LIMIT, in whole minutes rounded DOWN: never more than is really there.
pub fn minutes_limit(milliseconds: i64) -> i64 {
    milliseconds.max(0) / 60_000
}

// ---- refusals -----------------------------------------------------------------------------------

/// The sentence for a paid-family refusal over REST, or `None` for any other error (the caller's
/// own sentences then):
///
/// - `subscription_required` — `POST /families` with subscriptions on and none active; terminal;
/// - `storage_allowance_used` — an upload for a family past its storage; terminal, never retried;
/// - `ai_allowance_used` — a transcript or a board backdrop past this month's minutes or pictures;
///   the answer carries no day, so it names the 1st of the next month, UTC.
pub fn refusal(error: &ApiError, say: &dyn StringCatalog, days: &Days<'_>) -> Option<String> {
    match error.code.as_str() {
        error_codes::SUBSCRIPTION_REQUIRED => Some(subscription_required(say)),
        error_codes::AI_ALLOWANCE_USED => {
            Some(assistant_allowance_used(Some(days.next_month()), say, days))
        }
        code => upload_refusal(code, say),
    }
}

/// What a send whose upload was refused with `code` says on its failed row, when it is a paid
/// family's refusal: `storage_allowance_used` alone.
pub fn upload_refusal(code: &str, say: &dyn StringCatalog) -> Option<String> {
    (code == error_codes::STORAGE_ALLOWANCE_USED).then(|| {
        say.get("This family has used its storage allowance.")
            .to_string()
    })
}

/// `subscription_required`'s sentence.
pub fn subscription_required(say: &dyn StringCatalog) -> String {
    say.get("A new family on this server needs a subscription.")
        .to_string()
}

/// The assistant's allowance is used up, and it starts again on `renews_at` — the `ai_error`
/// frame's, or (`None`, unreadable) the 1st of the next month, UTC — named as its UTC calendar day.
pub fn assistant_allowance_used(
    renews_at: Option<OffsetDateTime>,
    say: &dyn StringCatalog,
    days: &Days<'_>,
) -> String {
    let at = renews_at.unwrap_or_else(|| days.next_month());
    say.format(
        "This family has used this month’s assistant allowance. It renews on %@.",
        &[&days.utc_day(at)],
    )
}
