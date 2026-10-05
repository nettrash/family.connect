//
//  MacRecordSendSlot.swift
//  FamilyConnect
//
//  The Mac composer's trailing slot (#79, Phase 1 —
//  docs/audio-video-messages-2026-10-04.md, S1.3, S1.6, S6, S7.1, S8.3):
//  Send when there is something to send, a microphone when the composer is
//  empty, the Send arrow or the Stop square while a voice message records.
//
//  A CLICK, NEVER A HOLD. A click records hands-free and the same slot sends
//  it (S8.3; `AudioRecorder`'s header has always said why a mouse must not be
//  held down for the length of a message). So this is an ordinary SwiftUI
//  button whose action is the reducer's `activate` — the phone's UIKit
//  control exists only because a finger can slide.
//
//  WHAT THE PHONE HAS THAT A MAC DOES NOT: the hold, the Undo window that
//  only a released hold opens, haptics, the coach mark and the Review Before
//  Sending setting. What it has that the phone does not: a tooltip in every
//  state — the slot had none, and no accessibility label either — and
//  `.contextMenu`, which is safe here because a Mac has no touch hold for it
//  to claim (S1.6, "Where it plugs in").
//
//  RETURN IS NOT THIS BUTTON'S SHORTCUT. Return activates the slot on rows 2
//  to 5 only, and the composer binds it to a hidden button that asks the
//  slot again at the moment of the key press (`takesReturn`) — so a key that
//  two bindings both see (the field's `onSubmit` and the shortcut) can never
//  reach a microphone that the first of them just created by sending.
//

#if os(macOS)

import SwiftUI

/// What the slot is in each S1.3 row — pure, so every state can be held to
/// the plan without a window (MacVoiceComposerTests). Nonisolated, as the
/// shared rules it reads are.
nonisolated enum MacRecordSlot {

    /// ⌥⌘R, as the menu bar draws it beside "Record Voice Message" and the
    /// microphone's tooltip quotes it (S1.6, S7.1).
    static let shortcutGlyphs = "⌥⌘R"

    /// S1.3's glyphs: `mic.circle.fill`, today's `arrow.up.circle.fill`, and
    /// `stop.circle.fill`. Save keeps today's arrow — the Mac's edit has
    /// always saved from the same button.
    static func symbol(for slot: ComposerSlot) -> String {
        switch slot {
        case .stopRecording: "stop.circle.fill"
        case .microphone, .dimmed, .heldMicrophone: "mic.circle.fill"
        case .recorder, .sendVoice, .save, .send, .sendDisabled: "arrow.up.circle.fill"
        }
    }

    /// The accessibility label, in every state (S6, S8.3).
    static func label(for slot: ComposerSlot) -> String {
        slot.label ?? String(localized: "Send")
    }

    /// The tooltip, in every state (S8.3): "Record a voice message (⌥⌘R)" on
    /// the microphone, dimmed or not (S7.1) — the reason a dimmed one gives is
    /// its value, and what a click on it says — and the label everywhere
    /// else: "Send voice message" while one records (S2.4), "Stop recording",
    /// "Send", "Save".
    static func tooltip(for slot: ComposerSlot) -> String {
        slot.isMicrophone
            ? String(localized: "Record a voice message (\(shortcutGlyphs))")
            : label(for: slot)
    }

    /// The dimmed microphone's reason, read after its label (S6).
    static func value(for slot: ComposerSlot) -> String? {
        slot.notice
    }

    /// Really disabled: today's Send with nothing it can send, Save with a
    /// blank field, the assistant's chat. NEVER a dimmed microphone, which
    /// only looks disabled and says why when clicked (S1.3). `canSend` is
    /// the composer's own Send rule — false while a location is being found.
    static func isDisabled(_ slot: ComposerSlot, canSend: Bool) -> Bool {
        switch slot {
        case .sendDisabled, .save(enabled: false), .recorder, .heldMicrophone: true
        case .send: !canSend
        case .sendVoice, .stopRecording, .save(enabled: true), .dimmed, .microphone: false
        }
    }

    /// Drawn in the disabled ink: the disabled rows, and the dimmed ones.
    static func looksDisabled(_ slot: ComposerSlot, canSend: Bool) -> Bool {
        if case .dimmed = slot { return true }
        return isDisabled(slot, canSend: canSend)
    }

    /// Whether Return activates the slot now (S1.3, S8.3): rows 2 to 5 —
    /// Send, Save, the Send arrow, the Stop square — and only when that row
    /// is not disabled. Never a microphone: Return in an empty field records
    /// nothing.
    static func takesReturn(_ slot: ComposerSlot, canSend: Bool) -> Bool {
        slot.takesReturn && !isDisabled(slot, canSend: canSend)
    }

    /// What activating the slot does — a click, VO-Space, Full Keyboard
    /// Access's Space, or Return on rows 2 to 5 (S1.3). A click records
    /// hands-free: a Mac has no hold (S8.3).
    enum Activation: Equatable {
        /// Today's Send: the words and staged items go.
        case send
        /// Today's Save: the edit goes.
        case save
        /// The voice flow's own: record from a microphone, say why from a
        /// dimmed one, send from the arrow, stop from the square.
        case voice
        /// Nothing: a disabled row, or a Send the activation guard holds.
        case nothing
    }

    /// `sendGuarded`: the slot's own activation changed it less than 600 ms
    /// ago (S1.1) — the Stop square staged the note it now offers to send,
    /// so a double click on Stop must not send it. Typing lifts the guard
    /// (`VoiceComposer.typedBinding`), so "ok" sent at once still goes.
    static func activation(of slot: ComposerSlot, sendGuarded: Bool) -> Activation {
        switch slot {
        case .send: sendGuarded ? .nothing : .send
        case .save(enabled: true): .save
        case .sendVoice, .stopRecording, .microphone, .dimmed: .voice
        case .save(enabled: false), .sendDisabled, .heldMicrophone, .recorder: .nothing
        }
    }

    /// The microphone's secondary menu — right-click, Control-click, a
    /// two-finger click, VoiceOver's VO-Shift-M — on rows 7 to 10 only. In
    /// rows 7 to 9 its item explains instead of recording (S1.6).
    static func offersMenu(_ slot: ComposerSlot) -> Bool {
        slot.isMicrophone
    }

    /// What a screen reader can do with the slot besides activating it.
    enum Action: Equatable, CaseIterable {
        /// "Stop and listen first": into review instead of out (S6).
        case stopAndListen
        /// "Delete recording".
        case delete
    }

    /// S6's actions while recording: the Send arrow offers both, the Stop
    /// square — which already stops — only Delete.
    static func actions(for slot: ComposerSlot) -> [Action] {
        switch slot {
        case .sendVoice: [.stopAndListen, .delete]
        case .stopRecording: [.delete]
        default: []
        }
    }

    /// Voice Control's names for it: the label, and on the microphone the
    /// three words S6 adds.
    static func inputLabels(for slot: ComposerSlot) -> [String] {
        guard slot.isMicrophone else { return [label(for: slot)] }
        return [
            label(for: slot),
            String(localized: "Microphone"),
            String(localized: "Record"),
            String(localized: "Voice message"),
        ]
    }
}

/// Return, once per key press (S1.3, S8.3).
///
/// Two bindings can see one Return: the field's `onSubmit` and the window
/// shortcut the composer gives the slot on rows 2 to 5. On row 5 the first
/// of them SENDS, which empties the composer and makes the slot a
/// microphone — so the second must neither act again nor, above all, record.
/// Each press is asked of the slot as it is at that moment, and a press
/// already answered is not answered twice.
nonisolated struct MacReturnKey: Equatable {
    /// The key event Return last answered, by its timestamp.
    private(set) var lastPress: TimeInterval?

    /// Whether this press activates the slot. `timestamp` is the key event's
    /// (`NSEvent.timestamp`), nil when no key event is at hand — which is
    /// then judged on the slot alone.
    mutating func acts(onPressAt timestamp: TimeInterval?, slot: ComposerSlot, canSend: Bool) -> Bool {
        if let timestamp {
            guard lastPress != timestamp else { return false }
            lastPress = timestamp
        }
        return MacRecordSlot.takesReturn(slot, canSend: canSend)
    }
}

/// The slot itself.
struct MacRecordSendSlot: View {
    let slot: ComposerSlot
    /// The composer's Send rule (`canSend`), for row 5.
    let canSend: Bool
    /// The composer's control box — the paperclip's, so the two sit level.
    let side: CGFloat
    /// Bumped when a recording starts: focus goes to the slot and stays there
    /// while it runs (S2.4) — VoiceOver's, and the keyboard's where Full
    /// Keyboard Access lets a button hold it.
    let focusRequest: Int
    /// A click, VO-Space, Full Keyboard Access's Space: the slot's own
    /// activation, whatever row it is in.
    let onActivate: () -> Void
    /// The secondary menu's Record Voice Message.
    let onRecordFromMenu: () -> Void
    let onStopAndListen: () -> Void
    let onDelete: () -> Void
    /// VoiceOver's escape: Stop, as Esc is (S6).
    let onEscape: () -> Void

    @AccessibilityFocusState private var voiceOverFocus: Bool
    @FocusState private var keyboardFocus: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var symbol: String { MacRecordSlot.symbol(for: slot) }

    var body: some View {
        Button(action: onActivate) {
            Image(systemName: symbol)
                .font(.system(size: 22))
                .foregroundStyle(
                    MacRecordSlot.looksDisabled(slot, canSend: canSend)
                        ? AnyShapeStyle(.tertiary) : AnyShapeStyle(.tint))
                // Send ↔ microphone is a 150 ms cross-fade, none under
                // Reduce Motion (S1.1, S1.3). The slot never moves or
                // changes size: one box for every glyph.
                .id(symbol)
                .transition(.opacity)
                .frame(width: side, height: side)
                .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .animation(
            reduceMotion ? nil : .easeInOut(duration: Double(RecordRules.slotCrossfadeMS) / 1000),
            value: symbol)
        .disabled(MacRecordSlot.isDisabled(slot, canSend: canSend))
        .help(MacRecordSlot.tooltip(for: slot))
        .accessibilityLabel(Text(verbatim: MacRecordSlot.label(for: slot)))
        .accessibilityValue(Text(verbatim: MacRecordSlot.value(for: slot) ?? ""))
        .accessibilityInputLabels(MacRecordSlot.inputLabels(for: slot).map { Text(verbatim: $0) })
        .modifier(RecordingActions(
            actions: MacRecordSlot.actions(for: slot), stop: onStopAndListen, delete: onDelete))
        .accessibilityAction(.escape, onEscape)
        .accessibilityFocused($voiceOverFocus)
        .focused($keyboardFocus)
        .contextMenu {
            if MacRecordSlot.offersMenu(slot) {
                // Record Video Message joins it in Phase 3, where round video
                // is available (S1.6) — never before (Decision 40).
                Button {
                    onRecordFromMenu()
                } label: {
                    Label("Record Voice Message", systemImage: "mic")
                }
                // Drawn beside the item, as S1.6 asks; the command that
                // answers ⌥⌘R is the menu bar's File ▸ Record Voice Message.
                // Were this one ever live too, it would do the same thing —
                // the item is the slot's own, and only on a microphone.
                .keyboardShortcut("r", modifiers: [.command, .option])
            }
        }
        .onChange(of: focusRequest) {
            voiceOverFocus = true
            keyboardFocus = true
        }
    }
}

/// S6's two actions while recording, only where the slot offers them —
/// a screen reader must not hear "Delete recording" on a Send button.
private struct RecordingActions: ViewModifier {
    let actions: [MacRecordSlot.Action]
    let stop: () -> Void
    let delete: () -> Void

    @ViewBuilder
    func body(content: Content) -> some View {
        if actions.contains(.stopAndListen) {
            content
                .accessibilityAction(named: Text("Stop and listen first"), stop)
                .accessibilityAction(named: Text("Delete recording"), delete)
        } else if actions.contains(.delete) {
            content
                .accessibilityAction(named: Text("Delete recording"), delete)
        } else {
            content
        }
    }
}

#endif
