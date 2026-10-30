//! How a chat's messages are laid out as rows — the web client's `timeline` and the apps'
//! `MessagePresentation`: day sections with one pill each, the single unread divider, whose name
//! shows above a bubble, where a run starts and ends, and where a chat opens (`Timeline.cs`).
//!
//! The C#'s `TimeZoneInfo` and `CultureInfo` are `date_culture`'s [`TimeZone`] and [`DateCulture`];
//! a `DateOnly` is a `time::Date`.

use fc_core::protocol::MessageDto;
use fc_core::store::times;
use fc_core::text::string_catalog::StringCatalog;
use time::Date;

use crate::conversation::Bubble;
use crate::date_culture::{DateCulture, TimeZone, local};

/// How far back from the newest row an opening may anchor: the apps refuse to anchor past their
/// window.
pub const ANCHOR_CAP: usize = 300;

/// One drawn row of a conversation, with nothing left to decide.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineRow {
    pub bubble: Bubble,
    /// The day pill above this row: the first row of each local day.
    pub day_above: Option<Date>,
    /// "N new messages" goes above this row — at most one row in a chat.
    pub unread_divider_above: bool,
    /// The sender's face and name above the bubble.
    pub shows_sender: bool,
    /// The first of a run by one sender: the gap above it is a turn's, and its corner is round.
    pub run_start: bool,
    /// The last of a run by one sender: the time shows here, and its corner is round.
    pub run_end: bool,
}

/// The local day a message was sent on, or `None` for a stamp that does not parse.
pub fn day_of(message: &MessageDto, zone: &dyn TimeZone) -> Option<Date> {
    let milliseconds = times::instant(Some(&message.created_at))?;
    let at = times::from_milliseconds(milliseconds)?;
    Some(local(at, zone).date())
}

/// Lay a chat's bubbles (oldest first) out as rows.
///
/// `shared_chat`: names over bubbles in the family chat or a group — more than one other person to
/// tell apart.
pub fn rows(
    bubbles: Vec<Bubble>,
    shared_chat: bool,
    me: i64,
    first_unread_id: Option<i64>,
    zone: &dyn TimeZone,
) -> Vec<TimelineRow> {
    // A row whose stamp does not parse belongs to the day it sits at the end of.
    let mut days: Vec<Option<Date>> = Vec::with_capacity(bubbles.len());
    let mut running = None;
    for bubble in &bubbles {
        running = day_of(&bubble.message, zone).or(running);
        days.push(running);
    }

    // Who sent each row, read before the bubbles are moved into their rows.
    let senders: Vec<i64> = bubbles
        .iter()
        .map(|bubble| bubble.message.sender_id)
        .collect();
    let mut rows = Vec::with_capacity(bubbles.len());
    let mut section: Option<Date> = None;
    let mut placed = false;
    for (at, bubble) in bubbles.into_iter().enumerate() {
        let day = days[at];
        let new_day = day != section;
        if new_day {
            section = day;
        }
        let sender = senders[at];
        // A run never crosses a day: the name shows again after the pill.
        let previous = (at > 0 && !new_day).then(|| senders[at - 1]);
        let next = (at + 1 < senders.len() && days[at + 1] == section).then(|| senders[at + 1]);
        // A hidden row still counts as its sender's run, so the next visible sender keeps their
        // caption.
        let shows_sender = shared_chat
            && sender != me
            && !bubble.hidden
            && previous.is_none_or(|previous| previous != sender);
        // At most one divider, structurally.
        let divider = !placed && first_unread_id == Some(bubble.message.id);
        placed |= divider;
        rows.push(TimelineRow {
            bubble,
            day_above: if new_day { day } else { None },
            unread_divider_above: divider,
            shows_sender,
            run_start: previous.is_none_or(|previous| previous != sender),
            run_end: next.is_none_or(|next| next != sender),
        });
    }
    rows
}

/// Where a chat OPENS: the first unread message, under the divider — or `None`, which is the
/// ordinary open at the newest. Decided once per open (ios `UnreadAnchor`): nothing unread is the
/// newest; with a read marker, the oldest message from somebody else above it, unless the ones held
/// do not include every unread one; with no marker, the unread count back from the newest; and
/// never past [`ANCHOR_CAP`] rows back.
///
/// `messages` oldest first.
pub fn open_anchor(
    messages: &[MessageDto],
    unread_count: i32,
    last_read: i64,
    me: i64,
) -> Option<i64> {
    let unread = usize::try_from(unread_count)
        .ok()
        .filter(|&count| count > 0)?;
    // Distance counts EVERY row back from the newest; only a numbered row from somebody else can be
    // unread.
    let inbound: Vec<(usize, i64)> = messages
        .iter()
        .rev()
        .enumerate()
        .filter(|(_, message)| message.id != 0 && message.sender_id != me)
        .map(|(distance, message)| (distance, message.id))
        .collect();
    let hit = if last_read > 0 {
        let above: Vec<(usize, i64)> = inbound
            .into_iter()
            .filter(|&(_, id)| id > last_read)
            .collect();
        if above.len() < unread {
            return None;
        }
        *above.last()?
    } else {
        if inbound.len() < unread {
            return None;
        }
        inbound[unread - 1]
    };
    (hit.0 <= ANCHOR_CAP).then_some(hit.1)
}

/// The pill between day sections: "Today", "Yesterday", otherwise the short weekday and the month
/// and day — no year — the three labels the apps draw.
pub fn day_label(day: Date, today: Date, culture: &DateCulture, say: &dyn StringCatalog) -> String {
    if day == today {
        return say.get("Today").to_string();
    }
    if today.previous_day() == Some(day) {
        return say.get("Yesterday").to_string();
    }
    // The culture's own month-and-day order, with the month shortened: "Aug 17", "17. Aug.",
    // "8月17日".
    let month_day = culture.month_day_pattern.replace("MMMM", "MMM");
    let weekday = usize::from(day.weekday().number_days_from_sunday());
    format!(
        "{}, {}",
        culture.abbreviated_day_names[weekday],
        culture.format(day.midnight(), &month_day)
    )
}

/// The divider's words.
pub fn divider_text(unread_count: i64, say: &dyn StringCatalog) -> String {
    say.plural("%lld new messages", unread_count, &[&unread_count])
}
