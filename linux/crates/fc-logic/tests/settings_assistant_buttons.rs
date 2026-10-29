//! `AssistantButtonsTests`' one test whose subject is `Settings.cs` (the rest is
//! `AssistantButtons`': `join_memory_assistant_buttons.rs`).

mod common;

use fc_logic::settings::settings_text;

#[test]
fn an_owner_who_left_is_told_who_has_the_family_now() {
    assert_eq!(
        settings_text::ownership_passed("Anna", &common::english()),
        (
            "Ownership passed on".to_string(),
            "Anna is now the owner of the family.".to_string()
        )
    );
}
