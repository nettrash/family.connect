//! `PollsModelTests`, the composer's half: the draft behind "Poll". (The model's tests — a tap, the
//! open-polls list — are `ConversationModel`'s and `OpenPolls`', later waves.)

use fc_core::text::polls;
use fc_logic::poll_draft::PollDraft;

#[test]
fn a_draft_holds_two_to_ten_options_and_answers_only_a_poll() {
    let mut draft = PollDraft::new();
    assert_eq!(draft.options().len(), 2);
    assert!(!draft.may_remove());
    assert!(!draft.remove(0));
    assert_eq!(draft.checked(), None);

    while draft.may_add() {
        assert!(draft.add());
    }
    assert_eq!(draft.options().len(), polls::MAX_OPTIONS);
    assert!(!draft.add());

    draft.set(3, "three");
    draft.set(10, "nowhere");
    draft.set(-1, "nowhere");
    assert!(draft.remove(2));
    assert_eq!(draft.options()[2], "three");
    assert!(!draft.remove(9));
    assert!(!draft.remove(-1));

    draft.set(0, " Pizza ");
    draft.set(1, "Pasta");
    assert_eq!(draft.checked(), None);
    draft.question = "  ".to_string();
    assert_eq!(draft.checked(), None);
    draft.question = " Dinner? ".to_string();
    let (question, options) = draft.checked().expect("a poll");
    assert_eq!(question, "Dinner?");
    assert_eq!(options, ["Pizza", "Pasta", "three"]);

    draft.set(1, "PIZZA");
    assert_eq!(draft.checked(), None);
}
