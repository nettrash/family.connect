//
//  RecordSendButton.swift
//  FamilyConnect
//
//  The composer's trailing slot on iPhone and iPad: Send when there is
//  something to send, the microphone when the composer is empty, the Send
//  arrow or the Stop square while a voice message records (#79,
//  docs/audio-video-messages-2026-10-04.md, S1.3, S2.3, S6, S8.1, S8.2).
//
//  WHY UIKIT. The hold needs a touch that can SLIDE — 100 points toward the
//  leading edge to cancel, 60 up to lock — and SwiftUI's long press fails
//  after 10 points, so it cannot drive one (Checked facts). And a press that
//  wanders past the slop and lifts inside the button must still be a tap, the
//  way any UIKit button's is. So the control is a `UIControl` in a
//  `UIViewRepresentable`:
//
//  - a `UILongPressGestureRecognizer` (0.5 s, `allowableMovement` 20, finger
//    and Pencil only) is the hold, and its own timer is the reducer's tick at
//    H; once it begins it owns the touch, reporting the slide and the lift;
//  - the control's own `.touchUpInside` / `.touchUpOutside` is the tap, with
//    UIKit's generous "inside" — and a hold that begins cancels it, because
//    a recognizer that recognises cancels the view's touches;
//  - NEVER a context menu: on iOS `.contextMenu` claims the touch long press.
//    An iPad pointer's SECONDARY click opens the microphone's menu through a
//    `UIEditMenuInteraction` presented at the click instead (S8.2).
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
    /// A press went down on the microphone, in window coordinates.
    var pressDown: (_ x: Double, _ y: Double, _ canHold: Bool) -> Void = { _, _, _ in }
    var pressMoved: (_ x: Double, _ y: Double) -> Void = { _, _ in }
    /// The press lifted; `inside` is UIKit's own touch-up-inside.
    var pressLifted: (_ x: Double, _ y: Double, _ inside: Bool, _ byTouch: Bool) -> Void = { _, _, _, _ in }
    /// The system cancelled the press.
    var pressCancelled: () -> Void = {}
    /// The long press reached H.
    var holdReached: () -> Void = {}
    /// The slot was activated, but not by a press on the microphone — a tap
    /// on Send or Stop, VoiceOver, Switch Control, Full Keyboard Access.
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
}

/// The slot: the glyph, and the control over it.
struct RecordSendSlot: View {
    let slot: ComposerSlot
    /// A finger is down on the microphone, before H.
    let isPressed: Bool
    /// Bumped to move VoiceOver's focus to the slot.
    let focusRequest: Int
    /// The composer's scaled control side and glyph size.
    let side: CGFloat
    let glyph: CGFloat
    let events: RecordSendEvents
    /// S1.2's **round available**: the menu's second item and the
    /// accessibility action exist (S1.6).
    var offersVideo = false

    @Environment(\.layoutDirection) private var layoutDirection
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let target = max(side, CGFloat(RecordRules.minTargetApplePT))
        ZStack {
            Image(systemName: symbol)
                .font(.system(size: glyph))
                .foregroundStyle(looksDisabled ? AnyShapeStyle(.tertiary) : AnyShapeStyle(.tint))
                .scaleEffect(isPressed || slot == .heldMicrophone ? 0.88 : 1)
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
                rtl: layoutDirection == .rightToLeft,
                focusRequest: focusRequest,
                events: events)
                .frame(width: target, height: target)
                .padding(-(target - side) / 2)
        }
    }

    private var symbol: String {
        switch slot {
        case .stopRecording: "stop.circle.fill"
        case .microphone, .dimmed, .heldMicrophone: "mic.circle.fill"
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
    let rtl: Bool
    let focusRequest: Int
    let events: RecordSendEvents

    func makeUIView(context: Context) -> RecordSendControl {
        let control = RecordSendControl()
        control.update(slot: slot, offersVideo: offersVideo, rtl: rtl, events: events)
        context.coordinator.focusRequest = focusRequest
        return control
    }

    func updateUIView(_ control: RecordSendControl, context: Context) {
        control.update(slot: slot, offersVideo: offersVideo, rtl: rtl, events: events)
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

/// The control itself: touches, the hold, the secondary click, and the
/// slot's accessibility.
final class RecordSendControl: UIControl, UIGestureRecognizerDelegate, UIEditMenuInteractionDelegate {

    private(set) var slot: ComposerSlot = .send
    /// Round video is available: Record Video Message in the menu, and the
    /// accessibility action (S1.6).
    private(set) var offersVideo = false
    private var rtl = false
    private var events = RecordSendEvents()

    /// Whether the press being tracked went down on the microphone — decided
    /// once, at touch-down, so a slot that changes under a finger does not
    /// turn half a press into something else.
    private var pressIsMicrophone = false
    private var pressByTouch = false
    private var pressTracked = false

    /// The hold, and an iPad pointer's secondary click. Internal rather
    /// than private so a test can read how they are configured — the
    /// numbers S8.1 and S8.2 name are the whole of what makes them right.
    let longPress = UILongPressGestureRecognizer()
    let secondaryClick = UITapGestureRecognizer()
    private var editMenu: UIEditMenuInteraction?

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .clear
        isAccessibilityElement = true

        addTarget(self, action: #selector(upInside(_:event:)), for: .touchUpInside)
        addTarget(self, action: #selector(upOutside(_:event:)), for: .touchUpOutside)
        addTarget(self, action: #selector(touchCancelled), for: .touchCancel)

        // The hold: finger and Pencil, never a pointer — a click of any
        // length is a tap (S8.2).
        longPress.minimumPressDuration = Double(VoiceComposer.systemLongPressMS) / 1000
        longPress.allowableMovement = CGFloat(RecordRules.tapSlop)
        longPress.allowedTouchTypes = [
            NSNumber(value: UITouch.TouchType.direct.rawValue),
            NSNumber(value: UITouch.TouchType.pencil.rawValue),
        ]
        longPress.addTarget(self, action: #selector(held(_:)))
        longPress.delegate = self
        addGestureRecognizer(longPress)

        // An iPad pointer's secondary click opens the menu (S1.6, S8.2).
        secondaryClick.buttonMaskRequired = .secondary
        secondaryClick.allowedTouchTypes = [NSNumber(value: UITouch.TouchType.indirectPointer.rawValue)]
        secondaryClick.addTarget(self, action: #selector(secondaryClicked(_:)))
        addGestureRecognizer(secondaryClick)

        let menu = UIEditMenuInteraction(delegate: self)
        addInteraction(menu)
        editMenu = menu
        // No pointer interaction: its press recognizer accepts a finger's
        // touch too (measured on iOS 27), and nothing but the hold may meet
        // a finger's long press here.
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    func update(slot: ComposerSlot, offersVideo: Bool = false, rtl: Bool, events: RecordSendEvents) {
        self.rtl = rtl
        self.events = events
        guard slot != self.slot || offersVideo != self.offersVideo || accessibilityLabel == nil else { return }
        self.slot = slot
        self.offersVideo = offersVideo
        // The hold exists only on the microphone — and while a held one
        // records, so the finger's slide and lift keep reaching it.
        longPress.isEnabled = slot.isMicrophone || slot == .heldMicrophone || longPressIsActive
        applyAccessibility()
        toolTip = Self.toolTip(for: slot)
    }

    /// An iPad pointer's tooltip (S7.1): "Record a voice message" on the
    /// microphone, the label otherwise. iPhone shows none.
    static func toolTip(for slot: ComposerSlot) -> String? {
        slot.isMicrophone ? String(localized: "Record a voice message") : slot.label
    }

    private var longPressIsActive: Bool {
        longPress.state == .began || longPress.state == .changed
    }

    // MARK: - The tap

    override func beginTracking(_ touch: UITouch, with event: UIEvent?) -> Bool {
        // A secondary click is the menu's, never a tap that records.
        if let event, event.buttonMask.contains(.secondary) { return false }
        pressTracked = true
        pressIsMicrophone = slot.isMicrophone
        pressByTouch = touch.type == .direct || touch.type == .pencil
        if pressIsMicrophone {
            let point = touch.location(in: nil)
            events.pressDown(Double(point.x), Double(point.y), pressByTouch)
        }
        return true
    }

    override func continueTracking(_ touch: UITouch, with event: UIEvent?) -> Bool {
        if pressIsMicrophone, !longPressIsActive {
            let point = touch.location(in: nil)
            events.pressMoved(Double(point.x), Double(point.y))
        }
        return true
    }

    @objc private func upInside(_ sender: UIControl, event: UIEvent) {
        lifted(event: event, inside: true)
    }

    @objc private func upOutside(_ sender: UIControl, event: UIEvent) {
        lifted(event: event, inside: false)
    }

    private func lifted(event: UIEvent, inside: Bool) {
        guard pressTracked else { return }
        pressTracked = false
        if pressIsMicrophone {
            let point = event.allTouches?.first?.location(in: nil) ?? .zero
            events.pressLifted(Double(point.x), Double(point.y), inside, pressByTouch)
        } else if inside {
            events.activated()
        }
    }

    @objc private func touchCancelled() {
        guard pressTracked else { return }
        pressTracked = false
        // A hold that begins cancels the tap's touches: that is the hold
        // taking over, not the system taking the touch away.
        guard pressIsMicrophone, !longPressIsActive else { return }
        events.pressCancelled()
    }

    // MARK: - The hold

    @objc private func held(_ recognizer: UILongPressGestureRecognizer) {
        let point = recognizer.location(in: nil)
        switch recognizer.state {
        case .began:
            pressTracked = false
            events.holdReached()
        case .changed:
            events.pressMoved(Double(point.x), Double(point.y))
        case .ended:
            events.pressLifted(Double(point.x), Double(point.y), bounds.contains(recognizer.location(in: self)), true)
            longPress.isEnabled = slot.isMicrophone
        case .cancelled, .failed:
            if recognizer.state == .cancelled { events.pressCancelled() }
            longPress.isEnabled = slot.isMicrophone
        default:
            break
        }
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
    ) -> Bool {
        false
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
        case .sendVoice, .heldMicrophone:
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

    /// VoiceOver's activation is an activation, never a touch: the reducer
    /// hears `activate`, so a screen reader can neither hold nor slide by
    /// accident.
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
