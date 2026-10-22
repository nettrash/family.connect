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
    /// The chat's kind as the server sent it (`family` | `direct` | `ai` |
    /// `group`), which decides sender names and seen ticks through the one
    /// table (fc_text::groups::offers). UNDEFAULTED on purpose, in the apps
    /// and here: a new surface that forgot to say would otherwise draw the
    /// family chat's seen ticks — which is exactly how it shipped wrong
    /// there once.
    pub chat_kind: &'a str,
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
    context.chat_kind == fc_text::groups::AI || Some(message.sender_id) == context.assistant_user_id
}

/// Lay a chat's messages (oldest first) out as rows.
pub fn rows<'a>(messages: &'a [Message], context: &Context) -> Vec<Row<'a>> {
    // A group draws as the family chat does: names, and no "seen"
    // (docs/protocol.md, "Groups").
    let offers = fc_text::groups::offers(context.chat_kind);
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
        let shows_sender = offers.sender_names
            && message.sender_id != context.my_user_id
            && !hidden
            && previous.is_none_or(|previous| previous.sender_id != message.sender_id);

        // At most one divider, structurally.
        let unread_divider_above = !divider_placed
            && context
                .first_unread_id
                .is_some_and(|first| message.id == first);
        divider_placed |= unread_divider_above;

        let seen = offers.seen
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

/// The rows from `start` on — exactly `rows(messages, context)[start..]`,
/// without laying out the rows above them that nobody is drawing.
///
/// A row's layout looks one row back (its day, and whether the sender
/// changed) and one row forward, and carries the day along through rows
/// with no timestamp of their own. So the walk starts at the nearest row
/// above `start` that HAS a day — usually the one just above — which is
/// where the full walk would have its day too, and that row is dropped. The
/// divider needs no history: it sits over one server id, and ids are unique
/// in a thread.
pub fn rows_from<'a>(messages: &'a [Message], start: usize, context: &Context) -> Vec<Row<'a>> {
    let start = start.min(messages.len());
    let mut seed = start.saturating_sub(1);
    while seed > 0 && time::day(&messages[seed].created_at).is_none() {
        seed -= 1;
    }
    let mut laid = rows(&messages[seed..], context);
    laid.drain(..start - seed);
    laid
}

/// How many of a chat's newest rows its pane draws — on opening, and while
/// its reader follows along at the bottom.
///
/// Every row is a bubble, with its markdown, its pictures and its menus,
/// and a chat opened after a long absence held every message it had ever
/// loaded — hundreds, after a catch-up — all of them laid out and drawn
/// again on every page that arrived, while the view tried to keep up at
/// the bottom. A reader sees a screen or two of it; the rest waits above
/// and is drawn as they scroll up to it (`Window::grow`).
///
/// Far more than a screen, and comfortably more than the rows below an
/// unread divider are allowed to be (`ANCHOR_CAP` is how far back an
/// opening may anchor; `Window::include` takes the window that far when it
/// does).
pub const WINDOW: usize = 100;

/// How many more rows scrolling near the top of the window draws.
pub const WINDOW_STEP: usize = 50;

/// How many rows above a row the window is taken to — the divider, a quoted
/// message being jumped to — so it is not the very first thing drawn.
pub const WINDOW_CONTEXT: usize = 10;

/// Which of a chat's rows its pane draws: from one row down to the newest.
///
/// Held as the server id of the oldest row drawn rather than as a count or
/// an index, because rows arrive at BOTH ends: a page of history lands
/// above (and a window that started at the oldest held keeps starting
/// there — "Earlier messages" is a request to SEE them), and a catch-up
/// lands below (and a reader up in the history keeps every row they are
/// reading). Only a reader following along at the bottom has the top of
/// the window cut back, which nothing on their screen shows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// The oldest row drawn, by server id — `i64::MAX` for the first
    /// pending row — or None to draw from the oldest held.
    top: Option<i64>,
}

impl Window {
    /// Where drawing starts in `messages` (oldest first, pending last), for
    /// this draw. `following` is a reader at the newest message: their
    /// window is the newest `WINDOW` rows. Anybody else's is never fewer
    /// than that either — a row that went (a pending bubble taken back) is
    /// made up from above, not left as a window drawing nothing.
    pub fn start(&mut self, messages: &[Message], following: bool) -> usize {
        let newest = messages.len().saturating_sub(WINDOW);
        let start = if following {
            newest
        } else {
            self.held_start(messages).min(newest)
        };
        self.set_start(messages, start);
        start
    }

    /// Where the window starts now, without changing it.
    pub fn held_start(&self, messages: &[Message]) -> usize {
        match self.top {
            None => 0,
            Some(top) => messages.partition_point(|held| held.id != 0 && held.id < top),
        }
    }

    /// Take the window up to row `index`, and `WINDOW_CONTEXT` rows above
    /// it — never down: a window already above it stays where it is.
    pub fn include(&mut self, messages: &[Message], index: usize) {
        let wanted = index.saturating_sub(WINDOW_CONTEXT);
        if wanted < self.held_start(messages) {
            self.set_start(messages, wanted);
        }
    }

    /// `WINDOW_STEP` more rows above. Whether there was anything to draw.
    pub fn grow(&mut self, messages: &[Message]) -> bool {
        let start = self.held_start(messages);
        if start == 0 {
            return false;
        }
        self.set_start(messages, start.saturating_sub(WINDOW_STEP));
        true
    }

    fn set_start(&mut self, messages: &[Message], start: usize) {
        self.top = match messages.get(start) {
            _ if start == 0 => None,
            Some(message) if message.id != 0 => Some(message.id),
            Some(_) => Some(i64::MAX),
            None => Some(i64::MAX),
        };
    }
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
            chat_kind: "family",
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
            chat_kind: "direct",
            ..family(&blocked)
        };
        assert!(
            super::rows(&messages, &direct)
                .iter()
                .all(|row| !row.shows_sender),
            "a direct chat has one other person; the name is noise"
        );
        // A group names its senders exactly as the family chat does.
        let group = Context {
            chat_kind: "group",
            ..family(&blocked)
        };
        let in_group: Vec<bool> = super::rows(&messages, &group)
            .iter()
            .map(|row| row.shows_sender)
            .collect();
        assert_eq!(in_group, names);
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
            chat_kind: "direct",
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

        // A group has many readers, as the family chat does: no "seen",
        // whatever a marker says (docs/protocol.md, "Groups").
        let in_group = Context {
            chat_kind: "group",
            peer_read_up_to: 99,
            ..family(&blocked)
        };
        assert!(rows(&messages, &in_group).iter().all(|row| !row.seen));
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
            chat_kind: "ai",
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

    /// The rows from any row on are exactly those rows of the whole layout:
    /// across day boundaries, runs, a hidden sender, the divider (above the
    /// start and below it) and the pending tail.
    #[wasm_bindgen_test]
    fn the_rows_from_a_start_are_the_whole_layouts_rows_from_there() {
        let mut messages: Vec<Message> = Vec::new();
        for id in 1..=60 {
            let sender = [ANNA, ANNA, ME, BOB, ANNA][(id % 5) as usize];
            messages.push(message(
                id,
                sender,
                at(9 + (id / 17) as i32, (id % 24) as i32),
            ));
        }
        for tail in ["p1", "p2"] {
            let mut pending = message(0, ME, String::new());
            pending.client_msg_id = Some(tail.into());
            messages.push(pending);
        }
        let blocked = HashSet::from([BOB]);
        for first_unread_id in [None, Some(5), Some(40)] {
            let context = Context {
                first_unread_id,
                peer_read_up_to: 30,
                ..family(&blocked)
            };
            let whole = rows(&messages, &context);
            for start in 0..=messages.len() {
                assert_eq!(
                    rows_from(&messages, start, &context),
                    whole[start..].to_vec(),
                    "from row {start}, divider at {first_unread_id:?}"
                );
            }
        }
    }

    fn numbered(count: i64) -> Vec<Message> {
        (1..=count).map(|id| message(id, ANNA, at(9, 10))).collect()
    }

    /// A reader following along sees the newest `WINDOW` rows, however many
    /// arrive — and a short chat is drawn whole.
    #[wasm_bindgen_test]
    fn a_reader_at_the_newest_is_drawn_the_newest_window() {
        let mut window = Window::default();
        assert_eq!(window.start(&numbered(40), true), 0, "a short chat, whole");
        assert_eq!(window.start(&numbered(500), true), 500 - WINDOW);
        assert_eq!(window.start(&numbered(650), true), 650 - WINDOW);
    }

    /// Up in the history, the window keeps every row it drew while rows
    /// arrive below — and while a page of history arrives above, a window
    /// that started at the oldest held keeps starting there.
    #[wasm_bindgen_test]
    fn a_reader_in_the_history_keeps_every_row_drawn() {
        let mut window = Window::default();
        let held = numbered(500);
        let start = window.start(&held, true);
        assert_eq!(start, 400);
        // The reader scrolls up: nothing moves while more arrive below.
        assert_eq!(window.start(&numbered(700), false), 400);

        // A window drawn from the oldest held…
        let mut whole = Window::default();
        let recent: Vec<Message> = (51..=130).map(|id| message(id, ANNA, at(9, 10))).collect();
        assert_eq!(whole.start(&recent, false), 0);
        // …still is once "Earlier messages" has put fifty above it.
        let paged: Vec<Message> = (1..=130).map(|id| message(id, ANNA, at(9, 10))).collect();
        assert_eq!(whole.start(&paged, false), 0, "the page is drawn");
    }

    /// Scrolling up draws `WINDOW_STEP` more at a time, down to the oldest
    /// held; a row to be shown — the divider, a quoted message — takes the
    /// window up to it with some context, and never down.
    #[wasm_bindgen_test]
    fn the_window_grows_upward_and_reaches_what_must_be_shown() {
        let held = numbered(500);
        let mut window = Window::default();
        window.start(&held, true);
        assert!(window.grow(&held));
        assert_eq!(window.start(&held, false), 400 - WINDOW_STEP);
        for _ in 0..20 {
            window.grow(&held);
        }
        assert_eq!(window.start(&held, false), 0);
        assert!(!window.grow(&held), "nothing left above");

        let mut window = Window::default();
        window.start(&held, true);
        window.include(&held, 200);
        assert_eq!(window.start(&held, false), 200 - WINDOW_CONTEXT);
        window.include(&held, 450);
        assert_eq!(
            window.start(&held, false),
            200 - WINDOW_CONTEXT,
            "a row already drawn moves nothing"
        );
        window.include(&held, 3);
        assert_eq!(window.start(&held, false), 0);
    }

    /// A window is never fewer than `WINDOW` rows: a row that goes — a
    /// pending bubble taken back — is made up from above.
    #[wasm_bindgen_test]
    fn a_window_is_never_left_short() {
        let mut held = numbered(300);
        let mut pending = message(0, ME, String::new());
        pending.client_msg_id = Some("p".into());
        held.push(pending);
        let mut window = Window::default();
        assert_eq!(window.start(&held, true), held.len() - WINDOW);
        held.pop();
        assert_eq!(window.start(&held, false), held.len() - WINDOW);
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
