//! `ConversationRowsTests`: which rows a conversation's redraw keeps, and how the box is turned
//! into the rows wanted — only what changed is built, nothing kept ever moves, and anything that
//! cannot be done that way falls back to building it all, as before.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use fc_core::protocol::{AttachmentDto, MessageDto, ReactionDto, ReplyToDto};
use fc_logic::conversation::Bubble;
use fc_logic::conversation_rows::{self, RowMark};
use fc_logic::timeline::TimelineRow;
use time::macros::date;

fn mark(key: &str) -> RowMark {
    RowMark::new(key, "s")
}

fn marked(key: &str, signature: &str) -> RowMark {
    RowMark::new(key, signature)
}

/// A widget: a thing with an identity, equal only to itself — what `glib::Object`'s `==` is, and
/// what the C# test made of a string compared by reference.
#[derive(Clone, Debug)]
struct Element(Rc<str>);

impl PartialEq for Element {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Element {}

impl Hash for Element {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::ptr::hash(Rc::as_ptr(&self.0) as *const u8, state);
    }
}

/// Elements by name, ONE per name — as the C#'s interned string literals were.
#[derive(Default)]
struct Elements(HashMap<String, Element>);

impl Elements {
    fn get(&mut self, name: &str) -> Element {
        self.0
            .entry(name.to_string())
            .or_insert_with(|| Element(Rc::from(name)))
            .clone()
    }

    fn all(&mut self, names: &[&str]) -> Vec<Element> {
        names.iter().map(|name| self.get(name)).collect()
    }
}

/// A box that records what was done to it — the window's children, played by a list.
#[derive(Default)]
struct Panel {
    children: Vec<Element>,
    done: Vec<String>,
}

impl Panel {
    fn remove_at(&mut self, index: usize) {
        self.done.push(format!("-{}", self.children[index].0));
        self.children.remove(index);
    }

    fn insert(&mut self, index: usize, element: &Element) {
        assert!(
            !self.children.contains(element),
            "{} is still in the box",
            element.0
        );
        self.done.push(format!("+{}@{index}", element.0));
        self.children.insert(index, element.clone());
    }
}

fn names(elements: &[Element]) -> Vec<&str> {
    elements.iter().map(|element| &*element.0).collect()
}

fn merge_into(panel: &mut Panel, shown: &mut Vec<Element>, wanted: &[Element]) -> Option<usize> {
    let panel = std::cell::RefCell::new(panel);
    conversation_rows::merge(
        shown,
        wanted,
        |index| panel.borrow_mut().remove_at(index),
        |index, element| panel.borrow_mut().insert(index, element),
    )
}

fn merge(before: &[&str], after: &[&str]) -> (Option<usize>, Panel, Vec<Element>) {
    let mut elements = Elements::default();
    let mut panel = Panel {
        children: elements.all(before),
        done: Vec::new(),
    };
    let mut shown = panel.children.clone();
    let wanted = elements.all(after);
    let changes = merge_into(&mut panel, &mut shown, &wanted);
    (changes, panel, shown)
}

#[test]
fn an_unchanged_row_is_kept_and_a_changed_one_is_built() {
    let plan = conversation_rows::plan(
        &[mark("m1"), mark("m2"), mark("m3")],
        &[mark("m1"), marked("m2", "edited"), mark("m3")],
        None,
    );
    assert_eq!(plan, vec![Some(0), None, Some(2)]);
}

/// What arrived goes below, a page of history above, and every row already drawn is kept where it
/// is.
#[test]
fn arrivals_and_history_keep_every_row_already_drawn() {
    let plan = conversation_rows::plan(
        &[mark("m5"), mark("m6")],
        &[
            mark("m3"),
            mark("m4"),
            mark("m5"),
            mark("m6"),
            mark("m7"),
            mark("p-a"),
        ],
        None,
    );
    assert_eq!(plan, vec![None, None, Some(0), Some(1), None, None]);
}

/// A row whose drawing never finished is built again — the retry every redraw used to give it.
#[test]
fn a_row_that_never_finished_drawing_is_built_again() {
    let plan = conversation_rows::plan(
        &[mark("m1"), mark("m2"), mark("m3")],
        &[mark("m1"), mark("m2"), mark("m3")],
        Some(&|index| index != 1),
    );
    assert_eq!(plan, vec![Some(0), None, Some(2)]);
}

/// Kept rows never move: one that would have to jump back over another is built again instead.
#[test]
fn a_row_that_would_have_to_move_is_built_again() {
    let plan = conversation_rows::plan(
        &[mark("a"), mark("b"), mark("c")],
        &[mark("c"), mark("a"), mark("b")],
        None,
    );
    assert_eq!(plan, vec![Some(2), None, None]);
}

#[test]
fn a_key_that_appears_twice_is_never_kept() {
    assert_eq!(
        conversation_rows::plan(
            &[mark("a"), mark("b"), mark("a")],
            &[mark("a"), mark("b")],
            None
        ),
        vec![None, Some(1)]
    );
    assert_eq!(
        conversation_rows::plan(
            &[mark("a"), mark("b")],
            &[mark("a"), mark("a"), mark("b")],
            None
        ),
        vec![None, None, Some(1)]
    );
}

#[test]
fn nothing_drawn_builds_everything() {
    assert_eq!(
        conversation_rows::plan(&[], &[mark("a"), mark("b")], None),
        vec![None, None]
    );
}

/// The window slides: what fell off the top goes, what arrived is added, and the rows in between
/// are untouched.
#[test]
fn the_merge_removes_and_inserts_and_never_touches_what_it_keeps() {
    let (changes, panel, shown) = merge(&["day", "a", "b", "c"], &["b", "c", "d", "pending"]);
    assert_eq!(changes, Some(4));
    assert_eq!(names(&panel.children), vec!["b", "c", "d", "pending"]);
    assert_eq!(panel.children, shown);
    assert_eq!(panel.done, vec!["-a", "-day", "+d@2", "+pending@3"]);
}

#[test]
fn a_page_of_history_is_inserted_above_and_a_changed_row_in_place() {
    let (changes, panel, _) = merge(&["b", "c-old", "d"], &["pill", "a", "b", "c-new", "d"]);
    assert_eq!(changes, Some(4));
    assert_eq!(names(&panel.children), vec!["pill", "a", "b", "c-new", "d"]);
    assert_eq!(panel.done, vec!["-c-old", "+pill@0", "+a@1", "+c-new@3"]);
}

#[test]
fn nothing_changed_is_no_change_at_all() {
    let (changes, panel, _) = merge(&["a", "b"], &["a", "b"]);
    assert_eq!(changes, Some(0));
    assert!(panel.done.is_empty());
}

/// Kept elements out of order would have to MOVE, and inserting one still in the box fails: the
/// merge says so before it touches anything, and the caller builds the whole box as it used to.
#[test]
fn kept_elements_out_of_order_change_nothing_and_ask_for_a_whole_rebuild() {
    let (changes, panel, shown) = merge(&["a", "b", "c"], &["c", "a"]);
    assert_eq!(changes, None);
    assert!(panel.done.is_empty());
    assert_eq!(names(&shown), vec!["a", "b", "c"]);
}

#[test]
fn the_same_element_wanted_twice_asks_for_a_whole_rebuild() {
    let twice = Element(Rc::from("x"));
    let mut panel = Panel::default();
    let mut shown = Vec::new();
    assert_eq!(
        merge_into(&mut panel, &mut shown, &[twice.clone(), twice]),
        None
    );
    assert!(panel.done.is_empty());
}

/// Elements are compared by identity: an equal string that is another element is another element.
#[test]
fn elements_are_the_same_only_when_they_are_the_same_object() {
    let kept = Element(Rc::from("a"));
    let lookalike = Element(Rc::from("a"));
    let mut panel = Panel {
        children: vec![kept.clone()],
        done: Vec::new(),
    };
    let mut shown = vec![kept];
    let changes = merge_into(&mut panel, &mut shown, std::slice::from_ref(&lookalike));
    assert_eq!(changes, Some(2));
    assert!(Rc::ptr_eq(&shown[0].0, &lookalike.0));
}

/// `new TimelineRow(new Bubble(message, hidden, revealed, false), null, false, false, true,
/// runEnd)`.
fn row(message: MessageDto, hidden: bool, revealed: bool, run_end: bool) -> TimelineRow {
    TimelineRow {
        bubble: Bubble::new(message, hidden, revealed, false),
        day_above: None,
        unread_divider_above: false,
        shows_sender: false,
        run_start: true,
        run_end,
    }
}

fn plain(message: MessageDto) -> TimelineRow {
    row(message, false, false, true)
}

fn message() -> MessageDto {
    MessageDto::new(5, 42, 9, None, "Dinner at 7?", "2026-09-12T10:00:00Z")
}

fn reacted() -> MessageDto {
    MessageDto {
        reactions: Some(vec![ReactionDto::new(7, "👍")]),
        ..message()
    }
}

/// The signature changes with EVERYTHING a row is drawn from — every field of the message (a
/// trimmed preview made whole carries no new sequence number), whether it reads, its place in the
/// timeline and what the view adds — and with nothing else: the same message read twice from the
/// store signs the same.
#[test]
fn the_signature_follows_everything_the_row_is_drawn_from() {
    let signature = conversation_rows::signature;
    let unchanged = signature(&plain(message()), "seen");
    assert_eq!(unchanged, signature(&plain(message().clone()), "seen"));
    assert_eq!(
        signature(&plain(reacted()), "seen"),
        signature(&plain(reacted()), "seen")
    );

    let changed = [
        signature(
            &plain(MessageDto {
                body: "Dinner at 8?".to_string(),
                ..message()
            }),
            "seen",
        ),
        signature(
            &plain(MessageDto {
                reply_count: Some(2),
                ..message()
            }),
            "seen",
        ),
        signature(&plain(reacted()), "seen"),
        signature(
            &plain(MessageDto {
                reply_to: Some(ReplyToDto {
                    message_id: 3,
                    sender_id: 7,
                    excerpt: "hi".to_string(),
                    parent: None,
                }),
                ..message()
            }),
            "seen",
        ),
        signature(
            &plain(MessageDto {
                attachments: Some(vec![AttachmentDto::new(8, "photo")]),
                ..message()
            }),
            "seen",
        ),
        signature(&row(message(), true, false, true), "seen"),
        signature(&row(message(), true, true, true), "seen"),
        signature(&row(message(), false, false, false), "seen"),
        signature(
            &TimelineRow {
                day_above: Some(date!(2026 - 09 - 12)),
                ..plain(message())
            },
            "seen",
        ),
        signature(
            &TimelineRow {
                unread_divider_above: true,
                ..plain(message())
            },
            "seen",
        ),
        signature(
            &TimelineRow {
                shows_sender: true,
                ..plain(message())
            },
            "seen",
        ),
        signature(&plain(message()), "sent"),
    ];
    assert!(!changed.contains(&unchanged));
    let distinct: std::collections::HashSet<&String> = changed.iter().collect();
    assert_eq!(distinct.len(), changed.len());
}

#[test]
fn keys_name_their_kind_of_row() {
    assert_eq!(conversation_rows::message_key(12), "m12");
    assert_eq!(conversation_rows::pending_key("abc"), "pabc");
    assert_ne!(
        conversation_rows::message_key(12),
        conversation_rows::pending_key("12")
    );
}

/// The port's own: the day above a row is signed as .NET's `DateOnly.DayNumber` counts it — days
/// since 0001-01-01.
#[test]
fn the_day_above_is_signed_by_its_day_number() {
    let day = |date| {
        conversation_rows::signature(
            &TimelineRow {
                day_above: Some(date),
                ..plain(message())
            },
            "",
        )
    };
    // DateOnly(2026, 9, 12).DayNumber is 739 870.
    assert!(day(date!(2026 - 09 - 12)).contains("\u{1F}739870\u{1F}"));
    assert!(day(date!(0001 - 01 - 01)).contains("\u{1F}0\u{1F}"));
}
