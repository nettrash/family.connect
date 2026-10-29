//! What a key in the composer's field does (shell-design §6.2) — pure, so it is tested without a
//! display. The rules are fc-logic's (`NewLineKeys`, `ComposerButton`'s slot); this only puts them
//! in the order a key event meets them.

use fc_logic::composer_button::Slot;
use fc_logic::new_line_keys;

/// What Enter (or keypad Enter) does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnterAction {
    /// An input method is composing: Enter commits the composition and is its own.
    ImeOwns,
    /// Shift, Ctrl or Alt held: a new line in the field (never U+2028).
    NewLine,
    /// Enter alone on a row whose slot takes it (Send, an enabled Save): the slot's activation.
    ActivateSlot,
    /// Enter alone anywhere else — an empty idle field, a microphone, a disabled Send: nothing.
    Nothing,
}

/// Whether the slot in this row takes Enter: Send, or a Save with something to save.
pub fn takes_return(slot: Slot) -> bool {
    matches!(slot, Slot::Send | Slot::Save { enabled: true })
}

/// Enter's action, in the order §6.2 lists it.
pub fn enter(composing: bool, shift: bool, control: bool, alt: bool, slot: Slot) -> EnterAction {
    if composing {
        return EnterAction::ImeOwns;
    }
    if new_line_keys::enter_makes_line(shift, control, alt) {
        return EnterAction::NewLine;
    }
    if takes_return(slot) {
        EnterAction::ActivateSlot
    } else {
        EnterAction::Nothing
    }
}

/// What Escape does in the composer (bubble phase), first match wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeAction {
    /// Close the `@` suggestions.
    CloseMentions,
    /// A voice recording runs: stop it into review.
    StopRecording,
    /// An edit is open: cancel it, the draft comes back.
    CancelEdit,
    /// A reply is primed: drop it.
    DropReply,
    /// Nothing to undo: the key goes on (a dialog may close).
    Nothing,
}

pub fn escape(mentions: bool, recording: bool, editing: bool, replying: bool) -> EscapeAction {
    if mentions {
        EscapeAction::CloseMentions
    } else if recording {
        EscapeAction::StopRecording
    } else if editing {
        EscapeAction::CancelEdit
    } else if replying {
        EscapeAction::DropReply
    } else {
        EscapeAction::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fc_logic::composer_button::Dimmed;

    #[test]
    fn enter_alone_sends_only_where_the_slot_takes_it() {
        assert_eq!(
            enter(false, false, false, false, Slot::Send),
            EnterAction::ActivateSlot
        );
        assert_eq!(
            enter(false, false, false, false, Slot::Save { enabled: true }),
            EnterAction::ActivateSlot
        );
        assert_eq!(
            enter(false, false, false, false, Slot::Save { enabled: false }),
            EnterAction::Nothing
        );
        // An empty idle field is never a microphone on Enter.
        assert_eq!(
            enter(false, false, false, false, Slot::Microphone),
            EnterAction::Nothing
        );
        assert_eq!(
            enter(false, false, false, false, Slot::SendDisabled),
            EnterAction::Nothing
        );
        assert_eq!(
            enter(false, false, false, false, Slot::Dimmed(Dimmed::Call)),
            EnterAction::Nothing
        );
        assert_eq!(
            enter(false, false, false, false, Slot::SendVoice),
            EnterAction::Nothing
        );
    }

    #[test]
    fn any_of_the_three_modifiers_makes_a_line() {
        for (shift, control, alt) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            assert_eq!(
                enter(false, shift, control, alt, Slot::Send),
                EnterAction::NewLine
            );
        }
    }

    #[test]
    fn an_input_method_composing_owns_enter() {
        assert_eq!(
            enter(true, false, false, false, Slot::Send),
            EnterAction::ImeOwns
        );
        assert_eq!(
            enter(true, true, false, false, Slot::Send),
            EnterAction::ImeOwns
        );
    }

    #[test]
    fn escape_undoes_the_nearest_thing_first() {
        assert_eq!(escape(true, true, true, true), EscapeAction::CloseMentions);
        assert_eq!(escape(false, true, true, true), EscapeAction::StopRecording);
        assert_eq!(escape(false, false, true, true), EscapeAction::CancelEdit);
        assert_eq!(escape(false, false, false, true), EscapeAction::DropReply);
        assert_eq!(escape(false, false, false, false), EscapeAction::Nothing);
    }
}
