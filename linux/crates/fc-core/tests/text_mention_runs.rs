//! `MentionRunsTests`: a text cut into words and names, on the right CHARACTERS whatever the bytes
//! before them.

use fc_core::text::mentions::runs;
use fc_text::mentions::Member;

fn named(user_id: i64, name: &str) -> Member<'_> {
    Member { user_id, name }
}

fn owned(expected: &[(&str, Option<i64>)]) -> Vec<(String, Option<i64>)> {
    expected
        .iter()
        .map(|(text, user)| (text.to_string(), *user))
        .collect()
}

#[test]
fn a_text_is_cut_into_its_words_and_the_names_it_says() {
    let cut = runs(
        "hi @Anna Lee and @Bob!",
        &[named(1, "Anna"), named(2, "Anna Lee"), named(3, "Bob")],
    );

    assert_eq!(
        cut,
        owned(&[
            ("hi ", None),
            ("@Anna Lee", Some(2)),
            (" and ", None),
            ("@Bob", Some(3)),
            ("!", None),
        ])
    );
}

#[test]
fn names_after_wide_characters_land_on_the_right_characters() {
    let family = "👨‍👩‍👧";
    let mark = "\u{301}";
    let text = format!("{family} @Анна{mark} ok");

    let cut = runs(&text, &[named(5, "Анна")]);

    assert_eq!(
        cut,
        vec![
            (format!("{family} "), None),
            (format!("@Анна{mark}"), Some(5)),
            (" ok".to_string(), None),
        ]
    );
}

#[test]
fn a_text_with_no_names_is_one_run() {
    assert_eq!(runs("plain", &[]), owned(&[("plain", None)]));
    assert_eq!(
        runs("@Anna", &[named(7, "Anna")]),
        owned(&[("@Anna", Some(7))])
    );
}
