//! `NoteHandTests`: a sticker in hand — the web client's drag and keyboard tests, ported.

use fc_logic::note_hand::{HandResult, Letting, NoteHand};

const WALL: (f64, f64) = (1000.0, 1600.0);
const CARD: (f64, f64) = (150.0, 110.0);

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn a_few_pixels_are_a_click_not_a_drag() {
    let mut hand = NoteHand::new();
    hand.down(1, (100.0, 100.0), (0.1, 0.1), CARD, WALL);
    assert!(!hand.move_to(1, (102.0, 101.0)));
    assert!(!hand.dragging());
    assert_eq!(
        Letting::new(HandResult::Click),
        hand.up(1, (0.1, 0.1), CARD, WALL)
    );
    assert!(!hand.sending());
}

/// A drag follows the pointer, is one write on release, and is held where it was dropped until
/// answered.
#[test]
fn a_drag_reports_where_it_was_dropped_and_is_held_there_until_answered() {
    let mut hand = NoteHand::new();
    let fraction = (0.1, 0.1);
    hand.down(1, (100.0, 100.0), fraction, CARD, WALL);
    assert!(hand.move_to(1, (160.0, 130.0)));
    assert!(hand.move_to(1, (190.0, 150.0)));
    assert!(hand.dragging());
    assert_eq!((190.0, 210.0), hand.corner(fraction, CARD, WALL));

    let letting = hand.up(1, fraction, CARD, WALL);
    assert_eq!(HandResult::Drop, letting.result);
    assert!(letting.send_now);
    let target = letting.target.expect("a target");
    assert!(near(0.19, target.0) && near(210.0 / 1600.0, target.1));
    assert!(hand.held());
    assert_eq!((190.0, 210.0), hand.corner(fraction, CARD, WALL));

    // A move that came to nothing lets go, and the note is where the server has it.
    assert_eq!(None, hand.answered());
    assert!(!hand.held());
    assert_eq!((100.0, 160.0), hand.corner(fraction, CARD, WALL));
}

/// One move at a time: a second drop while the first is out waits, and only the LATEST waiting is
/// sent.
#[test]
fn moves_of_one_note_go_one_at_a_time_and_the_latest_wins() {
    let mut hand = NoteHand::new();
    let fraction = (0.1, 0.1);
    hand.nudge(0.01 * WALL.0, 0.0, fraction, CARD, WALL);
    let first = hand.put_down(fraction, CARD, WALL);
    assert!(first.send_now);
    assert!(near(0.11, first.target.expect("a target").0));

    hand.nudge(0.01 * WALL.0, 0.0, fraction, CARD, WALL);
    let second = hand.put_down(fraction, CARD, WALL);
    assert!(!second.send_now, "waits behind the first");
    hand.nudge(0.0, 0.01 * WALL.1, fraction, CARD, WALL);
    assert!(!hand.put_down(fraction, CARD, WALL).send_now);
    // Drawn where it was LAST put.
    assert!(near(0.12 * WALL.0, hand.corner(fraction, CARD, WALL).0));

    let next = hand.answered().expect("the drop that waited");
    assert!(near(0.12, next.0) && near(0.11, next.1));
    assert!(hand.sending());
    assert_eq!(None, hand.answered());
    assert!(!hand.held());
}

/// An arrow moves a hundredth of the wall a press, nothing is written while it is down, and letting
/// go puts it down.
#[test]
fn the_arrow_keys_move_a_note_and_letting_go_puts_it_down() {
    let mut hand = NoteHand::new();
    let fraction = (0.2, 0.3);
    let square = (1000.0, 1000.0);
    for _ in 0..3 {
        hand.nudge(0.01 * square.0, 0.0, fraction, CARD, square);
    }
    hand.nudge(0.0, -0.01 * square.1, fraction, CARD, square);
    assert!(hand.dragging());
    assert!(!hand.sending(), "nothing written while the key is down");
    let put = hand.put_down(fraction, CARD, square);
    let target = put.target.expect("a target");
    assert!(near(0.23, target.0) && near(0.29, target.1));
    // A pointer release has nothing to put down after the keyboard has.
    assert_eq!(
        Letting::new(HandResult::None),
        hand.put_down(fraction, CARD, square)
    );
}

#[test]
fn a_drag_is_held_inside_the_wall_and_a_press_past_the_edge_is_not_owed_back() {
    let mut hand = NoteHand::new();
    let fraction = (0.0, 0.0);
    hand.down(1, (10.0, 10.0), fraction, CARD, WALL);
    hand.move_to(1, (-500.0, -500.0));
    assert_eq!((0.0, 0.0), hand.corner(fraction, CARD, WALL));
    hand.cancel(1);

    hand.nudge(-100.0, 0.0, fraction, CARD, WALL);
    hand.nudge(20.0, 0.0, fraction, CARD, WALL);
    assert_eq!((20.0, 0.0), hand.corner(fraction, CARD, WALL));
}

/// A cancelled drag goes back where it was; another pointer is somebody else's.
#[test]
fn a_cancelled_drag_is_put_back_and_another_pointer_is_ignored() {
    let mut hand = NoteHand::new();
    let fraction = (0.1, 0.1);
    hand.down(1, (100.0, 100.0), fraction, CARD, WALL);
    assert!(!hand.move_to(2, (300.0, 300.0)));
    hand.move_to(1, (180.0, 100.0));
    hand.cancel(2);
    assert!(hand.dragging());
    hand.cancel(1);
    assert!(!hand.dragging());
    assert_eq!((100.0, 160.0), hand.corner(fraction, CARD, WALL));
    assert_eq!(
        Letting::new(HandResult::None),
        hand.up(1, fraction, CARD, WALL)
    );
}

/// Picked up again before its move landed, a note travels from where it was DROPPED.
#[test]
fn a_pick_up_before_the_answer_starts_from_where_it_was_dropped() {
    let mut hand = NoteHand::new();
    let fraction = (0.1, 0.1);
    hand.down(1, (100.0, 100.0), fraction, CARD, WALL);
    hand.move_to(1, (200.0, 100.0));
    hand.up(1, fraction, CARD, WALL);
    hand.down(1, (0.0, 0.0), fraction, CARD, WALL);
    hand.move_to(1, (50.0, 0.0));
    assert_eq!((250.0, 160.0), hand.corner(fraction, CARD, WALL));
}

/// The focus going while a POINTER holds the note is not the keyboard's put-down: the pointer still
/// has it, and its release decides where it lands.
#[test]
fn losing_the_focus_does_not_drop_a_note_the_pointer_still_holds() {
    let mut hand = NoteHand::new();
    let fraction = (0.1, 0.1);
    hand.down(1, (100.0, 100.0), fraction, CARD, WALL);
    hand.move_to(1, (300.0, 200.0));

    assert_eq!(
        Letting::new(HandResult::None),
        hand.put_down(fraction, CARD, WALL)
    );
    assert!(hand.dragging());
    assert!(!hand.sending());
    assert_eq!(HandResult::Drop, hand.up(1, fraction, CARD, WALL).result);
}
