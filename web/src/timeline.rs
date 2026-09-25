//! How a chat's messages are laid out as rows — the same rules the apps
//! draw by (ios Models/Snapshots.swift, `MessagePresentation`).
//!
//! Pure over what it is given, so every rule is tested without drawing
//! anything: day sections with one pill each, the single unread divider,
//! whose name shows above a bubble, where a run ends and the time shows,
//! which rows hide behind a block, and which of mine the peer has seen.

use std::collections::HashSet;

use crate::model::Message;
use crate::time::{self, Day};

/// Everything the layout depends on that is not the messages themselves.
pub struct Context<'a> {
    pub my_user_id: i64,
    /// UNDEFAULTED on purpose, in the apps and here: a new surface that
    /// forgot to say would otherwise draw the family chat's seen ticks —
    /// which is exactly how it shipped wrong there once.
    pub is_family_chat: bool,
    pub is_ai_chat: bool,
    pub assistant_user_id: Option<i64>,
    pub blocked: &'a HashSet<i64>,
    /// The first message the reader had not read when the chat was OPENED —
    /// decided once per open, so the divider does not walk as they read.
    pub first_unread_id: Option<i64>,
    /// The peer's read marker, in a direct chat.
    pub peer_read_up_to: i64,
}

/// One drawn row.
#[derive(Debug, Clone, PartialEq)]
pub struct Row<'a> {
    pub message: &'a Message,
    /// The day pill above this row: the first row of each local day.
    pub day_above: Option<Day>,
    /// "N new messages" goes above this row — at most one row in a chat.
    pub unread_divider_above: bool,
    /// The sender's name above the bubble.
    pub shows_sender: bool,
    /// The last of a run by one sender: the time shows here.
    pub run_end: bool,
    /// Drawn as "Hidden — blocked member" until revealed.
    pub hidden: bool,
    /// One of mine the peer has read — a direct chat only.
    pub seen: bool,
    /// An assistant answer that has not arrived yet: the working state.
    pub awaited: bool,
}

/// Whether a message draws as the hidden row: the sender is blocked. Never
/// my own — blocking yourself is refused, so this is belt and braces.
pub fn is_hidden_by_block(message: &Message, my_user_id: i64, blocked: &HashSet<i64>) -> bool {
    message.sender_id != my_user_id && blocked.contains(&message.sender_id)
}

/// Whether a row is an assistant answer still being written. Asked of the
/// MESSAGE and nothing else — EMPTY and carrying nothing, not mine, and
/// from somebody who can be the assistant: anything not mine in its own
/// chat, or its reserved account in the family chat. A set of ids a live
/// `ai_delta` touched would forget the state on reload, which is the
/// blank bubble this rule exists to remove (ios isAwaitedAssistantAnswer).
pub fn is_awaited_answer(message: &Message, context: &Context) -> bool {
    let carries_nothing = message.body.is_empty()
        && message.attachments().is_empty()
        && message.poll.is_none()
        && message.call.is_none();
    if !carries_nothing || message.sender_id == context.my_user_id || message.id == 0 {
        return false;
    }
    context.is_ai_chat || Some(message.sender_id) == context.assistant_user_id
}

/// Lay a chat's messages (oldest first) out as rows.
pub fn rows<'a>(messages: &'a [Message], context: &Context) -> Vec<Row<'a>> {
    let mut rows = Vec::with_capacity(messages.len());
    let mut current_day: Option<Day> = None;
    let mut divider_placed = false;
    for (index, message) in messages.iter().enumerate() {
        // A pending row has no timestamp yet and belongs to the day it is
        // being written on — the section it already sits at the end of.
        let day = time::day(&message.created_at).or(current_day);
        let new_day = day != current_day;
        if new_day {
            current_day = day;
        }
        let previous = index
            .checked_sub(1)
            .map(|at| &messages[at])
            .filter(|_| !new_day);
        let next = messages.get(index + 1).filter(|next| {
            let next_day = time::day(&next.created_at).or(current_day);
            next_day == current_day
        });

        let hidden = is_hidden_by_block(message, context.my_user_id, context.blocked);
        let shows_sender = context.is_family_chat
            && message.sender_id != context.my_user_id
            && !hidden
            && previous.is_none_or(|previous| previous.sender_id != message.sender_id);

        // At most one divider, structurally.
        let unread_divider_above = !divider_placed
            && context
                .first_unread_id
                .is_some_and(|first| message.id == first);
        divider_placed |= unread_divider_above;

        let seen = !context.is_family_chat
            && message.sender_id == context.my_user_id
            && message.id != 0
            && message.id <= context.peer_read_up_to;

        rows.push(Row {
            message,
            day_above: if new_day { day } else { None },
            unread_divider_above,
            shows_sender,
            run_end: next.is_none_or(|next| next.sender_id != message.sender_id),
            hidden,
            seen,
            awaited: is_awaited_answer(message, context),
        });
    }
    rows
}

/// How far back from the newest row an opening may anchor. The apps render
/// a bounded window of rows and refuse to anchor past it; this client
/// follows the same cap so the same chat opens at the same place.
pub const ANCHOR_CAP: usize = 300;

/// Where a chat OPENS: the first unread row, under which the "N new
/// messages" divider is drawn — or None, which is the ordinary open at the
/// newest. Decided ONCE per open (ios Views/UnreadAnchor.swift,
/// `openAnchor`):
///
/// 1. nothing unread → the newest;
/// 2. with a read marker, the oldest row from somebody else above it —
///    unless the rows held do not include every unread one, which means the
///    oldest unread message is older than anything loaded, and any row
///    picked would be the wrong one;
/// 3. with no marker (0: never reported), count back `unread_count` rows
///    from somebody else, newest first — and give up on a short count;
/// 4. and beyond the cap, give up too.
///
/// Only a numbered row from somebody else can be unread — the server's own
/// predicate, spelled here so the two cannot drift.
pub fn open_anchor(
    messages: &[Message],
    unread_count: i64,
    last_read: i64,
    my_user_id: i64,
) -> Option<i64> {
    if unread_count <= 0 {
        return None;
    }
    let inbound: Vec<(usize, i64)> = messages
        .iter()
        .rev()
        .enumerate()
        .filter(|(_, message)| message.id != 0 && message.sender_id != my_user_id)
        .map(|(distance, message)| (distance, message.id))
        .collect();
    let wanted = unread_count as usize;
    let hit = if last_read > 0 {
        let above: Vec<&(usize, i64)> = inbound.iter().filter(|(_, id)| *id > last_read).collect();
        if above.len() < wanted {
            return None;
        }
        above.last().copied().copied()
    } else {
        if inbound.len() < wanted {
            return None;
        }
        Some(inbound[wanted - 1])
    };
    let (distance, id) = hit?;
    (distance <= ANCHOR_CAP).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use js_sys::Date;
    use wasm_bindgen_test::*;

    const ME: i64 = 7;
    const ANNA: i64 = 9;
    const BOB: i64 = 11;
    const ASSISTANT: i64 = 2;

    fn at(day: i32, hour: i32) -> String {
        Date::new_with_year_month_day_hr_min(2026, 8, day, hour, 0)
            .to_iso_string()
            .into()
    }

    fn message(id: i64, sender: i64, created_at: String) -> Message {
        Message {
            id,
            chat_id: 42,
            sender_id: sender,
            body: format!("message {id}"),
            created_at,
            ..Message::default()
        }
    }

    fn family<'a>(blocked: &'a HashSet<i64>) -> Context<'a> {
        Context {
            my_user_id: ME,
            is_family_chat: true,
            is_ai_chat: false,
            assistant_user_id: Some(ASSISTANT),
            blocked,
            first_unread_id: None,
            peer_read_up_to: 0,
        }
    }

    #[wasm_bindgen_test]
    fn each_local_day_starts_a_section_with_its_own_pill() {
        let messages = vec![
            message(1, ANNA, at(9, 10)),
            message(2, ANNA, at(9, 11)),
            message(3, ANNA, at(10, 9)),
        ];
        let blocked = HashSet::new();
        let rows = rows(&messages, &family(&blocked));
        assert!(rows[0].day_above.is_some());
        assert!(rows[1].day_above.is_none(), "same day, no second pill");
        assert!(rows[2].day_above.is_some(), "a new day");
        // A run never crosses a day: the name shows again after the pill.
        assert!(rows[0].shows_sender && !rows[1].shows_sender && rows[2].shows_sender);
        assert!(rows[1].run_end, "the day ends the run");
    }

    /// The name: family chat only, never mine, never on a hidden row, and
    /// only when the sender changes — a hidden row still counts as its
    /// sender's run, so the next visible sender keeps their caption.
    #[wasm_bindgen_test]
    fn the_sender_name_shows_where_the_sender_changes_in_the_family_chat() {
        let messages = vec![
            message(1, ANNA, at(9, 10)),
            message(2, ANNA, at(9, 10)),
            message(3, ME, at(9, 10)),
            message(4, BOB, at(9, 10)),
            message(5, ANNA, at(9, 10)),
        ];
        let blocked = HashSet::from([BOB]);
        let rows = rows(&messages, &family(&blocked));
        let names: Vec<bool> = rows.iter().map(|row| row.shows_sender).collect();
        assert_eq!(names, vec![true, false, false, false, true]);
        assert!(rows[3].hidden, "Bob is blocked");
        assert!(!rows[2].hidden, "my own message is never hidden");

        let direct = Context {
            is_family_chat: false,
            ..family(&blocked)
        };
        assert!(
            super::rows(&messages, &direct)
                .iter()
                .all(|row| !row.shows_sender),
            "a direct chat has one other person; the name is noise"
        );
    }

    #[wasm_bindgen_test]
    fn the_time_shows_at_the_end_of_each_run() {
        let messages = vec![
            message(1, ANNA, at(9, 10)),
            message(2, ANNA, at(9, 10)),
            message(3, ME, at(9, 10)),
        ];
        let blocked = HashSet::new();
        let ends: Vec<bool> = rows(&messages, &family(&blocked))
            .iter()
            .map(|row| row.run_end)
            .collect();
        assert_eq!(ends, vec![false, true, true]);
    }

    /// One divider, above the first unread row, decided by the caller once.
    #[wasm_bindgen_test]
    fn the_unread_divider_sits_above_the_first_unread_row_and_only_once() {
        let messages = vec![
            message(1, ANNA, at(9, 10)),
            message(2, ANNA, at(9, 11)),
            message(3, ANNA, at(9, 12)),
        ];
        let blocked = HashSet::new();
        let context = Context {
            first_unread_id: Some(2),
            ..family(&blocked)
        };
        let dividers: Vec<bool> = rows(&messages, &context)
            .iter()
            .map(|row| row.unread_divider_above)
            .collect();
        assert_eq!(dividers, vec![false, true, false]);
    }

    /// The marker branch: the oldest inbound row above the marker — and
    /// nothing at all when the rows held cannot account for the count.
    #[wasm_bindgen_test]
    fn a_chat_opens_at_the_oldest_unread_row_above_the_marker() {
        let messages = vec![
            message(1, ANNA, at(9, 10)),
            message(2, ME, at(9, 11)),
            message(3, ANNA, at(9, 12)),
            message(4, ANNA, at(9, 13)),
        ];
        assert_eq!(open_anchor(&messages, 2, 2, ME), Some(3));
        assert_eq!(open_anchor(&messages, 0, 2, ME), None, "nothing unread");
        assert_eq!(
            open_anchor(&messages, 5, 2, ME),
            None,
            "the oldest unread is older than anything held: no row is the right one"
        );
    }

    /// The count-back branch, for a marker of 0 — and my own rows are never
    /// counted.
    #[wasm_bindgen_test]
    fn with_no_marker_the_anchor_counts_back_from_the_newest() {
        let messages = vec![
            message(1, ANNA, at(9, 10)),
            message(2, ANNA, at(9, 11)),
            message(3, ME, at(9, 12)),
            message(4, ANNA, at(9, 13)),
        ];
        assert_eq!(open_anchor(&messages, 2, 0, ME), Some(2));
        assert_eq!(
            open_anchor(&messages, 4, 0, ME),
            None,
            "a short count gives up"
        );
    }

    #[wasm_bindgen_test]
    fn an_anchor_past_the_cap_is_refused() {
        let mut messages: Vec<Message> = (1..=400).map(|id| message(id, ANNA, at(9, 10))).collect();
        messages.push(message(401, ME, at(9, 11)));
        assert_eq!(open_anchor(&messages, 350, 50, ME), None);
        assert!(open_anchor(&messages, 10, 390, ME).is_some());
    }

    /// Seen ticks: a direct chat only, mine only, numbered only, and only
    /// up to the peer's marker. NEVER in the family chat.
    #[wasm_bindgen_test]
    fn seen_is_a_direct_chat_fact_about_my_numbered_messages() {
        let mut pending = message(0, ME, String::new());
        pending.client_msg_id = Some("p".into());
        let messages = vec![
            message(1, ME, at(9, 10)),
            message(2, ME, at(9, 11)),
            message(3, ANNA, at(9, 12)),
            pending,
        ];
        let blocked = HashSet::new();
        let direct = Context {
            is_family_chat: false,
            peer_read_up_to: 1,
            ..family(&blocked)
        };
        let seen: Vec<bool> = rows(&messages, &direct)
            .iter()
            .map(|row| row.seen)
            .collect();
        assert_eq!(seen, vec![true, false, false, false]);

        let in_family = Context {
            peer_read_up_to: 99,
            ..family(&blocked)
        };
        assert!(rows(&messages, &in_family).iter().all(|row| !row.seen));
    }

    /// The working state is a fact about the message: empty, carrying
    /// nothing, not mine, and from somebody who can be the assistant.
    #[wasm_bindgen_test]
    fn an_empty_assistant_answer_reads_as_still_working() {
        let blocked = HashSet::new();
        let mut empty = message(8, ASSISTANT, at(9, 10));
        empty.body.clear();
        assert!(is_awaited_answer(&empty, &family(&blocked)));

        // Anybody else's empty message in the family chat is not it.
        let mut from_anna = message(8, ANNA, at(9, 10));
        from_anna.body.clear();
        assert!(!is_awaited_answer(&from_anna, &family(&blocked)));

        // In its own chat, anything not mine is the assistant's.
        let ai = Context {
            is_family_chat: false,
            is_ai_chat: true,
            assistant_user_id: None,
            ..family(&blocked)
        };
        assert!(is_awaited_answer(&from_anna, &ai));

        // A picture answer is an answer.
        empty.attachments = Some(vec![crate::model::Attachment {
            id: 1,
            kind: "photo".into(),
            ..Default::default()
        }]);
        assert!(!is_awaited_answer(&empty, &family(&blocked)));
    }

    /// A pending row has no timestamp: it stays in the section it is being
    /// written in rather than opening a section of its own.
    #[wasm_bindgen_test]
    fn a_pending_row_joins_the_day_it_is_written_on() {
        let mut pending = message(0, ME, String::new());
        pending.client_msg_id = Some("p".into());
        let messages = vec![message(1, ANNA, at(9, 10)), pending];
        let blocked = HashSet::new();
        let rows = rows(&messages, &family(&blocked));
        assert!(rows[1].day_above.is_none(), "no pill of its own");
    }
}
