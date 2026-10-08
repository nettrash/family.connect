//
//  RecordSendButtonTests.swift
//  FamilyConnectTests
//
//  #79, Phase 1: the slot's UIKit control (docs/audio-video-messages-2026-10-04.md,
//  S1.3, S1.6, S6, S8.1, S8.2). A touch cannot be made in a unit test, but
//  everything a touch meets can be read and driven: that the control answers
//  ONLY a completed tap — no long-press recognizer, no context menu, nothing
//  for a press going down or being held (revised 2026-10-06: the hold is
//  gone) — the secondary click's button mask, the menu a secondary click
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
@Suite("The slot's control: one tap, the menu, VoiceOver")
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
        control.update(slot: slot, events: (spy ?? Spy()).events)
        return control
    }

    // MARK: - One tap, and no long press

    private static let everySlot: [ComposerSlot] = [
        .microphone, .dimmed(.call), .dimmed(.busy), .dimmed(.notSent), .send, .sendVoice,
        .stopRecording, .save(enabled: true), .save(enabled: false), .sendDisabled, .recorder,
    ]

    @Test("no long press of any kind: no long-press recognizer, no context menu — the pointer's secondary click is the only recognizer", arguments: everySlot)
    func noLongPress(slot: ComposerSlot) {
        let button = control(slot)
        let recognizers = button.gestureRecognizers ?? []
        #expect(!recognizers.contains { $0 is UILongPressGestureRecognizer },
                "\(slot): a long-press recognizer is back on the slot")
        #expect(recognizers.contains { $0 === button.secondaryClick })
        // The edit menu brings a private bridge of its own — a context-menu
        // interaction subclass driven by the SECONDARY CLICK, and recognizers
        // that only order others (measured on the iOS 26 SDK). Nothing else:
        // no context menu of OURS, which on iOS claims the touch long press,
        // and no recognizer a finger reaches that is not one of the bridge's.
        #expect(!button.interactions.contains { type(of: $0) == UIContextMenuInteraction.self },
                "\(slot): a context menu would claim the touch long press")
        #expect(!button.isContextMenuInteractionEnabled)
        let finger = NSNumber(value: UITouch.TouchType.direct.rawValue)
        let harmless: Set<String> = [
            "_UISecondaryClickDriverGestureRecognizer", "_UIRelationshipGestureRecognizer",
        ]
        for recognizer in recognizers where recognizer !== button.secondaryClick
            && recognizer.allowedTouchTypes.contains(finger) {
            let name = String(describing: type(of: recognizer))
            #expect(harmless.contains(name), "\(slot): a finger can reach \(name)")
        }
        // A finger never reaches the menu's recognizer.
        let types = Set(button.secondaryClick.allowedTouchTypes.map(\.intValue))
        #expect(!types.contains(UITouch.TouchType.direct.rawValue))
        #expect(!types.contains(UITouch.TouchType.pencil.rawValue))
    }

    @Test("the control's one action is the completed tap: nothing for touch-down, a drag, a cancel or a lift outside")
    func onlyTheCompletedTap() {
        let button = control(.microphone)
        #expect(button.allControlEvents == .touchUpInside,
                "the control listens to more than a completed tap: \(button.allControlEvents.rawValue)")
    }

    @Test("a long press on the microphone starts nothing while the finger is down, and is one tap when it lifts inside")
    func longPressIsATap() {
        let spy = Spy()
        let mic = control(.microphone, spy: spy)
        // The finger goes down, wanders and stays — far past the old 0.5 s
        // hold threshold. Nothing may record and nothing may open.
        mic.sendActions(for: .touchDown)
        mic.sendActions(for: .touchDragInside)
        RunLoop.main.run(until: Date().addingTimeInterval(0.7))
        mic.sendActions(for: .touchDragExit)
        mic.sendActions(for: .touchDragEnter)
        mic.sendActions(for: .touchDownRepeat)
        #expect(spy.activated == 0, "a press going down or being held started something")
        #expect(spy.fromMenu == 0, "a long press opened the microphone's menu")
        // It lifts inside: the button's ordinary tap — a slow or unsteady
        // press is never a dead button.
        mic.sendActions(for: .touchUpInside)
        #expect(spy.activated == 1)

        // Lifted outside, or taken away by the system: nothing at all.
        mic.sendActions(for: .touchDown)
        mic.sendActions(for: .touchUpOutside)
        mic.sendActions(for: .touchDown)
        mic.sendActions(for: .touchCancel)
        #expect(spy.activated == 1, "a press that did not lift inside activated the slot")
    }

    @Test("a press that goes down inside the activation guard is ignored whole, however late it lifts (S1.1)")
    func aPressDownInsideTheGuardIsIgnoredWhole() {
        let spy = Spy()
        var guardRuns = true
        var asked = 0
        var events = spy.events
        events.pressIgnored = {
            asked += 1
            return guardRuns
        }
        let mic = RecordSendControl(frame: CGRect(x: 0, y: 0, width: 44, height: 44))
        mic.update(slot: .microphone, events: events)

        // Down while the guard runs; it ends while the finger is still down,
        // and the press lifts inside: no tap — Android, Windows and the web
        // ask at the down too.
        #expect(mic.beginTracking(UITouch(), with: nil))
        #expect(asked == 1, "the guard is asked as the press goes down")
        guardRuns = false
        mic.sendActions(for: .touchUpInside)
        #expect(spy.activated == 0, "a press that went down inside the guard became a tap")

        // The next press, down after the guard: an ordinary tap.
        #expect(mic.beginTracking(UITouch(), with: nil))
        mic.sendActions(for: .touchUpInside)
        #expect(spy.activated == 1)

        // A guarded press the system takes away leaves nothing behind.
        guardRuns = true
        #expect(mic.beginTracking(UITouch(), with: nil))
        mic.cancelTracking(with: nil)
        guardRuns = false
        mic.sendActions(for: .touchUpInside)
        #expect(spy.activated == 2)

        // VoiceOver's activation has no press: the composer's reducer asks
        // the guard at the activation itself.
        guardRuns = true
        #expect(mic.accessibilityActivate())
        #expect(spy.activated == 3)
        #expect(asked == 3, "a screen reader's activation asked the press guard")
    }

    @Test("a pointer's secondary click is the menu's, and the menu is Record Voice Message")
    func secondaryClickMenu() throws {
        let spy = Spy()
        let mic = control(.microphone, spy: spy)
        #expect(mic.secondaryClick.buttonMaskRequired == .secondary)
        #expect(mic.secondaryClick.allowedTouchTypes.map(\.intValue) == [UITouch.TouchType.indirectPointer.rawValue])

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
        button.update(slot: .send, events: RecordSendEvents())
        #expect(button.accessibilityLabel == String(localized: "Send"))
        #expect(button.accessibilityHint == nil, "Send kept the microphone's hint")
        #expect((button.accessibilityUserInputLabels ?? []).isEmpty, "Send kept the microphone's spoken names")
    }
}

#endif
