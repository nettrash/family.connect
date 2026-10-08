//
//  RecordSendButton.swift
//  FamilyConnect
//
//  The composer's trailing slot on iPhone and iPad: Send when there is
//  something to send, the microphone when the composer is empty, the Send
//  arrow or the Stop square while a voice message records (#79,
//  docs/audio-video-messages-2026-10-04.md, S1.3, S6, S8.1, S8.2).
//
//  ONE GESTURE (revised 2026-10-06). The hold-to-talk walkie-talkie is gone:
//  the microphone does one thing, and a tap starts a hands-free recording.
//  The control is a plain `UIControl` whose only action is `.touchUpInside`
//  — UIKit's own completed tap, with its generous "inside" — so a press that
//  is held a long time and lifts inside is still a tap, and nothing at all
//  happens while the finger is down: no recording, no menu, no callout.
//
//  - NO long-press recognizer, and NEVER a context menu: on iOS
//    `.contextMenu` claims the touch long press. An iPad pointer's SECONDARY
//    click opens the microphone's menu through a `UIEditMenuInteraction`
//    presented at the click instead (S8.2) — presented only by that click.
//  - No pointer interaction either: its press recognizer accepts a finger's
//    touch too (measured on iOS 27).
//
//  The glyph is drawn by SwiftUI underneath (`RecordSendSlot`), in the
//  composer's own scaled sizes and tint; this control is transparent, and it
//  is the one accessibility element of the slot — label, hint, value, custom
//  actions, Magic Tap, the escape gesture and Voice Control's input labels
//  all live on it (S6).
//

#if os(iOS)

import SwiftUI
import UIKit

/// What the slot reports. The composer turns these into reducer events.
struct RecordSendEvents {
    /// The slot was activated: a tap that lifted inside (however long it was
    /// held), VoiceOver, Switch Control, Full Keyboard Access. A press going
    /// down or being held reports nothing.
    var activated: () -> Void = {}
    /// The secondary menu's Record Voice Message.
    var recordFromMenu: () -> Void = {}
    /// The secondary menu's Record Video Message, and the accessibility
    /// action "Record video message" (S1.6, S6) — offered only when round
    /// video is available (`RecordSendSlot.offersVideo`).
    var recordVideo: () -> Void = {}
    /// "Stop and listen first" (S6).
    var stopAndListen: () -> Void = {}
    /// "Delete recording" (S6).
    var deleteRecording: () -> Void = {}
    /// Magic Tap; false lets the system have it.
    var magicTap: () -> Bool = { false }
    /// VoiceOver's escape gesture; false lets it go on up.
    var escape: () -> Bool = { false }
    /// Asked the moment a finger, a pen or a pointer goes DOWN on the
    /// control: true when the activation guard runs then. Such a press is
    /// ignored WHOLE, however late it lifts (S1.1) — a slow second tap must
    /// not send the recording the first tap started, nor what a Stop just
    /// staged. It is a question, never an event: nothing reaches the reducer
    /// for a press going down. Android, Windows and the web ask the same at
    /// their press's down.
    var pressIgnored: () -> Bool = { false }
}

/// The slot: the glyph, and the control over it.
struct RecordSendSlot: View {
    let slot: ComposerSlot
    /// Bumped to move VoiceOver's focus to the slot.
    let focusRequest: Int
    /// The composer's scaled control side and glyph size.
    let side: CGFloat
    let glyph: CGFloat
    let events: RecordSendEvents
    /// S1.2's **round available**: the menu's second item and the
    /// accessibility action exist (S1.6).
    var offersVideo = false

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// The control is highlighted — a finger is down on it. Drawn only: it
    /// changes nothing the slot does.
    @State private var isPressed = false

    var body: some View {
        let target = max(side, CGFloat(RecordRules.minTargetApplePT))
        ZStack {
            Image(systemName: symbol)
                .font(.system(size: glyph))
                .foregroundStyle(looksDisabled ? AnyShapeStyle(.tertiary) : AnyShapeStyle(.tint))
                .scaleEffect(isPressed ? 0.88 : 1)
                // Send ↔ microphone is a 150 ms cross-fade, none under Reduce
                // Motion (S1.1, S1.3).
                .id(symbol)
                .transition(.opacity)
                // The control over it is the slot's one accessibility element.
                .accessibilityHidden(true)
        }
        .frame(width: side, height: side)
        .animation(reduceMotion ? nil : .easeInOut(duration: Double(RecordRules.slotCrossfadeMS) / 1000), value: symbol)
        .animation(reduceMotion ? nil : .easeOut(duration: 0.12), value: isPressed)
        .overlay {
            // The hit area grows to 44 points; the glyph and the bar do not
            // (S1.1): the control is laid out at the target and padded back
            // to the visual, so it overhangs into the spacing around it.
            RecordSendControlView(
                slot: slot,
                offersVideo: offersVideo,
                focusRequest: focusRequest,
                events: events,
                highlighted: { isPressed = $0 })
                .frame(width: target, height: target)
                .padding(-(target - side) / 2)
        }
    }

    private var symbol: String {
        switch slot {
        case .stopRecording: "stop.circle.fill"
        case .microphone, .dimmed: "mic.circle.fill"
        case .recorder, .sendVoice, .save, .send, .sendDisabled: "arrow.up.circle.fill"
        }
    }

    /// Disabled rows, and dimmed ones — which LOOK disabled but are not.
    private var looksDisabled: Bool {
        switch slot {
        case .sendDisabled, .save(enabled: false), .dimmed: true
        default: false
        }
    }
}

/// The representable around the control.
private struct RecordSendControlView: UIViewRepresentable {
    let slot: ComposerSlot
    let offersVideo: Bool
    let focusRequest: Int
    let events: RecordSendEvents
    let highlighted: (Bool) -> Void

    func makeUIView(context: Context) -> RecordSendControl {
        let control = RecordSendControl()
        control.update(slot: slot, offersVideo: offersVideo, events: events)
        control.highlightChanged = highlighted
        context.coordinator.focusRequest = focusRequest
        return control
    }

    func updateUIView(_ control: RecordSendControl, context: Context) {
        control.update(slot: slot, offersVideo: offersVideo, events: events)
        control.highlightChanged = highlighted
        if context.coordinator.focusRequest != focusRequest {
            context.coordinator.focusRequest = focusRequest
            // After this update lands, so the label VoiceOver reads is the
            // recording's ("Send voice message"), not the microphone's.
            DispatchQueue.main.async {
                UIAccessibility.post(notification: .layoutChanged, argument: control)
            }
        }
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    final class Coordinator {
        var focusRequest = 0
    }
}

/// The control itself: the tap, the secondary click, and the slot's
/// accessibility.
final class RecordSendControl: UIControl, UIEditMenuInteractionDelegate {

    private(set) var slot: ComposerSlot = .send
    /// Round video is available: Record Video Message in the menu, and the
    /// accessibility action (S1.6).
    private(set) var offersVideo = false
    private var events = RecordSendEvents()
    /// Told when the control's highlight changes, for the glyph's press.
    var highlightChanged: (Bool) -> Void = { _ in }

    /// An iPad pointer's secondary click — the control's ONLY gesture
    /// recognizer. Internal rather than private so a test can read how it
    /// is configured: S8.2's numbers are the whole of what makes it right.
    let secondaryClick = UITapGestureRecognizer()
    private var editMenu: UIEditMenuInteraction?

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .clear
        isAccessibilityElement = true

        // The one action: UIKit's completed tap. Nothing for touch-down,
        // nothing while it is held.
        addTarget(self, action: #selector(tapped), for: .touchUpInside)

        // An iPad pointer's secondary click opens the menu (S1.6, S8.2).
        secondaryClick.buttonMaskRequired = .secondary
        secondaryClick.allowedTouchTypes = [NSNumber(value: UITouch.TouchType.indirectPointer.rawValue)]
        secondaryClick.addTarget(self, action: #selector(secondaryClicked(_:)))
        addGestureRecognizer(secondaryClick)

        let menu = UIEditMenuInteraction(delegate: self)
        addInteraction(menu)
        editMenu = menu
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    func update(slot: ComposerSlot, offersVideo: Bool = false, events: RecordSendEvents) {
        self.events = events
        guard slot != self.slot || offersVideo != self.offersVideo || accessibilityLabel == nil else { return }
        self.slot = slot
        self.offersVideo = offersVideo
        applyAccessibility()
        toolTip = Self.toolTip(for: slot)
    }

    /// An iPad pointer's tooltip (S7.1): "Record a voice message" on the
    /// microphone, the label otherwise. iPhone shows none.
    static func toolTip(for slot: ComposerSlot) -> String? {
        slot.isMicrophone ? String(localized: "Record a voice message") : slot.label
    }

    override var isHighlighted: Bool {
        didSet {
            if isHighlighted != oldValue { highlightChanged(isHighlighted) }
        }
    }

    // MARK: - The tap

    /// The press now down went down inside the activation guard (S1.1), so
    /// its lift is no tap. Decided at the down, kept until the lift.
    private var pressWentDownGuarded = false

    override func beginTracking(_ touch: UITouch, with event: UIEvent?) -> Bool {
        // A secondary click is the menu's, never a tap that records.
        if let event, event.buttonMask.contains(.secondary) { return false }
        // Whether this press can BE a tap is decided now, as it goes down
        // (S1.1). Nothing else is decided while it is down.
        pressWentDownGuarded = events.pressIgnored()
        return super.beginTracking(touch, with: event)
    }

    override func cancelTracking(with event: UIEvent?) {
        pressWentDownGuarded = false
        super.cancelTracking(with: event)
    }

    /// A press that lifted inside, however long it was held — unless it went
    /// down inside the activation guard. The composer decides what each slot
    /// does with it, as it does for VoiceOver's activation.
    @objc private func tapped() {
        let guarded = pressWentDownGuarded
        pressWentDownGuarded = false
        guard !guarded else { return }
        events.activated()
    }

    // MARK: - The secondary click (iPad)

    @objc private func secondaryClicked(_ recognizer: UITapGestureRecognizer) {
        guard slot.isMicrophone else { return }
        let point = recognizer.location(in: self)
        editMenu?.presentEditMenu(with: UIEditMenuConfiguration(identifier: nil, sourcePoint: point))
    }

    func editMenuInteraction(
        _ interaction: UIEditMenuInteraction,
        menuFor configuration: UIEditMenuConfiguration,
        suggestedActions: [UIMenuElement]
    ) -> UIMenu? {
        guard slot.isMicrophone else { return nil }
        let record = UIAction(
            title: String(localized: "Record Voice Message"),
            image: UIImage(systemName: "mic")
        ) { [weak self] _ in
            self?.events.recordFromMenu()
        }
        // Record Video Message, only where round video is available
        // (S1.6). In rows 7–8 both items explain; in row 9 this one opens
        // the recorder — the composer decides, as the video button does.
        guard offersVideo else { return UIMenu(children: [record]) }
        let video = UIAction(
            title: String(localized: "Record Video Message"),
            image: UIImage(systemName: "video.circle")
        ) { [weak self] _ in
            self?.events.recordVideo()
        }
        return UIMenu(children: [record, video])
    }

    // MARK: - Accessibility (S6)

    private func applyAccessibility() {
        accessibilityLabel = slot.label
        accessibilityTraits = .button
        accessibilityHint = nil
        accessibilityValue = nil
        accessibilityUserInputLabels = nil
        accessibilityCustomActions = nil
        switch slot {
        case .microphone:
            accessibilityHint = String(localized: "Starts recording.")
            accessibilityUserInputLabels = microphoneInputLabels
            accessibilityCustomActions = videoActions
        case .dimmed(let reason):
            // Dimmed, not disabled: still activatable, and the reason is its
            // value — the same sentence the notice line says.
            accessibilityValue = reason.notice
            accessibilityUserInputLabels = microphoneInputLabels
            accessibilityCustomActions = videoActions
        case .sendVoice:
            accessibilityCustomActions = [
                UIAccessibilityCustomAction(name: String(localized: "Stop and listen first")) { [weak self] _ in
                    self?.events.stopAndListen()
                    return true
                },
                deleteAction,
            ]
        case .stopRecording:
            accessibilityCustomActions = [deleteAction]
        case .sendDisabled, .save(enabled: false):
            accessibilityTraits = [.button, .notEnabled]
        case .recorder, .save, .send:
            break
        }
    }

    /// "Record video message" on the microphone, when round video is
    /// available (S1.6, S6).
    private var videoActions: [UIAccessibilityCustomAction]? {
        guard offersVideo else { return nil }
        return [
            UIAccessibilityCustomAction(name: String(localized: "Record video message")) { [weak self] _ in
                self?.events.recordVideo()
                return true
            },
        ]
    }

    private var deleteAction: UIAccessibilityCustomAction {
        UIAccessibilityCustomAction(name: String(localized: "Delete recording")) { [weak self] _ in
            self?.events.deleteRecording()
            return true
        }
    }

    /// Voice Control: the label, and the three words S6 adds.
    private var microphoneInputLabels: [String] {
        [
            String(localized: "Record voice message"),
            String(localized: "Microphone"),
            String(localized: "Record"),
            String(localized: "Voice message"),
        ]
    }

    /// VoiceOver's activation is the same activation as a tap.
    override func accessibilityActivate() -> Bool {
        switch slot {
        case .sendDisabled, .save(enabled: false), .recorder:
            return false
        default:
            events.activated()
            return true
        }
    }

    override func accessibilityPerformMagicTap() -> Bool {
        events.magicTap()
    }

    override func accessibilityPerformEscape() -> Bool {
        events.escape()
    }
}

#endif
