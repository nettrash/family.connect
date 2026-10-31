//! `BackdropConsentTests.AnUnansweredAgreeFirstIsThePlainFailure`, whose subject is
//! `NoteSheetText.BackdropFailure` (`note_draft`): an "agree first" that the question could not
//! settle is said as the plain failure it is.

mod common;

use fc_core::protocol::{ApiError, error_codes};
use fc_logic::note_draft::note_sheet_text;

#[test]
fn an_unanswered_agree_first_is_the_plain_failure() {
    let not_agreed = ApiError::new(error_codes::ASSISTANT_CONSENT_REQUIRED, "agree first", 403);
    assert_eq!(
        "Couldn't draw that.",
        note_sheet_text::backdrop_failure(&not_agreed, &common::english(), &common::days())
    );
}
