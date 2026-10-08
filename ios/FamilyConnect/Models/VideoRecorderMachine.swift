//
//  VideoRecorderMachine.swift
//  FamilyConnect
//
//  The round-video recorder's rules as a reducer (#79, Phase 3 —
//  docs/audio-video-messages-2026-10-04.md, S3.2–S3.6, S4, S6).
//
//  WHERE THE LINE IS, as in `VoiceComposer`: this file DECIDES — the states
//  PREVIEW, RECORDING and REVIEW, every transition between them, the 600 ms
//  guard, the 10-second "Delete video message?" rule, the 60-second idle close
//  and what each interruption does — and says so as a list of `Effect`s.
//  `VideoMessageSession` DOES them: opens the camera, writes the file, asks the
//  system for permission, speaks. Pure and nonisolated, so every row of S3.4
//  and S4 can be pinned in a test with no camera, no window and no clock.
//
//  THE CAMERA IS OFF IN REVIEW AND IN EVERY REFUSAL, and the microphone is
//  open only while RECORDING: the reducer turns the camera off on the way
//  into REVIEW and turns it on only on the way into PREVIEW, so no state can
//  hold a light the status line contradicts.
//
//  NOTHING IS SENT BUT BY SEND (S1.7). A length limit, an interruption, a
//  closing window — each stops a take into REVIEW at most; only `.send` in
//  REVIEW ever produces `.send`.
//

import Foundation

nonisolated enum VideoRecorderMachine {

    // MARK: - Inputs

    /// A system permission as it stands before anything is asked.
    nonisolated enum Permission: Equatable, Sendable {
        case granted
        case denied
        case notAsked
    }

    /// Which refusal the recorder opened onto (S3.2).
    nonisolated enum Refusal: Equatable, Sendable {
        /// The camera — offered: Open Settings, and a voice message instead.
        case camera
        /// The microphone — offered: Open Settings, and nothing else, since a
        /// voice message needs it too.
        case microphone
    }

    /// Why the picture is not there (S3.6, S4).
    nonisolated enum CameraProblem: Equatable, Sendable {
        /// "The camera is being used by another app."
        case inUse
        /// "The camera isn't available while other apps are on screen." —
        /// an iPad sharing the screen where multitasking camera access is
        /// not supported.
        case multitasking
        /// The camera went away, or the system is under pressure: said as
        /// the camera being busy, the one sentence S3.6 has for it.
        case unavailable
    }

    /// The window being closed, or the app quitting, over the recorder (S4).
    nonisolated enum Closing: Equatable, Sendable {
        case window
        case quit
    }

    /// What the recorder says in its status line besides the clock (S3.4,
    /// S3.6). Cleared by the next control used.
    nonisolated enum Notice: Equatable, Sendable {
        /// "That video was too short." — a take under 1.0 s.
        case tooShort
        /// "Recording stopped at one minute." — the length limit.
        case stoppedAtLimit
        /// The camera sentence a take was stopped by (S3.6).
        case camera(CameraProblem)
        /// "The recording stopped unexpectedly." (S4)
        case stoppedUnexpectedly
        /// Row 9's sentence, for "Record a voice message instead" while a
        /// not-sent voice message waits (S3.4).
        case notSent
    }

    /// "Delete video message?" — and what Delete then does.
    nonisolated enum Question: Equatable, Sendable {
        /// Delete in RECORDING at 10 s or more: the take stopped first;
        /// Delete goes back to PREVIEW, Keep to REVIEW.
        case discardTake
        /// Retake in REVIEW at 10 s or more: Delete goes back to PREVIEW.
        case retake
        /// Delete in REVIEW at 10 s or more: Delete closes.
        case delete
        /// Esc, Back or VoiceOver's escape in REVIEW — always asked.
        case escape
        /// The window closing or the app quitting over a clip (S4, S8.3):
        /// Delete closes it AND lets the close go on; Keep cancels it.
        case closing(Closing)
    }

    /// Why a take is being finished — what REVIEW says, and where a take
    /// too short to keep goes.
    nonisolated enum Finish: Equatable, Sendable {
        /// Stop (or Esc, or Magic Tap).
        case stop
        /// The length limit.
        case limit
        /// Delete at 10 s or more: asked once the file is whole.
        case deleteAsked
        /// The camera stopped it.
        case camera(CameraProblem)
        /// The microphone was taken, or the recorder failed.
        case unexpected
        /// A call, the background, a lock, sleep, a minimised window: the
        /// recorder would CLOSE in PREVIEW, so a take too short to keep
        /// closes it too.
        case away
    }

    nonisolated enum Phase: Equatable, Sendable {
        /// Opened onto the system's question: a neutral circle and "Video
        /// messages need the camera and the microphone." (S3.2).
        case asking
        /// A refusal; the camera is off.
        case refused(Refusal)
        /// The live camera; the microphone is closed.
        case preview
        /// Recording since `startMS` on the recorder's clock.
        case recording(startMS: UInt64)
        /// Stopped, waiting for the file to be whole.
        case finishing(Finish)
        /// The clip as it will be sent.
        case review(durationMS: UInt64)
        /// Gone. Nothing happens after this.
        case closed
    }

    /// What is spoken (S6) — polite, state changes only.
    nonisolated enum Announcement: Equatable, Sendable {
        case cameraReady
        case recordingVideo
        case tenSecondsLeft
        case stoppedAtLimit
        case tooShort
        case sent
        case cameraTurnedOff
    }

    nonisolated enum Event: Equatable, Sendable {
        /// The recorder opened; both statuses read FIRST (S3.2).
        case opened(camera: Permission, microphone: Permission, atMS: UInt64)
        /// The system's answer to the camera question.
        case cameraAnswered(Bool, atMS: UInt64)
        /// The system's answer to the microphone question.
        case microphoneAnswered(Bool, atMS: UInt64)
        /// The camera delivered its first frame since it was turned on.
        case firstFrame(atMS: UInt64)
        /// A sampled frame's brightness: dark or not (`NearBlack`).
        case frame(dark: Bool, atMS: UInt64)
        /// The camera became unavailable, or (nil) available again.
        case camera(CameraProblem?)
        /// The red disc, Return, Magic Tap in PREVIEW. `speakFirst`: VoiceOver
        /// is running, so the microphone opens only once "Recording video"
        /// has been spoken and the clock starts with it (S6: the app's own
        /// speech stays out of the clip).
        case record(atMS: UInt64, speakFirst: Bool = false)
        /// "Recording video" has been spoken: open the microphone now.
        case spoken(atMS: UInt64)
        /// The Stop square or Return in RECORDING.
        case stop(atMS: UInt64)
        /// Delete — the leading control in RECORDING and REVIEW.
        case delete(atMS: UInt64)
        case retake(atMS: UInt64)
        /// The slot in REVIEW.
        case send(atMS: UInt64)
        /// The Close control in PREVIEW (and in a refusal).
        case close
        /// Esc, Back, VoiceOver's escape.
        case escape(atMS: UInt64)
        /// Magic Tap: Record, Stop, or play/pause (S6).
        case magicTap(atMS: UInt64, speakFirst: Bool = false)
        /// Space in REVIEW, a tap on the clip (S3.4).
        case playPause
        /// "Delete video message?" answered: true is Delete.
        case answer(delete: Bool, atMS: UInt64)
        /// "Record a voice message instead"; `notSent`: a voice message
        /// that was not sent waits in this chat (row 9).
        case voiceInstead(notSent: Bool)
        /// A control that changes nothing here but counts as use: Switch
        /// camera, Choose camera, the reply's ✕ — the idle close restarts.
        case used(atMS: UInt64)
        /// The recorder's clock, about four times a second.
        case tick(atMS: UInt64)
        /// The writer reached the length limit by itself.
        case limitReached
        /// The file is whole. Nil: nothing readable came of it. `tooBig`:
        /// over `max_round_video_bytes` (S3.6).
        case finished(durationMS: UInt64?, tooBig: Bool, atMS: UInt64)
        /// A call rang, started or was placed (S4).
        case callStarted
        /// The background, a lock, sleep, the screen saver, a minimised or
        /// hidden window (S4).
        case wentAway
        /// A desktop window lost focus but is still on screen; `asking`:
        /// a permission prompt the recorder raised is up.
        case focusLost
        /// Siri, an alarm or another app took the microphone.
        case microphoneTaken
        /// The writer failed mid-take.
        case recorderFailed
        /// The window is closing or the app quitting (S4, S8.3).
        case closing(Closing)
        /// Signed out: everything recorded and not sent is deleted (S4).
        case signedOut
    }

    nonisolated enum Effect: Equatable, Sendable {
        case requestCamera
        case requestMicrophone
        /// Turn the camera on (PREVIEW). The microphone stays closed.
        case startCamera
        /// Turn the camera off — the light goes out.
        case stopCamera
        /// Open the microphone and start writing at the next frame.
        case startRecording
        /// Pause the app's playback and take the one recording, say
        /// "Recording video", and report `.spoken` once it has been said —
        /// the microphone stays closed until then (S6).
        case speakThenRecord
        /// Stop writing; `finished` follows.
        case stopRecording
        /// Stop writing and throw the take away; no `finished` follows.
        case cancelRecording
        /// Delete the clip in REVIEW.
        case deleteClip
        /// Hand the clip to the outbox; `round` false when it is too big
        /// for a video message and goes as a regular video (S3.6).
        case send(round: Bool)
        case announce(Announcement)
        /// A medium haptic, on phones (S3.4).
        case haptic
        case togglePlayback
        case stopPlayback
        /// The phone's orientation is held from Record until Stop (S3.5).
        case holdOrientation(Bool)
        /// Close the recorder; focus goes back to what opened it.
        case close
        /// Let the window close or the app quit after all.
        case proceed(Closing)
        /// Start a hands-free voice recording in the composer.
        case startVoice
        /// The first-time line has been seen through to a recording.
        case taught
    }

    // MARK: - State

    nonisolated struct State: Equatable, Sendable {
        var phase: Phase = .asking
        /// Where a take stops: `max_round_video_ms` − 500 ms (S1.1).
        var capMS: UInt64
        /// Where "10 seconds left" starts.
        var warningMS: UInt64
        /// The microphone's status as it was read at opening — what the
        /// camera's answer is followed by.
        var microphone: Permission = .notAsked
        /// The camera has delivered a frame since it was last turned on;
        /// Record is dimmed until it has (S3.4).
        var firstFrameAtMS: UInt64?
        var problem: CameraProblem?
        /// The first of an unbroken run of near-black frames (S3.6).
        var darkSinceMS: UInt64?
        /// The last control used in PREVIEW, for the 60 s close.
        var lastUsedMS: UInt64 = 0
        /// The slot ignores activation until then (S1.1: Record → Stop →
        /// Send).
        var guardUntilMS: UInt64 = 0
        /// "10 seconds left" has been said for this take.
        var warned = false
        /// RECORDING, but "Recording video" is still being spoken: the
        /// microphone is closed and the clock has not started (S6).
        var speaking = false
        var notice: Notice?
        var question: Question?
        /// A close that arrived while a take was being finished: asked about
        /// once the clip is whole.
        var pendingClosing: Closing?
        /// The clip is over `max_round_video_bytes` (S3.6).
        var tooBig = false

        init(maxRoundVideoMS: UInt64 = RecordRules.defaultMaxRoundVideoMS) {
            capMS = RoundVideo.capMS(maxRoundVideoMS: maxRoundVideoMS)
            warningMS = RoundVideo.warningMS(maxRoundVideoMS: maxRoundVideoMS)
        }

        /// The slot ignores activation now.
        func guarded(atMS now: UInt64) -> Bool { now < guardUntilMS }

        /// How long the take has run.
        func recordedMS(atMS now: UInt64) -> UInt64 {
            if case .recording(let start) = phase, !speaking { return RecordGesture.saturatingSub(now, start) }
            return 0
        }

        /// From the warning on, the ring is orange and "10 seconds left"
        /// shows (S3.4).
        func inWarning(atMS now: UInt64) -> Bool {
            if case .recording = phase { return recordedMS(atMS: now) >= warningMS }
            return false
        }

        /// "We can't see anything. Is the camera turned off or covered?" —
        /// two seconds of near-black PREVIEW (S3.6).
        func looksDark(atMS now: UInt64) -> Bool {
            guard phase == .preview, let since = darkSinceMS else { return false }
            return RecordGesture.saturatingSub(now, since) >= VideoRecorderMachine.darkAfterMS
        }

        /// Record can be used: PREVIEW, a frame delivered, no camera problem
        /// (S3.4, S3.6).
        var canRecord: Bool {
            phase == .preview && firstFrameAtMS != nil && problem == nil && question == nil
        }

        /// A window may close, or the app quit, without asking: nothing in
        /// it would be lost (S4).
        var closesFreely: Bool {
            switch phase {
            case .asking, .refused, .preview, .closed: true
            case .recording, .finishing, .review: false
            }
        }

        /// The camera is on in this phase.
        var cameraOn: Bool {
            switch phase {
            case .preview, .recording, .finishing: true
            default: false
            }
        }
    }

    /// How long PREVIEW must stay near-black before it says so (S3.6).
    static let darkAfterMS: UInt64 = 2_000

    // MARK: - The reducer

    static func step(_ state: State, _ event: Event) -> (State, [Effect]) {
        var s = state
        var fx: [Effect] = []
        guard s.phase != .closed else { return (s, []) }

        switch event {
        case let .opened(camera, microphone, now):
            s.microphone = microphone
            // A camera grant is no use to somebody whose microphone is off,
            // and the voice message the camera refusal offers needs the
            // microphone too — so a refused microphone is said first.
            if microphone == .denied {
                s.phase = .refused(.microphone)
            } else if camera == .denied {
                s.phase = .refused(.camera)
            } else if camera == .notAsked {
                s.phase = .asking
                fx.append(.requestCamera)
            } else if microphone == .notAsked {
                s.phase = .asking
                fx.append(.requestMicrophone)
            } else {
                enterPreview(&s, &fx, atMS: now)
            }

        case let .cameraAnswered(granted, now):
            guard s.phase == .asking else { break }
            if !granted {
                s.phase = .refused(.camera)
            } else if s.microphone != .granted {
                fx.append(.requestMicrophone)
            } else {
                enterPreview(&s, &fx, atMS: now)
            }

        case let .microphoneAnswered(granted, now):
            guard s.phase == .asking else { break }
            s.microphone = granted ? .granted : .denied
            if granted {
                enterPreview(&s, &fx, atMS: now)
            } else {
                s.phase = .refused(.microphone)
            }

        case let .firstFrame(now):
            guard s.cameraOn, s.firstFrameAtMS == nil else { break }
            s.firstFrameAtMS = now
            if s.phase == .preview { fx.append(.announce(.cameraReady)) }

        case let .frame(dark, now):
            guard s.phase == .preview else { break }
            if dark {
                if s.darkSinceMS == nil { s.darkSinceMS = now }
            } else {
                s.darkSinceMS = nil
            }

        case .camera(let problem):
            s.problem = problem
            if let problem, case .recording = s.phase {
                finish(&s, &fx, .camera(problem))
            }

        case let .record(now, speakFirst):
            guard s.canRecord, !s.guarded(atMS: now) else { break }
            s.phase = .recording(startMS: now)
            s.guardUntilMS = RecordGesture.saturatingAdd(now, RecordRules.activationGuardMS)
            s.warned = false
            s.notice = nil
            s.darkSinceMS = nil
            s.tooBig = false
            if speakFirst {
                // With VoiceOver the app's own "Recording video" must not
                // open the clip: said first, the microphone after (S6).
                s.speaking = true
                fx += [.holdOrientation(true), .haptic, .speakThenRecord, .taught]
            } else {
                s.speaking = false
                fx += [.holdOrientation(true), .startRecording, .haptic, .announce(.recordingVideo), .taught]
            }

        case .spoken(let now):
            guard case .recording = s.phase, s.speaking else { break }
            s.speaking = false
            // The clock starts with the microphone.
            s.phase = .recording(startMS: now)
            fx.append(.startRecording)

        case .stop(let now):
            guard case .recording = s.phase, !s.guarded(atMS: now) else { break }
            s.guardUntilMS = RecordGesture.saturatingAdd(now, RecordRules.activationGuardMS)
            finish(&s, &fx, .stop)

        case .delete(let now):
            switch s.phase {
            case .recording:
                if s.recordedMS(atMS: now) >= RecordRules.deleteAsksFromMS {
                    finish(&s, &fx, .deleteAsked)
                } else {
                    // Under 10 s it goes at once, and the camera stays on.
                    fx += [.cancelRecording, .holdOrientation(false)]
                    s.speaking = false
                    s.phase = .preview
                    s.lastUsedMS = now
                    s.guardUntilMS = RecordGesture.saturatingAdd(now, RecordRules.activationGuardMS)
                }
            case .review(let duration):
                guard s.question == nil else { break }
                if duration >= RecordRules.deleteAsksFromMS {
                    s.question = .delete
                } else {
                    fx += [.stopPlayback, .deleteClip]
                    close(&s, &fx)
                }
            default:
                break
            }

        case .retake(let now):
            guard case .review(let duration) = s.phase, s.question == nil else { break }
            if duration >= RecordRules.deleteAsksFromMS {
                s.question = .retake
            } else {
                fx += [.stopPlayback, .deleteClip]
                enterPreview(&s, &fx, atMS: now)
            }

        case .send(let now):
            guard case .review = s.phase, s.question == nil, !s.guarded(atMS: now) else { break }
            fx += [.stopPlayback, .send(round: !s.tooBig), .announce(.sent)]
            close(&s, &fx)

        case .close:
            switch s.phase {
            case .asking, .refused, .preview:
                if s.cameraOn { fx.append(.stopCamera) }
                close(&s, &fx)
            default:
                break
            }

        case .escape(let now):
            switch s.phase {
            case .asking, .refused, .preview:
                if s.cameraOn { fx.append(.stopCamera) }
                close(&s, &fx)
            case .recording:
                // Esc is Stop (S3.4) — a key, not the slot, so no guard.
                s.guardUntilMS = RecordGesture.saturatingAdd(now, RecordRules.activationGuardMS)
                finish(&s, &fx, .stop)
            case .review:
                if s.question == nil { s.question = .escape }
            case .finishing, .closed:
                break
            }

        case let .magicTap(now, speakFirst):
            switch s.phase {
            case .preview: return step(s, .record(atMS: now, speakFirst: speakFirst))
            case .recording: return step(s, .stop(atMS: now))
            case .review: return step(s, .playPause)
            default: break
            }

        case .playPause:
            guard case .review = s.phase, s.question == nil else { break }
            fx.append(.togglePlayback)

        case let .answer(delete, now):
            guard let question = s.question else { break }
            s.question = nil
            guard delete else {
                // Keep: back to REVIEW as it was; a close is cancelled.
                s.pendingClosing = nil
                break
            }
            fx += [.stopPlayback, .deleteClip]
            switch question {
            case .discardTake, .retake:
                enterPreview(&s, &fx, atMS: now)
            case .delete, .escape:
                close(&s, &fx)
            case .closing(let closing):
                close(&s, &fx)
                fx.append(.proceed(closing))
            }

        case .voiceInstead(let notSent):
            let offered: Bool
            switch s.phase {
            case .preview, .refused(.camera): offered = true
            default: offered = false
            }
            guard offered else { break }
            if notSent {
                s.notice = .notSent
            } else {
                if s.cameraOn { fx.append(.stopCamera) }
                close(&s, &fx)
                fx.append(.startVoice)
            }

        case .used(let now):
            if s.phase == .preview { s.lastUsedMS = now }
            if s.notice == .tooShort || s.notice == .notSent { s.notice = nil }

        case .tick(let now):
            switch s.phase {
            case .recording:
                let recorded = s.recordedMS(atMS: now)
                if recorded >= s.capMS {
                    finish(&s, &fx, .limit)
                } else if !s.warned, recorded >= s.warningMS {
                    s.warned = true
                    fx.append(.announce(.tenSecondsLeft))
                }
            case .preview:
                if s.question == nil,
                   RecordGesture.saturatingSub(now, s.lastUsedMS) >= RecordRules.previewIdleCloseMS
                {
                    fx += [.stopCamera, .announce(.cameraTurnedOff)]
                    close(&s, &fx)
                }
            default:
                break
            }

        case .limitReached:
            guard case .recording = s.phase else { break }
            finish(&s, &fx, .limit)

        case let .finished(duration, tooBig, now):
            guard case .finishing(let why) = s.phase else { break }
            fx.append(.holdOrientation(false))
            let closing = s.pendingClosing
            s.pendingClosing = nil
            guard let duration, duration >= RecordRules.shortestRecordingMS else {
                // Nothing worth keeping (S4: "Under 1.0 s, 'not sent' below
                // means deleted instead").
                fx.append(.deleteClip)
                if duration == nil, why != .away { s.notice = .stoppedUnexpectedly }
                if let closing {
                    fx.append(.stopCamera)
                    close(&s, &fx)
                    fx.append(.proceed(closing))
                } else if why == .away {
                    fx.append(.stopCamera)
                    close(&s, &fx)
                } else {
                    if duration != nil {
                        s.notice = .tooShort
                        fx.append(.announce(.tooShort))
                    }
                    s.phase = .preview
                    s.lastUsedMS = now
                }
                break
            }
            s.phase = .review(durationMS: duration)
            s.tooBig = tooBig
            s.darkSinceMS = nil
            s.firstFrameAtMS = nil
            fx.append(.stopCamera)
            switch why {
            case .stop, .deleteAsked:
                break
            case .limit, .camera, .unexpected, .away:
                // Stop became Send without the slot being used: a tap meant
                // for Stop that lands just after must not send (S1.7,
                // "nothing is ever sent by a length limit running out").
                s.guardUntilMS = max(
                    s.guardUntilMS, RecordGesture.saturatingAdd(now, RecordRules.activationGuardMS))
            }
            switch why {
            case .limit:
                s.notice = .stoppedAtLimit
                fx.append(.announce(.stoppedAtLimit))
            case .camera(let problem):
                s.notice = .camera(problem)
            case .unexpected:
                s.notice = .stoppedUnexpectedly
            case .deleteAsked:
                s.question = .discardTake
            case .stop, .away:
                break
            }
            if let closing { s.question = .closing(closing) }

        case .callStarted, .wentAway:
            switch s.phase {
            case .asking, .refused, .preview:
                if s.cameraOn { fx.append(.stopCamera) }
                close(&s, &fx)
            case .recording:
                finish(&s, &fx, .away)
            case .review:
                fx.append(.stopPlayback)
            case .finishing, .closed:
                break
            }

        case .focusLost:
            // Never to a permission prompt the recorder raised: that is
            // `.asking`, which is not PREVIEW (S3.4).
            if s.phase == .preview {
                fx.append(.stopCamera)
                close(&s, &fx)
            }

        case .microphoneTaken, .recorderFailed:
            guard case .recording = s.phase else { break }
            finish(&s, &fx, .unexpected)

        case .closing(let closing):
            switch s.phase {
            case .asking, .refused, .preview:
                if s.cameraOn { fx.append(.stopCamera) }
                close(&s, &fx)
                fx.append(.proceed(closing))
            case .recording:
                // A RECORDING stops into REVIEW first, then asks (S8.3).
                s.pendingClosing = closing
                finish(&s, &fx, .stop)
            case .finishing:
                s.pendingClosing = closing
            case .review:
                fx.append(.stopPlayback)
                s.question = .closing(closing)
            case .closed:
                break
            }

        case .signedOut:
            switch s.phase {
            case .recording, .finishing: fx += [.cancelRecording, .holdOrientation(false)]
            case .review: fx += [.stopPlayback, .deleteClip]
            default: break
            }
            s.speaking = false
            if s.cameraOn { fx.append(.stopCamera) }
            close(&s, &fx)
        }
        return (s, fx)
    }

    // MARK: - Pieces

    private static func enterPreview(_ s: inout State, _ fx: inout [Effect], atMS now: UInt64) {
        let wasOn = s.cameraOn
        s.phase = .preview
        s.lastUsedMS = now
        s.question = nil
        s.darkSinceMS = nil
        s.tooBig = false
        if !wasOn {
            s.firstFrameAtMS = nil
            fx.append(.startCamera)
        }
    }

    private static func finish(_ s: inout State, _ fx: inout [Effect], _ why: Finish) {
        if s.speaking, case .recording(let start) = s.phase {
            // Still saying "Recording video": the microphone never opened
            // and nothing was written — a take of nothing, too short to
            // keep, ended where a take under 1.0 s would end.
            s.speaking = false
            s.phase = .finishing(why)
            fx.append(.cancelRecording)
            let (next, more) = step(s, .finished(durationMS: 0, tooBig: false, atMS: start))
            s = next
            fx += more
            return
        }
        s.phase = .finishing(why)
        fx.append(.stopRecording)
    }

    private static func close(_ s: inout State, _ fx: inout [Effect]) {
        s.phase = .closed
        s.question = nil
        fx.append(.close)
    }
}

// MARK: - Words

extension VideoRecorderMachine.Announcement {
    var text: String {
        switch self {
        case .cameraReady: String(localized: "Camera ready")
        case .recordingVideo: String(localized: "Recording video")
        case .tenSecondsLeft: String(localized: "10 seconds left")
        case .stoppedAtLimit: String(localized: "Recording stopped at one minute.")
        case .tooShort: String(localized: "That video was too short.")
        case .sent: String(localized: "Video message sent")
        case .cameraTurnedOff: String(localized: "Camera turned off")
        }
    }
}

extension VideoRecorderMachine.CameraProblem {
    var sentence: String {
        switch self {
        case .inUse, .unavailable: String(localized: "The camera is being used by another app.")
        case .multitasking: String(localized: "The camera isn't available while other apps are on screen.")
        }
    }
}

extension VideoRecorderMachine.Notice {
    var sentence: String {
        switch self {
        case .tooShort: String(localized: "That video was too short.")
        case .stoppedAtLimit: String(localized: "Recording stopped at one minute.")
        case .camera(let problem): problem.sentence
        case .stoppedUnexpectedly: String(localized: "The recording stopped unexpectedly.")
        case .notSent: ComposerSlot.Dimmed.notSent.notice
        }
    }
}

/// "We can't see anything" — whether a frame is near-black (S3.6).
///
/// A laptop's privacy shutter and Android 12's camera toggle give a picture
/// with no error at all; a dark room is NOT one, so the line only adds a
/// question and Record stays usable.
nonisolated enum NearBlack {
    /// Mean luma, 0…1, at or under which a frame counts as near-black. Black
    /// is 0 after the video-range offset is taken off; a dim room sits well
    /// above this, a covered lens at the sensor's noise floor below it.
    static let threshold = 0.05

    static func isDark(meanLuma: Double) -> Bool {
        meanLuma <= threshold
    }

    /// The mean luma of 8-bit samples, 0…1, from video range (16…235) or
    /// full range (0…255).
    static func meanLuma(sum: UInt64, count: Int, videoRange: Bool) -> Double {
        guard count > 0 else { return 0 }
        let mean = Double(sum) / Double(count)
        let normalized = videoRange ? (mean - 16) / 219 : mean / 255
        return min(1, max(0, normalized))
    }
}
