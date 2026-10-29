//! The family's wall: the stickers on it, the badge over it, and the writes that change it
//! (`Board.cs`).
//!
//! THE ARITHMETIC IS NOT THIS LAYER'S. Card sizes, the wall's own size, the clamp, the fraction and
//! the tilt all come from `fc_text::board` (through `fc_core::board`) — the module the differential
//! oracle pins to the web client's Rust — so a Linux wall lays out exactly where the other clients
//! lay out. What lives here is when to ask and what to do with the answer.
//!
//! A POSITION IS A FRACTION OF THE WALL, read back from where the card actually is. A fraction
//! derived from a raw drag delta stores a position the note was never at, and a note stored at
//! 0.98 must hug the edge rather than hang off it — which is why the clamp is applied before the
//! fraction is taken, and again when it is drawn.
//!
//! THE BADGE COUNTS WHAT IS NEW TO READ. A note somebody dragged, recoloured, resized or answered
//! is not news; `content_seq` is, and the marks only move when the wall has actually been on
//! screen.
//!
//! A REDRAWN BACKDROP IS A NEW ATTACHMENT. The answer carries the whole note with a new attachment
//! id, so a client that caches a picture by NOTE shows the old one for ever — which is exactly how
//! this was found on three platforms (issue #70). Callers redraw by attachment id.
//!
//! Every write answers `StoreResult<Option<ApiError>>`: `Err` is the cache refusing to record the
//! answer (the C#'s exception), `Ok(Some(error))` the server's refusal or a request that never
//! arrived, `Ok(None)` done.

use std::rc::Rc;

use fc_core::board::{board_picture, board_wall};
use fc_core::protocol::{
    ApiClient, ApiError, ApiResult, AttachmentDto, NoteDto, NotePatch, NoteRequest, NoteResponse,
};
use fc_core::store::{BoardStore, ChatStore, SeqRoute, StoreResult};
use fc_text::board::{
    Answer, Font, Kind, Size, clamp_corner, fraction_of, is_compact, is_unread, origin,
    tilt_degrees,
};

/// One note as the wall draws it: the note, its look resolved, where it goes and how big it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Sticker {
    pub note: NoteDto,
    pub kind: Kind,
    pub size: Size,
    pub font: Font,
    pub color: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub tilt_degrees: i64,
    /// Something NEW TO READ — not merely something that changed.
    pub unread: bool,
    /// Written by this reader, which is who may edit and delete it.
    pub mine: bool,
}

impl Sticker {
    pub fn is_event(&self) -> bool {
        self.kind == Kind::Event
    }

    pub fn is_photo(&self) -> bool {
        self.kind == Kind::Photo
    }

    pub fn is_tasks(&self) -> bool {
        self.kind == Kind::Tasks
    }

    /// The backdrop or the photograph — one attachment either way.
    pub fn picture(&self) -> Option<&AttachmentDto> {
        self.note.attachment.as_ref()
    }
}

/// The family's wall.
pub struct BoardModel {
    board: Rc<BoardStore>,
    chats: Rc<ChatStore>,
    api: Rc<ApiClient>,
}

impl std::fmt::Debug for BoardModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoardModel").finish_non_exhaustive()
    }
}

impl BoardModel {
    pub fn new(board: Rc<BoardStore>, chats: Rc<ChatStore>, api: Rc<ApiClient>) -> Self {
        BoardModel { board, chats, api }
    }

    /// The wall's own size for a viewport: taller than the screen, so it can be pinned.
    pub fn wall_size(visible_width: f64, visible_height: f64) -> (f64, f64) {
        board_wall::size(visible_width, visible_height)
    }

    /// How many notes have something new to read.
    pub fn badge(&self) -> StoreResult<usize> {
        self.board.unread()
    }

    /// The wall has been on screen: everything on it has been shown.
    pub fn shown(&self) -> StoreResult<()> {
        self.board.mark_shown()
    }

    /// The stickers, newest first as the store answers them, each placed and sized for a wall of
    /// this size.
    pub fn stickers(&self, visible_width: f64, visible_height: f64) -> StoreResult<Vec<Sticker>> {
        let wall = BoardModel::wall_size(visible_width, visible_height);
        let compact = is_compact(wall.0);
        let marks = self.board.marks()?;
        let me = self.chats.reader();
        let mut stickers = Vec::new();
        for note in self.board.notes()? {
            let kind = Kind::from_name(note.kind.as_deref());
            let size = Size::from_name(note.size.as_deref());
            let mut card = size.frame(compact);
            // A BARE PHOTO'S CARD IS THE PICTURE, fitted into the card it would otherwise have
            // had — the rule issue #71 settled: a photo is drawn WHOLE (both dimensions), and a
            // card with nothing else on it hugs it (docs/protocol.md, "A photo is drawn whole").
            if kind == Kind::Photo
                && note.text.as_deref().is_none_or(str::is_empty)
                && let Some(picture) = &note.attachment
            {
                card = board_picture::fitted(card.0, card.1, picture.width, picture.height);
            }
            let (x, y) = origin((note.x, note.y), card, wall);
            stickers.push(Sticker {
                kind,
                size,
                font: Font::from_name(note.font.as_deref()),
                color: note.color.clone().unwrap_or_else(|| "yellow".to_string()),
                x,
                y,
                width: card.0,
                height: card.1,
                tilt_degrees: tilt_degrees(note.id),
                unread: is_unread(note.id, note.content_seq, marks),
                mine: note.author_id == me,
                note,
            });
        }
        Ok(stickers)
    }

    /// A new note. The answer is the note the server made of it.
    pub async fn create(&self, note: &NoteRequest) -> StoreResult<Option<ApiError>> {
        let answer = self.api.create_note(note).await;
        self.applied(answer)
    }

    /// A change. ONLY WHAT CHANGED is sent — which fields are present decides what changes, and a
    /// patch that sent everything would overwrite what somebody else edited in between.
    pub async fn patch(&self, note_id: i64, patch: &NotePatch) -> StoreResult<Option<ApiError>> {
        let answer = self.api.patch_note(note_id, patch).await;
        self.applied(answer)
    }

    /// Where a note was DROPPED, as a fraction of the wall (`MoveAsync`). The corner is where the
    /// card actually is, clamped inside the wall before the fraction is taken.
    pub async fn move_note(
        &self,
        note_id: i64,
        corner: (f64, f64),
        card: (f64, f64),
        wall: (f64, f64),
    ) -> StoreResult<Option<ApiError>> {
        let held = clamp_corner(corner.0, corner.1, card, wall);
        let (x, y) = fraction_of(held, wall);
        let patch = NotePatch {
            x: Some(x),
            y: Some(y),
            ..NotePatch::default()
        };
        self.patch(note_id, &patch).await
    }

    /// Take it down. A delete is a tombstone in the feed, not a silence.
    pub async fn delete(&self, note_id: i64) -> StoreResult<Option<ApiError>> {
        if let Err(error) = self.api.delete_note(note_id).await.into_result() {
            return Ok(Some(error));
        }
        // The frame and the catch-up both carry the tombstone; applying it here as well is what
        // makes the note leave the wall as the tap lands rather than a round trip later. As
        // EVIDENCE, so it moves no cursor: the seq this delete was given is the server's to
        // announce, and a cursor moved to a number nobody issued steps the feed past whatever
        // really holds it.
        let board_seq = self.board.note(note_id)?.map_or(0, |held| held.board_seq);
        let tombstone = NoteDto {
            deleted: true,
            board_seq,
            ..NoteDto::new(note_id, 0)
        };
        self.board.apply(&tombstone, SeqRoute::Evidence)?;
        Ok(None)
    }

    /// Going, or not. An idempotent STATE-SET rather than a toggle, and any member may send it —
    /// the note's author has no special standing over who is coming.
    pub async fn answer(&self, note_id: i64, answer: Answer) -> StoreResult<Option<ApiError>> {
        let reply = self.api.answer(note_id, answer.name()).await;
        self.applied(reply)
    }

    /// Unsay it: back to no answer at all, which is not the same as "not going".
    pub async fn retract(&self, note_id: i64) -> StoreResult<Option<ApiError>> {
        let answer = self.api.retract_answer(note_id).await;
        self.applied(answer)
    }

    /// One line of a list, ticked or unticked. The server records WHO ticked it.
    pub async fn tick(
        &self,
        note_id: i64,
        item_id: i64,
        done: bool,
    ) -> StoreResult<Option<ApiError>> {
        let answer = self.api.tick_task(note_id, item_id, done).await;
        self.applied(answer)
    }

    /// Ask the assistant to draw this event's backdrop. Answers the NEW attachment (`None` only
    /// from a server whose answer carried none) — a redraw is a new attachment id, and a caller
    /// that caches by note rather than by attachment shows the old picture for ever.
    pub async fn draw_backdrop(
        &self,
        note_id: i64,
    ) -> StoreResult<Result<Option<AttachmentDto>, ApiError>> {
        match self.api.draw_backdrop(note_id).await.into_result() {
            Err(error) => Ok(Err(error)),
            Ok(answer) => {
                self.board.apply(&answer.note, SeqRoute::Evidence)?;
                Ok(Ok(answer.note.attachment))
            }
        }
    }

    fn applied(&self, answer: ApiResult<NoteResponse>) -> StoreResult<Option<ApiError>> {
        match answer.into_result() {
            Err(error) => Ok(Some(error)),
            Ok(answer) => {
                // Applied under the board_seq guard, exactly as a frame is: this answer and the
                // frame the same write raised can arrive in either order. As EVIDENCE: it is one
                // note's news and says nothing about what else has changed, so it may not move the
                // board's cursor.
                self.board.apply(&answer.note, SeqRoute::Evidence)?;
                Ok(None)
            }
        }
    }
}
