//! `NoteDraftTests`: the note editor's draft — the web client's Draft tests, ported — and the words
//! the board's sheet says about itself and its refusals.

mod common;

use fc_core::protocol::{
    ApiError, MemberDto, MentionDto, NoteDto, NotePatch, TaskItemDto, TaskLineRequest, error_codes,
};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::note_draft::{DraftLine, NoteDraft, next_round_hour, note_sheet_text};
use fc_text::board::{Kind, MAX_TASK_ITEMS, Size};
use time::macros::{datetime, offset};
use time::{Duration, OffsetDateTime, UtcOffset};

const ME: i64 = 7;
const ANNA: i64 = 11;
const NOW: OffsetDateTime = datetime!(2026-09-10 14:37 UTC);
const UTC: UtcOffset = UtcOffset::UTC;

fn note(id: i64, text: &str) -> NoteDto {
    NoteDto {
        text: Some(text.to_string()),
        x: 0.1,
        y: 0.1,
        ..NoteDto::new(id, ME)
    }
}

fn event_note(id: i64, ends_at: Option<&str>) -> NoteDto {
    NoteDto {
        kind: Some("event".to_string()),
        text: Some("Picnic".to_string()),
        color: Some("blue".to_string()),
        starts_at: Some("2026-09-12T11:00:00Z".to_string()),
        ends_at: ends_at.map(str::to_string),
        place: Some("The park".to_string()),
        ..NoteDto::new(id, ME)
    }
}

fn blank(kind: Kind, pick: &dyn Fn(usize) -> usize) -> NoteDraft {
    NoteDraft::blank(kind, NOW, &UTC, pick)
}

#[test]
fn a_draft_patches_only_what_it_changed() {
    let stored = NoteDto {
        size: Some("huge".to_string()),
        font: Some("gothic".to_string()),
        color: Some("pink".to_string()),
        ..note(1, "Milk")
    };
    // A size and a face from a newer server draw as the defaults and are NOT written back as them.
    let mut draft = NoteDraft::of(&stored);
    assert_eq!(Size::Medium, draft.size);
    assert!(
        NoteDraft::is_empty(&draft.patch(&stored, &[])),
        "nothing changed, nothing sent"
    );

    draft.set_text("Milk and eggs");
    assert_eq!(
        NotePatch {
            text: Some("Milk and eggs".to_string()),
            ..NotePatch::default()
        },
        draft.patch(&stored, &[])
    );

    let mut padded = NoteDraft::of(&stored);
    padded.set_text("Milk  ");
    assert!(
        NoteDraft::is_empty(&padded.patch(&stored, &[])),
        "the server trims"
    );

    let mut bigger = NoteDraft::of(&stored);
    bigger.size = Size::Large;
    assert_eq!(Some("large".to_string()), bigger.patch(&stored, &[]).size);

    // An event: its end taken off is a null, its place cleared an empty string, a start that did
    // not move is not sent.
    let ended = event_note(2, Some("2026-09-12T15:00:00Z"));
    let mut event_draft = NoteDraft::of(&ended);
    assert!(event_draft.has_end);
    assert!(NoteDraft::is_empty(&event_draft.patch(&ended, &[])));
    event_draft.has_end = false;
    event_draft.place = "  ".to_string();
    let patch = event_draft.patch(&ended, &[]);
    assert!(patch.clears_end);
    assert_eq!(None, patch.ends_at);
    assert_eq!(Some(String::new()), patch.place);
    assert_eq!(None, patch.starts_at);

    // An event with no end offers an hour after its start to begin from, with the end still off.
    let open = NoteDraft::of(&event_note(3, None));
    assert!(!open.has_end);
    assert_eq!(
        open.starts.map(|starts| starts + Duration::hours(1)),
        open.ends
    );

    let mut moved = NoteDraft::of(&ended);
    moved.starts = moved.starts.map(|starts| starts + Duration::hours(1));
    assert_eq!(
        Some("2026-09-12T12:00:00Z".to_string()),
        moved.patch(&ended, &[]).starts_at
    );

    // A text note never sends an event's fields.
    let mut plain = NoteDraft::of(&stored);
    plain.place = "Somewhere".to_string();
    plain.has_end = true;
    assert!(NoteDraft::is_empty(&plain.patch(&stored, &[])));
}

#[test]
fn what_may_be_saved_depends_on_the_kind() {
    let say = common::english();
    let mut draft = blank(Kind::Text, &|_| 0);
    assert_eq!(Some(String::new()), draft.problem(Kind::Text, &say));
    assert_eq!(None, draft.problem(Kind::Photo, &say));
    draft.set_text("Picnic");
    assert_eq!(None, draft.problem(Kind::Text, &say));
    assert_eq!(None, draft.problem(Kind::Event, &say));
    draft.starts = None;
    assert_eq!(
        Some("Pick when it starts.".to_string()),
        draft.problem(Kind::Event, &say)
    );

    let mut event = blank(Kind::Event, &|_| 3);
    assert_eq!("blue", event.color);
    assert!(!event.has_end);
    assert_eq!(Some(datetime!(2026-09-10 15:00 UTC)), event.starts);
    event.set_text("Picnic");
    event.has_end = true;
    event.ends = event.starts.map(|starts| starts - Duration::minutes(5));
    assert_eq!(
        Some("The end can't be before the start.".to_string()),
        event.problem(Kind::Event, &say)
    );
    event.ends = None;
    assert_eq!(
        Some("Pick when it ends, or turn the end off.".to_string()),
        event.problem(Kind::Event, &say)
    );
    // An end picked and then switched off is not sent: the switch is the answer, not the picker.
    event.ends = event.starts.map(|starts| starts + Duration::hours(2));
    event.has_end = false;
    event.place = " The park ".to_string();
    let created = event.new_note(Kind::Event, (0.3, 0.4), &[]);
    assert_eq!(Some("event".to_string()), created.kind);
    assert_eq!(Some("2026-09-10T15:00:00Z".to_string()), created.starts_at);
    assert_eq!(None, created.ends_at);
    assert_eq!(Some("The park".to_string()), created.place);

    assert_eq!("green", blank(Kind::Text, &|count| count - 3).color);
    assert_eq!(
        None,
        blank(Kind::Text, &|_| 0)
            .new_note(Kind::Text, (0.1, 0.1), &[])
            .kind
    );
}

#[test]
fn a_start_moved_past_the_end_takes_the_end_with_it() {
    let mut draft = blank(Kind::Event, &|_| 0);
    draft.starts = draft.ends.map(|ends| ends + Duration::hours(2));
    draft.keep_end_after_start();
    assert_eq!(
        draft.starts.map(|starts| starts + Duration::hours(1)),
        draft.ends
    );
}

/// The next round hour is on the READER'S clock: half past two in a zone of +5:30 is three there.
#[test]
fn a_new_event_starts_on_the_next_round_hour() {
    assert_eq!(datetime!(2026-09-10 15:00 UTC), next_round_hour(NOW, &UTC));
    let india = offset!(+5:30);
    let there = next_round_hour(NOW, &india);
    assert_eq!(datetime!(2026-09-10 21:00 +5:30), there);
    assert_eq!(india, there.offset());
    assert_eq!(
        datetime!(2026-09-11 00:00 UTC),
        next_round_hour(datetime!(2026-09-10 23:05 UTC), &UTC)
    );
}

#[test]
fn a_list_is_written_and_its_lines_keep_their_ids() {
    let say = common::english();
    let mut blank = blank(Kind::Tasks, &|_| 0);
    assert_eq!(1, blank.lines.len());
    blank.set_text("Shopping");
    blank.lines[0] = DraftLine::new(None, " milk ");
    blank.lines.push(DraftLine::new(None, "   "));
    let created = blank.new_note(Kind::Tasks, (0.2, 0.2), &[]);
    assert_eq!(
        Some(vec![TaskLineRequest::new("milk", None)]),
        created.items
    );

    let stored = NoteDto {
        kind: Some("tasks".to_string()),
        text: Some("Shopping".to_string()),
        items: Some(vec![
            TaskItemDto {
                id: 1,
                text: "milk".to_string(),
                done: true,
                ..TaskItemDto::default()
            },
            TaskItemDto {
                id: 2,
                text: "eggs".to_string(),
                done: false,
                ..TaskItemDto::default()
            },
        ]),
        ..NoteDto::new(5, ME)
    };
    let mut draft = NoteDraft::of(&stored);
    assert_eq!(None, draft.patch(&stored, &[]).items);
    draft.lines.remove(1);
    draft.lines.push(DraftLine::new(None, "bread"));
    assert_eq!(
        Some(vec![
            TaskLineRequest::new("milk", Some(1)),
            TaskLineRequest::new("bread", None)
        ]),
        draft.patch(&stored, &[]).items
    );

    for _ in 0..MAX_TASK_ITEMS {
        draft.lines.push(DraftLine::new(None, "x"));
    }
    assert_eq!(
        Some("That's more things than one list holds.".to_string()),
        draft.problem(Kind::Tasks, &say)
    );
}

#[test]
fn the_names_are_resolved_from_the_text_at_save() {
    let roster = [
        MemberDto::new(ANNA, "anna", "Anna"),
        MemberDto::new(99, "gran", "Gran"),
        MemberDto {
            has_left: true,
            ..MemberDto::new(50, "ghost", "Ghost")
        },
    ];
    let mut draft = blank(Kind::Text, &|_| 0);
    draft.set_text("@Anna the kit is in the hall");
    let created = draft.new_note(Kind::Text, (0.1, 0.2), &roster);
    assert_eq!(Some(vec![MentionDto::new(ANNA, "Anna")]), created.mentions);

    draft.set_text("@Nobody hello @Ghost");
    assert_eq!(
        None,
        draft.new_note(Kind::Text, (0.1, 0.2), &roster).mentions
    );

    // An EDIT sends the list with the text, re-decided.
    let stored = note(6, "@Anna the kit");
    let mut moved_on = NoteDraft::of(&stored);
    moved_on.set_text("@Gran the kit");
    let patch = moved_on.patch(&stored, &roster);
    assert_eq!(Some("@Gran the kit".to_string()), patch.text);
    assert_eq!(Some(vec![MentionDto::new(99, "Gran")]), patch.mentions);

    let mut nobody = NoteDraft::of(&stored);
    nobody.set_text("the kit is in the hall");
    assert_eq!(None, nobody.patch(&stored, &roster).mentions);
}

#[test]
fn the_sheet_says_what_it_is_and_why_a_change_did_not_go_in() {
    let say = common::english();
    assert_eq!("New Event", note_sheet_text::title(true, Kind::Event, &say));
    assert_eq!("New List", note_sheet_text::title(true, Kind::Tasks, &say));
    assert_eq!("New Note", note_sheet_text::title(true, Kind::Photo, &say));
    assert_eq!("Photo", note_sheet_text::title(false, Kind::Photo, &say));
    assert_eq!("Note", note_sheet_text::title(false, Kind::Text, &say));
    assert_eq!("Title", note_sheet_text::field_label(Kind::Tasks, &say));
    assert_eq!("Caption", note_sheet_text::field_label(Kind::Photo, &say));
    assert_eq!(
        "The board is full. Take a note down to make room for this one.",
        note_sheet_text::failure(&ApiError::new(error_codes::BOARD_FULL, "x", 409), &say)
    );
    assert_eq!(
        "That note has been taken down.",
        note_sheet_text::failure(&ApiError::new(error_codes::NOTE_NOT_FOUND, "x", 404), &say)
    );
    assert_eq!(
        "Can't reach the server. Check your connection.",
        note_sheet_text::failure(&ApiError::transport("x"), &say)
    );
}

/// A backdrop the provider's own filter refused says so, in the sentence a refused answer says;
/// every other failure — including a refusal this client has no word for — is the plain one it
/// always was.
#[test]
fn a_refused_backdrop_says_the_provider_refused_it() {
    let say = common::english();
    let refused = ApiError::new(
        error_codes::PICTURE_REFUSED,
        "the assistant's provider refused to draw this",
        400,
    );
    assert!(!refused.transient());
    assert_eq!(
        "The assistant's provider refused that. Try putting it another way.",
        note_sheet_text::backdrop_failure(&refused, &say)
    );
    assert_eq!(
        "Couldn't draw that.",
        note_sheet_text::backdrop_failure(&ApiError::new(error_codes::INTERNAL, "x", 500), &say)
    );
    assert_eq!(
        "Couldn't draw that.",
        note_sheet_text::backdrop_failure(
            &ApiError::new(error_codes::PICTURES_UNAVAILABLE, "x", 409),
            &say
        )
    );
    assert_eq!(
        "Couldn't draw that.",
        note_sheet_text::backdrop_failure(&ApiError::transport("x"), &say)
    );
    // Translated with everything else, not left in English.
    let german = common::catalogues().for_language("de");
    assert_eq!(
        german.get("The assistant's provider refused that. Try putting it another way."),
        note_sheet_text::backdrop_failure(&refused, &german)
    );
    assert_ne!(
        "The assistant's provider refused that. Try putting it another way.",
        note_sheet_text::backdrop_failure(&refused, &german)
    );
}
