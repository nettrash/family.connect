//! `NotePreviewTests`: the note sheet's preview — the draft as the wall will draw it (the web
//! client's sheet preview).

mod common;

use fc_core::board::board_picture;
use fc_core::protocol::{AttachmentDto, NoteDto, RsvpDto};
use fc_logic::board::Sticker;
use fc_logic::note_draft::NoteDraft;
use fc_logic::note_preview;
use fc_text::board::{Font, Kind, Size};
use time::macros::datetime;

const ME: i64 = 7;

fn size(sticker: &Sticker) -> (f64, f64) {
    (sticker.width, sticker.height)
}

fn written(text: &str) -> NoteDraft {
    let mut draft = NoteDraft::new();
    draft.set_text(text);
    draft
}

#[test]
fn an_unwritten_note_reads_as_its_kinds_placeholder_and_written_words_as_written() {
    let say = common::english();
    let mut draft = written("  \n");
    let text = |draft: &NoteDraft, kind: Kind| {
        note_preview::of(draft, kind, None, ME, false, &say)
            .note
            .text
            .expect("a text")
    };
    assert_eq!("Your note", text(&draft, Kind::Text));
    assert_eq!("Your list", text(&draft, Kind::Tasks));
    assert_eq!("Your event", text(&draft, Kind::Event));
    assert_eq!("", text(&draft, Kind::Photo));

    draft.set_text(" Milk ");
    assert_eq!(" Milk ", text(&draft, Kind::Text));
}

#[test]
fn the_preview_wears_the_drafts_look_upright_and_as_the_readers() {
    let say = common::english();
    let mut draft = written("Milk");
    draft.color = "pink".to_string();
    draft.size = Size::Large;
    draft.font = Font::Serif;
    let sticker = note_preview::of(&draft, Kind::Tasks, None, ME, false, &say);

    assert_eq!(
        ("pink", Size::Large, Font::Serif, Kind::Tasks),
        (
            sticker.color.as_str(),
            sticker.size,
            sticker.font,
            sticker.kind
        )
    );
    assert_eq!(
        (Some("pink"), Some("large"), Some("serif"), Some("tasks")),
        (
            sticker.note.color.as_deref(),
            sticker.note.size.as_deref(),
            sticker.note.font.as_deref(),
            sticker.note.kind.as_deref()
        )
    );
    assert_eq!(Size::Large.frame(false), size(&sticker));
    let mut small = NoteDraft::new();
    small.size = Size::Small;
    assert_eq!(
        Size::Small.frame(true),
        size(&note_preview::of(&small, Kind::Text, None, ME, true, &say))
    );
    assert_eq!(0, sticker.tilt_degrees);
    assert!(sticker.mine);
    assert!(!sticker.unread);
    assert_eq!((0, ME), (sticker.note.id, sticker.note.author_id));

    // A note being edited keeps who it is.
    let old = NoteDto {
        text: Some("old".to_string()),
        ..NoteDto::new(12, 11)
    };
    let held = note_preview::of(&draft, Kind::Text, Some(&old), ME, false, &say);
    assert_eq!((12, 11), (held.note.id, held.note.author_id));
}

/// A bare photo's card IS its picture, fitted, as on the wall; with a caption the card is the
/// size's own.
#[test]
fn an_uncaptioned_photo_is_its_picture_and_a_captioned_one_keeps_the_card() {
    let say = common::english();
    let photo = AttachmentDto {
        width: Some(400),
        height: Some(1200),
        has_preview: true,
        ..AttachmentDto::new(34, "photo")
    };
    let held = NoteDto {
        kind: Some("photo".to_string()),
        attachment: Some(photo.clone()),
        ..NoteDto::new(12, ME)
    };
    let card = Size::Medium.frame(false);

    let bare = note_preview::of(&NoteDraft::new(), Kind::Photo, Some(&held), ME, false, &say);
    assert_eq!(
        board_picture::fitted(card.0, card.1, Some(400), Some(1200)),
        size(&bare)
    );
    assert_ne!(card, size(&bare));
    assert_eq!(Some(&photo), bare.picture());

    let captioned = note_preview::of(&written("Beach"), Kind::Photo, Some(&held), ME, false, &say);
    assert_eq!(card, size(&captioned));
    assert_eq!(Some(&photo), captioned.picture());

    // An event's picture is its backdrop; a note of any other kind carries none.
    assert_eq!(
        Some(&photo),
        note_preview::of(&NoteDraft::new(), Kind::Event, Some(&held), ME, false, &say).picture()
    );
    assert_eq!(
        None,
        note_preview::of(&written("x"), Kind::Text, Some(&held), ME, false, &say).picture()
    );
    assert_eq!(
        None,
        note_preview::of(&written("x"), Kind::Tasks, Some(&held), ME, false, &say).picture()
    );
    // A photo not uploaded yet has nothing to fit.
    assert_eq!(
        card,
        size(&note_preview::of(
            &NoteDraft::new(),
            Kind::Photo,
            None,
            ME,
            false,
            &say
        ))
    );
}

#[test]
fn an_event_carries_its_when_where_and_answers_and_nothing_else_does() {
    let say = common::english();
    let mut draft = NoteDraft::new();
    draft.starts = Some(datetime!(2026-09-12 13:00 +2));
    draft.ends = Some(datetime!(2026-09-12 15:00 +2));
    draft.place = "  The park ".to_string();
    let held = NoteDto {
        kind: Some("event".to_string()),
        rsvps: Some(vec![RsvpDto {
            user_id: 11,
            answer: "going".to_string(),
        }]),
        ..NoteDto::new(12, ME)
    };

    let open = note_preview::of(&draft, Kind::Event, Some(&held), ME, false, &say).note;
    assert_eq!(Some("2026-09-12T11:00:00Z"), open.starts_at.as_deref());
    assert_eq!(None, open.ends_at);
    assert_eq!(Some("The park"), open.place.as_deref());
    assert_eq!(1, open.count("going"));

    draft.has_end = true;
    assert_eq!(
        Some("2026-09-12T13:00:00Z".to_string()),
        note_preview::of(&draft, Kind::Event, Some(&held), ME, false, &say)
            .note
            .ends_at
    );
    draft.place = "   ".to_string();
    assert_eq!(
        None,
        note_preview::of(&draft, Kind::Event, Some(&held), ME, false, &say)
            .note
            .place
    );

    draft.place = "The park".to_string();
    let text = note_preview::of(&draft, Kind::Text, Some(&held), ME, false, &say).note;
    assert_eq!(
        (None, None, None, None),
        (text.starts_at, text.ends_at, text.place, text.rsvps)
    );
}
