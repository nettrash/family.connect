//! `WallTextTests`: what a sticker says and hides.

mod common;

use std::collections::HashSet;

use fc_core::protocol::NoteDto;
use fc_logic::board::Sticker;
use fc_logic::wall_text;
use fc_text::board::{Font, Kind, Size};

fn sticker(id: i64) -> Sticker {
    Sticker {
        note: NoteDto::new(id, 7),
        kind: Kind::Text,
        size: Size::Medium,
        font: Font::Plain,
        color: "yellow".to_string(),
        x: 0.0,
        y: 0.0,
        width: 150.0,
        height: 110.0,
        tilt_degrees: 0,
        unread: false,
        mine: false,
    }
}

#[test]
fn a_note_is_hidden_while_its_author_is_blocked_until_the_peek() {
    let bobs = NoteDto::new(3, 11);
    let mine = NoteDto::new(4, 7);
    let blocked = |id: i64| id == 11 || id == 7;

    assert!(wall_text::is_hidden(&bobs, 7, blocked, &HashSet::new()));
    assert!(!wall_text::is_hidden(
        &bobs,
        7,
        blocked,
        &HashSet::from([3])
    ));
    assert!(!wall_text::is_hidden(&bobs, 7, |_| false, &HashSet::new()));
    // Never the reader's own, whatever the list says.
    assert!(!wall_text::is_hidden(&mine, 7, blocked, &HashSet::new()));
}

#[test]
fn a_sticker_is_signed_by_who_wrote_it() {
    let say = common::english();
    let names = |id: i64| match id {
        11 => Some("Bob".to_string()),
        12 => Some(String::new()),
        _ => None,
    };
    assert_eq!(
        "You",
        wall_text::author_name(&NoteDto::new(1, 7), 7, names, &say)
    );
    assert_eq!(
        "Bob",
        wall_text::author_name(&NoteDto::new(1, 11), 7, names, &say)
    );
    assert_eq!(
        "Someone",
        wall_text::author_name(&NoteDto::new(1, 12), 7, names, &say)
    );
    assert_eq!(
        "Someone",
        wall_text::author_name(&NoteDto::new(1, 99), 7, names, &say)
    );
}

/// A bare photo is the picture: no words and no author. A hidden note: words saying so, and no
/// author.
#[test]
fn what_a_card_carries_follows_what_it_is() {
    assert!(!wall_text::shows_text(Kind::Photo, "", false));
    assert!(!wall_text::shows_author(Kind::Photo, "", false));
    assert!(wall_text::shows_text(Kind::Photo, "Beach", false));
    assert!(wall_text::shows_author(Kind::Photo, "Beach", false));
    assert!(wall_text::shows_text(Kind::Photo, "", true));
    assert!(!wall_text::shows_author(Kind::Text, "Milk", true));
    assert!(wall_text::shows_text(Kind::Text, "", false));
    assert!(wall_text::shows_author(Kind::Event, "Dinner", false));
}

#[test]
fn a_label_stands_in_for_the_cards_content() {
    let say = common::english();
    assert_eq!(
        "a photo",
        wall_text::what(Kind::Photo, "", None, None, None, &say)
    );
    assert_eq!(
        "Beach",
        wall_text::what(
            Kind::Photo,
            "Beach",
            Some("ignored"),
            Some("x"),
            Some("y"),
            &say
        )
    );
    // Only a PHOTO with no words is "a photo": a note with none is a note with none.
    assert_eq!("", wall_text::what(Kind::Text, "", None, None, None, &say));
    assert_eq!(
        "Dinner, Thu, 24 Dec, 16:00, Kitchen, 2 going",
        wall_text::what(
            Kind::Event,
            "Dinner",
            Some("Thu, 24 Dec, 16:00"),
            Some("Kitchen"),
            Some("2 going"),
            &say
        )
    );
    assert_eq!(
        "Dinner, Thu, 24 Dec, 16:00",
        wall_text::what(
            Kind::Event,
            "Dinner",
            Some("Thu, 24 Dec, 16:00"),
            Some(""),
            None,
            &say
        )
    );
    assert_eq!(
        "Dinner",
        wall_text::what(
            Kind::Event,
            "Dinner",
            Some(""),
            Some("Kitchen"),
            Some("2 going"),
            &say
        )
    );

    assert_eq!(
        "Hidden note from a blocked member",
        wall_text::label(true, false, "Bob", "Milk", &say)
    );
    assert_eq!(
        "Your note: Milk",
        wall_text::label(false, true, "You", "Milk", &say)
    );
    assert_eq!(
        "Note from Bob: Milk",
        wall_text::label(false, false, "Bob", "Milk", &say)
    );
}

#[test]
fn the_most_recently_changed_note_stacks_highest() {
    let layers = wall_text::layers(&[sticker(9), sticker(2), sticker(5)]);

    assert_eq!(3, layers[&9]);
    assert_eq!(2, layers[&2]);
    assert_eq!(1, layers[&5]);
}

#[test]
fn a_long_list_says_how_many_it_left_off() {
    let say = common::english();
    assert_eq!("+1 more", wall_text::more_line(1, &say));
    assert_eq!("+3 more", wall_text::more_line(3, &say));
}
