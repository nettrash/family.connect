//! `ComposerMentionsTests`: who the strip offers, what a send carries, whom a name opens, the
//! assistant at the top of the strip, and a group's own members (docs/protocol.md, "Mentioning a
//! member").

mod common;

use fc_core::protocol::{AssistantDto, ChatDto, MemberDto, MentionDto};
use fc_logic::composer_mentions::{
    MentionChoice, candidates, choices, enabled, for_send, offered, opens_chat,
};

use common::consent::{PROCESSOR, assistant};

const ME: i64 = 7;

fn roster() -> Vec<MemberDto> {
    vec![
        MemberDto::new(7, "me", "Anja"),
        MemberDto::new(11, "anna", "Anna"),
        MemberDto::new(12, "annalee", "Anna Lee"),
        MemberDto::new(13, "bob", "Bob"),
        MemberDto {
            has_left: true,
            ..MemberDto::new(14, "ann", "Annika")
        },
        MemberDto {
            deleted: true,
            ..MemberDto::new(15, "gone", "Anneliese")
        },
    ]
}

fn bob_blocked(id: i64) -> bool {
    id == 13
}

fn nobody_blocked(_: i64) -> bool {
    false
}

fn ids(members: &[MemberDto]) -> Vec<i64> {
    members.iter().map(|member| member.id).collect()
}

fn names(choices: &[MentionChoice]) -> Vec<&str> {
    choices.iter().map(|choice| choice.name.as_str()).collect()
}

fn helper() -> AssistantDto {
    AssistantDto {
        processor: Some(PROCESSOR.to_string()),
        ..assistant(99, "Helper")
    }
}

/// Offered in roster order: never the reader, never the blocked, never a former or deleted member.
#[test]
fn a_half_typed_at_offers_the_names_it_could_mean() {
    let roster = roster();
    assert_eq!(
        vec![11, 12],
        ids(&offered(
            "dinner @An",
            &roster,
            ME,
            &bob_blocked,
            true,
            false
        ))
    );
    assert_eq!(
        vec![11, 12],
        ids(&offered("@", &roster, ME, &bob_blocked, true, false))
    );
    assert_eq!(
        vec![13],
        ids(&offered("@b", &roster, ME, &nobody_blocked, true, false))
    );
}

#[test]
fn nothing_is_offered_outside_the_family_chat_while_editing_or_mid_word() {
    let roster = roster();
    assert!(offered("@An", &roster, ME, &bob_blocked, false, false).is_empty());
    assert!(offered("@An", &roster, ME, &bob_blocked, true, true).is_empty());
    assert!(offered("mail@An", &roster, ME, &bob_blocked, true, false).is_empty());
    assert!(offered("no at", &roster, ME, &bob_blocked, true, false).is_empty());
}

/// A send names whom its text names — in the family chat alone, and nothing at all when it names
/// nobody.
#[test]
fn a_send_carries_the_names_its_text_says() {
    let roster = roster();
    assert_eq!(
        Some(vec![
            MentionDto::new(12, "Anna Lee"),
            MentionDto::new(13, "Bob")
        ]),
        for_send("@Anna Lee and @Bob, 7?", &roster, true)
    );
    assert_eq!(None, for_send("@Anna Lee", &roster, false));
    assert_eq!(None, for_send("@Annika and @Anneliese", &roster, true));
    assert_eq!(None, for_send("nobody here", &roster, true));
}

#[test]
fn a_name_opens_a_chat_only_with_somebody_who_can_be_messaged() {
    let roster = roster();
    assert!(opens_chat(11, &roster, ME, &bob_blocked));
    assert!(!opens_chat(ME, &roster, ME, &bob_blocked));
    assert!(!opens_chat(13, &roster, ME, &bob_blocked));
    assert!(!opens_chat(14, &roster, ME, &bob_blocked));
    assert!(!opens_chat(99, &roster, ME, &bob_blocked));
}

/// #106 (docs/protocol.md, "Mentioning a member", amended 2026-10-20): the assistant FIRST, above
/// the members, under its display name — narrowed by that name and by "ai" — and a pick writes
/// "@ai ", never "@Helper ".
#[test]
fn the_assistant_is_offered_first_and_writes_the_ai_token() {
    let roster = roster();
    let helper = helper();
    let all = choices("@", &roster, ME, &bob_blocked, true, false, Some(&helper));
    assert_eq!(vec!["Helper", "Anna", "Anna Lee"], names(&all));
    assert_eq!(None, all[0].member);
    assert_eq!("ai", all[0].writes);
    assert_eq!("@ai", all[0].handle);
    assert_eq!(
        "dinner @ai ",
        fc_text::mentions::accept("dinner @He", &all[0].writes)
    );
    assert_eq!(11, all[1].member.as_ref().expect("a member").id);
    assert_eq!("Anna", all[1].writes);
    assert_eq!("@anna", all[1].handle);

    assert_eq!(
        vec!["Helper"],
        names(&choices(
            "@he",
            &roster,
            ME,
            &bob_blocked,
            true,
            false,
            Some(&helper)
        ))
    );
    // Narrowed by "ai" as well as by its name: "@ai" finds an assistant called Robin, and "@ro"
    // does too.
    let robin = AssistantDto {
        display_name: "Robin".to_string(),
        ..helper.clone()
    };
    assert_eq!(
        vec!["Robin"],
        names(&choices(
            "@ai",
            &roster,
            ME,
            &bob_blocked,
            true,
            false,
            Some(&robin)
        ))
    );
    assert_eq!(
        vec!["Robin"],
        names(&choices(
            "@Ro",
            &roster,
            ME,
            &bob_blocked,
            true,
            false,
            Some(&robin)
        ))
    );
    assert_eq!(
        vec!["Robin", "Anna", "Anna Lee"],
        names(&choices(
            "@a",
            &roster,
            ME,
            &bob_blocked,
            true,
            false,
            Some(&robin)
        ))
    );
    assert_eq!(
        vec!["Anna", "Anna Lee"],
        names(&choices(
            "@An",
            &roster,
            ME,
            &bob_blocked,
            true,
            false,
            Some(&helper)
        ))
    );
}

/// Only where @ai may be written: the family chat, not editing, on a server whose assistant names
/// who answers.
#[test]
fn the_assistant_is_offered_only_where_it_can_be_asked() {
    let roster = roster();
    let helper = helper();
    assert!(choices("@", &roster, ME, &bob_blocked, false, false, Some(&helper)).is_empty());
    assert!(choices("@", &roster, ME, &bob_blocked, true, true, Some(&helper)).is_empty());
    assert!(
        choices(
            "no at",
            &roster,
            ME,
            &bob_blocked,
            true,
            false,
            Some(&helper)
        )
        .is_empty()
    );
    let unnamed = AssistantDto {
        processor: None,
        ..helper
    };
    assert!(
        choices("@", &roster, ME, &bob_blocked, true, false, Some(&unnamed))
            .iter()
            .all(|choice| choice.member.is_some())
    );
    assert!(
        choices("@", &roster, ME, &bob_blocked, true, false, None)
            .iter()
            .all(|choice| choice.member.is_some())
    );
}

/// The assistant's own account, should the roster carry it, is never offered as a member.
#[test]
fn the_assistants_account_is_never_a_member_line() {
    let mut roster = roster();
    roster.push(MemberDto::new(99, "helper", "Helper"));
    let choices = choices(
        "@H",
        &roster,
        ME,
        &bob_blocked,
        true,
        false,
        Some(&helper()),
    );
    assert_eq!(1, choices.len());
    assert_eq!(None, choices[0].member);
}

/// IN A GROUP THE NAMES ARE THAT GROUP'S (docs/protocol.md, "Mentioning a member", amended
/// 2026-10-22): the candidates are its live members, in the order they joined; the family chat's are
/// the live roster; anywhere else nobody.
#[test]
fn a_groups_candidates_are_its_own_live_members() {
    let roster = roster();
    let group = ChatDto {
        member_ids: Some(vec![13, ME, 12, 14]),
        admin_user_id: Some(13),
        ..ChatDto::new(77, "group", "Cousins")
    };

    assert_eq!(vec![13, ME, 12], ids(&candidates(Some(&group), &roster)));
    assert_eq!(
        vec![7, 11, 12, 13],
        ids(&candidates(
            Some(&ChatDto::new(42, "family", "The Smiths")),
            &roster
        ))
    );
    let direct = ChatDto {
        peer_user_id: Some(13),
        ..ChatDto::new(43, "direct", "Bob")
    };
    assert!(candidates(Some(&direct), &roster).is_empty());
    assert!(candidates(Some(&ChatDto::new(44, "ai", "Assistant")), &roster).is_empty());
    assert!(candidates(None, &roster).is_empty());
    // A group whose members this device has not read offers nobody, never the whole family.
    assert!(candidates(Some(&ChatDto::new(78, "group", "Old")), &roster).is_empty());

    assert!(enabled(Some("family")));
    assert!(enabled(Some("group")));
    assert!(!enabled(Some("direct")));
    assert!(!enabled(Some("ai")));
}

/// The strip and the send in a group: Anna is in the family but not in this group, so she is
/// neither offered nor sent — the server would refuse the whole message — while the group's own
/// members are.
#[test]
fn a_group_offers_and_sends_only_its_members() {
    let roster = roster();
    let group = ChatDto {
        member_ids: Some(vec![ME, 12, 13]),
        admin_user_id: Some(ME),
        ..ChatDto::new(77, "group", "Cousins")
    };
    let candidates = candidates(Some(&group), &roster);

    assert_eq!(
        vec![12],
        ids(&offered(
            "@An",
            &candidates,
            ME,
            &nobody_blocked,
            true,
            false
        ))
    );
    assert_eq!(
        Some(vec![MentionDto::new(12, "Anna Lee")]),
        for_send("@Anna Lee and @Anna", &candidates, true)
    );
    assert_eq!(None, for_send("@Anna", &candidates, true));

    let choices = choices(
        "@a",
        &candidates,
        ME,
        &nobody_blocked,
        true,
        false,
        Some(&helper()),
    );
    assert_eq!(None, choices[0].member);
    assert_eq!("ai", choices[0].writes);
    assert_eq!(
        vec![12],
        choices[1..]
            .iter()
            .map(|choice| choice.member.as_ref().expect("a member").id)
            .collect::<Vec<_>>()
    );
}
