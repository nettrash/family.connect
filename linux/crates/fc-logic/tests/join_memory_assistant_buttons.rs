//! `AssistantButtonsTests`, the composer's two assistant doors (`AssistantButtons`). Every door
//! needs a NAMED processor now: a client that cannot say who receives the words offers no assistant
//! (docs/protocol.md, "Consenting to the assistant"). (The file's other test,
//! `AnOwnerWhoLeftIsToldWhoHasTheFamilyNow`, is `settings_assistant_buttons.rs`.)

mod common;

use fc_logic::join_memory::AssistantButtons;

use common::consent::{draws, talks, unnamed};

const NEITHER: AssistantButtons = AssistantButtons {
    ask_assistant: false,
    ask_picture: false,
};

/// "Ask the assistant" belongs to the family chat of a server that has one — nowhere else.
#[test]
fn the_mention_door_is_the_family_chats_where_there_is_an_assistant() {
    assert!(AssistantButtons::offered(Some("family"), Some(&talks()), false).ask_assistant);
    assert!(!AssistantButtons::offered(Some("family"), None, false).ask_assistant);
    assert!(!AssistantButtons::offered(Some("direct"), Some(&talks()), false).ask_assistant);
    assert!(!AssistantButtons::offered(Some("ai"), Some(&draws()), false).ask_assistant);
    assert!(!AssistantButtons::offered(None, Some(&talks()), false).ask_assistant);
}

/// "Ask for a picture" belongs to the assistant's own chat, and only where it can draw.
#[test]
fn the_picture_door_is_the_assistants_chat_where_it_draws() {
    assert!(AssistantButtons::offered(Some("ai"), Some(&draws()), false).ask_picture);
    assert!(!AssistantButtons::offered(Some("ai"), Some(&talks()), false).ask_picture);
    assert!(!AssistantButtons::offered(Some("ai"), None, false).ask_picture);
    assert!(!AssistantButtons::offered(Some("family"), Some(&draws()), false).ask_picture);
}

/// And neither door on a server that named nobody: there is no honest way to ask, so the
/// affordances that would send something are not offered (docs/protocol.md, "Consenting to the
/// assistant").
#[test]
fn neither_door_where_the_server_named_no_processor() {
    assert_eq!(
        NEITHER,
        AssistantButtons::offered(Some("family"), Some(&unnamed()), false)
    );
    assert_eq!(
        NEITHER,
        AssistantButtons::offered(Some("ai"), Some(&unnamed()), false)
    );
}

#[test]
fn neither_door_while_a_message_is_edited() {
    assert_eq!(
        NEITHER,
        AssistantButtons::offered(Some("family"), Some(&draws()), true)
    );
    assert_eq!(
        NEITHER,
        AssistantButtons::offered(Some("ai"), Some(&draws()), true)
    );
}

/// "Ask the assistant" is offered in a GROUP as in the family chat; the paintbrush stays the
/// assistant's chat's.
#[test]
fn a_group_offers_the_assistant_like_the_family_chat() {
    assert!(AssistantButtons::offered(Some("group"), Some(&talks()), false).ask_assistant);
    assert!(!AssistantButtons::offered(Some("group"), Some(&talks()), true).ask_assistant);
    assert!(!AssistantButtons::offered(Some("group"), None, false).ask_assistant);
    assert!(!AssistantButtons::offered(Some("group"), Some(&draws()), false).ask_picture);
}
