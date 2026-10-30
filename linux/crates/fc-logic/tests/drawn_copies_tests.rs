//! `DrawnCopiesTests`: every copy drawn of one voice message — the conversation's bubble and the
//! thread panel's both hear that it was played, and a redraw's new copy never displaces the other
//! surface's.

use std::rc::Rc;

use fc_logic::drawn_copies::DrawnCopies;

#[derive(Debug)]
struct Row {
    #[allow(dead_code)]
    place: &'static str,
}

fn row(place: &'static str) -> Rc<Row> {
    Rc::new(Row { place })
}

fn same(left: &[Rc<Row>], right: &[&Rc<Row>]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(one, other)| Rc::ptr_eq(one, other))
}

/// Two copies of one note — the conversation's and the thread panel's — are both kept, not the
/// last drawn.
#[test]
fn both_copies_of_one_note_are_kept() {
    let mut copies = DrawnCopies::<i64, Row>::new();
    let conversation = row("conversation");
    let thread = row("thread");
    copies.add(40, conversation.clone());
    copies.add(40, thread.clone());
    assert!(same(&copies.of(&40), &[&conversation, &thread]));
    assert_eq!(copies.all().len(), 2);
    // The same copy drawn again (back on screen) is still one.
    copies.add(40, thread.clone());
    assert_eq!(copies.of(&40).len(), 2);
}

/// A copy that leaves the screen goes by reference; the other surface's stays, and an empty key
/// is gone.
#[test]
fn a_copy_leaving_takes_only_itself() {
    let mut copies = DrawnCopies::<i64, Row>::new();
    let old = row("conversation, before the redraw");
    let redrawn = row("conversation, after");
    let thread = row("thread");
    copies.add(40, old.clone());
    copies.add(40, thread.clone());
    copies.add(40, redrawn.clone());
    assert!(copies.remove(&40, &old));
    assert!(same(&copies.of(&40), &[&thread, &redrawn]));
    assert!(!copies.remove(&40, &old));
    assert!(!copies.remove(&41, &old));
    copies.remove(&40, &thread);
    copies.remove(&40, &redrawn);
    assert!(copies.of(&40).is_empty());
    assert!(copies.all().is_empty());
    copies.add(7, thread.clone());
    copies.clear();
    assert!(copies.all().is_empty());
}
