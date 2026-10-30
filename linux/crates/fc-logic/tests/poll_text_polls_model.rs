//! `PollsModelTests`, the words a poll is drawn with.
//!
//! The C# reads the roster out of a `ChatStore` seeded with five members and two who left; the
//! store is another port, so the same seven rows are a list here, and `chats.Member` /
//! `chats.Members()` are a lookup over it — the subject is `PollText`, not the store.

mod common;

use fc_core::protocol::{MemberDto, PollDto, PollOptionDto};
use fc_logic::poll_text;

const ME: i64 = 7;

fn poll(seq: i64, closed: bool, pizza: &[i64], pasta: &[i64]) -> PollDto {
    PollDto {
        poll_seq: seq,
        closed,
        options: vec![
            PollOptionDto {
                id: 1,
                text: "Pizza".to_string(),
                votes: pizza.to_vec(),
            },
            PollOptionDto {
                id: 2,
                text: "Pasta".to_string(),
                votes: pasta.to_vec(),
            },
        ],
    }
}

/// The C#'s roster: Anna (the reader), Bob, Dora, a member with no name, Gus whose account was
/// deleted — and Eve (left, deleted) and Fay (left).
fn roster() -> Vec<MemberDto> {
    let deleted = |member: MemberDto| MemberDto {
        deleted: true,
        ..member
    };
    let left = |member: MemberDto| MemberDto {
        has_left: true,
        ..member
    };
    vec![
        MemberDto::new(7, "anna", "Anna"),
        MemberDto::new(11, "bob", "Bob"),
        MemberDto::new(13, "dora", "Dora"),
        MemberDto::new(16, "nameless", ""),
        deleted(MemberDto::new(17, "ghost", "Gus")),
        left(deleted(MemberDto::new(14, "gone", "Eve"))),
        left(MemberDto::new(15, "left", "Fay")),
    ]
}

#[test]
fn a_poll_says_how_many_voted_and_who_chose_what() {
    let say = common::english();
    let members = roster();
    let member = |id: i64| members.iter().find(|member| member.id == id).cloned();

    let held = poll(1, false, &[ME, 11], &[13]);
    assert_eq!(poll_text::footer(&held, 4, &say), "3 of 4 voted");
    assert_eq!(poll_text::footer(&held, 0, &say), "3 voted");
    assert_eq!(
        poll_text::footer(&poll(1, false, &[ME], &[]), 4, &say),
        "1 of 4 voted"
    );

    assert_eq!(
        poll_text::option_label(&held.options[1], false, &say),
        "Pasta. 1 vote"
    );
    assert_eq!(
        poll_text::option_label(&held.options[0], true, &say),
        "Pizza. 2 votes. Your choice"
    );

    assert_eq!(poll_text::name(ME, ME, member, &say), "You");
    assert_eq!(poll_text::name(11, ME, member, &say), "Bob");
    assert_eq!(poll_text::name(14, ME, member, &say), "Deleted account");
    assert_eq!(poll_text::name(17, ME, member, &say), "Deleted account");
    assert_eq!(poll_text::name(16, ME, member, &say), "Someone");
    assert_eq!(poll_text::name(99, ME, member, &say), "Someone");

    let letter = |id: i64| char::from(b'a' + u8::try_from(id).unwrap()).to_string();
    assert_eq!(
        poll_text::voters(&[0, 1, 2, 3, 4, 5, 6], letter, &say),
        "a, b, c, d, e +2"
    );
    assert_eq!(
        poll_text::voters(&[0, 1, 2, 3, 4], letter, &say),
        "a, b, c, d, e"
    );
    assert_eq!(
        poll_text::voters(&[0, 1, 2, 3, 4, 5], letter, &say),
        "a, b, c, d, e +1"
    );
    assert_eq!(poll_text::voters(&[], letter, &say), "");

    // Neither the people who left nor the deleted account: Anna, Bob, Dora and the nameless member.
    assert_eq!(poll_text::member_count(&members), 4);
}
