//
//  MacVoiceCommands.swift
//  FamilyConnect
//
//  File ▸ Record Voice Message ⌥⌘R (#79, Phase 1 —
//  docs/audio-video-messages-2026-10-04.md, S1.6, S8.3, Decision 32): after
//  the new-item group, acting on the KEY WINDOW's conversation through a
//  focused value, and disabled where S1.3 would dim the microphone.
//
//  The menu bar is where a Mac keeps its shortcuts — the iPad's hidden ⌥⌘R
//  button is the same door for a keyboard without one. R is for record; ⌘R is
//  already Refresh, and nothing else in the app uses R.
//
//  Pressed during a recording it STOPS it into review: a shortcut never
//  sends (S1.6, S1.7). Record Video Message joins it in Phase 3, without a
//  shortcut — no shortcut ever opens the camera.
//

#if os(macOS)

import SwiftUI

/// What the key window's conversation offers the menu bar.
struct MacVoiceMessageTarget {
    /// `MacVoiceMenu.isEnabled` for that conversation, now.
    let isEnabled: Bool
    /// Record Voice Message — or, during a recording, Stop into review.
    let record: () -> Void
}

private struct MacVoiceMessageKey: FocusedValueKey {
    typealias Value = MacVoiceMessageTarget
}

extension FocusedValues {
    /// Published by the conversation in the key window
    /// (`MacConversationView`, `.focusedSceneValue`) — nil when the key
    /// window holds none, which disables the command.
    var macVoiceMessage: MacVoiceMessageTarget? {
        get { self[MacVoiceMessageKey.self] }
        set { self[MacVoiceMessageKey.self] = newValue }
    }
}

/// The rule, pure, so a test can hold it to the plan.
nonisolated enum MacVoiceMenu {
    /// Whether File ▸ Record Voice Message is enabled for a conversation:
    ///
    /// - during a recording, always — it stops it into review (S1.6);
    /// - never while the video recorder owns the window (S8.3, Phase 3), in
    ///   the assistant's chat, which records nothing (S1.5, Decision 24), or
    ///   while an edit borrows the composer (S1.5);
    /// - otherwise wherever S1.3 would NOT dim the microphone: no call, no
    ///   attachment guard, no "Voice message not sent" waiting. Beside words
    ///   or staged items it is enabled — it records beside them, and the slot
    ///   becomes Stop (row 3).
    static func isEnabled(_ inputs: ComposerSlot.Inputs) -> Bool {
        if inputs.recorderOpen { return false }
        if inputs.recording != .none { return true }
        if inputs.assistantChat || !inputs.canRecord || inputs.editing { return false }
        return inputs.blocked == nil
    }
}

/// The menu bar's half: one item in the File menu.
struct MacVoiceCommands: Commands {
    @FocusedValue(\.macVoiceMessage) private var target

    var body: some Commands {
        CommandGroup(after: .newItem) {
            Divider()
            Button("Record Voice Message") {
                target?.record()
            }
            .keyboardShortcut("r", modifiers: [.command, .option])
            .disabled(!(target?.isEnabled ?? false))
        }
    }
}

#endif
