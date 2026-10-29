//! `HouseRulesTests`. The member limit's footer is `fc_text::account` (`cap_state`,
//! `cap_footer`), said in `fc_text::i18n`'s English; the policies and the switches' notes are
//! `fc_core::text::house_rules`.

use fc_core::text::house_rules::{POLICIES, assistant_failure, pictures_note, policy_caption};
use fc_core::text::string_catalog::EnglishCatalog;
use fc_text::account::{cap_footer, cap_state};

#[test]
fn the_member_limit_footer_says_which_of_its_three_states_it_is_in() {
    assert_eq!(
        cap_footer(cap_state(None, 3, 50)),
        "No limit of your own. This server allows up to 50 members in a family."
    );
    assert_eq!(cap_footer(cap_state(Some(5), 3, 50)), "3 of 5 seats used.");
    // The LOWER of the two binds: a cap of 40 under a ceiling of 10 is ten seats.
    assert_eq!(
        cap_footer(cap_state(Some(40), 3, 10)),
        "3 of 10 seats used."
    );
    assert_eq!(
        cap_footer(cap_state(Some(2), 4, 10)),
        "4 members now. Nobody new can join until somebody leaves; no one is removed."
    );
    assert_eq!(
        cap_footer(cap_state(Some(1), 1, 10)),
        "1 member now. Nobody new can join until somebody leaves; no one is removed."
    );
}

#[test]
fn the_policy_caption_follows_the_policy() {
    let say = EnglishCatalog::new();
    assert_eq!(
        policy_caption(Some("open"), &say),
        "Anyone with the invite code joins straight away."
    );
    assert_eq!(
        policy_caption(None, &say),
        "Anyone with the invite code joins straight away."
    );
    assert!(policy_caption(Some("approval"), &say).starts_with("With approval"));
    assert!(policy_caption(Some("closed"), &say).starts_with("The invite code stops working"));
    let codes: Vec<&str> = POLICIES.iter().map(|(code, _)| *code).collect();
    assert_eq!(codes, ["open", "approval", "closed"]);
}

/// A switch that rides on vision says why it does nothing — the server, then vision, then the
/// history it draws from — and nothing once it does something. (The C# passes a `FamilyDto` with
/// `AiVision` and `AiHistory`; here those two are passed.)
#[test]
fn a_switch_that_rides_on_vision_says_why_it_does_nothing() {
    let say = EnglishCatalog::new();
    let history_off = "history is off";
    assert!(
        pictures_note(false, true, true, history_off, &say)
            .unwrap()
            .starts_with("Not available here")
    );
    assert!(
        pictures_note(true, false, false, history_off, &say)
            .unwrap()
            .starts_with("Turn on Can be shown photos first")
    );
    assert_eq!(
        pictures_note(true, true, false, history_off, &say).as_deref(),
        Some(history_off)
    );
    assert_eq!(pictures_note(true, true, true, history_off, &say), None);
}

/// (The C# passes an `ApiError`; here its code: `not_family_owner`, and `__transport` for
/// `ApiError.Transport`.)
#[test]
fn an_assistant_change_refused_is_said_for_what_it_was() {
    let say = EnglishCatalog::new();
    assert_eq!(
        assistant_failure("not_family_owner", &say),
        "Only the family owner can change this."
    );
    assert_eq!(
        assistant_failure("__transport", &say),
        "Couldn't save that. Try again."
    );
}
