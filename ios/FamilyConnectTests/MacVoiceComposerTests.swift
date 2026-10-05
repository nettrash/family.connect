//
//  MacVoiceComposerTests.swift
//  FamilyConnectTests
//
//  #79, Phase 1 on the Mac (docs/audio-video-messages-2026-10-04.md, S1.3,
//  S1.5, S1.6, S2.4, S6, S7.1, S8.3): what the Mac's slot shows and says in
//  every row, when Return reaches it, when File ▸ Record Voice Message is
//  enabled, how long the microphone waits for VoiceOver, and that the
//  recording row never grows the bar.
//
//  The shared rules themselves — which row the slot is in, what the reducer
//  does with a click — are held to the vectors by RecordVectorTests and
//  driven end to end by VoiceComposerTests; what is pinned HERE is the Mac's
//  own reading of them.
//

#if os(macOS)

import CoreGraphics
import Foundation
import SwiftUI
import Testing
@testable import FamilyConnect

@Suite("Mac slot: what it shows and says in every row")
struct MacRecordSlotTests {

    private static let slots: [ComposerSlot] = [
        .recorder, .heldMicrophone, .sendVoice, .stopRecording, .save(enabled: true), .save(enabled: false),
        .send, .sendDisabled, .dimmed(.call), .dimmed(.busy), .dimmed(.notSent), .microphone,
    ]

    private static let microphones: [ComposerSlot] = [
        .dimmed(.call), .dimmed(.busy), .dimmed(.notSent), .microphone,
    ]

    private func catalogue(_ key: String) -> String {
        String(localized: String.LocalizationValue(key))
    }

    @Test("a tooltip and an accessibility label in every state", arguments: slots)
    func everyStateIsNamed(slot: ComposerSlot) {
        #expect(!MacRecordSlot.tooltip(for: slot).isEmpty, "\(slot) has no tooltip")
        #expect(!MacRecordSlot.label(for: slot).isEmpty, "\(slot) has no accessibility label")
    }

    @Test("the microphone's tooltip names ⌥⌘R, dimmed or not; every other row's is its label")
    func tooltips() {
        let microphone = String(format: catalogue("Record a voice message (%@)"), "⌥⌘R")
        for slot in Self.slots {
            if slot.isMicrophone {
                #expect(MacRecordSlot.tooltip(for: slot) == microphone, "\(slot)")
            } else {
                #expect(MacRecordSlot.tooltip(for: slot) == MacRecordSlot.label(for: slot), "\(slot)")
            }
        }
        #expect(MacRecordSlot.tooltip(for: .sendVoice) == catalogue("Send voice message"))
        #expect(MacRecordSlot.tooltip(for: .stopRecording) == catalogue("Stop recording"))
        #expect(MacRecordSlot.label(for: .microphone) == catalogue("Record voice message"))
    }

    @Test("S1.3's glyphs: the microphone on rows 7 to 10, the Stop square on row 3, the arrow everywhere else")
    func glyphs() {
        for slot in Self.slots {
            let expected: String
            switch slot {
            case .stopRecording: expected = "stop.circle.fill"
            case .microphone, .dimmed, .heldMicrophone: expected = "mic.circle.fill"
            default: expected = "arrow.up.circle.fill"
            }
            #expect(MacRecordSlot.symbol(for: slot) == expected, "\(slot)")
        }
    }

    @Test("dimmed is not disabled: the microphone stays clickable, looks disabled, and its reason is its value")
    func dimmedIsNotDisabled() {
        for reason in [ComposerSlot.Dimmed.call, .busy, .notSent] {
            let slot = ComposerSlot.dimmed(reason)
            #expect(!MacRecordSlot.isDisabled(slot, canSend: true), "\(slot) cannot be clicked to say why")
            #expect(MacRecordSlot.looksDisabled(slot, canSend: true))
            #expect(MacRecordSlot.value(for: slot) == reason.notice)
        }
        #expect(!MacRecordSlot.looksDisabled(.microphone, canSend: true))
        #expect(MacRecordSlot.value(for: .microphone) == nil)
        #expect(MacRecordSlot.value(for: .sendVoice) == nil)
    }

    @Test("really disabled: the assistant's Send, a blank Save, and today's Send while it cannot send")
    func disabledRows() {
        #expect(MacRecordSlot.isDisabled(.sendDisabled, canSend: true))
        #expect(MacRecordSlot.isDisabled(.save(enabled: false), canSend: true))
        #expect(MacRecordSlot.isDisabled(.send, canSend: false))
        #expect(!MacRecordSlot.isDisabled(.send, canSend: true))
        for slot in [ComposerSlot.sendVoice, .stopRecording, .save(enabled: true), .microphone] {
            // A recording can always be sent or stopped, whatever `canSend` says.
            #expect(!MacRecordSlot.isDisabled(slot, canSend: false), "\(slot)")
        }
    }

    @Test("Return takes rows 2 to 5 only, and never a disabled one — never a microphone")
    func returnRows() {
        for slot in Self.slots {
            for canSend in [true, false] {
                let expected = (2...5).contains(slot.row) && !MacRecordSlot.isDisabled(slot, canSend: canSend)
                #expect(MacRecordSlot.takesReturn(slot, canSend: canSend) == expected, "\(slot), canSend \(canSend)")
            }
        }
        for slot in Self.microphones {
            #expect(!MacRecordSlot.takesReturn(slot, canSend: true), "Return reached a microphone: \(slot)")
        }
        #expect(!MacRecordSlot.takesReturn(.save(enabled: false), canSend: true))
        #expect(!MacRecordSlot.takesReturn(.send, canSend: false))
    }

    @Test("the secondary menu is the microphone's alone: rows 7 to 10")
    func secondaryMenu() {
        for slot in Self.slots {
            #expect(MacRecordSlot.offersMenu(slot) == (7...10).contains(slot.row), "\(slot)")
        }
    }

    @Test("while recording: the Send arrow offers Stop and listen first and Delete, the Stop square only Delete")
    func recordingActions() {
        #expect(MacRecordSlot.actions(for: .sendVoice) == [.stopAndListen, .delete])
        #expect(MacRecordSlot.actions(for: .stopRecording) == [.delete])
        for slot in Self.slots where slot != .sendVoice && slot != .stopRecording {
            #expect(MacRecordSlot.actions(for: slot).isEmpty, "\(slot) offers recording actions")
        }
    }

    @Test("a click: Send sends, Save saves, every voice row goes to the voice flow, a disabled row does nothing")
    func activation() {
        for slot in Self.slots {
            let expected: MacRecordSlot.Activation
            switch slot {
            case .send: expected = .send
            case .save(enabled: true): expected = .save
            case .sendVoice, .stopRecording, .microphone, .dimmed: expected = .voice
            default: expected = .nothing
            }
            #expect(MacRecordSlot.activation(of: slot, sendGuarded: false) == expected, "\(slot)")
        }
        // A dimmed microphone is clicked to say why — the voice flow says it.
        #expect(MacRecordSlot.activation(of: .dimmed(.notSent), sendGuarded: false) == .voice)
    }

    /// S1.1: a Stop in row 3 staged the note, and the slot it leaves is
    /// Send — a double click on Stop must not send it. The guard holds
    /// ONLY a text Send: a recording's own Send arrow and Stop square are
    /// the reducer's to guard, and Save is never the slot's own change.
    @Test("the activation guard holds a text Send and nothing else")
    func guardHoldsOnlySend() {
        #expect(MacRecordSlot.activation(of: .send, sendGuarded: true) == .nothing)
        for slot in Self.slots where slot != .send {
            #expect(
                MacRecordSlot.activation(of: slot, sendGuarded: true)
                    == MacRecordSlot.activation(of: slot, sendGuarded: false),
                "the guard changed what \(slot) does")
        }
    }

    @Test("Voice Control: the label, and on the microphone the three words S6 adds")
    func inputLabels() {
        for slot in Self.microphones {
            #expect(MacRecordSlot.inputLabels(for: slot) == [
                catalogue("Record voice message"), catalogue("Microphone"), catalogue("Record"),
                catalogue("Voice message"),
            ], "\(slot)")
        }
        #expect(MacRecordSlot.inputLabels(for: .send) == [catalogue("Send")])
        #expect(MacRecordSlot.inputLabels(for: .sendVoice) == [catalogue("Send voice message")])
    }
}

@Suite("Mac Return: the slot's rows, once per key press")
struct MacReturnKeyTests {

    /// One press, through a fresh key — `#expect` cannot hold a mutating call.
    private func press(_ slot: ComposerSlot, canSend: Bool = true) -> Bool {
        var key = MacReturnKey()
        return key.acts(onPressAt: 1, slot: slot, canSend: canSend)
    }

    @Test("a press acts on rows 2 to 5 and nowhere else")
    func rows() {
        for slot in [ComposerSlot.sendVoice, .stopRecording, .save(enabled: true), .send] {
            #expect(press(slot), "\(slot)")
        }
        for slot in [ComposerSlot.microphone, .dimmed(.call), .sendDisabled, .save(enabled: false)] {
            #expect(!press(slot), "Return acted on \(slot)")
        }
        #expect(!press(.send, canSend: false), "Return sent what cannot be sent")
    }

    /// The case the rule exists for: Return in the field sends (row 5), the
    /// composer empties, the slot becomes a microphone — and the window
    /// shortcut, seeing the SAME press, must not reach it.
    @Test("one press seen by two bindings acts once, and never on the microphone the first one made")
    func oncePerPress() {
        var key = MacReturnKey()
        let first = key.acts(onPressAt: 42.5, slot: .send, canSend: true)
        let again = key.acts(onPressAt: 42.5, slot: .send, canSend: true)
        let onTheMicrophone = key.acts(onPressAt: 42.5, slot: .microphone, canSend: true)
        let next = key.acts(onPressAt: 43.1, slot: .save(enabled: true), canSend: true)
        #expect(first)
        #expect(!again, "one press acted twice")
        #expect(!onTheMicrophone)
        // The next press is a press of its own.
        #expect(next)
    }

    @Test("with no key event at hand it is judged on the slot alone")
    func noEvent() {
        var key = MacReturnKey()
        let first = key.acts(onPressAt: nil, slot: .send, canSend: true)
        let second = key.acts(onPressAt: nil, slot: .send, canSend: true)
        let microphone = key.acts(onPressAt: nil, slot: .microphone, canSend: true)
        #expect(first && second)
        #expect(!microphone)
    }
}

/// S1.1 on the Mac, end to end through the voice flow: the guard a Stop
/// leaves on the Send it turns into, and the typing that lifts it.
@MainActor
@Suite("Mac slot: the activation guard after Stop, and typing", .serialized)
struct MacSlotGuardTests {

    @Test("beside words, Stop stages the note and a double click cannot send it; a character typed can")
    func stopThenTyping() async throws {
        let h = try VoiceComposerTests.Harness()
        var draft = "see you"
        // ⌥⌘R beside words: hands-free, and the slot is the Stop square.
        h.now = 20_000
        h.voice.record(besideDraft: true)
        await h.voice.startTask?.value
        #expect(h.voice.isBesideDraft)
        h.recorded(2.0)
        h.now = 21_000
        h.voice.activate()
        #expect(h.reviewed.count == 1, "the Stop square did not stage the note")

        // The second click of a double click lands on Send.
        h.now = 21_200
        let slot = ComposerSlot.of(ComposerSlot.Inputs(draftBlank: false, staged: true))
        #expect(slot == .send)
        #expect(MacRecordSlot.activation(of: slot, sendGuarded: h.voice.sendIsGuarded) == .nothing,
                "a double click on Stop sent the note it had just staged")

        // A character typed is the person's own change: Send goes at once.
        let field = VoiceComposer.typedBinding(Binding(get: { draft }, set: { draft = $0 })) {
            h.voice.otherAction()
        }
        h.now = 21_300
        field.wrappedValue = "see you!"
        #expect(draft == "see you!")
        #expect(MacRecordSlot.activation(of: slot, sendGuarded: h.voice.sendIsGuarded) == .send,
                "typing after Stop left the Send guarded")
    }
}

@Suite("Mac menu bar: File ▸ Record Voice Message")
struct MacVoiceMenuTests {

    @Test("enabled on an empty composer, and beside words or staged items — where it records beside them")
    func enabled() {
        #expect(MacVoiceMenu.isEnabled(ComposerSlot.Inputs()))
        #expect(MacVoiceMenu.isEnabled(ComposerSlot.Inputs(draftBlank: false)))
        #expect(MacVoiceMenu.isEnabled(ComposerSlot.Inputs(staged: true)))
    }

    @Test("disabled where S1.3 dims the microphone: a call, a busy composer, a voice message not sent")
    func dimmedRowsDisable() {
        #expect(!MacVoiceMenu.isEnabled(ComposerSlot.Inputs(call: true)))
        #expect(!MacVoiceMenu.isEnabled(ComposerSlot.Inputs(busy: true)))
        #expect(!MacVoiceMenu.isEnabled(ComposerSlot.Inputs(notSent: true)))
        // Beside words too: the paperclip's item meets a call exactly as the
        // microphone does.
        #expect(!MacVoiceMenu.isEnabled(ComposerSlot.Inputs(draftBlank: false, call: true)))
    }

    @Test("disabled in the assistant's chat, while editing, where nothing records, and while the recorder is open")
    func otherRefusals() {
        #expect(!MacVoiceMenu.isEnabled(ComposerSlot.Inputs(assistantChat: true)))
        #expect(!MacVoiceMenu.isEnabled(ComposerSlot.Inputs(editing: true)))
        #expect(!MacVoiceMenu.isEnabled(ComposerSlot.Inputs(canRecord: false)))
        #expect(!MacVoiceMenu.isEnabled(ComposerSlot.Inputs(recorderOpen: true)))
    }

    @Test("during a recording it stays enabled — pressed again it stops into review — whatever else is true")
    func recordingStopsIt() {
        for recording in [ComposerSlot.Recording.handsFree, .handsFreeBesideDraft, .held] {
            #expect(MacVoiceMenu.isEnabled(ComposerSlot.Inputs(recording: recording)), "\(recording)")
            #expect(MacVoiceMenu.isEnabled(ComposerSlot.Inputs(
                recording: recording, editing: true, call: true, busy: true, notSent: true)),
                "a recording could not be stopped from the menu: \(recording)")
        }
        // The video recorder owns the window (Phase 3): not even then.
        #expect(!MacVoiceMenu.isEnabled(ComposerSlot.Inputs(recorderOpen: true, recording: .handsFree)))
    }
}

@MainActor
@Suite("Mac VoiceOver: the microphone waits for 'Recording' to be said")
struct MacSpeechAllowanceTests {

    /// AppKit says when an announcement is ASKED for, never when it has been
    /// said — so the Mac waits a fixed second (S6), and the microphone opens
    /// only after it.
    @Test("asking VoiceOver to say a sentence returns only after a full second")
    func waitsASecond() async {
        #expect(VoiceComposer.macSpeechAllowanceMS == 1_000)
        let clock = ContinuousClock()
        let took = await clock.measure {
            await VoiceComposer.speakAndWait("Recording")
        }
        #expect(took >= .milliseconds(1_000), "the microphone would open after \(took)")
    }
}

@MainActor
@Suite("Mac recording row keeps the bar's height")
struct MacVoiceRecordingRowTests {

    /// The Mac composer's control box.
    private static let control: CGFloat = 24

    private func row(besideDraft: Bool = false, warning: Bool = false) -> MacVoiceRecordingRow {
        MacVoiceRecordingRow(
            elapsed: 42, litBars: 5, besideDraft: besideDraft, warning: warning,
            height: Self.control, onDelete: {}, onStop: {})
    }

    private func size(_ view: some View) throws -> CGSize {
        let renderer = ImageRenderer(content: view)
        renderer.scale = 1
        let image = try #require(renderer.cgImage, "the view did not render")
        return CGSize(width: image.width, height: image.height)
    }

    @Test("one control tall, beside words or not, warned or not", arguments: [
        (false, false), (true, false), (false, true), (true, true),
    ])
    func oneControlTall(besideDraft: Bool, warning: Bool) throws {
        let h = try size(row(besideDraft: besideDraft, warning: warning).frame(width: 360)).height
        #expect(h == Self.control, "the recording row is \(h) points tall")
    }

    @Test("too narrow, it gives way rather than growing")
    func narrow() throws {
        let h = try size(row().frame(width: 120)).height
        #expect(h == Self.control, "a narrow recording row grew to \(h) points")
    }

    /// S2.4: "the level meter goes first; on desktops the text buttons then
    /// become icons with the same labels" — so each fallback must really be
    /// narrower than the one before it, or ViewThatFits never reaches it.
    @Test("each fallback is narrower: first without the meter, then icons for words")
    func fallbacksNarrow() throws {
        let full = try size(row().row(.full).fixedSize()).width
        let withoutMeter = try size(row().row(.withoutMeter).fixedSize()).width
        let icons = try size(row().row(.icons).fixedSize()).width
        #expect(full > withoutMeter, "dropping the meter saved nothing (\(full) → \(withoutMeter))")
        #expect(withoutMeter > icons, "icons for words saved nothing (\(withoutMeter) → \(icons))")
    }

    @Test("'30 seconds left' takes the meter's place, and stays when the meter would go")
    func warningStays() throws {
        let warned = try size(row(warning: true).row(.withoutMeter).fixedSize()).width
        let plain = try size(row(warning: false).row(.withoutMeter).fixedSize()).width
        #expect(warned > plain, "the warning's words were dropped with the meter")
    }
}

#endif
