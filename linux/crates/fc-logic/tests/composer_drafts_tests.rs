//! `ComposerDraftsTests`: half a thought kept for the chat it was written in (the web client's
//! drafts).

use fc_logic::composer_drafts::ComposerDrafts;

#[test]
fn a_draft_is_kept_per_chat_and_given_back() {
    let mut drafts = ComposerDrafts::new();
    drafts.save(42, "half a thought");
    drafts.save(50, "a question ");
    assert_eq!(drafts.of(42), "half a thought");
    // Kept as written — the white space inside and after the words is the author's.
    assert_eq!(drafts.of(50), "a question ");
    assert_eq!(drafts.of(77), "");
}

#[test]
fn white_space_is_no_draft_and_forgets_the_last() {
    let mut drafts = ComposerDrafts::new();
    drafts.save(42, "half a thought");
    drafts.save(42, " \n\t");
    assert_eq!(drafts.of(42), "");
    assert_eq!(drafts.count(), 0);
    drafts.save(42, "again");
    drafts.save(42, "");
    assert_eq!(drafts.count(), 0);
}

#[test]
fn sending_takes_the_draft_with_it() {
    let mut drafts = ComposerDrafts::new();
    drafts.save(42, "on its way");
    drafts.save(50, "stays");
    drafts.sent(42);
    assert_eq!(drafts.of(42), "");
    assert_eq!(drafts.of(50), "stays");
}

#[test]
fn a_chat_that_is_gone_takes_its_draft_with_it() {
    let mut drafts = ComposerDrafts::new();
    drafts.save(42, "half a thought");
    drafts.save(50, "a question");
    drafts.retain([50, 60]);
    assert_eq!(drafts.of(42), "");
    assert_eq!(drafts.of(50), "a question");
    assert_eq!(drafts.count(), 1);
}
