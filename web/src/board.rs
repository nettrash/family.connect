//! The family board as this tab holds it, and every rule about changing it
//! (docs/protocol.md, "Board"). Pure, like store.rs: nothing here talks to
//! the server or draws anything, so each rule is tested on its own.
//!
//! Three rules the Mac app gets wrong, kept here on purpose:
//! - A FULL READ REPLACES what is held. It never returns tombstones, so a
//!   note it leaves out is a note that is gone — except one held at a
//!   `board_seq` above the read's mark, which a frame brought after the read
//!   was taken. Merely applying the notes it returned left every note
//!   deleted while nobody was listening on the wall for good.
//! - ONLY A FRAME AND A CATCH-UP PAGE MOVE THE CURSOR, and a frame only once
//!   this connection has caught up. The note in the answer to this device's
//!   own create, move or RSVP is evidence about that one note, and REST
//!   works while the socket is down — which is when the frames with lower
//!   seqs were missed.
//! - A NOTE SEEN DELETED STAYS DELETED. Ids are never reused, so an older
//!   copy arriving late — a page, a frame, a reply already in flight — is
//!   never allowed to bring one back.

use std::collections::{HashMap, HashSet};

use fc_text::board::{self as rules, Marks};

use crate::model::Note;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Board {
    /// id → the live note as last applied.
    pub notes: HashMap<i64, Note>,
    /// Every id this tab has seen deleted.
    pub gone: HashSet<i64>,
    /// The sync cursor: every change at or below it has been applied.
    pub cursor: i64,
    /// Whether this session has read the whole wall. Until it has there is
    /// nothing to catch up FROM.
    pub loaded: bool,
    /// Whether the board has caught up on the CURRENT connection. Until it
    /// has, a frame applies its note and moves no cursor: a frame for seq 130
    /// arriving before a catch-up from 100 had read its cursor would have the
    /// catch-up start past everything between.
    pub caught_up: bool,
    /// Hidden notes the reader has peeked at: per note, per tab, never on
    /// the wire and gone on reload (docs/protocol.md, "Board").
    pub revealed: HashSet<i64>,
    /// How much of the wall this browser has shown this account — the
    /// badge's marks, which are NOT the cursor: the cursor moves on a resync
    /// nobody saw.
    pub marks: Marks,
    /// Whose marks those are: 0 until `/me` has said who this is.
    pub marks_for: i64,
    /// A photo on its way to the wall.
    pub pinning: bool,
    /// The `max_board_seq` of the newest full read applied. Two can be in
    /// flight at once — the one before the socket opened and the one it
    /// opened with — and the older landing second must change nothing: it
    /// could list a note deleted since that this tab never held, and put it
    /// back beside a cursor already past its tombstone.
    pub full_mark: Option<i64>,
}

impl Board {
    /// One copy of one note, under the per-note guard: written only when its
    /// `board_seq` is above the one held, so an out-of-order copy cannot undo
    /// a newer move. A tombstone always wins — deleting is the last thing
    /// that ever happens to a note. Answers whether the wall changed.
    pub fn apply(&mut self, note: Note) -> bool {
        if self.gone.contains(&note.id) {
            return false;
        }
        if note.deleted {
            self.gone.insert(note.id);
            return self.notes.remove(&note.id).is_some();
        }
        // A live note with no author, text, colour or place is a server
        // fault; dropping it beats drawing a blank sticker.
        if !note.is_drawable() {
            return false;
        }
        if self
            .notes
            .get(&note.id)
            .is_some_and(|held| note.board_seq <= held.board_seq)
        {
            return false;
        }
        self.notes.insert(note.id, note);
        true
    }

    /// A live frame: applied, and the cursor moved — once this connection
    /// has caught up.
    pub fn apply_frame(&mut self, note: Note) {
        let seq = note.board_seq;
        self.apply(note);
        if self.caught_up {
            self.cursor = self.cursor.max(seq);
        }
    }

    /// One page of the change feed: every note applied, and the cursor moved
    /// to the highest seq on it.
    pub fn apply_page(&mut self, page: Vec<Note>) {
        let high = page.iter().map(|note| note.board_seq).max();
        for note in page {
            self.apply(note);
        }
        if let Some(high) = high {
            self.cursor = self.cursor.max(high);
        }
    }

    /// The whole wall, which REPLACES what is held: every held note the read
    /// left out is gone, unless it is held at a seq above the read's mark —
    /// a frame that landed after the read was taken. The cursor goes to the
    /// mark, which the server reads BEFORE the notes and so is never past a
    /// change they missed.
    pub fn apply_full(&mut self, notes: Vec<Note>, max_board_seq: i64) {
        if self.full_mark.is_some_and(|mark| max_board_seq < mark) {
            return;
        }
        self.full_mark = Some(max_board_seq);
        let listed: HashSet<i64> = notes.iter().map(|note| note.id).collect();
        let left_out: Vec<i64> = self
            .notes
            .values()
            .filter(|held| !listed.contains(&held.id) && held.board_seq <= max_board_seq)
            .map(|held| held.id)
            .collect();
        for id in left_out {
            self.forget(id);
        }
        for note in notes {
            self.apply(note);
        }
        self.cursor = self.cursor.max(max_board_seq);
        self.loaded = true;
    }

    /// A note known to be gone — this device's own delete was answered, or
    /// the server said there is no such note.
    pub fn forget(&mut self, id: i64) {
        self.notes.remove(&id);
        self.gone.insert(id);
        self.revealed.remove(&id);
    }

    /// The wall in the order it is drawn: oldest change first, so the note
    /// somebody has just written or moved lies on top.
    pub fn drawn(&self) -> Vec<Note> {
        let mut notes: Vec<Note> = self.notes.values().cloned().collect();
        notes.sort_by_key(|note| (note.board_seq, note.id));
        notes
    }

    /// How many notes have something this browser has not shown its reader.
    pub fn unread(&self) -> usize {
        self.notes
            .values()
            .filter(|note| rules::is_unread(note.id, note.content_seq, self.marks))
            .count()
    }

    /// The marks as they would be if the wall were shown now — what a view
    /// watches, so the marks move when something new lands and not when a
    /// note is dragged.
    pub fn marks_if_shown(&self) -> Marks {
        rules::marks_after_showing(
            self.notes.values().map(|note| (note.id, note.content_seq)),
            self.marks,
        )
    }

    /// The wall has been on screen: the marks rise to it. Answers whether
    /// they moved, which is when they are worth keeping.
    pub fn shown(&mut self) -> bool {
        let after = self.marks_if_shown();
        let moved = after != self.marks;
        self.marks = after;
        moved
    }

    /// The marks kept for `user_id`, taken on once `/me` has said who this
    /// is — and never walked back by a copy another tab kept lower.
    pub fn take_marks(&mut self, user_id: i64, kept: Marks) {
        if self.marks_for == user_id {
            self.marks = rules::later(self.marks, kept);
        } else {
            self.marks = kept;
            self.marks_for = user_id;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    fn note(id: i64, board_seq: i64) -> Note {
        Note {
            id,
            board_seq,
            author_id: Some(7),
            text: Some(format!("note {id} at {board_seq}")),
            color: Some("yellow".into()),
            x: Some(0.1),
            y: Some(0.2),
            content_seq: Some(board_seq),
            ..Note::default()
        }
    }

    fn moved(id: i64, board_seq: i64, content_seq: i64) -> Note {
        Note {
            x: Some(0.5),
            content_seq: Some(content_seq),
            ..note(id, board_seq)
        }
    }

    fn tombstone(id: i64, board_seq: i64) -> Note {
        Note {
            id,
            board_seq,
            deleted: true,
            ..Note::default()
        }
    }

    fn ids(board: &Board) -> Vec<i64> {
        board.drawn().iter().map(|note| note.id).collect()
    }

    /// An out-of-order copy cannot undo a newer move.
    #[wasm_bindgen_test]
    fn an_older_copy_never_undoes_a_newer_one() {
        let mut board = Board::default();
        assert!(board.apply(note(1, 10)));
        assert!(board.apply(moved(1, 12, 10)));
        assert!(!board.apply(note(1, 11)), "older");
        assert!(!board.apply(moved(1, 12, 10)), "the same one again");
        assert_eq!(board.notes[&1].board_seq, 12);
        assert_eq!(board.notes[&1].x, Some(0.5));
    }

    /// Deleted is deleted: a tombstone takes the note off, and no older copy
    /// — from a page, a frame or a reply already in flight — brings it back.
    #[wasm_bindgen_test]
    fn a_note_seen_deleted_never_comes_back() {
        let mut board = Board::default();
        board.apply(note(1, 10));
        assert!(board.apply(tombstone(1, 20)));
        assert!(board.notes.is_empty());
        assert!(!board.apply(note(1, 15)), "an older live copy");
        assert!(
            !board.apply(note(1, 25)),
            "any live copy at all: ids are never reused"
        );
        assert!(board.notes.is_empty());
        // A tombstone for a note never held is remembered all the same.
        assert!(!board.apply(tombstone(2, 30)));
        assert!(!board.apply(note(2, 5)));
        assert!(board.notes.is_empty());
        // And this device's own delete is as final.
        board.apply(note(3, 40));
        board.forget(3);
        assert!(!board.apply(note(3, 41)));
        assert!(board.notes.is_empty());
    }

    /// A live copy missing what every live note has is a server fault, and
    /// it is refused rather than drawn blank.
    #[wasm_bindgen_test]
    fn a_broken_live_note_is_refused() {
        let mut board = Board::default();
        let mut broken = note(1, 10);
        broken.author_id = None;
        assert!(!board.apply(broken));
        let mut nowhere = note(2, 10);
        nowhere.x = Some(f64::NAN);
        assert!(!board.apply(nowhere));
        assert!(board.notes.is_empty());
    }

    /// THE MAC'S BUG: a full read only ever ADDED. Here a note it leaves
    /// out is gone — and a note a frame brought after the read was taken is
    /// not.
    #[wasm_bindgen_test]
    fn a_full_read_replaces_what_is_held() {
        let mut board = Board::default();
        board.apply(note(1, 10));
        board.apply(note(2, 11));
        board.apply(note(3, 60)); // a frame, after the read was taken
        board.apply_full(vec![note(2, 11), note(4, 30)], 50);
        assert_eq!(ids(&board), vec![2, 4, 3]);
        assert!(board.gone.contains(&1), "left out is gone");
        assert!(!board.apply(note(1, 10)), "and stays gone");
        assert_eq!(board.cursor, 50);
        assert!(board.loaded);
        // An empty read of an empty wall empties it.
        board.apply_full(Vec::new(), 70);
        assert!(board.notes.is_empty());
        assert_eq!(board.cursor, 70);
    }

    /// Two full reads landing out of order agree with each other: the older
    /// one cannot resurrect, move back or remove what the newer one said.
    #[wasm_bindgen_test]
    fn full_reads_landing_out_of_order_agree() {
        let mut board = Board::default();
        // Newer read: 1 moved to 55, 2 deleted at 53, 5 created at 52.
        board.apply_full(vec![moved(1, 55, 10), note(5, 52)], 56);
        board.apply(tombstone(2, 53));
        // Older read: 1 at 10, 2 still live at 20.
        board.apply_full(vec![note(1, 10), note(2, 20)], 50);
        assert_eq!(ids(&board), vec![5, 1]);
        assert_eq!(board.notes[&1].board_seq, 55);
        assert_eq!(board.cursor, 56, "the cursor never walks back");
    }

    /// THE REVIEW'S RACE: an older full read landing after a newer one
    /// must not bring back a note deleted between them — even one this tab
    /// never held, which nothing else would remember was gone.
    #[wasm_bindgen_test]
    fn an_older_full_read_landing_second_changes_nothing() {
        let mut board = Board::default();
        // Newer (mark 60): note 2 already deleted, so not listed.
        board.apply_full(vec![note(1, 10)], 60);
        // Older (mark 50): note 2 still live.
        board.apply_full(vec![note(1, 10), note(2, 20)], 50);
        assert_eq!(ids(&board), vec![1], "the deleted note did not come back");
        assert_eq!(board.cursor, 60);
        // A read at the same mark is the same wall, and applies.
        board.apply_full(vec![note(1, 10), note(3, 55)], 60);
        assert_eq!(ids(&board), vec![1, 3]);
    }

    /// Frames move the cursor only once this connection has caught up;
    /// pages always do; and neither ever walks it back.
    #[wasm_bindgen_test]
    fn only_a_caught_up_frame_or_a_page_moves_the_cursor() {
        let mut board = Board::default();
        board.apply_frame(note(1, 130));
        assert_eq!(
            board.cursor, 0,
            "before the catch-up: applied, cursor still"
        );
        assert!(board.notes.contains_key(&1));
        board.apply_page(vec![note(2, 101), tombstone(3, 104)]);
        assert_eq!(board.cursor, 104);
        board.caught_up = true;
        board.apply_frame(note(4, 140));
        assert_eq!(board.cursor, 140);
        board.apply_page(vec![note(5, 120)]);
        assert_eq!(board.cursor, 140, "never back");
        board.apply_page(Vec::new());
        assert_eq!(board.cursor, 140);
        // A REST reply is `apply` alone: it moves nothing.
        board.apply(note(6, 200));
        assert_eq!(board.cursor, 140);
    }

    /// Drawn oldest change first, so what was just written or moved is on
    /// top.
    #[wasm_bindgen_test]
    fn the_last_touched_note_is_drawn_on_top() {
        let mut board = Board::default();
        board.apply(note(1, 10));
        board.apply(note(2, 20));
        board.apply(moved(1, 30, 10));
        assert_eq!(ids(&board), vec![2, 1]);
    }

    /// The badge counts words, not tidying — and the wall being shown clears
    /// it for everything on it, a hidden note included.
    #[wasm_bindgen_test]
    fn the_badge_counts_what_there_is_to_read() {
        let mut board = Board::default();
        board.take_marks(
            7,
            Marks {
                note_id: 2,
                content_seq: 20,
            },
        );
        board.apply(note(1, 10));
        board.apply(note(3, 30));
        assert_eq!(board.unread(), 1);
        board.apply(moved(1, 40, 10));
        assert_eq!(board.unread(), 1, "a move is not news");
        assert_eq!(board.marks_if_shown(), board.marks_if_shown());
        assert!(board.shown());
        assert_eq!(board.unread(), 0);
        assert!(!board.shown(), "nothing new: the marks stay put");
        // A rewrite of an old note is news.
        board.apply(moved(1, 50, 50));
        assert_eq!(board.unread(), 1);
        // A note from an older server is judged by its id.
        let mut old = note(9, 60);
        old.content_seq = None;
        board.apply(old);
        assert_eq!(board.unread(), 2);
    }

    /// Whose marks: taken whole for a new account, never walked back for the
    /// same one.
    #[wasm_bindgen_test]
    fn the_marks_belong_to_one_account_and_only_rise() {
        let mut board = Board::default();
        let high = Marks {
            note_id: 9,
            content_seq: 90,
        };
        let low = Marks {
            note_id: 3,
            content_seq: 30,
        };
        board.take_marks(7, high);
        board.take_marks(7, low);
        assert_eq!(board.marks, high);
        board.take_marks(8, low);
        assert_eq!(board.marks, low, "another account's are their own");
        assert_eq!(board.marks_for, 8);
    }
}
