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
//  sends (S1.6, S1.7). Record Video Message sits under it (Phase 3), without
//  a shortcut — no shortcut ever opens the camera.
//

#if os(macOS)

import SwiftUI

/// What the key window's conversation offers the menu bar.
struct MacVoiceMessageTarget {
    /// `MacVoiceMenu.isEnabled` for that conversation, now.
    let isEnabled: Bool
    /// Record Voice Message — or, during a recording, Stop into review.
    let record: () -> Void
    /// Round video is available here: File ▸ Record Video Message exists.
    var offersVideo = false
    /// `MacVoiceMenu.videoIsEnabled` for that conversation, now.
    var videoIsEnabled = false
    /// Record Video Message: opens the recorder (Phase 3).
    var recordVideo: () -> Void = {}
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

    /// Whether File ▸ Record Video Message is enabled (S1.5, S1.6, S8.3):
    /// only where round video is available; never while the recorder is
    /// already open, a voice message records, an edit borrows the composer,
    /// in the assistant's chat, during a call or while an attachment is
    /// busy (rows 7–8). A waiting voice message does NOT disable it — that
    /// rule is about voice — and words typed or items staged do not either:
    /// a video message travels alone.
    static func videoIsEnabled(_ inputs: ComposerSlot.Inputs, roundAvailable: Bool) -> Bool {
        guard roundAvailable, !inputs.recorderOpen, inputs.recording == .none else { return false }
        if inputs.assistantChat || inputs.editing { return false }
        return !inputs.call && !inputs.busy
    }
}

/// The menu bar's half: one item in the File menu.
struct MacVoiceCommands: Commands {
    @FocusedValue(\.macVoiceMessage) private var target
    @FocusedValue(\.macRecorderOpen) private var recorderOpen

    var body: some Commands {
        CommandGroup(after: .newItem) {
            Divider()
            Button("Record Voice Message") {
                target?.record()
            }
            .keyboardShortcut("r", modifiers: [.command, .option])
            .disabled(!(target?.isEnabled ?? false) || recorderOpen == true)
            // No shortcut: no shortcut ever opens the camera (S1.6).
            if target?.offersVideo == true {
                Button("Record Video Message") {
                    target?.recordVideo()
                }
                .disabled(!(target?.videoIsEnabled ?? false) || recorderOpen == true)
            }
        }
    }
}

/// View ▸ Refresh ⌘R — disabled while the key window's video recorder is
/// open, as the conversation's other commands are (S8.3).
struct MacRefreshCommands: Commands {
    @FocusedValue(\.macRecorderOpen) private var recorderOpen

    var body: some Commands {
        CommandGroup(after: .toolbar) {
            Button("Refresh") {
                NotificationCenter.default.post(name: .macRequestResync, object: nil)
            }
            .keyboardShortcut("r", modifiers: .command)
            .disabled(recorderOpen == true)
        }
    }
}

#endif
