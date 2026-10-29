//! `CultureTests.TheRowTimeIsTheSameDecisionInEveryLanguage`, whose subject is `ChatListModel.When`:
//! a row's time is a DECISION here, not a format — so the model answers the same thing in any
//! language, and the window draws it in the reader's own. (Rust has no current culture to switch;
//! the reader's language is `fc_text::i18n::use_lang`, which is switched instead — and the answer
//! does not move.)

use fc_logic::chat_list::{ChatListModel, RowTimeKind};
use time::macros::datetime;
use time::{Duration, UtcOffset};

#[test]
fn the_row_time_is_the_same_decision_in_every_language() {
    let now = datetime!(2026-09-12 12:00 UTC);
    let zone = UtcOffset::UTC;
    let english = ChatListModel::when(now - Duration::days(1), now, &zone);

    fc_text::i18n::use_lang(fc_text::i18n::Lang::Ja);

    assert_eq!(
        english,
        ChatListModel::when(now - Duration::days(1), now, &zone)
    );
    assert_eq!(RowTimeKind::Yesterday, english);
    fc_text::i18n::use_lang(fc_text::i18n::Lang::En);
}
