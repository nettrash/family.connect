//
//  CallManager.swift
//  FamilyConnect
//
//  The voice-call state machine (docs/protocol.md, "Voice calls"). One
//  call at a time, one to one, audio only, peer to peer: the server only
//  passes the JSON below between the two people and wakes a phone that
//  has no socket. This object owns the whole client half of that:
//
//    - it mints the call id (a UUID, exactly as a send mints its
//      client_msg_id), fetches the ICE servers, drives the media client,
//      and sends the signalling frames through the coordinator;
//    - it applies an inbound frame ONLY to the call it holds and ignores
//      every other one in silence — the rule that makes the multi-device
//      story work without the server tracking which device is doing what;
//    - it treats a second `call_offer` for a call it already holds as the
//      duplicate it is (the server replays the offer to a late socket);
//    - it buffers remote candidates until the remote description is set,
//      and local ones until the offer or answer has actually gone out;
//    - it keeps a guard clock on every waiting state, because a server
//      that vanished mid-ring would otherwise leave a phone ringing forever.
//
//  Every OS-specific thing — CallKit, PushKit, the audio session — is
//  behind a seam (CallSystemBridge, CallMediaClient, the closures below),
//  which is what lets the whole machine be tested with fakes.
//

import Foundation
import Observation
import os
// For RTCVideoRenderer alone — the call screen's video surfaces are
// handed through here so they can be attached before media exists.
import WebRTC
import AVFoundation
#if os(iOS)
import AVFAudio
#endif

/// How frames leave: the coordinator owns the socket and implements this.
@MainActor
protocol CallSignaling: AnyObject {
    func sendCallFrame(_ frame: ClientFrame) async throws
}

/// The system's call UI — CallKit on iOS. nil on the Mac, where the app
/// draws its own window and raises its own notification.
///
/// Two directions. The manager REPORTS what happened (an outgoing call was
/// placed, a call connected, a call ended) and it REQUESTS what the user
/// asked for in the app's own UI (answer, hang up) — the request goes to
/// the system, and the system calls back into `systemDidAnswer` /
/// `systemDidEnd`, so the system UI and the app's never disagree about
/// whether a call is up.
@MainActor
protocol CallSystemBridge: AnyObject {
    func reportOutgoing(callID: UUID, peerName: String, isVideo: Bool)
    func reportOutgoingConnecting(callID: UUID)
    func reportOutgoingConnected(callID: UUID)
    /// An incoming call the SOCKET delivered. The push path reports its
    /// own before the manager hears of the call at all. `hasVideo` is the
    /// offer's flag, so the system UI rings with a camera when it should.
    func reportIncoming(callID: UUID, peerName: String, hasVideo: Bool)
    func reportEnded(callID: UUID, reason: CallEndReason)
    func requestAnswer(callID: UUID)
    func requestEnd(callID: UUID)
}

/// Why a call is over. The first six are the wire's reasons; the rest are
/// this client's own, for offers the server refused or a call that never
/// got as far as the wire.
nonisolated enum CallEndReason: String, Equatable, Sendable {
    case hangup
    case decline
    case cancel
    case timeout
    case failed
    case answeredElsewhere = "answered_elsewhere"
    /// `call_busy` (this account is on another call) or `peer_busy`.
    case busy
    /// `peer_unreachable`: no socket and nothing to wake.
    case unreachable
    /// `calls_disabled`, `invalid_call`, or any other refusal.
    case unavailable
    /// `video_calls_disabled`: this server takes voice calls but refuses
    /// video ones. Worth its own words — the generic "Unavailable" reads
    /// as a failure and invites retries against a deliberate setting.
    case videoUnavailable = "video_calls_disabled"
    /// The microphone was refused, so nothing was placed or answered.
    case microphoneDenied = "microphone_denied"

    /// From the wire's spelling; anything unknown is `failed`.
    init(wire: String) {
        self = CallEndReason(rawValue: wire) ?? .failed
    }
}

/// What a VoIP push carries (docs/protocol.md, "Incoming calls"). Parsed
/// here rather than in the PushKit delegate so the parse is testable on
/// every platform.
nonisolated struct IncomingCallPush: Equatable, Sendable {
    let callID: String
    let chatID: Int64
    let fromUserID: Int64
    let callerName: String
    /// Present-and-true on the wire when — and only when — the ringing
    /// call is a VIDEO call (docs/protocol.md, "Incoming calls"); the
    /// woken device rings with a camera UI, `hasVideo` on CallKit
    /// included. Absent means voice.
    let video: Bool

    init(callID: String, chatID: Int64, fromUserID: Int64, callerName: String, video: Bool = false) {
        self.callID = callID
        self.chatID = chatID
        self.fromUserID = fromUserID
        self.callerName = callerName
        self.video = video
    }

    /// Total over the dictionary shapes APNs hands over: numbers arrive as
    /// NSNumber, and a hand-typed test payload may carry them as strings.
    static func parse(_ payload: [AnyHashable: Any]) -> IncomingCallPush? {
        guard payload["kind"] as? String == "call",
              let callID = payload["call_id"] as? String, !callID.isEmpty,
              let chatID = int64(payload["chat_id"]),
              let fromUserID = int64(payload["from_user_id"]) else { return nil }
        let name = (payload["caller_name"] as? String).flatMap { $0.isEmpty ? nil : $0 }
        return IncomingCallPush(
            callID: callID, chatID: chatID, fromUserID: fromUserID,
            callerName: name ?? "", video: bool(payload["video"]))
    }

    private static func int64(_ value: Any?) -> Int64? {
        if let number = value as? NSNumber { return number.int64Value }
        if let string = value as? String { return Int64(string) }
        return nil
    }

    /// APNs hands a JSON `true` over as NSNumber; a hand-typed test
    /// payload may say "true". Anything else — absent included — is voice.
    private static func bool(_ value: Any?) -> Bool {
        if let number = value as? NSNumber { return number.boolValue }
        if let string = value as? String { return string == "true" }
        return false
    }
}

@MainActor @Observable
final class CallManager {

    enum Phase: Equatable, Sendable {
        case idle
        /// Placed; `ringing` once the server has answered `call_ringing`.
        case outgoing(ringing: Bool)
        /// Somebody is ringing this device.
        case incoming
        /// Answered on one side, media not yet up.
        case connecting
        case active(since: Date)
        /// Shown briefly, then back to idle.
        case ended(CallEndReason)
    }

    // `nonisolated`: nested in a @MainActor class it would inherit the
    // actor's isolation, and the wording table compares it off the actor.
    nonisolated enum Direction: Equatable, Sendable {
        case outgoing
        case incoming
    }

    // MARK: - Observable state

    private(set) var phase: Phase = .idle
    private(set) var direction: Direction?
    /// The wire id — a lowercase UUID string, minted here for an outgoing
    /// call and taken from the offer or the push for an incoming one.
    private(set) var callID: String?
    /// The same id as CallKit wants it.
    private(set) var callUUID: UUID?
    private(set) var chatID: Int64?
    private(set) var peerUserID: Int64?
    private(set) var peerName: String = ""
    private(set) var peerAvatarVersion: Int64 = 0
    private(set) var isMuted = false
    /// Starts TRUE on a video call — the audio session's .videoChat mode
    /// routes to the speaker by default (a face on screen means the phone
    /// is not at an ear), and the toggle must draw the state the ear
    /// hears. Seeded wherever `isVideo` is set, mirroring Android's
    /// `audio.begin(video)` pairing of route and state.
    private(set) var isSpeaker = false
    /// The call's KIND (docs/protocol.md, "Video"): set when the call is
    /// placed or when the offer/push arrives, fixed for the call's life.
    /// Cameras toggle below; this never does.
    private(set) var isVideo = false
    /// Whether THIS side's camera is sending. Starts on for a video call
    /// whose camera permission was granted, off when it was denied — the
    /// call proceeds either way, receive-only.
    private(set) var isCameraOn = false
    private(set) var isFrontCamera = true
    /// The camera permission was refused on a video call — a status note
    /// on the call screen, deliberately NOT an end reason.
    private(set) var cameraDenied = false
    /// The far side's picture is actually RENDERING: flips on when the
    /// first decoded frame reaches the remote renderer (the media
    /// client's first-frame relay), never on the track's mere arrival —
    /// a camera-denied far side adds a track and sends nothing. Flipping
    /// back off is best effort (see the media client).
    private(set) var isRemoteVideoActive = false

    /// Nothing is happening — the `.ended` linger counts, so a new call
    /// can start while the last one's reason is still on screen.
    var isIdle: Bool {
        switch phase {
        case .idle, .ended: true
        default: false
        }
    }

    // MARK: - Seams (wired by the composition root; fakes in tests)

    weak var signaling: (any CallSignaling)?
    weak var systemBridge: (any CallSystemBridge)?
    var iceServers: () async throws -> [IceServerDTO] = { [] }
    var makeMediaClient: ([IceServerDTO], _ video: Bool) -> (any CallMediaClient)? = { WebRTCClient(iceServers: $0, video: $1) }
    var requestMicrophone: () async -> Bool = { await CallManager.systemMicrophonePermission() }
    /// The camera's twin of `requestMicrophone` — but a refusal here never
    /// ends a call, it only starts it camera-off (docs/protocol.md,
    /// "Video").
    var requestCamera: () async -> Bool = { await CallManager.systemCameraPermission() }
    /// The camera grant AS IT STANDS, without ever prompting: true or
    /// false once the system has decided, nil while it is undetermined.
    /// The system-answer path keys on this — iOS cannot present a TCC
    /// alert behind CallKit's lock-screen UI, so an undetermined grant
    /// must never be AWAITED there.
    var cameraGrantState: () -> Bool? = { CallManager.systemCameraGrantState() }
    /// Who a user id is, for the screen and the system UI.
    var resolvePeer: (Int64) -> (name: String, avatarVersion: Int64) = { _ in (String(localized: "Someone"), 0) }
    /// Bring the socket up (ChatSyncCoordinator.ensureConnected).
    var ensureConnected: () async -> Void = {}
    /// The call is over — the coordinator decides about the socket.
    var onEnded: () -> Void = {}
    /// Every phase change, in order. The Mac raises and takes down its
    /// incoming-call notification from this, and it has to be a hook
    /// rather than a view's `onChange` because a Mac app with every window
    /// closed still has to ring.
    var onPhaseChange: (Phase) -> Void = { _ in }
    /// The server's ring timeout (45 s); the guards here are looser.
    var ringTimeout: TimeInterval = 45
    /// How much longer than the server's timeout an incoming call may wait
    /// for its offer or its end before this device gives up on its own.
    var guardSlack: TimeInterval = 15
    /// How long `.ended` stays on screen before `.idle`.
    var endedLinger: TimeInterval = 2
    var now: () -> Date = { Date() }

    // MARK: - Private state

    private var media: (any CallMediaClient)?
    private var pendingOfferSDP: String?
    private var acceptRequested = false
    private var remoteDescriptionSet = false
    private var localDescriptionSent = false
    private var bufferedRemoteCandidates: [IceCandidatePayload] = []
    private var bufferedLocalCandidates: [IceCandidatePayload] = []
    private var guardTask: Task<Void, Never>?
    private var lingerTask: Task<Void, Never>?
    /// Inbound candidates are applied in order, one after another.
    private var candidateChain: Task<Void, Never>?
    /// Ids of calls that ended recently, so a late frame for one is
    /// ignored rather than mistaken for a new call.
    private var recentlyEnded: [String] = []

    /// The async step most recently started by a UI action or a frame.
    ///
    /// Test seam, the `pendingDelivery` idiom: `startCall` returns as soon
    /// as the state flips, and the offer goes out in a detached step. A
    /// test asserting on the frames sent has to await this first. Nothing
    /// in the app reads it.
    private(set) var pendingWork: Task<Void, Never>?

    init() {}

    // MARK: - UI actions

    /// Place a call to the peer of a direct chat. False when a call is
    /// already in progress (the server would answer `call_busy` too).
    @discardableResult
    func startCall(chatID: Int64, peerUserID: Int64, video: Bool = false) -> Bool {
        guard isIdle else { return false }
        resetToIdle()
        let uuid = UUID()
        let id = uuid.uuidString.lowercased()
        let peer = resolvePeer(peerUserID)
        self.callUUID = uuid
        self.callID = id
        self.chatID = chatID
        self.peerUserID = peerUserID
        self.peerName = peer.name
        self.peerAvatarVersion = peer.avatarVersion
        self.direction = .outgoing
        self.isVideo = video
        // The route .videoChat gives by default, mirrored into the state
        // (see isSpeaker's declaration).
        self.isSpeaker = video
        transition(to: .outgoing(ringing: false))
        systemBridge?.reportOutgoing(callID: uuid, peerName: peer.name, isVideo: video)
        pendingWork = Task { await self.placeOutgoing(id: id, chatID: chatID) }
        return true
    }

    /// Accept from the app's own UI. Routed through the system when there
    /// is one, so CallKit's idea of the call stays in step.
    ///
    /// On a video call whose camera grant is still undetermined this asks
    /// FIRST and accepts after: the app is foreground (the user just
    /// tapped its Accept button), so the alert can actually show, and the
    /// answer then starts with the camera truthfully on or off. The
    /// SYSTEM answer path (CallKit → systemDidAnswer) never comes through
    /// here and never waits — see answer().
    func acceptIncoming() {
        guard case .incoming = phase else { return }
        if isVideo, cameraGrantState() == nil {
            let id = callID
            pendingWork = Task {
                _ = await self.requestCamera()
                guard id == self.callID, case .incoming = self.phase else { return }
                self.routeAccept()
            }
            return
        }
        routeAccept()
    }

    private func routeAccept() {
        if let systemBridge, let callUUID {
            systemBridge.requestAnswer(callID: callUUID)
        } else {
            performAccept()
        }
    }

    func declineIncoming() {
        guard case .incoming = phase, let callID else { return }
        sendEndFrame(callID: callID, reason: .decline)
        finish(.decline)
    }

    /// Hang up, cancel, or decline — whichever the phase makes it.
    func hangUp() {
        guard !isIdle else { return }
        if let systemBridge, let callUUID {
            systemBridge.requestEnd(callID: callUUID)
        } else {
            performHangUp(reportToSystem: false)
        }
    }

    func toggleMute() {
        isMuted.toggle()
        media?.setMuted(isMuted)
    }

    func toggleSpeaker() {
        isSpeaker.toggle()
        media?.setSpeaker(isSpeaker)
    }

    /// Camera on/off, on a video call. The call's kind never changes —
    /// the track is enabled or disabled, no renegotiation, no frame.
    ///
    /// Turning ON consults the grant: with it granted the camera starts
    /// straight away (and a stale denied note from before a Settings
    /// change is cleared); denied or undetermined goes back through the
    /// requestCamera seam — a grant turns the camera on, a refusal keeps
    /// it off with the existing denied note. Mirrors Android. Without
    /// this, a denied user’s tap flipped `isCameraOn` and showed a
    /// permanently black self-tile under a footnote still claiming the
    /// call is voice-only.
    func toggleCamera() {
        guard isVideo else { return }
        if isCameraOn {
            isCameraOn = false
            media?.setCameraEnabled(false)
            return
        }
        if cameraGrantState() == true {
            cameraDenied = false
            isCameraOn = true
            media?.setCameraEnabled(true)
            return
        }
        let id = callID
        pendingWork = Task {
            let granted = await self.requestCamera()
            guard id == self.callID, self.isVideo, !self.isIdle, !self.isCameraOn else { return }
            self.cameraDenied = !granted
            if granted {
                self.isCameraOn = true
                self.media?.setCameraEnabled(true)
            }
        }
    }

    func flipCamera() {
        guard isVideo, isCameraOn else { return }
        isFrontCamera.toggle()
        media?.flipCamera()
    }

    // MARK: - Video renderers

    /// The call screen's video surfaces, kept here so a renderer set
    /// while media does not exist yet (the screen appears before the
    /// answer) is attached the moment it does — and re-attached to a
    /// window reopened mid-call.
    private var localVideoRenderer: (any RTCVideoRenderer)?
    private var remoteVideoRenderer: (any RTCVideoRenderer)?

    func setLocalVideoRenderer(_ renderer: (any RTCVideoRenderer)?) {
        localVideoRenderer = renderer
        media?.setLocalVideoRenderer(renderer)
    }

    func setRemoteVideoRenderer(_ renderer: (any RTCVideoRenderer)?) {
        remoteVideoRenderer = renderer
        media?.setRemoteVideoRenderer(renderer)
    }

    /// After `media` is created on a video call: the camera state the
    /// permissions decided, and whatever renderers the screen has
    /// already offered.
    private func applyVideoState() {
        guard isVideo, let media else { return }
        media.setCameraEnabled(isCameraOn)
        if let localVideoRenderer { media.setLocalVideoRenderer(localVideoRenderer) }
        if let remoteVideoRenderer { media.setRemoteVideoRenderer(remoteVideoRenderer) }
    }

    // MARK: - System callbacks (CallKit actions)

    func systemDidAnswer() {
        performAccept()
    }

    func systemDidEnd() {
        performHangUp(reportToSystem: false)
    }

    func systemDidSetMuted(_ muted: Bool) {
        isMuted = muted
        media?.setMuted(muted)
    }

    // MARK: - The push path

    /// A VoIP push says somebody is ringing this device. The system UI
    /// has ALREADY been shown by the registrar (iOS requires it before the
    /// push completes); this records the call and brings the socket up so
    /// the replayed `call_offer` arrives (docs/protocol.md, "Late
    /// arrivals"). False when the device is busy — the registrar then ends
    /// the call it reported.
    @discardableResult
    func handleIncomingPush(_ push: IncomingCallPush) -> Bool {
        if push.callID == callID { return true }
        guard isIdle, !recentlyEnded.contains(push.callID) else { return false }
        resetToIdle()
        beginIncoming(
            callID: push.callID, chatID: push.chatID, fromUserID: push.fromUserID,
            fallbackName: push.callerName, offerSDP: nil, video: push.video)
        // A generous guard: the socket has to come up and the server has to
        // replay the offer, or say the call is over. If neither happens
        // before the server's own timeout would have, give up.
        startGuard(seconds: ringTimeout + guardSlack, reason: .timeout)
        pendingWork = Task { await self.ensureConnected() }
        return true
    }

    // MARK: - Inbound frames

    func handle(frame: ServerFrame) {
        switch frame {
        case .callOffer(let payload):
            handleOffer(payload)

        case .callRinging(let id):
            guard id == callID, case .outgoing = phase else { return }
            transition(to: .outgoing(ringing: true))

        case .callAnswer(let id, let sdp):
            guard id == callID, case .outgoing = phase, let media else { return }
            transition(to: .connecting)
            if let callUUID { systemBridge?.reportOutgoingConnecting(callID: callUUID) }
            // Media has its own clock now: a connection that never comes up
            // is `failed`, not a call that rang out.
            startGuard(seconds: 30, reason: .failed, sendsFrame: true)
            pendingWork = Task {
                do {
                    try await media.setRemoteDescription(sdp: sdp, isOffer: false)
                    guard id == self.callID else { return }
                    self.remoteDescriptionSet = true
                    self.flushRemoteCandidates()
                } catch {
                    AppLog.socket.error("Applying the answer failed: \(String(describing: error))")
                    guard id == self.callID else { return }
                    self.sendEndFrame(callID: id, reason: .failed)
                    self.finish(.failed)
                }
            }

        case .callIce(let id, let candidate):
            guard id == callID else { return }
            if remoteDescriptionSet, media != nil {
                enqueueRemoteCandidate(candidate)
            } else {
                bufferedRemoteCandidates.append(candidate)
            }

        case .callEnd(let id, let reason):
            guard id == callID, !isIdle else { return }
            finish(CallEndReason(wire: reason))

        case .error(let code, _, _, let id):
            guard let id, id == callID, !isIdle else { return }
            let reason: CallEndReason
            switch code {
            case "call_busy", "peer_busy": reason = .busy
            case "peer_unreachable": reason = .unreachable
            case "video_calls_disabled": reason = .videoUnavailable
            default: reason = .unavailable
            }
            finish(reason)

        default:
            break
        }
    }

    // MARK: - Outgoing

    private func placeOutgoing(id: String, chatID: Int64) async {
        guard await requestMicrophone() else {
            guard id == callID else { return }
            finish(.microphoneDenied)
            return
        }
        if isVideo {
            // A denied camera still PLACES the call (docs/protocol.md,
            // "Video"): the SDP negotiates the video receive-only and the
            // far side's picture still shows. Microphone rules above are
            // unchanged — only the mic can end a call here.
            let granted = await requestCamera()
            guard id == callID else { return }
            isCameraOn = granted
            cameraDenied = !granted
        }
        await ensureConnected()
        guard id == callID else { return }
        do {
            let servers = try await iceServers()
            guard id == callID else { return }
            guard let media = makeMediaClient(servers, isVideo) else {
                throw CallSetupError.noMedia
            }
            self.media = media
            media.delegate = self
            applyVideoState()
            let sdp = try await media.createOffer()
            guard id == callID else { return }
            try await send(.callOffer(callID: id, chatID: chatID, sdp: sdp, video: isVideo))
            guard id == callID else { return }
            localDescriptionSent = true
            flushLocalCandidates()
            // Twice the server's own timeout: the server's `timeout` is the
            // ordinary end of an unanswered call; this only catches a
            // server that went away.
            startGuard(seconds: ringTimeout * 2, reason: .timeout)
        } catch {
            AppLog.socket.error("Placing the call failed: \(String(describing: error))")
            guard id == callID else { return }
            sendEndFrame(callID: id, reason: .cancel)
            finish(.failed)
        }
    }

    // MARK: - Incoming

    private func handleOffer(_ payload: CallOfferPayload) {
        if payload.callID == callID {
            // The replay, or the live copy behind a push: the same call. It
            // is news only when the push got here first and this is the
            // first time the SDP is seen.
            guard direction == .incoming, pendingOfferSDP == nil else { return }
            pendingOfferSDP = payload.sdp
            // The push and the offer say the same thing about the call's
            // kind (it is fixed at placement); belt and braces for a push
            // whose flag was lost somewhere.
            if payload.video {
                isVideo = true
                isSpeaker = true
            }
            if acceptRequested { answerPendingOffer() }
            return
        }
        guard isIdle, !recentlyEnded.contains(payload.callID) else {
            // The server refuses to ring somebody who is busy, so this is a
            // frame for a call that ended on this side moments ago.
            return
        }
        resetToIdle()
        beginIncoming(
            callID: payload.callID, chatID: payload.chatID, fromUserID: payload.fromUserID,
            fallbackName: "", offerSDP: payload.sdp, video: payload.video)
        if let callUUID { systemBridge?.reportIncoming(callID: callUUID, peerName: peerName, hasVideo: isVideo) }
        startGuard(seconds: ringTimeout + guardSlack, reason: .timeout)
    }

    private func beginIncoming(callID: String, chatID: Int64, fromUserID: Int64, fallbackName: String, offerSDP: String?, video: Bool) {
        let peer = resolvePeer(fromUserID)
        self.callID = callID
        self.callUUID = UUID(uuidString: callID) ?? UUID()
        self.chatID = chatID
        self.peerUserID = fromUserID
        self.peerName = peer.name == String(localized: "Someone") && !fallbackName.isEmpty ? fallbackName : peer.name
        self.peerAvatarVersion = peer.avatarVersion
        self.direction = .incoming
        self.pendingOfferSDP = offerSDP
        self.isVideo = video
        self.isSpeaker = video
        transition(to: .incoming)
    }

    private func performAccept() {
        guard case .incoming = phase else { return }
        acceptRequested = true
        transition(to: .connecting)
        // No offer within this window means the server never replayed it
        // and never said the call ended either — give up rather than show
        // "Connecting…" forever.
        startGuard(seconds: 30, reason: .failed, sendsFrame: true)
        if pendingOfferSDP != nil { answerPendingOffer() }
    }

    private func answerPendingOffer() {
        guard let id = callID, let offer = pendingOfferSDP else { return }
        pendingWork = Task { await self.answer(id: id, offer: offer) }
    }

    private func answer(id: String, offer: String) async {
        guard await requestMicrophone() else {
            guard id == callID else { return }
            sendEndFrame(callID: id, reason: .decline)
            finish(.microphoneDenied)
            return
        }
        // A callee whose camera is denied still ANSWERS a video call —
        // camera off, microphone rules unchanged (docs/protocol.md,
        // "Video"). Refusing the call outright would turn a privacy
        // setting into missed calls.
        //
        // And the camera is never AWAITED here: this path is also the
        // CallKit system answer, where the app may be waking locked in
        // the background — AVCaptureDevice.requestAccess would suspend on
        // an alert iOS cannot show, no call_answer would go out, and the
        // 30 s accept guard would kill an ANSWERED call as failed. A
        // grant the system has already decided is read synchronously; an
        // undetermined one answers camera-off NOW and asks as a follow-up
        // below. (The in-app accept prompts BEFORE routing here — see
        // acceptIncoming.)
        var cameraUndetermined = false
        if isVideo {
            let known = cameraGrantState()
            isCameraOn = known == true
            cameraDenied = known == false
            cameraUndetermined = known == nil
        }
        do {
            let servers = try await iceServers()
            guard id == callID else { return }
            guard let media = makeMediaClient(servers, isVideo) else {
                throw CallSetupError.noMedia
            }
            self.media = media
            media.delegate = self
            applyVideoState()
            try await media.setRemoteDescription(sdp: offer, isOffer: true)
            guard id == callID else { return }
            remoteDescriptionSet = true
            flushRemoteCandidates()
            let sdp = try await media.createAnswer()
            guard id == callID else { return }
            try await send(.callAnswer(callID: id, sdp: sdp))
            guard id == callID else { return }
            localDescriptionSent = true
            flushLocalCandidates()
            if cameraUndetermined { requestCameraLate(id: id) }
        } catch {
            AppLog.socket.error("Answering the call failed: \(String(describing: error))")
            guard id == callID else { return }
            sendEndFrame(callID: id, reason: .failed)
            finish(.failed)
        }
    }

    /// The follow-up half of the system-answer path: the SDP answer is
    /// out and the call is live camera-off; now the prompt can wait for
    /// the person as long as it likes. A grant turns the camera on
    /// exactly as the user toggling it on would.
    private func requestCameraLate(id: String) {
        Task {
            let granted = await self.requestCamera()
            guard id == self.callID, self.isVideo, !self.isIdle else { return }
            self.cameraDenied = !granted
            if granted, !self.isCameraOn {
                self.isCameraOn = true
                self.media?.setCameraEnabled(true)
            }
        }
    }

    // MARK: - Ending

    private func performHangUp(reportToSystem: Bool) {
        guard let callID else { return }
        let reason: CallEndReason
        switch phase {
        case .outgoing: reason = .cancel
        case .incoming: reason = .decline
        case .connecting, .active: reason = .hangup
        case .idle, .ended: return
        }
        sendEndFrame(callID: callID, reason: reason)
        finish(reason, reportToSystem: reportToSystem)
    }

    /// Tear the call down and show why. `reportToSystem` is false when the
    /// system already knows — its own end action is what got us here.
    private func finish(_ reason: CallEndReason, reportToSystem: Bool = true) {
        guard !isIdle else { return }
        guardTask?.cancel()
        guardTask = nil
        media?.close()
        media = nil
        if reportToSystem, let callUUID {
            systemBridge?.reportEnded(callID: callUUID, reason: reason)
        }
        if let callID {
            recentlyEnded.append(callID)
            if recentlyEnded.count > 16 { recentlyEnded.removeFirst() }
        }
        transition(to: .ended(reason))
        onEnded()
        let linger = endedLinger
        lingerTask?.cancel()
        lingerTask = Task {
            if linger > 0 {
                try? await Task.sleep(for: .seconds(linger))
            }
            guard !Task.isCancelled, case .ended = self.phase else { return }
            self.resetToIdle()
        }
    }

    private func transition(to next: Phase) {
        guard phase != next else { return }
        phase = next
        onPhaseChange(next)
    }

    private func resetToIdle() {
        lingerTask?.cancel()
        lingerTask = nil
        guardTask?.cancel()
        guardTask = nil
        media?.close()
        media = nil
        transition(to: .idle)
        direction = nil
        callID = nil
        callUUID = nil
        chatID = nil
        peerUserID = nil
        peerName = ""
        peerAvatarVersion = 0
        isMuted = false
        isSpeaker = false
        isVideo = false
        isCameraOn = false
        isFrontCamera = true
        cameraDenied = false
        isRemoteVideoActive = false
        // The renderers are deliberately NOT cleared: they follow the call
        // SCREEN's lifecycle, not the call's — the surface detaches itself
        // (nil) when it is dismantled, and a renderer offered just before
        // startCall's own reset must survive into the next call.
        pendingOfferSDP = nil
        acceptRequested = false
        remoteDescriptionSet = false
        localDescriptionSent = false
        bufferedRemoteCandidates = []
        bufferedLocalCandidates = []
    }

    /// Awaits the step in flight, the candidate queue and the linger, so a
    /// test can see the settled state without sleeping.
    func settle() async {
        await pendingWork?.value
        await candidateChain?.value
        await lingerTask?.value
    }

    // MARK: - Guards

    private func startGuard(seconds: TimeInterval, reason: CallEndReason, sendsFrame: Bool = false) {
        guardTask?.cancel()
        let id = callID
        guardTask = Task {
            try? await Task.sleep(for: .seconds(seconds))
            guard !Task.isCancelled, let id, id == self.callID, !self.isIdle else { return }
            AppLog.socket.info("Call guard fired: \(reason.rawValue, privacy: .public)")
            if sendsFrame { self.sendEndFrame(callID: id, reason: .failed) }
            self.finish(reason)
        }
    }

    // MARK: - Candidates

    private func flushRemoteCandidates() {
        let buffered = bufferedRemoteCandidates
        bufferedRemoteCandidates = []
        for candidate in buffered { enqueueRemoteCandidate(candidate) }
    }

    private func enqueueRemoteCandidate(_ candidate: IceCandidatePayload) {
        guard let media else { return }
        let previous = candidateChain
        candidateChain = Task {
            await previous?.value
            do {
                try await media.addRemoteCandidate(candidate)
            } catch {
                AppLog.socket.info("Remote candidate refused: \(String(describing: error))")
            }
        }
    }

    private func flushLocalCandidates() {
        guard let callID else { return }
        let buffered = bufferedLocalCandidates
        bufferedLocalCandidates = []
        for candidate in buffered {
            Task { try? await self.send(.callIce(callID: callID, candidate: candidate)) }
        }
    }

    // MARK: - Sending

    private enum CallSetupError: Error {
        case noMedia
        case noSignaling
    }

    /// A frame over the socket, waiting briefly for a socket that is
    /// still coming up — `ensureConnected` returns before the upgrade has
    /// happened, and the first frame of a call is often the race.
    private func send(_ frame: ClientFrame) async throws {
        guard let signaling else { throw CallSetupError.noSignaling }
        var attempt = 0
        while true {
            do {
                try await signaling.sendCallFrame(frame)
                return
            } catch let error as SocketError where error == .notConnected && attempt < 10 {
                attempt += 1
                try await Task.sleep(for: .milliseconds(500))
            }
        }
    }

    /// Best effort: the call is ending on this side whatever happens to
    /// the frame, and the server has its own clocks for the rest.
    private func sendEndFrame(callID: String, reason: CallEndReason) {
        Task { try? await self.send(.callEnd(callID: callID, reason: reason.rawValue)) }
    }

    // MARK: - Microphone

    static func systemMicrophonePermission() async -> Bool {
        #if os(iOS)
        switch AVAudioApplication.shared.recordPermission {
        case .granted:
            return true
        case .denied:
            return false
        default:
            return await withCheckedContinuation { continuation in
                AVAudioApplication.requestRecordPermission { granted in
                    continuation.resume(returning: granted)
                }
            }
        }
        #else
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized:
            return true
        case .notDetermined:
            return await AVCaptureDevice.requestAccess(for: .audio)
        default:
            return false
        }
        #endif
    }

    // MARK: - Camera

    /// The grant as it stands, no prompt ever: the non-suspending probe
    /// behind the `cameraGrantState` seam.
    static func systemCameraGrantState() -> Bool? {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized: true
        case .notDetermined: nil
        default: false
        }
    }

    /// The camera's mirror of the microphone helper — AVCaptureDevice on
    /// BOTH platforms (iOS has no AVAudioApplication equivalent for
    /// video). Its answer is never a call's end: a denial means the call
    /// proceeds camera-off.
    static func systemCameraPermission() async -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            return true
        case .notDetermined:
            return await AVCaptureDevice.requestAccess(for: .video)
        default:
            return false
        }
    }
}

// MARK: - Media callbacks

extension CallManager: CallMediaClientDelegate {
    func mediaClient(_ client: any CallMediaClient, didGatherLocalCandidate candidate: IceCandidatePayload) {
        guard client === media, let callID else { return }
        if localDescriptionSent {
            Task { try? await self.send(.callIce(callID: callID, candidate: candidate)) }
        } else {
            bufferedLocalCandidates.append(candidate)
        }
    }

    func mediaClient(_ client: any CallMediaClient, remoteVideoActiveChanged active: Bool) {
        guard client === media else { return }
        // true means a frame really rendered (the client's first-frame
        // relay); false is the best-effort track removal. Stored as is —
        // the honesty lives in WHEN the client fires this.
        isRemoteVideoActive = active
    }

    func mediaClient(_ client: any CallMediaClient, connectionStateChanged state: CallMediaConnectionState) {
        guard client === media, let callID else { return }
        switch state {
        case .connected:
            if case .connecting = phase {
                guardTask?.cancel()
                guardTask = nil
                transition(to: .active(since: now()))
                if direction == .outgoing, let callUUID {
                    systemBridge?.reportOutgoingConnected(callID: callUUID)
                }
                // The connected-but-silent fallback: give CallKit a moment
                // to activate the audio session, then make sure it did.
                let id = callID
                Task {
                    try? await Task.sleep(for: .seconds(1.5))
                    guard id == self.callID, case .active = self.phase else { return }
                    self.media?.ensureAudioRunning()
                }
            }
        case .failed:
            // WebRTC has given up; a `disconnected` before it may still
            // recover and is deliberately not acted on.
            sendEndFrame(callID: callID, reason: .failed)
            finish(.failed)
        case .new, .connecting, .disconnected, .closed:
            break
        }
    }
}
