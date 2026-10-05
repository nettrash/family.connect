//
//  RecordSendButtonTests.swift
//  FamilyConnectTests
//
//  #79, Phase 1: the slot's UIKit control (docs/audio-video-messages-2026-10-04.md,
//  S1.3, S1.6, S6, S8.1, S8.2). A touch cannot be made in a unit test, but
//  everything a touch meets can be read: the hold's recognizer and its three
//  numbers, the secondary click's button mask, the menu a secondary click
//  opens, and the whole accessibility surface in every state — the label,
//  the hint, the dimmed reason as the value, the custom actions, Voice
//  Control's words, and what activation, Magic Tap and the escape gesture do.
//

#if os(iOS)

import Foundation
import Testing
import UIKit
@testable import FamilyConnect

@MainActor
@Suite("The slot's control: the hold, the menu, VoiceOver")
struct RecordSendButtonTests {

    @MainActor
    final class Spy {
        var activated = 0
        var fromMenu = 0
        var stoppedToListen = 0
        var deleted = 0
        var magicTaps = 0
        var escapes = 0
        var magicTapAnswer = false
        var escapeAnswer = false

        var events: RecordSendEvents {
            var events = RecordSendEvents()
            events.activated = { [weak self] in self?.activated += 1 }
            events.recordFromMenu = { [weak self] in self?.fromMenu += 1 }
            events.stopAndListen = { [weak self] in self?.stoppedToListen += 1 }
            events.deleteRecording = { [weak self] in self?.deleted += 1 }
            events.magicTap = { [weak self] in
                self?.magicTaps += 1
                return self?.magicTapAnswer ?? false
            }
            events.escape = { [weak self] in
                self?.escapes += 1
                return self?.escapeAnswer ?? false
            }
            return events
        }
    }

    private func control(_ slot: ComposerSlot, spy: Spy? = nil) -> RecordSendControl {
        let control = RecordSendControl(frame: CGRect(x: 0, y: 0, width: 44, height: 44))
        control.update(slot: slot, rtl: false, events: (spy ?? Spy()).events)
        return control
    }

    // MARK: - The hold and the click

    @Test("the hold is 0.5 s within 20 points, for a finger or a Pencil — never a pointer")
    func holdRecognizer() {
        let hold = control(.microphone).longPress
        #expect(hold.minimumPressDuration == 0.5)
        #expect(hold.allowableMovement == 20)
        let types = Set(hold.allowedTouchTypes.map(\.intValue))
        #expect(types == [UITouch.TouchType.direct.rawValue, UITouch.TouchType.pencil.rawValue])
        #expect(!types.contains(UITouch.TouchType.indirectPointer.rawValue))
    }

    @Test("only a microphone can be held", arguments: [
        (ComposerSlot.microphone, true), (.dimmed(.call), true), (.dimmed(.notSent), true),
        (.send, false), (.sendVoice, false), (.stopRecording, false), (.save(enabled: true), false),
        (.sendDisabled, false),
    ])
    func holdOnlyOnTheMicrophone(slot: ComposerSlot, holds: Bool) {
        #expect(control(slot).longPress.isEnabled == holds, "\(slot)")
    }

    @Test("a pointer's secondary click is the menu's, and the menu is Record Voice Message")
    func secondaryClickMenu() throws {
        let spy = Spy()
        let mic = control(.microphone, spy: spy)
        #expect(mic.secondaryClick.buttonMaskRequired == .secondary)
        #expect(mic.secondaryClick.allowedTouchTypes.map(\.intValue) == [UITouch.TouchType.indirectPointer.rawValue])
        // No context menu of ours: on iOS it claims the touch long press. (The
        // edit menu brings a private bridge of its own, which drives the
        // secondary click and nothing a finger does — so the only recognizer
        // a finger's long press can meet is the hold's.)
        #expect(!mic.interactions.contains { type(of: $0) == UIContextMenuInteraction.self })
        #expect(!mic.isContextMenuInteractionEnabled)
        let fingerLongPresses = (mic.gestureRecognizers ?? []).filter {
            $0 is UILongPressGestureRecognizer
                && $0.allowedTouchTypes.contains(NSNumber(value: UITouch.TouchType.direct.rawValue))
        }
        #expect(fingerLongPresses.count == 1 && fingerLongPresses.first === mic.longPress,
                "something besides the hold answers a finger's long press: \(fingerLongPresses.map { type(of: $0) })")

        let interaction = try #require(mic.interactions.compactMap { $0 as? UIEditMenuInteraction }.first)
        let menu = try #require(mic.editMenuInteraction(
            interaction, menuFor: UIEditMenuConfiguration(identifier: nil, sourcePoint: .zero), suggestedActions: []))
        let items = menu.children.compactMap { $0 as? UIAction }
        #expect(items.map(\.title) == [String(localized: "Record Voice Message")])

        let send = control(.send)
        #expect(send.editMenuInteraction(
            interaction, menuFor: UIEditMenuConfiguration(identifier: nil, sourcePoint: .zero),
            suggestedActions: []) == nil, "Send opened the microphone's menu")
    }

    // MARK: - VoiceOver (S6)

    @Test("the microphone: its label, 'Starts recording.', and Voice Control's three words")
    func microphoneAccessibility() {
        let mic = control(.microphone)
        #expect(mic.isAccessibilityElement)
        #expect(mic.accessibilityLabel == String(localized: "Record voice message"))
        #expect(mic.accessibilityHint == String(localized: "Starts recording."))
        #expect(mic.accessibilityValue == nil)
        #expect(mic.accessibilityTraits.contains(.button))
        #expect(!mic.accessibilityTraits.contains(.notEnabled), "the microphone read as disabled")
        #expect(mic.accessibilityUserInputLabels == [
            String(localized: "Record voice message"), String(localized: "Microphone"),
            String(localized: "Record"), String(localized: "Voice message"),
        ])
        // An iPad pointer's tooltip (S7.1); iPhone has no tooltips at all.
        #expect(RecordSendControl.toolTip(for: .microphone) == String(localized: "Record a voice message"))
        #expect(RecordSendControl.toolTip(for: .dimmed(.call)) == String(localized: "Record a voice message"))
        #expect(RecordSendControl.toolTip(for: .send) == String(localized: "Send"))
    }

    @Test("dimmed is not disabled: the reason is the value, and it stays activatable", arguments: [
        ComposerSlot.Dimmed.call, .busy, .notSent,
    ])
    func dimmedAccessibility(reason: ComposerSlot.Dimmed) {
        let spy = Spy()
        let mic = control(.dimmed(reason), spy: spy)
        #expect(mic.accessibilityLabel == String(localized: "Record voice message"))
        #expect(mic.accessibilityValue == reason.notice)
        #expect(!mic.accessibilityTraits.contains(.notEnabled))
        #expect(mic.accessibilityActivate())
        #expect(spy.activated == 1, "a dimmed microphone did not say why when activated")
    }

    @Test("while it records: Send voice message, with 'Stop and listen first' and 'Delete recording'")
    func recordingAccessibility() throws {
        let spy = Spy()
        let send = control(.sendVoice, spy: spy)
        #expect(send.accessibilityLabel == String(localized: "Send voice message"))
        let actions = try #require(send.accessibilityCustomActions)
        #expect(actions.map(\.name) == [String(localized: "Stop and listen first"), String(localized: "Delete recording")])
        _ = actions[0].actionHandler?(actions[0])
        _ = actions[1].actionHandler?(actions[1])
        #expect(spy.stoppedToListen == 1)
        #expect(spy.deleted == 1)

        let stop = control(.stopRecording)
        #expect(stop.accessibilityLabel == String(localized: "Stop recording"))
        #expect(stop.accessibilityCustomActions?.map(\.name) == [String(localized: "Delete recording")],
                "the Stop square offered to stop as an action besides itself")
    }

    @Test("a disabled Send reads as disabled and does nothing")
    func disabledSend() {
        let spy = Spy()
        for slot in [ComposerSlot.sendDisabled, .save(enabled: false)] {
            let button = control(slot, spy: spy)
            #expect(button.accessibilityTraits.contains(.notEnabled), "\(slot)")
            #expect(!button.accessibilityActivate())
        }
        #expect(spy.activated == 0)
    }

    @Test("a screen reader's activation is an activation, never a touch")
    func activationIsNotATouch() {
        let spy = Spy()
        for slot in [ComposerSlot.microphone, .send, .sendVoice, .stopRecording, .save(enabled: true)] {
            #expect(control(slot, spy: spy).accessibilityActivate(), "\(slot)")
        }
        #expect(spy.activated == 5)
    }

    @Test("Magic Tap and the escape gesture are the composer's to take or decline")
    func magicTapAndEscape() {
        let spy = Spy()
        let mic = control(.microphone, spy: spy)
        #expect(!mic.accessibilityPerformMagicTap(), "Magic Tap was taken with nothing to do")
        #expect(!mic.accessibilityPerformEscape())
        spy.magicTapAnswer = true
        spy.escapeAnswer = true
        #expect(mic.accessibilityPerformMagicTap())
        #expect(mic.accessibilityPerformEscape())
        #expect(spy.magicTaps == 2)
        #expect(spy.escapes == 2)
    }

    @Test("its label follows the slot as it changes")
    func labelFollows() {
        let button = control(.microphone)
        button.update(slot: .send, rtl: false, events: RecordSendEvents())
        #expect(button.accessibilityLabel == String(localized: "Send"))
        #expect(button.accessibilityHint == nil, "Send kept the microphone's hint")
        #expect((button.accessibilityUserInputLabels ?? []).isEmpty, "Send kept the microphone's spoken names")
        #expect(!button.longPress.isEnabled)
    }
}

#endif
