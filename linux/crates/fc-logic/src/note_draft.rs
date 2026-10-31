//! What the author is writing: a note's words and look, an event's when and where, a list's lines —
//! kept apart from the note so a save sends ONLY WHAT CHANGED (the web client's `Draft`) — and the
//! words the board's sheet says about itself and its refusals (`NoteDraft.cs`).
//!
//! A PATCH IS A CHANGE FROM THE NOTE AS IT STOOD WHEN THE SHEET OPENED — the caller keeps that copy.
//! Diffing against the note as it stands now would send the author's stale copy of anything changed
//! meanwhile on another device.
//!
//! THE NAMES ARE RESOLVED FROM THE TEXT AT SAVE, against the live roster, and sent WITH the text: a
//! text patch that carries no names clears the note's old ones, which is right when the words no
//! longer say any and is exactly why an editor that forgot them would erase every mention on a typo
//! fix.

use fc_core::protocol::{MemberDto, MentionDto, NoteDto, NotePatch, NoteRequest, TaskLineRequest};
use fc_core::text::string_catalog::StringCatalog;
use fc_text::board::{COLORS, Font, Kind, MAX_TASK_ITEMS, Size};
use time::{Duration, OffsetDateTime, UtcOffset};

use crate::composer_mentions;
use crate::date_culture::{self, TimeZone};
use crate::event_text;
use crate::new_line_keys;

/// One line of a list as the author is writing it. The id says "the line you already have", which
/// is what carries its TICK through a rewrite.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DraftLine {
    pub id: Option<i64>,
    pub text: String,
}

impl DraftLine {
    pub fn new(id: Option<i64>, text: &str) -> Self {
        DraftLine {
            id,
            text: text.to_string(),
        }
    }
}

/// What the author is writing (`NoteDraft`).
#[derive(Debug, Clone, PartialEq)]
pub struct NoteDraft {
    text: String,
    pub color: String,
    pub size: Size,
    pub font: Font,
    pub starts: Option<OffsetDateTime>,
    pub has_end: bool,
    pub ends: Option<OffsetDateTime>,
    pub place: String,
    pub lines: Vec<DraftLine>,
}

impl Default for NoteDraft {
    /// `new NoteDraft()`: no words, yellow, medium, plain.
    fn default() -> Self {
        NoteDraft {
            text: String::new(),
            color: "yellow".to_string(),
            size: Size::Medium,
            font: Font::Plain,
            starts: None,
            has_end: false,
            ends: None,
            place: String::new(),
            lines: Vec::new(),
        }
    }
}

impl NoteDraft {
    pub fn new() -> Self {
        NoteDraft::default()
    }

    /// The words, with a text field's carriage returns held as line feeds
    /// ([`new_line_keys::for_wire`]).
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, value: &str) {
        self.text = new_line_keys::for_wire(value).into_owned();
    }

    /// A blank of `kind`: an event starts on the next round hour and is blue, as on the phone; any
    /// other note takes a colour at random (`pick(count)` answers an index below `count`), so a run
    /// of them is not one yellow pile; a list starts with one empty line, so the first thing to do
    /// is one tap away.
    ///
    /// An index `pick` answers out of range wraps rather than panicking (the C# would throw).
    pub fn blank(
        kind: Kind,
        now: OffsetDateTime,
        zone: &dyn TimeZone,
        pick: &dyn Fn(usize) -> usize,
    ) -> NoteDraft {
        let starts = next_round_hour(now, zone);
        let mut draft = NoteDraft {
            color: if kind == Kind::Event {
                "blue".to_string()
            } else {
                COLORS[pick(COLORS.len()) % COLORS.len()].to_string()
            },
            starts: Some(starts),
            ends: Some(starts + Duration::hours(1)),
            ..NoteDraft::default()
        };
        if kind == Kind::Tasks {
            draft.lines.push(DraftLine::new(None, ""));
        }
        draft
    }

    /// The note as it stands, to be edited. An event with no end offers an hour after its start to
    /// begin from.
    pub fn of(note: &NoteDto) -> NoteDraft {
        let starts = event_text::instant(note.starts_at.as_deref());
        let mut draft = NoteDraft {
            color: note.color.clone().unwrap_or_else(|| "yellow".to_string()),
            size: Size::from_name(note.size.as_deref()),
            font: Font::from_name(note.font.as_deref()),
            starts,
            has_end: note.ends_at.is_some(),
            ends: event_text::instant(note.ends_at.as_deref())
                .or_else(|| starts.map(|starts| starts + Duration::hours(1))),
            place: note.place.clone().unwrap_or_default(),
            lines: note
                .task_list()
                .iter()
                .map(|item| DraftLine::new(Some(item.id), &item.text))
                .collect(),
            ..NoteDraft::default()
        };
        draft.set_text(note.text.as_deref().unwrap_or_default());
        draft
    }

    /// A start moved past the end takes the end with it, an hour on, rather than refusing the save.
    pub fn keep_end_after_start(&mut self) {
        if let (Some(starts), Some(ends)) = (self.starts, self.ends)
            && ends < starts
        {
            self.ends = Some(starts + Duration::hours(1));
        }
    }

    /// Why this cannot be saved as it stands, or `None`. EMPTY means "not yet, and nothing to say":
    /// the Save button is simply off while a note has no words.
    pub fn problem(&self, kind: Kind, say: &dyn StringCatalog) -> Option<String> {
        if kind != Kind::Photo && self.text.trim().is_empty() {
            return Some(String::new());
        }
        if kind == Kind::Tasks && self.lines.len() > MAX_TASK_ITEMS {
            return Some(
                say.get("That's more things than one list holds.")
                    .to_string(),
            );
        }
        if kind == Kind::Event {
            let Some(starts) = self.starts else {
                return Some(say.get("Pick when it starts.").to_string());
            };
            if self.has_end {
                let Some(ends) = self.ends else {
                    return Some(
                        say.get("Pick when it ends, or turn the end off.")
                            .to_string(),
                    );
                };
                if ends < starts {
                    return Some(say.get("The end can't be before the start.").to_string());
                }
            }
        }
        None
    }

    /// The lines that say something, trimmed — what a save sends. An empty row is somebody who
    /// started typing and stopped.
    pub fn written_lines(&self) -> Vec<DraftLine> {
        self.lines
            .iter()
            .filter(|line| !line.text.trim().is_empty())
            .map(|line| DraftLine::new(line.id, line.text.trim()))
            .collect()
    }

    /// A new note of `kind`, dropped at `at`, a fraction of the wall.
    pub fn new_note(&self, kind: Kind, at: (f64, f64), roster: &[MemberDto]) -> NoteRequest {
        let is_event = kind == Kind::Event;
        let place = self.place.trim();
        let named = named(&self.text, roster);
        NoteRequest {
            size: Some(self.size.name().to_string()),
            font: Some(self.font.name().to_string()),
            // Named unless it is a plain note, which is what an absent kind means.
            kind: (kind != Kind::Text).then(|| kind.name().to_string()),
            starts_at: if is_event {
                wire_time(self.starts)
            } else {
                None
            },
            ends_at: if is_event && self.has_end {
                wire_time(self.ends)
            } else {
                None
            },
            place: (is_event && !place.is_empty()).then(|| place.to_string()),
            mentions: (!named.is_empty()).then_some(named),
            items: (kind == Kind::Tasks).then(|| {
                self.written_lines()
                    .iter()
                    .map(|line| TaskLineRequest::new(&line.text, line.id))
                    .collect()
            }),
            ..NoteRequest::new(&self.text, &self.color, at.0, at.1)
        }
    }

    /// What changed against `note` — and nothing else, so a size or face this client does not know
    /// is not written back as its default, and a save that changed nothing sends nothing.
    pub fn patch(&self, note: &NoteDto, roster: &[MemberDto]) -> NotePatch {
        let mut patch = NotePatch::default();
        // Trailing space is not a change: the server trims.
        if self.text.trim() != note.text.as_deref().unwrap_or_default().trim() {
            patch.text = Some(self.text.clone());
            let named = named(&self.text, roster);
            patch.mentions = (!named.is_empty()).then_some(named);
        }
        let kind = Kind::from_name(note.kind.as_deref());
        if kind == Kind::Event {
            if let Some(starts) = self.starts
                && Some(starts) != event_text::instant(note.starts_at.as_deref())
            {
                patch.starts_at = wire_time(Some(starts));
            }
            let ends = if self.has_end { self.ends } else { None };
            if ends != event_text::instant(note.ends_at.as_deref()) {
                // An end taken off is a NULL on the wire; a field left out leaves it alone.
                match ends {
                    Some(end) => patch.ends_at = wire_time(Some(end)),
                    None => patch.clears_end = true,
                }
            }
            if self.place.trim() != note.place.as_deref().unwrap_or_default().trim() {
                patch.place = Some(self.place.trim().to_string());
            }
        }
        if kind == Kind::Tasks {
            let written = self.written_lines();
            let held: Vec<DraftLine> = note
                .task_list()
                .iter()
                .map(|item| DraftLine::new(Some(item.id), &item.text))
                .collect();
            // The list is the AUTHOR's, and a patch that carried it unchanged would make opening a
            // note to read it an edit.
            if written != held {
                patch.items = Some(
                    written
                        .iter()
                        .map(|line| TaskLineRequest::new(&line.text, line.id))
                        .collect(),
                );
            }
        }
        if Some(self.color.as_str()) != note.color.as_deref() {
            patch.color = Some(self.color.clone());
        }
        patch.size = self
            .size
            .patch_name(note.size.as_deref())
            .map(str::to_string);
        patch.font = self
            .font
            .patch_name(note.font.as_deref())
            .map(str::to_string);
        patch
    }

    /// Whether a patch changes nothing at all.
    pub fn is_empty(patch: &NotePatch) -> bool {
        patch.is_empty()
    }
}

/// The next round hour after `now`, on the reader's clock.
pub fn next_round_hour(now: OffsetDateTime, zone: &dyn TimeZone) -> OffsetDateTime {
    let local = date_culture::local(now + Duration::hours(1), zone);
    local
        .replace_minute(0)
        .and_then(|at| at.replace_second(0))
        .and_then(|at| at.replace_nanosecond(0))
        .unwrap_or(local)
}

/// An instant as the wire writes one: RFC 3339, UTC, whole seconds.
pub fn wire_time(at: Option<OffsetDateTime>) -> Option<String> {
    let at = at?.to_offset(UtcOffset::UTC);
    Some(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second()
    ))
}

fn named(text: &str, roster: &[MemberDto]) -> Vec<MentionDto> {
    composer_mentions::resolve(text, roster)
}

/// The words the board's sheet says about itself and its refusals (the web client's board pane and
/// `board_failure`; `NoteSheetText`).
pub mod note_sheet_text {
    use fc_core::protocol::{AiErrorReason, ApiError, error_codes};
    use fc_core::text::string_catalog::StringCatalog;
    use fc_text::board::Kind;

    use crate::assistant_answers::AssistantAnswers;
    use crate::family_console::family_text;
    use crate::paid_families::{self, Days};

    pub fn title(is_new: bool, kind: Kind, say: &dyn StringCatalog) -> String {
        match (is_new, kind) {
            (true, Kind::Event) => say.get("New Event"),
            (true, Kind::Tasks) => say.get("New List"),
            (true, _) => say.get("New Note"),
            (false, Kind::Event) => say.get("Event"),
            (false, Kind::Tasks) => say.get("List"),
            (false, Kind::Photo) => say.get("Photo"),
            (false, Kind::Text) => say.get("Note"),
        }
        .to_string()
    }

    pub fn field_label(kind: Kind, say: &dyn StringCatalog) -> String {
        match kind {
            Kind::Event | Kind::Tasks => say.get("Title"),
            Kind::Photo => say.get("Caption"),
            Kind::Text => say.get("Note"),
        }
        .to_string()
    }

    /// What to tell somebody whose change to the board did not go in — said, never swallowed.
    pub fn failure(error: &ApiError, say: &dyn StringCatalog) -> String {
        let key = match error.code.as_str() {
            error_codes::BOARD_FULL => {
                "The board is full. Take a note down to make room for this one."
            }
            error_codes::NOT_NOTE_AUTHOR => "Only the person who wrote a note can change it.",
            error_codes::NOTE_NOT_FOUND => "That note has been taken down.",
            error_codes::ATTACHMENT_EXPIRED => "The photo took too long to pin. Try again.",
            error_codes::ATTACHMENT_TOO_LARGE => "That photo is too large to pin.",
            error_codes::INVALID_ATTACHMENT => "The board pins photos only.",
            error_codes::NOT_IN_FAMILY => "You're not in a family, so there is no board.",
            // A pinned photo is an upload like any other (docs/protocol.md, "Paid families").
            error_codes::STORAGE_ALLOWANCE_USED => {
                return paid_families::upload_refusal(&error.code, say).unwrap_or_default();
            }
            _ => return family_text::generic_failure(error, say),
        };
        say.get(key).to_string()
    }

    /// What to tell the author whose backdrop was not drawn. A title the provider's own filter
    /// refused (`picture_refused`, docs/protocol.md "Board") says so, in the sentence a refused
    /// answer says — drawing the same title again gets the same refusal. A paid family past this
    /// month's pictures (`ai_allowance_used`, "Paid families") says on which day they renew.
    /// Anything else is the plain failure it always was.
    pub fn backdrop_failure(error: &ApiError, say: &dyn StringCatalog, days: &Days<'_>) -> String {
        match error.code.as_str() {
            error_codes::PICTURE_REFUSED => {
                AssistantAnswers::sentence(Some(AiErrorReason::Refused), say, days)
            }
            error_codes::AI_ALLOWANCE_USED => {
                paid_families::assistant_allowance_used(Some(days.next_month()), say, days)
            }
            _ => say.get("Couldn't draw that.").to_string(),
        }
    }
}
