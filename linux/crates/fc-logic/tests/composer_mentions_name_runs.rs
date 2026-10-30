//! `NameRunsTests`: the names a text says, cut out of it, and which of them is a door
//! (docs/protocol.md, "Board" and "Mentioning a member") — the same answer for a bubble and for an
//! opened note.

use fc_core::protocol::{MemberDto, MentionDto};
use fc_logic::composer_mentions::{NameRun, runs};

const ME: i64 = 7;

fn roster() -> Vec<MemberDto> {
    vec![
        MemberDto::new(7, "me", "Anja"),
        MemberDto::new(11, "anna", "Anna"),
        MemberDto::new(13, "bob", "Bob"),
        MemberDto {
            has_left: true,
            ..MemberDto::new(14, "gran", "Gran")
        },
        MemberDto {
            deleted: true,
            ..MemberDto::new(15, "gone", "Gus")
        },
    ]
}

fn seen(runs: Vec<NameRun>) -> Vec<(String, Option<i64>, bool)> {
    runs.into_iter()
        .map(|run| (run.text, run.user_id, run.opens))
        .collect()
}

fn run(text: &str, user_id: Option<i64>, opens: bool) -> (String, Option<i64>, bool) {
    (text.to_string(), user_id, opens)
}

/// The reader's own name, a blocked member's, and a name whose member left or was deleted are names
/// and not doors.
#[test]
fn a_name_is_a_door_only_where_there_is_somebody_to_open_it_with() {
    let named = [
        MentionDto::new(11, "Anna"),
        MentionDto::new(13, "Bob"),
        MentionDto::new(14, "Gran"),
        MentionDto::new(15, "Gus"),
        MentionDto::new(7, "Anja"),
    ];

    let runs = runs(
        "@Anna and @Bob asked @Gran and @Gus, not @Anja",
        Some(&named),
        &roster(),
        ME,
        &|id| id == 13,
    );

    assert_eq!(
        vec![
            run("@Anna", Some(11), true),
            run(" and ", None, false),
            run("@Bob", Some(13), false),
            run(" asked ", None, false),
            run("@Gran", Some(14), false),
            run(" and ", None, false),
            run("@Gus", Some(15), false),
            run(", not ", None, false),
            run("@Anja", Some(7), false),
        ],
        seen(runs)
    );
}

#[test]
fn a_text_that_names_nobody_is_one_plain_stretch() {
    let roster = roster();
    let plain = vec![run("@Anna the kit", None, false)];
    assert_eq!(
        plain,
        seen(runs("@Anna the kit", None, &roster, ME, &|_| false))
    );
    assert_eq!(
        plain,
        seen(runs("@Anna the kit", Some(&[]), &roster, ME, &|_| false))
    );

    // Named, but not said: the words stay words.
    assert_eq!(
        vec![run("the kit", None, false)],
        seen(runs(
            "the kit",
            Some(&[MentionDto::new(11, "Anna")]),
            &roster,
            ME,
            &|_| false
        ))
    );
}
