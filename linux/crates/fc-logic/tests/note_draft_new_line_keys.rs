//! `NewLineKeysTests.ANoteDraftHoldsLineFeeds`, whose subject is `NoteDraft`: a note's words hold a
//! text field's carriage returns as line feeds (`new_line_keys::for_wire`).

use fc_logic::note_draft::NoteDraft;

#[test]
fn a_note_draft_holds_line_feeds() {
    let mut draft = NoteDraft::new();
    draft.set_text("milk\reggs");
    assert_eq!("milk\neggs", draft.text());
}
