//! `PictureHintTests`: the line said while a picture is being asked for — real names and brands are
//! often refused (docs/protocol.md, "A refused description is reworded once") — and only there.

mod common;

use fc_core::text::string_catalog::EnglishCatalog;
use fc_logic::picture_hint::{for_composer, offers_backdrop, sentence};

use common::consent::{draws, talks, unnamed};

/// From the moment the token is typed — "Ask for a picture" leaves `/draw ` with nothing after it,
/// and that is exactly when the description is about to be written — and through the description.
#[test]
fn the_assistants_chat_says_it_once_a_draw_is_begun() {
    for draft in [
        "/draw ",
        "/draw a cat in a hat",
        "  /draw a cat",
        "\n/Draw a cat",
        "/DRAW\ta cat",
        "@ai /draw a cat",
    ] {
        assert!(
            for_composer(Some("ai"), Some(draft), Some(&draws()), false),
            "{draft:?}"
        );
    }
}

#[test]
fn not_for_a_draft_that_is_no_picture_request() {
    for draft in [
        "",
        "/draw",
        "/drawer a cat",
        "draw a cat",
        "what does /draw do?",
        "a cat /draw ",
        "@ai @ai /draw a cat",
    ] {
        assert!(
            !for_composer(Some("ai"), Some(draft), Some(&draws()), false),
            "{draft:?}"
        );
    }
    assert!(!for_composer(Some("ai"), None, Some(&draws()), false));
}

/// In the family chat a picture is `@ai /draw`: a bare `/draw` reaches nobody and nothing refuses
/// it.
#[test]
fn the_family_chat_says_it_only_for_a_mention_that_draws() {
    let draws = draws();
    assert!(for_composer(
        Some("family"),
        Some("@ai /draw "),
        Some(&draws),
        false
    ));
    assert!(for_composer(
        Some("family"),
        Some(" @AI /draw a cat"),
        Some(&draws),
        false
    ));
    assert!(!for_composer(
        Some("family"),
        Some("/draw a cat"),
        Some(&draws),
        false
    ));
    assert!(!for_composer(
        Some("family"),
        Some("hey @ai /draw a cat"),
        Some(&draws),
        false
    ));
    assert!(!for_composer(
        Some("family"),
        Some("@ai what is a cat?"),
        Some(&draws),
        false
    ));
}

#[test]
fn not_where_nothing_draws() {
    assert!(!for_composer(
        Some("direct"),
        Some("/draw a cat"),
        Some(&draws()),
        false
    ));
    assert!(!for_composer(
        None,
        Some("/draw a cat"),
        Some(&draws()),
        false
    ));
    assert!(!for_composer(
        Some("ai"),
        Some("/draw a cat"),
        Some(&talks()),
        false
    ));
    assert!(!for_composer(Some("ai"), Some("/draw a cat"), None, false));
    assert!(!for_composer(
        Some("ai"),
        Some("/draw a cat"),
        Some(&unnamed()),
        false
    ));
    assert!(!for_composer(
        Some("family"),
        Some("@ai /draw a cat"),
        Some(&talks()),
        false
    ));
}

/// An edit rewrites what was said; it cannot become a request, so nothing is about to be refused.
#[test]
fn not_while_a_message_is_edited() {
    assert!(!for_composer(
        Some("ai"),
        Some("/draw a cat"),
        Some(&draws()),
        true
    ));
    assert!(!for_composer(
        Some("family"),
        Some("@ai /draw a cat"),
        Some(&draws()),
        true
    ));
}

/// The backdrop's line goes with its button: the author's event, on a server that draws — and names
/// who does, since the title needs the author's consent and nobody can agree to an unnamed
/// recipient.
#[test]
fn the_backdrop_says_it_wherever_its_button_is() {
    assert!(offers_backdrop(true, Some(&draws())));
    assert!(!offers_backdrop(false, Some(&draws())));
    assert!(!offers_backdrop(true, Some(&talks())));
    assert!(!offers_backdrop(true, None));
    assert!(!offers_backdrop(true, Some(&unnamed())));
}

#[test]
fn the_line_is_the_apps_sentence() {
    assert_eq!(
        "Describe people and things in general words — real names and brands are often refused.",
        sentence(&EnglishCatalog::new())
    );
}
