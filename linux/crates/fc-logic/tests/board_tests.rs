//! `BoardModelTests`: the family's wall — what is on it, where, the badge over it, and the writes
//! that change it.

mod common;

use std::rc::Rc;

use fc_core::board::board_picture;
use fc_core::protocol::{AttachmentDto, NoteDto, NotePatch, NoteRequest};
use fc_core::store::{BoardStore, ChatStore, Database, SeqRoute, StoreResult};
use fc_core::testing::{FakeClock, drive};
use fc_logic::board::BoardModel;
use fc_text::board::{Answer, Kind, Size, origin, tilt_degrees};
use time::macros::datetime;

use common::server::{Server, api};

const ME: i64 = 7;

struct Rig {
    board: Rc<BoardStore>,
    chats: Rc<ChatStore>,
    clock: FakeClock,
}

impl Rig {
    fn new() -> StoreResult<Rig> {
        let cache = Database::open_in_memory()?;
        Ok(Rig {
            board: Rc::new(BoardStore::new(&cache)),
            chats: Rc::new(ChatStore::new(&cache).with_me(|| ME)),
            clock: FakeClock::at(datetime!(2026-09-12 12:00 UTC)),
        })
    }

    fn model(&self, server: &Server) -> BoardModel {
        BoardModel::new(
            self.board.clone(),
            self.chats.clone(),
            api(server, &self.clock),
        )
    }

    fn ids(&self) -> StoreResult<Vec<i64>> {
        Ok(self.board.notes()?.iter().map(|note| note.id).collect())
    }
}

/// The C# `Note(id, seq, kind, text, author, x, y, contentSeq, attachment, size)`.
#[derive(Clone)]
struct N {
    id: i64,
    seq: i64,
    kind: &'static str,
    text: Option<&'static str>,
    author: i64,
    x: f64,
    y: f64,
    content_seq: Option<i64>,
    attachment: Option<AttachmentDto>,
}

fn n(id: i64, seq: i64) -> N {
    N {
        id,
        seq,
        kind: "text",
        text: Some("Milk"),
        author: 7,
        x: 0.25,
        y: 0.25,
        content_seq: None,
        attachment: None,
    }
}

impl N {
    fn note(self) -> NoteDto {
        NoteDto {
            kind: Some(self.kind.to_string()),
            text: self.text.map(str::to_string),
            color: Some("yellow".to_string()),
            size: Some("medium".to_string()),
            font: Some("plain".to_string()),
            x: self.x,
            y: self.y,
            board_seq: self.seq,
            content_seq: Some(self.content_seq.unwrap_or(self.seq)),
            attachment: self.attachment,
            ..NoteDto::new(self.id, self.author)
        }
    }
}

fn note_json(id: i64, seq: i64, text: &str) -> String {
    format!(
        r#"{{"note": {{"id": {id}, "author_id": 7, "kind": "text", "text": "{text}",
                  "color": "yellow", "size": "medium", "font": "plain", "x": 0.5, "y": 0.5,
                  "board_seq": {seq}, "content_seq": {seq}}}}}"#
    )
}

fn photo(id: i64, width: i32, height: i32) -> AttachmentDto {
    AttachmentDto {
        width: Some(width),
        height: Some(height),
        ..AttachmentDto::new(id, "photo")
    }
}

#[test]
fn a_sticker_is_placed_and_sized_by_the_shared_arithmetic() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.board.replace(
        &[N {
            x: 0.5,
            y: 0.25,
            ..n(12, 11)
        }
        .note()],
        11,
    )?;
    let model = rig.model(&Server::new());

    let wall = BoardModel::wall_size(1000.0, 800.0);
    let stickers = model.stickers(1000.0, 800.0)?;
    assert_eq!(stickers.len(), 1);
    let sticker = &stickers[0];

    // The same module the oracle pins to the web client's Rust decides all of this.
    let card = Size::Medium.frame(fc_text::board::is_compact(wall.0));
    let (x, y) = origin((0.5, 0.25), card, wall);
    assert_eq!(x, sticker.x);
    assert_eq!(y, sticker.y);
    assert_eq!(card.0, sticker.width);
    assert_eq!(card.1, sticker.height);
    assert_eq!(tilt_degrees(12), sticker.tilt_degrees);
    assert_eq!(Kind::Text, sticker.kind);
    assert!(sticker.mine);
    Ok(())
}

/// A stored fraction is CLAMPED as it is drawn: 0.98 hugs the edge rather than hanging off it,
/// which is what a wall that has been resized smaller since the drop leaves behind.
#[test]
fn a_note_pinned_at_the_edge_is_drawn_inside_the_wall() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.board.replace(
        &[N {
            x: 0.98,
            y: 0.99,
            ..n(12, 11)
        }
        .note()],
        11,
    )?;
    let model = rig.model(&Server::new());

    let wall = BoardModel::wall_size(1000.0, 800.0);
    let stickers = model.stickers(1000.0, 800.0)?;
    assert_eq!(stickers.len(), 1);
    let sticker = &stickers[0];

    assert_eq!(wall.0 - sticker.width, sticker.x);
    assert_eq!(wall.1 - sticker.height, sticker.y);
    // Fully on the wall, which is the whole of the rule.
    assert!(sticker.x + sticker.width <= wall.0);
    assert!(sticker.y + sticker.height <= wall.1);
    Ok(())
}

#[test]
fn somebody_elses_note_is_not_yours() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.board.replace(
        &[
            n(12, 11).note(),
            N {
                author: 11,
                x: 0.1,
                ..n(13, 12)
            }
            .note(),
        ],
        12,
    )?;
    let model = rig.model(&Server::new());

    let stickers = model.stickers(1000.0, 800.0)?;

    assert!(stickers.iter().find(|s| s.note.id == 12).unwrap().mine);
    assert!(!stickers.iter().find(|s| s.note.id == 13).unwrap().mine);
    Ok(())
}

/// A BARE PHOTO'S CARD IS THE PICTURE, fitted into the card it would otherwise have had — the rule
/// issue #71 settled. A photo with a caption keeps the ordinary card.
#[test]
fn a_bare_photos_card_is_the_picture_and_a_captioned_ones_is_not() -> StoreResult<()> {
    let rig = Rig::new()?;
    let tall = photo(34, 600, 1200);
    rig.board.replace(
        &[
            N {
                kind: "photo",
                text: Some(""),
                attachment: Some(tall.clone()),
                ..n(12, 11)
            }
            .note(),
            N {
                kind: "photo",
                text: Some("at the lake"),
                attachment: Some(tall),
                x: 0.1,
                y: 0.1,
                ..n(13, 12)
            }
            .note(),
        ],
        12,
    )?;
    let model = rig.model(&Server::new());

    let wall = BoardModel::wall_size(1000.0, 800.0);
    let card = Size::Medium.frame(fc_text::board::is_compact(wall.0));
    let stickers = model.stickers(1000.0, 800.0)?;
    let bare = stickers.iter().find(|s| s.note.id == 12).unwrap();
    let captioned = stickers.iter().find(|s| s.note.id == 13).unwrap();

    let fitted = board_picture::fitted(card.0, card.1, Some(600), Some(1200));
    assert_eq!(fitted.0, bare.width);
    assert_eq!(fitted.1, bare.height);
    // Fitted in BOTH dimensions, so a tall photograph comes back narrow and never cropped.
    assert!(bare.width < card.0);

    assert_eq!(card.0, captioned.width);
    assert_eq!(card.1, captioned.height);
    Ok(())
}

/// THE BADGE COUNTS WHAT IS NEW TO READ. A note somebody dragged, recoloured or answered is not
/// news — `content_seq` is — and the marks move only when the wall has been on screen.
#[test]
fn the_badge_counts_news_and_showing_the_wall_clears_it() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.board
        .replace(&[n(12, 11).note(), n(13, 12).note()], 12)?;
    let model = rig.model(&Server::new());
    assert_eq!(2, model.badge()?);
    assert!(model.stickers(1000.0, 800.0)?.iter().all(|s| s.unread));

    model.shown()?;
    assert_eq!(0, model.badge()?);
    assert!(model.stickers(1000.0, 800.0)?.iter().all(|s| !s.unread));

    // Somebody drags it across the wall: board_seq moves, content_seq does not, and the badge stays
    // quiet — on the wall as well as in the count.
    rig.board.apply(
        &N {
            x: 0.9,
            content_seq: Some(11),
            ..n(12, 20)
        }
        .note(),
        SeqRoute::CatchUpPage,
    )?;
    assert_eq!(0, model.badge()?);
    assert!(model.stickers(1000.0, 800.0)?.iter().all(|s| !s.unread));

    // Somebody rewrites what it says, and that IS news.
    rig.board.apply(
        &N {
            text: Some("Milk and eggs"),
            content_seq: Some(21),
            ..n(12, 21)
        }
        .note(),
        SeqRoute::CatchUpPage,
    )?;
    assert_eq!(1, model.badge()?);
    let stickers = model.stickers(1000.0, 800.0)?;
    assert!(stickers.iter().find(|s| s.note.id == 12).unwrap().unread);
    assert!(!stickers.iter().find(|s| s.note.id == 13).unwrap().unread);
    Ok(())
}

#[test]
fn writing_a_note_applies_the_answer_the_server_made_of_it() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on("/families/mine/board/notes", &note_json(12, 11, "Milk"));
    let model = rig.model(&server);

    assert_eq!(
        None,
        drive(
            &rig.clock,
            model.create(&NoteRequest::new("Milk", "yellow", 0.5, 0.5))
        )?
    );

    let notes = rig.board.notes()?;
    assert_eq!(notes.len(), 1);
    assert_eq!(Some("Milk"), notes[0].text.as_deref());
    // The answer to our OWN write moves no cursor: it is one note's news and says nothing about
    // what else has happened, and a cursor moved by it would step the changes feed past somebody
    // else's change.
    assert_eq!(0, rig.board.cursor()?);
    Ok(())
}

/// A POSITION IS A FRACTION OF THE WALL, read back from where the card actually is and clamped
/// first: a note dropped past the edge is stored hugging it, not hanging off it.
#[test]
fn moving_a_note_stores_the_fraction_it_was_dropped_at() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.board.replace(&[n(12, 11).note()], 11)?;
    let server = Server::new();
    server.on("/families/mine/board/notes/12", &note_json(12, 12, "Milk"));
    let model = rig.model(&server);
    let wall = BoardModel::wall_size(1000.0, 800.0);
    let card = Size::Medium.frame(fc_text::board::is_compact(wall.0));

    // Dropped well past the right-hand edge.
    assert_eq!(
        None,
        drive(
            &rig.clock,
            model.move_note(12, (wall.0 + 500.0, 0.0), card, wall)
        )?
    );

    let bodies = server.bodies();
    assert_eq!(bodies.len(), 1);
    let patch: NotePatch = serde_json::from_str(&bodies[0]).expect("a patch");
    // Clamped to the edge and then turned into a fraction, so the note is fully on the wall.
    assert!(((wall.0 - card.0) / wall.0 - patch.x.unwrap()).abs() < 1e-6);
    assert_eq!(0.0, patch.y.unwrap());
    // ONLY what changed: a patch carrying the text would overwrite an edit made in between.
    assert_eq!(None, patch.text);
    assert_eq!(None, patch.color);
    Ok(())
}

#[test]
fn taking_a_note_down_takes_it_off_the_wall_at_once() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.board
        .replace(&[n(12, 11).note(), n(13, 12).note()], 12)?;
    let server = Server::new();
    server.on_status("/families/mine/board/notes/12", 204, None);
    let model = rig.model(&server);

    assert_eq!(None, drive(&rig.clock, model.delete(12))?);

    assert_eq!(vec![13], rig.ids()?);
    // And the board's cursor did not move: the seq this delete was given is the server's to
    // announce, and a cursor moved to a number nobody issued steps the feed past whatever really
    // holds it.
    assert_eq!(12, rig.board.cursor()?);
    // It stays down, too, even if an older copy of it is still travelling.
    rig.board.apply(&n(12, 99).note(), SeqRoute::CatchUpPage)?;
    assert_eq!(vec![13], rig.ids()?);
    Ok(())
}

#[test]
fn a_refused_delete_leaves_the_note_where_it_is() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.board.replace(&[n(12, 11).note()], 11)?;
    let server = Server::new();
    server.on_status(
        "/families/mine/board/notes/12",
        403,
        Some(r#"{"error": {"code": "not_note_author", "message": "no"}}"#),
    );
    let model = rig.model(&server);

    let refused = drive(&rig.clock, model.delete(12))?;

    assert_eq!("not_note_author", refused.expect("refused").code);
    assert_eq!(1, rig.board.notes()?.len());
    Ok(())
}

#[test]
fn answering_an_event_is_a_state_set_and_retracting_is_not_a_no() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.board.replace(
        &[N {
            kind: "event",
            text: Some("Gran's birthday"),
            ..n(12, 11)
        }
        .note()],
        11,
    )?;
    let server = Server::new();
    server.on(
        "/families/mine/board/notes/12/rsvp",
        r#"
            {"note": {"id": 12, "author_id": 7, "kind": "event", "text": "Gran's birthday",
                      "board_seq": 12, "content_seq": 11,
                      "rsvps": [{"user_id": 7, "answer": "going"}]}}
            "#,
    );
    let model = rig.model(&server);

    assert_eq!(None, drive(&rig.clock, model.answer(12, Answer::Going))?);

    let note = rig.board.note(12)?.expect("held");
    assert_eq!(Some("going"), note.my_answer(ME));
    let bodies = server.bodies();
    assert_eq!(bodies.len(), 1);
    assert!(bodies[0].contains("\"answer\":\"going\""));
    // Answering is not news: the words did not change, so the badge stays where it was.
    assert_eq!(Some(11), note.content_seq);
    Ok(())
}

#[test]
fn ticking_a_line_is_per_line_and_the_answer_is_the_whole_note() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.board.replace(
        &[N {
            kind: "tasks",
            text: Some("Shopping"),
            ..n(12, 11)
        }
        .note()],
        11,
    )?;
    let server = Server::new();
    server.on(
        "/families/mine/board/notes/12/tasks/5",
        r#"
            {"note": {"id": 12, "author_id": 7, "kind": "tasks", "text": "Shopping",
                      "board_seq": 12, "content_seq": 12,
                      "items": [{"id": 5, "text": "Milk", "done": true, "done_by": 7},
                                {"id": 6, "text": "Eggs", "done": false}]}}
            "#,
    );
    let model = rig.model(&server);

    assert_eq!(None, drive(&rig.clock, model.tick(12, 5, true))?);

    let note = rig.board.note(12)?.expect("held");
    let items = note.task_list();
    assert!(items[0].done);
    assert_eq!(Some(7), items[0].done_by);
    assert!(!items[1].done);
    let bodies = server.bodies();
    assert_eq!(bodies.len(), 1);
    assert!(bodies[0].contains("\"done\":true"));
    Ok(())
}

/// A REDRAWN BACKDROP IS A NEW ATTACHMENT — the answer carries a new id, and a caller that cached
/// the picture by NOTE would show the old one for ever (issue #70, on three platforms).
#[test]
fn a_drawn_backdrop_comes_back_as_a_new_attachment() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.board.replace(
        &[N {
            kind: "event",
            text: Some("Gran's birthday"),
            attachment: Some(photo(34, 1024, 512)),
            ..n(12, 11)
        }
        .note()],
        11,
    )?;
    let server = Server::new();
    server.on(
        "/families/mine/board/notes/12/backdrop",
        r#"
            {"note": {"id": 12, "author_id": 7, "kind": "event", "text": "Gran's birthday",
                      "board_seq": 12, "content_seq": 11,
                      "attachment": {"id": 77, "kind": "photo", "width": 1024, "height": 512}}}
            "#,
    );
    let model = rig.model(&server);

    let drawn = drive(&rig.clock, model.draw_backdrop(12))?;

    // WHAT THE ASSISTANT DREW, from the answer itself rather than read back out of the store — the
    // caller redraws by attachment id, and an answer that crossed a newer change on the wire would
    // otherwise hand back the picture already on the wall.
    assert_eq!(77, drawn.expect("drawn").expect("an attachment").id);
    let note = rig.board.note(12)?.expect("held");
    assert_eq!(77, note.attachment.as_ref().unwrap().id);
    // The picture changed and the words did not: a redraw is not news either.
    assert_eq!(Some(11), note.content_seq);
    Ok(())
}

#[test]
fn a_refused_draw_says_why_and_changes_nothing() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.board.replace(
        &[N {
            kind: "event",
            text: Some("Gran's birthday"),
            ..n(12, 11)
        }
        .note()],
        11,
    )?;
    let server = Server::new();
    server.on_status(
        "/families/mine/board/notes/12/backdrop",
        503,
        Some(r#"{"error": {"code": "ai_unavailable", "message": "no"}}"#),
    );
    let model = rig.model(&server);

    let drawn = drive(&rig.clock, model.draw_backdrop(12))?;

    assert_eq!("ai_unavailable", drawn.expect_err("refused").code);
    assert_eq!(None, rig.board.note(12)?.expect("held").attachment);
    Ok(())
}

/// A backdrop answer that crossed a newer change still says what was DRAWN: the store keeps the
/// newer note, and the caller is told the attachment it asked about.
#[test]
fn a_draw_answer_says_what_was_drawn_even_when_the_store_keeps_something_newer() -> StoreResult<()>
{
    let rig = Rig::new()?;
    rig.board.replace(
        &[N {
            kind: "event",
            text: Some("Gran's birthday"),
            content_seq: Some(30),
            attachment: Some(photo(34, 1024, 512)),
            ..n(12, 30)
        }
        .note()],
        30,
    )?;
    let server = Server::new();
    server.on(
        "/families/mine/board/notes/12/backdrop",
        r#"
            {"note": {"id": 12, "author_id": 7, "kind": "event", "text": "Gran's birthday",
                      "board_seq": 12, "content_seq": 11,
                      "attachment": {"id": 77, "kind": "photo", "width": 1024, "height": 512}}}
            "#,
    );
    let model = rig.model(&server);

    let drawn = drive(&rig.clock, model.draw_backdrop(12))?;

    assert_eq!(77, drawn.expect("drawn").expect("an attachment").id);
    // The wall kept the newer note, guard and all.
    assert_eq!(
        34,
        rig.board
            .note(12)?
            .expect("held")
            .attachment
            .as_ref()
            .unwrap()
            .id
    );
    Ok(())
}

/// An answer and the frame the same write raised can arrive in either order, so the answer is
/// applied under the `board_seq` guard like everything else.
#[test]
fn an_answer_that_is_older_than_what_is_held_does_not_undo_it() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.board.replace(
        &[N {
            text: Some("Milk and eggs"),
            content_seq: Some(30),
            ..n(12, 30)
        }
        .note()],
        30,
    )?;
    let server = Server::new();
    server.on("/families/mine/board/notes/12", &note_json(12, 12, "Milk"));
    let model = rig.model(&server);

    let patch = NotePatch {
        color: Some("pink".to_string()),
        ..NotePatch::default()
    };
    assert_eq!(None, drive(&rig.clock, model.patch(12, &patch))?);

    assert_eq!(
        Some("Milk and eggs"),
        rig.board.note(12)?.expect("held").text.as_deref()
    );
    Ok(())
}
