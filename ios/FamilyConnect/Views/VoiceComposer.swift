//
//  VoiceComposer.swift
//  FamilyConnect
//
//  The voice half of a composer (#79, Phase 1 —
//  docs/audio-video-messages-2026-10-04.md, S2): the reducer's state, the
//  recorder, the Undo window and the clock it runs on, and the turning of
//  each `RecordGesture.HoldEffect` into the thing it names.
//
//  WHERE THE LINE IS. `RecordGesture.step` decides — a pure function held to
//  the shared vectors. This object DOES: it opens and closes the microphone,
//  writes the crash-safe "sending" entry a release leaves in the parked store,
//  arms the timer that ends the Undo window, speaks, and buzzes. What only the
//  composer knows — its reply, its staged items, its notice line, the
//  coordinator that queues a send — it is asked through `Hooks`, closures the
//  composer sets once. So the whole voice flow can be driven in a test with a
//  fake recorder, a scratch store and a clock that moves only when told to
//  (VoiceComposerTests), which is the only way the five-second window, a
//  crash inside it and the interruptions that end it can be checked at all.
//
//  Shared, with no `#if os` around the type: the Mac's Phase 1 asks the same
//  questions with a click where the phone has a finger, and must not grow a
//  second copy of the flow.
//
//  THE RECORDING'S ONE WAY OUT. Every effect that ends a recording takes it
//  through `takeRecording`, which also gives the microphone back to the
//  arbiter. A recording the recorder itself already stopped (the cap, an
//  interruption, a failure) or that "Delete this recording?" stopped is held
//  until its effect arrives, so the effect — not the event that caused it —
//  decides where it goes. Anything the reducer had no use for is kept as "not
//  sent": a recording cannot be made again (S2.8).
//
//  WHAT IS SAID IS WHAT HAPPENED. The reducer follows `.send` and `.undoSend`
//  with "Voice message sent" and the success haptic, and `.review` with
//  "Ready to review" — it cannot know that the outbox refused the note or
//  that the recorder kept nothing. This object does, so an effect that moves
//  a recording reports a `Miss`, and the words, haptic, hint and lesson the
//  reducer placed after it are replaced by what really happened: "That
//  recording was too short." when nothing was kept, "Couldn't send that —
//  try again." when the outbox would not take it. A screen reader must never
//  hear "sent" about a note still on the device (S2.5, S6; WCAG 4.1.3).
//

import AVFoundation
import Foundation
import Observation
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

/// A timer the composer can cancel; a test's fires when the test says.
@MainActor
protocol VoiceTimer: AnyObject {
    func cancel()
}

@MainActor
@Observable
final class VoiceComposer {

    // MARK: - What the composer draws

    /// The reducer's state — the phase, the guard and the Undo note.
    private(set) var state = RecordGesture.HoldState()

    /// The released note waiting out its Undo window, and where it is kept.
    private(set) var undoNote: UndoNote?

    /// "Still recording. Tap Send when you're done." in the row, for 3 s.
    private(set) var showsStillRecording = false

    /// The haptic to play next — a serial so that two alike play twice.
    private(set) var haptic: HapticCue?

    /// Bumped when a hands-free recording starts: VoiceOver's focus goes to
    /// the slot, and stays there while it runs (S2.4).
    private(set) var slotFocusRequest = 0

    /// Where a released note waits for its five seconds (S2.6).
    nonisolated enum UndoNote: Equatable, Sendable {
        /// In the parked store, marked "sending" — so a crash inside the
        /// window leaves a "not sent" row, never an orphan.
        case parked(ParkedRecordings.Entry)
        /// The store would not take it (nobody signed in, a full disk): the
        /// window still runs, from the recorder's own file.
        case loose(AudioRecorder.Recording, replyTo: ReplyToDTO?)
    }

    nonisolated struct HapticCue: Equatable, Sendable {
        let haptic: RecordGesture.Haptic
        let serial: Int
    }

    /// What only the composer knows, and what only it can do. Every closure
    /// has a harmless default, so an unwired composer records nothing it
    /// cannot account for.
    struct Hooks {
        /// The dimmed row a recording would meet now (call, busy, not sent).
        var blocked: () -> ComposerSlot.Dimmed? = { nil }
        /// The chat a "sending" entry is parked under.
        var chatID: () -> Int64? = { nil }
        /// Take the primed reply off the composer for a note leaving with it.
        var takeReply: () -> ReplyToDTO? = { nil }
        /// Give a reply back — a note that went to review instead of out.
        var restoreReply: (ReplyToDTO?) -> Void = { _ in }
        /// Hand a recording to the outbox with this reply. False when it
        /// could not be queued — the hook keeps it then, never loses it.
        var send: (AudioRecorder.Recording, ReplyToDTO?) -> Bool = { _, _ in false }
        /// Stage a recording in the chip above the field (S2.7).
        var review: (AudioRecorder.Recording) -> Void = { _ in }
        /// Keep a recording as the chat's "not sent" row (S2.8).
        var park: (AudioRecorder.Recording) -> Void = { _ in }
        /// Hand a parked "sending" entry to the outbox. False when it could
        /// not be queued.
        var sendParked: (ParkedRecordings.Entry) -> Bool = { _ in false }
        /// Stage a parked "sending" entry for review, and remove it — with
        /// this sentence in the composer's notice line, if one is given.
        /// What cannot be staged stays as an ordinary "not sent" row.
        var reviewParked: (ParkedRecordings.Entry, String?) -> Void = { _, _ in }
        /// Say a dimmed row's sentence.
        var explain: (ComposerSlot.Dimmed) -> Void = { _ in }
        /// The denial notice, with Open Settings.
        var denied: () -> Void = {}
        /// Show a hint in the composer's notice line.
        var hint: (RecordGesture.Hint) -> Void = { _ in }
        /// The recorder would not open.
        var startFailed: (AudioRecorder.Failure) -> Void = { _ in }
        /// The recorder failed mid-recording (S4).
        var stoppedUnexpectedly: () -> Void = {}
        /// The hold locked: the keyboard goes down (S2.3).
        var locked: () -> Void = {}
        /// A hands-free recording started: the field gives up its focus.
        var startedHandsFree: () -> Void = {}
        /// Sent, deleted or too short: keyboard focus back to the field.
        var returnFocus: () -> Void = {}
        /// A hands-free voice message was sent from a touch screen (S7.2).
        var sentHandsFreeByTouch: () -> Void = {}
    }

    // MARK: - What it is made of

    let recorder: AudioRecorder
    let constants: RecordGesture.HoldConstants
    /// This composer's name to the app's one-recording rule.
    let id = UUID()

    @ObservationIgnored var hooks = Hooks()

    // MARK: - Seams (the app gets the system; a test gets fakes)

    /// One monotonic clock, in milliseconds, for the press, the guard and
    /// the Undo window.
    @ObservationIgnored var clock: () -> UInt64 = VoiceComposer.uptimeMS
    /// Run `fire` after this many milliseconds.
    @ObservationIgnored var schedule: (UInt64, @escaping @MainActor () -> Void) -> any VoiceTimer = VoiceComposer.taskTimer
    @ObservationIgnored var permission: () -> RecordGesture.Permission = VoiceComposer.systemPermission
    @ObservationIgnored var requestPermission: () async -> Bool = VoiceComposer.askTheSystem
    /// VoiceOver or Switch Control: a held release reviews (S2.3).
    @ObservationIgnored var assistive: () -> Bool = VoiceComposer.assistiveTechnologyRuns
    /// VoiceOver alone: the microphone opens once "Recording" is spoken (S6).
    @ObservationIgnored var voiceOverRunning: () -> Bool = VoiceComposer.voiceOverRuns
    /// Say this and return once it has been said (bounded).
    @ObservationIgnored var speak: (String) async -> Void = VoiceComposer.speakAndWait
    /// Say this, politely.
    @ObservationIgnored var announce: (String) -> Void = VoiceComposer.post
    @ObservationIgnored var reviewBeforeSending: () -> Bool = { AppSettings.voiceReviewBeforeSending }
    @ObservationIgnored var firstReleaseTaught: () -> Bool = { AppSettings.voiceFirstReleaseTaught }
    @ObservationIgnored var rememberFirstReleaseTaught: () -> Void = { AppSettings.voiceFirstReleaseTaught = true }
    @ObservationIgnored var store: ParkedRecordings = .shared
    @ObservationIgnored var arbiter: VoiceRecordingArbiter = .shared
    @ObservationIgnored var nowPlaying: NowPlaying = .shared
    /// The window this composer is drawn in, handed to the arbiter with the
    /// microphone: on the Mac, minimising or closing THAT window stops the
    /// recording and another window's does not (S4,
    /// `VoiceRecordingArbiter.windowWentAway`). Nil on iPhone and iPad,
    /// where the app going to the background is what stops one.
    @ObservationIgnored var window: () -> AnyObject? = { nil }

    // MARK: - Private

    /// A recording already stopped, waiting for the effect that says where
    /// it goes.
    @ObservationIgnored private var held: AudioRecorder.Recording?
    /// The recorder opening, and the permission prompt's answer, in flight.
    /// `private(set)` rather than `private` for one reason, the one
    /// AttachmentStreamPlayer's `loadTask` gives: a test awaits these
    /// handles instead of sleeping on a clock. Nothing in the app reads them.
    @ObservationIgnored private(set) var startTask: Task<Void, Never>?
    @ObservationIgnored private(set) var permissionTask: Task<Void, Never>?
    @ObservationIgnored private var startAttempt = 0
    /// "Recording" was spoken before the microphone opened, so the
    /// reducer's announcement of it is not said twice.
    @ObservationIgnored private var spokeRecording = false
    @ObservationIgnored private var undoTimer: (any VoiceTimer)?
    @ObservationIgnored private var armedUntil: UInt64?
    @ObservationIgnored private var hintTimer: (any VoiceTimer)?
    @ObservationIgnored private var hapticSerial = 0
    /// The lift being handled came from a finger or a Pencil.
    @ObservationIgnored private var touchInFlight = false
    /// The recording running now was started by a finger's tap.
    @ObservationIgnored private var startedByTouch = false
    @ObservationIgnored private var warnedThirty = false
    @ObservationIgnored private var warnedSilence = false

    init(
        recorder: AudioRecorder? = nil,
        constants: RecordGesture.HoldConstants = .forSystem(longPressMS: VoiceComposer.systemLongPressMS)
    ) {
        self.recorder = recorder ?? AudioRecorder()
        self.constants = constants
    }

    /// The long press iOS names (S1.1): 500 ms — the recognizer's minimum
    /// duration that drives the hold.
    nonisolated static let systemLongPressMS: UInt64 = 500

    // MARK: - Reading the state

    var isPressed: Bool {
        if case .pressed = state.phase { return true }
        return false
    }

    var isHolding: Bool {
        if case .holding = state.phase { return true }
        return false
    }

    var isArmed: Bool {
        if case .holding(_, _, _, true) = state.phase { return true }
        return false
    }

    var isHandsFree: Bool {
        if case .handsFree = state.phase { return true }
        return false
    }

    /// The recording runs beside words or staged items (row 3).
    var isBesideDraft: Bool {
        state.phase == .handsFree(besideDraft: true)
    }

    /// A voice recording runs, as far as the slot is concerned.
    var isRecording: Bool { isHolding || isHandsFree }

    var isAskingDelete: Bool {
        if case .askingDelete = state.phase { return true }
        return false
    }

    /// The Undo row is up.
    var inUndoWindow: Bool { state.undo != nil }

    /// A row-5 Send must be ignored now: the slot's own activation just
    /// changed it (S1.1).
    var sendIsGuarded: Bool { state.guarded(atMS: clock()) }

    /// "30 seconds left" — at 4:30, shown in the meter's place (S2.5).
    var showsThirtySecondsLeft: Bool {
        isRecording && recorder.elapsed * 1000 >= Double(RecordRules.voiceWarningMS)
    }

    /// "We can't hear anything. Is the microphone muted?" — 3 s in, until a
    /// peak rises above the silence line (S2.9).
    var showsSilenceWarning: Bool {
        isRecording && recorder.isRecording && !recorder.heardSound
            && recorder.elapsed * 1000 >= Double(RecordRules.silenceWarningAfterMS)
    }

    /// The length the recorder's clock says, in the reducer's unit.
    var recordedMS: UInt64 {
        UInt64(max(0, recorder.recordedNow) * 1000)
    }

    // MARK: - What happened: the microphone's touch

    /// A finger, a Pencil or a pointer went down on the microphone.
    func pressDown(x: Double, y: Double, canHold: Bool, rtl: Bool) {
        handle(.down(atMS: clock(), x: x, y: y, canHold: canHold, rtl: rtl))
    }

    func pressMoved(x: Double, y: Double) {
        handle(.move(atMS: clock(), x: x, y: y))
    }

    /// The press lifted; `inside` is the control's own hit test.
    func pressLifted(x: Double, y: Double, inside: Bool, byTouch: Bool) {
        touchInFlight = byTouch
        defer { touchInFlight = false }
        handle(.up(
            atMS: clock(), x: x, y: y, inside: inside, situation: situation(),
            recordedMS: recordedMS, heard: recorder.heardSound))
    }

    /// The system took the touch away; `background`: the app went there.
    func pressCancelled(background: Bool) {
        handle(.systemCancel(atMS: clock(), background: background, recordedMS: recordedMS))
    }

    /// The long-press recognizer reached H. Its own timer is the port's
    /// long-press timer, so the tick is placed no earlier than H after the
    /// press — a recognizer that fires a millisecond before this clock says
    /// H has passed must still start the hold.
    func holdReached() {
        var at = clock()
        if case let .pressed(downAtMS, _, _, _, _) = state.phase {
            at = max(at, RecordGesture.saturatingAdd(downAtMS, constants.holdThresholdMS))
        }
        handle(.tick(atMS: at, situation: situation()))
    }

    // MARK: - What happened: everything else

    /// The slot was activated without a touch of the microphone: the Send
    /// arrow, the Stop square, a click on a dimmed or plain microphone that
    /// is not a press (VoiceOver, Switch Control, Full Keyboard Access), ⌘↩.
    func activate() {
        handle(.activate(atMS: clock(), situation: situation(), recordedMS: recordedMS))
    }

    /// Record Voice Message from the paperclip or the microphone's menu, or
    /// ⌥⌘R — which, pressed during a recording, stops it into review.
    func record(besideDraft: Bool) {
        handle(.record(atMS: clock(), besideDraft: besideDraft, situation: situation(), recordedMS: recordedMS))
    }

    /// Stop, Esc, VoiceOver's escape, Magic Tap, "Stop and listen first".
    func stop() {
        handle(.stop(atMS: clock(), recordedMS: recordedMS))
    }

    /// The recording row's Delete.
    func delete() {
        handle(.delete(atMS: clock(), recordedMS: recordedMS))
    }

    /// "Delete this recording?" answered.
    func answerDelete(_ delete: Bool) {
        handle(.answer(atMS: clock(), delete: delete))
    }

    /// The Undo row's Undo.
    func undo() {
        handle(.undo(atMS: clock()))
    }

    /// Anything else the person did — a character typed or deleted, a paste,
    /// the paperclip, a sticker, Reply: the Undo window ends by sending, and
    /// an idle slot's activation guard is lifted (S1.1). Never for a change
    /// the slot's own action made — the draft a Send clears or a Save
    /// restores — or a double tap on Send could open the microphone.
    func otherAction() {
        handle(.otherAction(atMS: clock()))
    }

    /// A staged item's ✕ (S2.7): `remove` takes it off the strip, which is
    /// a change the person made — `otherAction`, as the shared module names
    /// "an item staged or taken off the strip": an idle slot's guard lifts
    /// (S1.1), so after a Stop beside words, ✕ on the note and Send at once
    /// still sends the words, as on Android and the web — and a recording
    /// taken off says "Recording deleted", the word the recording row's
    /// Delete and the not-sent row's ✕ say (S6). A picked sound file is not
    /// a recording. Both composers' chips call this, so they cannot drift.
    func tookOff(voiceNote: Bool, remove: () -> Void) {
        remove()
        otherAction()
        if voiceNote { announce(String(localized: "Recording deleted")) }
    }

    /// The message field's binding, through which only the PERSON's edits
    /// reach the draft: typing, deleting, a paste into the field, dictation,
    /// autocorrect. Each one that changes the text is `otherAction` (S1.1:
    /// "ok" typed and sent at once is never slowed). The composer's own
    /// writes — the draft a Send clears, the text a Save gives back, the
    /// length clamp — go to the draft directly and never through this, so a
    /// double click on Send still cannot open the microphone it turned into.
    /// A write that changes nothing (a field re-committing its own text) is
    /// not an edit.
    static func typedBinding(_ draft: Binding<String>, typed: @escaping () -> Void) -> Binding<String> {
        Binding(
            get: { draft.wrappedValue },
            set: { newValue in
                guard newValue != draft.wrappedValue else { return }
                draft.wrappedValue = newValue
                typed()
            })
    }

    /// The slot's own Send or Save just emptied the composer.
    func emptied() {
        handle(.emptied(atMS: clock()))
    }

    /// Something other than the person: leaving the chat, the background, a
    /// call, another recording starting anywhere (S4).
    func interrupt() {
        handle(.interruption(atMS: clock(), recordedMS: recordedMS))
    }

    /// Sign-out: everything recorded and not sent is deleted (S4). The store
    /// itself is emptied by the sign-out.
    func discard() {
        if recorder.isRecording {
            recorder.cancel()
        } else {
            cancelStart()
        }
        if let held {
            try? FileManager.default.removeItem(at: held.url)
            self.held = nil
        }
        if case .loose(let recording, _) = undoNote {
            try? FileManager.default.removeItem(at: recording.url)
        }
        undoNote = nil
        undoTimer?.cancel()
        undoTimer = nil
        armedUntil = nil
        hintTimer?.cancel()
        showsStillRecording = false
        state = RecordGesture.HoldState()
        arbiter.release(id)
    }

    /// VoiceOver's Magic Tap, in S6's order: a recording stops into review;
    /// something of the app's playing pauses; nothing else — it never starts
    /// a recording. False lets the system have it (it plays and pauses
    /// Music when no app takes it).
    func magicTap() -> Bool {
        if isRecording {
            stop()
            return true
        }
        return nowPlaying.pauseAll()
    }

    /// VoiceOver's escape gesture does what Esc does: Stop (S6).
    func escape() -> Bool {
        guard isRecording else { return false }
        stop()
        return true
    }

    // MARK: - The step

    /// Run one event through the reducer and do what it says, in its order —
    /// except what it says ABOUT an effect that missed: the announcement,
    /// haptic, hint and lesson right after it describe a success that did not
    /// happen, and give way to what did. The step's next thing (a recording
    /// a tap starts as it ends the Undo window) is still done and said.
    func handle(_ event: RecordGesture.HoldEvent) {
        let (next, effects) = RecordGesture.step(state, event, constants)
        state = next
        var missed: Miss?
        for effect in effects {
            if let miss = missed {
                if effect.describesTheEffectBefore { continue }
                tell(miss)
                missed = nil
            }
            missed = perform(effect)
        }
        if let missed { tell(missed) }
        armUndoTimer()
    }

    /// An effect that moves a recording, and did not.
    private enum Miss {
        /// The recorder stopped and kept nothing — under its 1024-byte floor,
        /// a recording that never got audio.
        case nothingKept
        /// There was nothing to take: the recording had already gone.
        case nothingThere
        /// The outbox would not queue it. The composer kept it — in review
        /// with the error, or as a "not sent" row — never lost.
        case notQueued
    }

    /// Say and show what really happened, in the reducer's own vocabulary.
    private func tell(_ miss: Miss) {
        switch miss {
        case .nothingKept:
            perform(.hint(.tooShort))
            perform(.announce(.tooShort))
            perform(.haptic(.warning))
            hooks.returnFocus()
        case .nothingThere:
            break
        case .notQueued:
            announce(String(localized: "Couldn't send that — try again."))
            perform(.haptic(.warning))
        }
    }

    private func situation() -> RecordGesture.Situation {
        RecordGesture.Situation(
            permission: permission(),
            blocked: hooks.blocked(),
            assistive: assistive(),
            firstRelease: !firstReleaseTaught(),
            reviewBeforeSending: reviewBeforeSending())
    }

    @discardableResult
    private func perform(_ effect: RecordGesture.HoldEffect) -> Miss? {
        switch effect {
        case .start(let held):
            begin(held: held)
        case .lock:
            hooks.locked()
        case .arm, .disarm:
            // Drawn from the phase.
            break
        case .delete:
            discardRecording()
        case .send:
            return sendRecording()
        case .review:
            let wasThere = hasRecording
            guard let recording = takeRecording(.ownStop) else {
                return wasThere ? .nothingKept : .nothingThere
            }
            hooks.review(recording)
        case .park:
            if let recording = takeRecording(.interruption) { hooks.park(recording) }
        case .undoWindow:
            return openUndoWindow()
        case .undoSend:
            return sendUndoNote()
        case .undoReview:
            reviewUndoNote()
        case .askDelete:
            // Stopped first, then asked: the recording waits for the answer.
            held = takeRecording(.ownStop)
        case .askPermission:
            askPermission()
        case .denied:
            hooks.denied()
        case .explain(let reason):
            hooks.explain(reason)
        case .hint(let hint):
            show(hint)
        case .announce(let announcement):
            if announcement == .recording, spokeRecording {
                spokeRecording = false
            } else {
                announce(announcement.text)
            }
        case .haptic(let haptic):
            hapticSerial += 1
            self.haptic = HapticCue(haptic: haptic, serial: hapticSerial)
        case .firstReleaseDone:
            rememberFirstReleaseTaught()
        }
        return nil
    }

    // MARK: - Starting

    private func begin(held: Bool) {
        // One recording in the whole app: another composer's is parked first
        // (S1.7). Then nothing of the app's plays under the microphone.
        arbiter.claim(id, window: window()) { [weak self] how in self?.letGo(how) }
        nowPlaying.pauseAll()
        warnedThirty = false
        warnedSilence = false
        showsStillRecording = false
        startedByTouch = !held && touchInFlight
        recorder.onEnded = { [weak self] ended in self?.recorderEnded(ended) }
        recorder.onTick = { [weak self] in self?.recorderTicked() }
        if !held {
            slotFocusRequest += 1
            hooks.startedHandsFree()
        }
        startAttempt += 1
        let attempt = startAttempt
        // With VoiceOver running the microphone opens only once "Recording"
        // has been said — the app's own voice must not open every note (S6).
        let waitsForSpeech = voiceOverRunning()
        spokeRecording = waitsForSpeech
        startTask = Task { [weak self] in
            guard let self else { return }
            if waitsForSpeech {
                await self.speak(RecordGesture.Announcement.recording.text)
            }
            guard attempt == self.startAttempt else { return }
            await self.recorder.start()
            // Ended while the recorder was opening: `cancelStart` already
            // told the recorder, whose own attempt count keeps that start
            // from opening the microphone. Nothing to undo here — and the
            // recorder may by now be recording for a NEWER start, which a
            // cancel from this stale one would throw away.
            guard attempt == self.startAttempt else { return }
            self.startTask = nil
            guard !self.recorder.isRecording else { return }
            // The recorder would not open — refused during a call, a denied
            // microphone, a session another app holds. A recorder failing is
            // an interruption, and under a second there is nothing to keep.
            let failure = self.recorder.failure
            self.handle(.interruption(atMS: self.clock(), recordedMS: 0))
            self.arbiter.release(self.id)
            if let failure { self.hooks.startFailed(failure) }
        }
    }

    private func askPermission() {
        permissionTask = Task { [weak self] in
            guard let self else { return }
            let granted = await self.requestPermission()
            self.handle(.permissionAnswer(atMS: self.clock(), granted: granted))
        }
    }

    /// A start still on its way must not open the microphone.
    private func cancelStart() {
        startAttempt += 1
        startTask?.cancel()
        startTask = nil
        spokeRecording = false
        if !recorder.isRecording { recorder.cancel() }
    }

    // MARK: - Ending

    private enum Take {
        /// The person's own decision: today's floor (a file with sound).
        case ownStop
        /// Something else stopped it: a second or more is kept (S4).
        case interruption
    }

    /// The one way a recording leaves the recorder. Gives the microphone
    /// back to the arbiter, whatever it finds.
    private func takeRecording(_ how: Take) -> AudioRecorder.Recording? {
        defer { arbiter.release(id) }
        if let held {
            self.held = nil
            return held
        }
        guard recorder.isRecording else {
            cancelStart()
            return nil
        }
        switch how {
        case .ownStop: return recorder.stopRecording()
        case .interruption: return recorder.stopKeeping()
        }
    }

    /// A recording waits to be taken: held, or still in the recorder.
    private var hasRecording: Bool { held != nil || recorder.isRecording }

    private func discardRecording() {
        if let held {
            try? FileManager.default.removeItem(at: held.url)
            self.held = nil
        } else if recorder.isRecording {
            recorder.cancel()
        } else {
            cancelStart()
        }
        arbiter.release(id)
        hooks.returnFocus()
    }

    private func sendRecording() -> Miss? {
        let byTouch = startedByTouch
        startedByTouch = false
        let wasThere = hasRecording
        guard let recording = takeRecording(.ownStop) else {
            return wasThere ? .nothingKept : .nothingThere
        }
        let reply = hooks.takeReply()
        let queued = hooks.send(recording, reply)
        if queued, byTouch, !voiceOverRunning() {
            hooks.sentHandsFreeByTouch()
        }
        hooks.returnFocus()
        return queued ? nil : .notQueued
    }

    /// The recorder stopped by itself: the cap, an interruption, a failure.
    private func recorderEnded(_ ended: AudioRecorder.Ended) {
        held = ended.recording
        let length = ended.recording?.duration ?? recorder.elapsed
        let recorded = UInt64(max(0, length) * 1000)
        switch ended.reason {
        case .cap:
            handle(.cap(atMS: clock()))
        case .interrupted, .failed:
            handle(.interruption(atMS: clock(), recordedMS: recorded))
        }
        if ended.reason == .failed {
            hooks.stoppedUnexpectedly()
        }
        // Never lost: a recording no effect took is kept as "not sent".
        if let leftover = held {
            held = nil
            hooks.park(leftover)
        }
        arbiter.release(id)
    }

    /// Display only: "30 seconds left" and the silence warning are said once
    /// each, when they first show (S2.5, S2.9, S6).
    private func recorderTicked() {
        if !warnedThirty, showsThirtySecondsLeft {
            warnedThirty = true
            announce(String(localized: "30 seconds left"))
        }
        if !warnedSilence, showsSilenceWarning {
            warnedSilence = true
            announce(String(localized: "We can't hear anything. Is the microphone muted?"))
        }
    }

    private func letGo(_ how: VoiceRecordingArbiter.LetGo) {
        switch how {
        case .park: interrupt()
        case .discard: discard()
        }
    }

    private func show(_ hint: RecordGesture.Hint) {
        guard hint == .stillRecording else {
            hooks.hint(hint)
            return
        }
        showsStillRecording = true
        hintTimer?.cancel()
        hintTimer = schedule(RecordRules.stillRecordingHintMS) { [weak self] in
            self?.showsStillRecording = false
        }
    }

    // MARK: - The Undo window (S2.6)

    /// A release that sends: stopped, and parked marked "sending" BEFORE the
    /// window shows — nothing has left the device, and a crash inside the
    /// five seconds leaves a "not sent" row rather than nothing at all.
    private func openUndoWindow() -> Miss? {
        let wasThere = hasRecording
        guard let recording = takeRecording(.ownStop) else {
            // No note, so no window: an Undo row over nothing would end in
            // a "Voice message sent" about nothing.
            state.undo = nil
            return wasThere ? .nothingKept : .nothingThere
        }
        let reply = hooks.takeReply()
        if let chatID = hooks.chatID(),
           let entry = store.park(
               fileAt: recording.url, duration: recording.duration, chatID: chatID,
               replyTo: reply, caption: nil, sending: true)
        {
            undoNote = .parked(entry)
        } else {
            undoNote = .loose(recording, replyTo: reply)
        }
        return nil
    }

    /// Five seconds passed, or something else happened: to the outbox,
    /// "exactly as S2.5's Send" (S2.6) — so a hand-off that fails lands in
    /// review with the error, never lost, as Send's does. (Leaving the chat,
    /// which also ends the window, cancels that staging, and the entry is an
    /// ordinary "not sent" row instead.)
    private func sendUndoNote() -> Miss? {
        guard let note = undoNote else { return .nothingThere }
        undoNote = nil
        switch note {
        case .parked(let entry):
            if hooks.sendParked(entry) {
                store.remove(entry)
                return nil
            }
            hooks.reviewParked(entry, String(localized: "Couldn't send that — try again."))
            return .notQueued
        case .loose(let recording, let reply):
            return hooks.send(recording, reply) ? nil : .notQueued
        }
    }

    /// Undo: review instead, and nothing is sent.
    private func reviewUndoNote() {
        guard let note = undoNote else { return }
        undoNote = nil
        switch note {
        case .parked(let entry):
            hooks.reviewParked(entry, nil)
        case .loose(let recording, let reply):
            hooks.restoreReply(reply)
            hooks.review(recording)
        }
    }

    /// The window runs out on the reducer's clock; the timer only makes sure
    /// an event arrives when it does. A timer that fires early finds the
    /// window still open and is armed again for what is left.
    private func armUndoTimer() {
        guard let until = state.undo?.untilMS else {
            undoTimer?.cancel()
            undoTimer = nil
            armedUntil = nil
            return
        }
        guard armedUntil != until else { return }
        undoTimer?.cancel()
        armedUntil = until
        let wait = RecordGesture.saturatingSub(until, clock())
        undoTimer = schedule(wait) { [weak self] in
            guard let self else { return }
            self.armedUntil = nil
            self.handle(.tick(atMS: self.clock(), situation: self.situation()))
        }
    }

    // MARK: - The system's defaults

    nonisolated static func uptimeMS() -> UInt64 {
        UInt64(ProcessInfo.processInfo.systemUptime * 1000)
    }

    private final class TaskTimer: VoiceTimer {
        let task: Task<Void, Never>
        init(task: Task<Void, Never>) { self.task = task }
        func cancel() { task.cancel() }
    }

    static func taskTimer(afterMS: UInt64, fire: @escaping @MainActor () -> Void) -> any VoiceTimer {
        TaskTimer(task: Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(Int(min(afterMS, UInt64(Int.max)))))
            guard !Task.isCancelled else { return }
            fire()
        })
    }

    static func systemPermission() -> RecordGesture.Permission {
        #if os(iOS)
        switch AVAudioApplication.shared.recordPermission {
        case .granted: return .granted
        case .denied: return .denied
        default: return .notAsked
        }
        #else
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: return .granted
        case .notDetermined: return .notAsked
        default: return .denied
        }
        #endif
    }

    static func askTheSystem() async -> Bool {
        #if os(iOS)
        await withCheckedContinuation { continuation in
            AVAudioApplication.requestRecordPermission { granted in
                continuation.resume(returning: granted)
            }
        }
        #else
        await AVCaptureDevice.requestAccess(for: .audio)
        #endif
    }

    static func assistiveTechnologyRuns() -> Bool {
        #if os(iOS)
        UIAccessibility.isVoiceOverRunning || UIAccessibility.isSwitchControlRunning
        #else
        NSWorkspace.shared.isVoiceOverEnabled || NSWorkspace.shared.isSwitchControlEnabled
        #endif
    }

    static func voiceOverRuns() -> Bool {
        #if os(iOS)
        UIAccessibility.isVoiceOverRunning
        #else
        NSWorkspace.shared.isVoiceOverEnabled
        #endif
    }

    static func post(_ sentence: String) {
        AccessibilityNotification.Announcement(sentence).post()
    }

    /// How long the Mac waits after asking VoiceOver to say "Recording"
    /// before it opens the microphone. AppKit has no "announcement finished"
    /// notification — only `NSAccessibilityAnnouncementRequestedNotification`
    /// — so the plan's "a fixed 1 s elsewhere" (S6) is the Mac's rule.
    nonisolated static let macSpeechAllowanceMS: UInt64 = 1_000

    /// Waits for VoiceOver to finish saying `sentence`, or 2.5 s at most —
    /// an announcement another one interrupts never reports finishing. On
    /// the Mac, which cannot hear it finish, a fixed allowance instead.
    static func speakAndWait(_ sentence: String) async {
        #if os(iOS)
        let wait = SpeechWait()
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            wait.continuation = continuation
            wait.token = NotificationCenter.default.addObserver(
                forName: UIAccessibility.announcementDidFinishNotification, object: nil, queue: .main
            ) { note in
                let said = note.userInfo?[UIAccessibility.announcementStringValueUserInfoKey] as? String
                guard said == sentence else { return }
                MainActor.assumeIsolated { wait.finish() }
            }
            wait.timeout = Task { @MainActor in
                try? await Task.sleep(for: .milliseconds(2_500))
                wait.finish()
            }
            UIAccessibility.post(notification: .announcement, argument: sentence)
        }
        #else
        post(sentence)
        try? await Task.sleep(for: .milliseconds(Int(macSpeechAllowanceMS)))
        #endif
    }

    #if os(iOS)
    private final class SpeechWait {
        var continuation: CheckedContinuation<Void, Never>?
        var token: (any NSObjectProtocol)?
        var timeout: Task<Void, Never>?

        func finish() {
            if let token { NotificationCenter.default.removeObserver(token) }
            token = nil
            timeout?.cancel()
            timeout = nil
            continuation?.resume()
            continuation = nil
        }
    }
    #endif
}

private extension RecordGesture.HoldEffect {
    /// What is shown, said, felt or learnt about the effect before it — the
    /// reducer always places these after the thing they describe.
    var describesTheEffectBefore: Bool {
        switch self {
        case .announce, .haptic, .hint, .firstReleaseDone: true
        default: false
        }
    }
}
