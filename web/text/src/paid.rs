//! Families a server hosts for a subscription (#89; docs/protocol.md, "Paid
//! families", and the decisions in docs/paid-families-2026-10-30.md): whether
//! this account may start a family here, what the banner over a family that
//! will be deleted says and to whom, what Family Statistics' allowance rows
//! read, and what each new refusal is told as.
//!
//! These are the words of a client that cannot SELL the subscription — the
//! web, Windows and Linux. Only Apple's apps can, so the sentences here send a
//! person there. Android says less, for Google Play's sake, and has its own.
//!
//! One table every surface reads, as the families' rules are: the gate and
//! "New Family…" decide the same way, and the banner is chosen in one place.

use crate::i18n::{t, t1, t2, t3};
use crate::media::display_size;

/// Whether "Create a family" and "New Family…" are offered: always on a
/// server that does not charge for families (`subscriptions_enabled` false or
/// absent), and on one that does only while the account's OWN subscription
/// is active. A shut door (`family_registration_enabled`,
/// `member_families_enabled`) is decided before this and keeps its own words.
pub fn may_start_family(subscriptions_enabled: bool, subscription_active: bool) -> bool {
    !subscriptions_enabled || subscription_active
}

/// What stands where the create action would, when it is not offered.
pub fn start_elsewhere() -> &'static str {
    t("On this server, a new family is started in the Family Connect app on iPhone, iPad or Mac.")
}

/// The most members an owner's cap may be: the family's own
/// `member_ceiling`, which a paid family lowers to the paid limit — else,
/// from a server that predates it, the operator's `max_family_members` on
/// `GET /me`. None when neither is known, and then no cap is offered.
pub fn member_ceiling(family: Option<i64>, operator: Option<i64>) -> Option<i64> {
    family.or(operator)
}

/// The banner over the chat list of a family that will be deleted on `date`
/// (`delete_at`, already written the reader's way). The owner is told it is
/// their subscription and where to renew it; a member is told who the owner
/// is, the one person who can — and None while the roster has not named the
/// owner yet, rather than a sentence with a hole in it.
pub fn banner(family: &str, date: &str, reader_owns: bool, owner: Option<&str>) -> Option<String> {
    if reader_owns {
        return Some(t2(
            "“%@” will be deleted on %@ because your subscription has ended. Renew it in the Family Connect app on iPhone, iPad or Mac.",
            family,
            date,
        ));
    }
    owner.map(|owner| {
        t3(
            "“%@” will be deleted on %@ because its subscription has ended. Only %@, the owner, can renew it.",
            family,
            date,
            owner,
        )
    })
}

/// What one of the new refusals is told as — None for a code that is not
/// one of them. `ai_allowance_used` names a day, and is
/// [`ai_allowance_used`].
pub fn refusal(code: Option<&str>) -> Option<&'static str> {
    match code? {
        // POST /families, for an account with no active subscription.
        "subscription_required" => Some(t("A new family on this server needs a subscription.")),
        // POST /attachments, for a family past its storage allowance.
        "storage_allowance_used" => Some(storage_allowance_used()),
        _ => None,
    }
}

/// An upload refused because the family has used its storage allowance —
/// not the server's disk being full, which is `storage_full`.
pub fn storage_allowance_used() -> &'static str {
    t("This family has used its storage allowance.")
}

/// The assistant refused for the month: an answer that failed with
/// `"allowance_used"`, a transcript or a backdrop refused with
/// `ai_allowance_used`. `date` is the day it renews — its UTC calendar day,
/// the 1st, for every reader — written in the reader's language.
pub fn ai_allowance_used(date: &str) -> String {
    t1(
        "This family has used this month’s assistant allowance. It renews on %@.",
        date,
    )
}

/// The moment the monthly allowance starts again, when the server did not
/// say: the 1st of the next calendar month, UTC — an `ai_allowance_used`
/// refusal over REST carries no `renews_at` (docs/protocol.md, "Paid
/// families"). RFC 3339, the wire's own shape, so it is written as any
/// other instant is.
pub fn next_month_utc(now_ms: f64) -> String {
    let days = (now_ms / 86_400_000.0).floor() as i64;
    let (year, month, _) = civil_from_days(days);
    let (year, month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    format!("{year:04}-{month:02}-01T00:00:00Z")
}

/// The proleptic Gregorian date of a day counted from 1970-01-01 (Howard
/// Hinnant's `civil_from_days`): exact for every day either side of 1970,
/// with no table and no floating point.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// A paid family's allowance, as Family Statistics carries it: each limit
/// as (used, limit), and None where the operator turned that one off
/// (docs/protocol.md, "Family statistics").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Allowance {
    pub storage_bytes: Option<(i64, i64)>,
    pub ai_tokens: Option<(i64, i64)>,
    pub images: Option<(i64, i64)>,
    pub transcript_ms: Option<(i64, i64)>,
}

impl Allowance {
    /// The section's rows, label and value, one per limit present and in
    /// this order: storage, the assistant, pictures, transcripts.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = Vec::new();
        if let Some((used, limit)) = self.storage_bytes {
            rows.push((t("Storage"), storage(used, limit)));
        }
        if let Some((used, limit)) = self.ai_tokens {
            rows.push((t("Assistant"), assistant(used, limit)));
        }
        if let Some((used, limit)) = self.images {
            rows.push((t("Pictures"), pictures(used, limit)));
        }
        if let Some((used, limit)) = self.transcript_ms {
            rows.push((t("Transcripts"), transcripts(used, limit)));
        }
        rows
    }
}

/// The section's title.
pub fn allowance_title() -> &'static str {
    t("This Month’s Allowance")
}

/// Its last line: when the monthly limits start again.
pub fn renews_on(date: &str) -> String {
    t1("Renews on %@", date)
}

/// "3.22 GB of 10.74 GB" — in the size style a file row uses.
pub fn storage(used: i64, limit: i64) -> String {
    t2(
        "%@ of %@",
        &display_size(used.max(0) as u64),
        &display_size(limit.max(0) as u64),
    )
}

/// "40% used" — the assistant's tokens are a number nobody can weigh, so
/// they are a share of the month's.
pub fn assistant(used: i64, limit: i64) -> String {
    t1("%@%% used", &percent(used, limit).to_string())
}

/// "4 of 30".
pub fn pictures(used: i64, limit: i64) -> String {
    t2(
        "%@ of %@",
        &used.max(0).to_string(),
        &limit.max(0).to_string(),
    )
}

/// "10 of 120 min" — what was used rounded UP, so a minute begun is a
/// minute counted, and the limit rounded DOWN, so the two never meet early.
pub fn transcripts(used_ms: i64, limit_ms: i64) -> String {
    t2(
        "%@ of %@ min",
        &minutes_up(used_ms).to_string(),
        &(limit_ms.max(0) / 60_000).to_string(),
    )
}

/// What share of `limit` is used, in whole per cent (rounded down), and
/// never more than 100: one answer may overshoot the allowance by its own
/// size. A limit of nothing is used up.
pub fn percent(used: i64, limit: i64) -> i64 {
    if limit <= 0 {
        return 100;
    }
    let share = i128::from(used.max(0)) * 100 / i128::from(limit);
    share.min(100) as i64
}

fn minutes_up(ms: i64) -> i64 {
    let ms = ms.max(0);
    ms / 60_000 + i64::from(ms % 60_000 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{use_lang, Lang};

    #[test]
    fn a_family_is_started_here_only_with_an_active_subscription() {
        assert!(
            may_start_family(false, false),
            "a server that does not charge"
        );
        assert!(may_start_family(false, true));
        assert!(may_start_family(true, true), "an active subscription");
        assert!(
            !may_start_family(true, false),
            "none, or one that has ended"
        );
        assert_eq!(
            start_elsewhere(),
            "On this server, a new family is started in the Family Connect app on iPhone, iPad or Mac."
        );
    }

    #[test]
    fn the_cap_reaches_the_familys_own_ceiling_first() {
        assert_eq!(
            member_ceiling(Some(25), Some(50)),
            Some(25),
            "a paid family"
        );
        assert_eq!(
            member_ceiling(None, Some(50)),
            Some(50),
            "a server that predates it"
        );
        assert_eq!(member_ceiling(Some(50), None), Some(50));
        assert_eq!(member_ceiling(None, None), None);
    }

    #[test]
    fn the_owner_is_told_where_to_renew_and_a_member_who_can() {
        assert_eq!(
            banner("Ivanovs", "14 November", true, Some("Anna")).as_deref(),
            Some("“Ivanovs” will be deleted on 14 November because your subscription has ended. Renew it in the Family Connect app on iPhone, iPad or Mac.")
        );
        assert_eq!(
            banner("Ivanovs", "14 November", true, None).as_deref(),
            Some("“Ivanovs” will be deleted on 14 November because your subscription has ended. Renew it in the Family Connect app on iPhone, iPad or Mac."),
            "the owner needs no roster to be told"
        );
        assert_eq!(
            banner("Ivanovs", "14 November", false, Some("Anna")).as_deref(),
            Some("“Ivanovs” will be deleted on 14 November because its subscription has ended. Only Anna, the owner, can renew it.")
        );
        assert_eq!(
            banner("Ivanovs", "14 November", false, None),
            None,
            "not before the roster names the owner"
        );
    }

    #[test]
    fn the_banner_reads_in_the_readers_language() {
        use_lang(Lang::De);
        let said = banner("Iwanows", "14. November", false, Some("Anna"));
        use_lang(Lang::Ja);
        let japanese = banner("Iwanows", "11月14日", true, None);
        use_lang(Lang::En);
        assert_eq!(
            said.as_deref(),
            Some("„Iwanows“ wird am 14. November gelöscht, weil das Abo abgelaufen ist. Nur Anna (Besitzer:in) kann es verlängern.")
        );
        assert_eq!(
            japanese.as_deref(),
            Some("あなたのサブスクリプションが終了したため、「Iwanows」は 11月14日 に削除されます。iPhone、iPad、Mac の Family Connect アプリで更新してください。")
        );
    }

    #[test]
    fn each_new_refusal_says_what_happened() {
        assert_eq!(
            refusal(Some("subscription_required")),
            Some("A new family on this server needs a subscription.")
        );
        assert_eq!(
            refusal(Some("storage_allowance_used")),
            Some("This family has used its storage allowance.")
        );
        assert_eq!(
            refusal(Some("storage_full")),
            None,
            "the server's disk, not the family's allowance"
        );
        assert_eq!(refusal(Some("ai_allowance_used")), None, "it names a day");
        assert_eq!(refusal(None), None);
        assert_eq!(
            ai_allowance_used("1 December"),
            "This family has used this month’s assistant allowance. It renews on 1 December."
        );
    }

    /// Instants as milliseconds, written out so the test reads as dates.
    fn at(iso_day: (i64, u32, u32), hour: i64) -> f64 {
        let (year, month, day) = iso_day;
        // Days from 1970-01-01 to the 1st of each month, through civil_from_days's inverse,
        // checked below by round trip rather than trusted.
        let mut days = 0i64;
        while civil_from_days(days) < (year, month, day) {
            days += 1;
        }
        while civil_from_days(days) > (year, month, day) {
            days -= 1;
        }
        (days * 86_400_000 + hour * 3_600_000) as f64
    }

    #[test]
    fn the_allowance_renews_on_the_first_of_next_month_utc() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(20_756), (2026, 10, 30));
        assert_eq!(
            next_month_utc(at((2026, 10, 30), 12)),
            "2026-11-01T00:00:00Z"
        );
        assert_eq!(
            next_month_utc(at((2026, 11, 1), 0)),
            "2026-12-01T00:00:00Z",
            "the first instant of a month is already in it"
        );
        assert_eq!(
            next_month_utc(at((2026, 12, 31), 23)),
            "2027-01-01T00:00:00Z",
            "across the year"
        );
        assert_eq!(
            next_month_utc(at((2028, 2, 29), 10)),
            "2028-03-01T00:00:00Z",
            "a leap day"
        );
    }

    #[test]
    fn a_row_for_each_limit_present_in_order() {
        let all = Allowance {
            storage_bytes: Some((3_221_225_472, 10_737_418_240)),
            ai_tokens: Some((812_345, 2_000_000)),
            images: Some((4, 30)),
            transcript_ms: Some((600_000, 7_200_000)),
        };
        assert_eq!(
            all.rows(),
            vec![
                ("Storage", "3.22 GB of 10.74 GB".to_string()),
                ("Assistant", "40% used".to_string()),
                ("Pictures", "4 of 30".to_string()),
                ("Transcripts", "10 of 120 min".to_string()),
            ]
        );
        let some = Allowance {
            images: Some((0, 30)),
            ..Allowance::default()
        };
        assert_eq!(some.rows(), vec![("Pictures", "0 of 30".to_string())]);
        assert!(
            Allowance::default().rows().is_empty(),
            "every limit turned off"
        );
        assert_eq!(allowance_title(), "This Month’s Allowance");
        assert_eq!(renews_on("1 December"), "Renews on 1 December");
    }

    #[test]
    fn the_assistants_share_is_whole_and_stops_at_a_hundred() {
        assert_eq!(percent(0, 2_000_000), 0);
        assert_eq!(percent(812_345, 2_000_000), 40, "rounded down");
        assert_eq!(percent(1_999_999, 2_000_000), 99, "not 100 before it is");
        assert_eq!(percent(2_000_000, 2_000_000), 100);
        assert_eq!(percent(2_400_000, 2_000_000), 100, "one answer overshot");
        assert_eq!(percent(-5, 2_000_000), 0);
        assert_eq!(percent(5, 0), 100, "a limit of nothing");
        assert_eq!(percent(i64::MAX, 1), 100, "no overflow");
        assert_eq!(assistant(2_400_000, 2_000_000), "100% used");
    }

    #[test]
    fn transcript_minutes_round_used_up_and_the_limit_down() {
        assert_eq!(transcripts(0, 7_200_000), "0 of 120 min");
        assert_eq!(transcripts(1, 7_200_000), "1 of 120 min", "a minute begun");
        assert_eq!(transcripts(60_000, 7_200_000), "1 of 120 min");
        assert_eq!(transcripts(60_001, 7_200_000), "2 of 120 min");
        assert_eq!(
            transcripts(60_000, 7_259_999),
            "1 of 120 min",
            "the limit down"
        );
        assert_eq!(transcripts(-1, -1), "0 of 0 min");
    }

    #[test]
    fn sizes_are_a_file_rows_and_pictures_are_counts() {
        assert_eq!(storage(0, 10_737_418_240), "Zero KB of 10.74 GB");
        assert_eq!(storage(1_500_000, 10_000_000_000), "1.5 MB of 10 GB");
        assert_eq!(pictures(31, 30), "31 of 30", "past it, as said");
    }

    #[test]
    fn the_allowance_reads_in_the_readers_language() {
        use_lang(Lang::Ja);
        let japanese = (pictures(4, 30), transcripts(600_000, 7_200_000));
        use_lang(Lang::De);
        let german = assistant(812_345, 2_000_000);
        use_lang(Lang::Ru);
        let russian = (assistant(812_345, 2_000_000), renews_on("1 декабря"));
        use_lang(Lang::En);
        assert_eq!(
            japanese,
            ("30 中 4".to_string(), "120 分中 10 分".to_string()),
            "the arguments the other way round"
        );
        assert_eq!(german, "40 % verbraucht");
        assert_eq!(
            russian,
            (
                "Использовано 40%".to_string(),
                "Обновится 1 декабря".to_string()
            )
        );
    }
}
