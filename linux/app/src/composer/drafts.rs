//! Each chat's draft while the reader is somewhere else (shell-design §6.6): fc-logic's
//! `ComposerDrafts`, one for the process, shared by the main window's page and every conversation
//! window. IN MEMORY — the Mac's `ComposerDrafts` ("a draft is a fragment of an unsent thought, not
//! data — it survives navigation, not the process") — and TEXT ONLY: a staged photo can be picked
//! again, and an edit's or a reply's banner names a message the reader is no longer looking at.
//! A new server forgets them (another server's chat ids are other chats).

use std::cell::RefCell;
use std::rc::Rc;

use fc_logic::composer_drafts::ComposerDrafts;

thread_local! {
    static DRAFTS: Rc<RefCell<ComposerDrafts>> = Rc::new(RefCell::new(ComposerDrafts::new()));
}

/// The process's drafts.
pub fn shared() -> Rc<RefCell<ComposerDrafts>> {
    DRAFTS.with(Rc::clone)
}

/// Forget every draft (a new server).
pub fn forget_all() {
    DRAFTS.with(|drafts| *drafts.borrow_mut() = ComposerDrafts::new());
}
