//
//  WebRTCClient.swift
//  FamilyConnect
//
//  The real CallMediaClient: one audio-only RTCPeerConnection over the
//  stasel/WebRTC binary framework (the project's first package). The
//  audio never touches the family server — Opus over DTLS-SRTP goes
//  straight between the two devices, and the only thing the server ever
//  carries is the JSON that sets it up (docs/protocol.md, "Voice calls").
//
//  Threading: WebRTC calls its delegate on its own signalling thread.
//  Every callback hops to the main actor before touching anything, which
//  is why the delegate methods below are `nonisolated` and do nothing but
//  hop.
//
//  Audio on iOS is MANUAL: CallKit owns the AVAudioSession's activation,
//  and WebRTC is told about it from the CXProvider delegate
//  (`RTCAudioSession.audioSessionDidActivate`) rather than left to
//  activate the session itself — the two fighting over it is the classic
//  "call connects but nobody hears anything". macOS has no RTCAudioSession
//  at all, and no CallKit; the framework drives Core Audio directly.
//

import Foundation
import os
import WebRTC

@MainActor
final class WebRTCClient: NSObject, CallMediaClient {

    /// One factory for the process — it owns the audio device module,
    /// and two of them would be two things asking for the microphone.
    private static let factory: RTCPeerConnectionFactory = {
        RTCInitializeSSL()
        #if os(iOS)
        // Manual audio: see the file header. Set before the first peer
        // connection exists, so nothing activates the session on its own.
        let session = RTCAudioSession.sharedInstance()
        session.useManualAudio = true
        session.isAudioEnabled = false
        #endif
        // The default video codec factories (H.264/VP8/VP9/AV1). Harmless
        // for a voice call — no video track, nothing to encode — and what
        // makes a video call's SDP carry real codecs.
        return RTCPeerConnectionFactory(
            encoderFactory: RTCDefaultVideoEncoderFactory(),
            decoderFactory: RTCDefaultVideoDecoderFactory())
    }()

    weak var delegate: (any CallMediaClientDelegate)?

    private let connection: RTCPeerConnection
    private let audioTrack: RTCAudioTrack

    // MARK: Video state (all nil/false on a voice call)

    /// Whether this connection was built FOR a video call — fixed at
    /// init, like the call's kind on the wire (docs/protocol.md, "Video").
    private let isVideoCall: Bool
    private let videoTrack: RTCVideoTrack?
    private let capturer: RTCCameraVideoCapturer?
    private var usingFrontCamera = true
    private var localRenderer: (any RTCVideoRenderer)?
    /// The far side's renderer, wrapped in the first-frame relay below.
    /// The RELAY — never the bare renderer — is what attaches to the
    /// track, so "the far side's picture is up" can mean a frame actually
    /// rendered rather than a track that merely exists.
    private var remoteRelay: RemoteFirstFrameRelay?
    /// The far side's track, from the unified-plan `didAdd rtpReceiver`
    /// delegate callback; held so a renderer set late still attaches.
    private var remoteVideoTrack: RTCVideoTrack?
    /// close() happened. Read by the async hops (capture restart, first
    /// frame) that can land after the call is over — CallManager's
    /// candidate tasks keep this client alive past `media = nil`.
    private var closed = false

    #if os(iOS)
    /// The route the manager last asked for — kept, because the session
    /// gets re-configured underneath the override (see init) and the
    /// answer each time is to say this again.
    private var speakerOn = false
    private var routeObserver: (any NSObjectProtocol)?
    /// Re-applications driven by route-change notifications, bounded so
    /// a route the hardware refuses (a wired headset ignores the speaker
    /// override) can never become a notification loop.
    private var notifiedReapplies = 0
    private static let maxNotifiedReapplies = 6
    #endif

    /// Fails only when libwebrtc refuses the configuration, which with an
    /// audio/video unified-plan connection it does not.
    init?(iceServers: [IceServerDTO], video: Bool = false) {
        let configuration = RTCConfiguration()
        configuration.iceServers = iceServers.map {
            RTCIceServer(urlStrings: $0.urls, username: $0.username, credential: $0.credential)
        }
        configuration.sdpSemantics = .unifiedPlan
        // Keep gathering after the offer goes out: candidates trickle as
        // `call_ice` frames, and a network change mid-call yields new ones.
        configuration.continualGatheringPolicy = .gatherContinually
        let constraints = RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil)
        guard let connection = Self.factory.peerConnection(
            with: configuration, constraints: constraints, delegate: nil) else {
            return nil
        }
        self.connection = connection
        let source = Self.factory.audioSource(with: RTCMediaConstraints(
            mandatoryConstraints: nil,
            optionalConstraints: nil))
        let track = Self.factory.audioTrack(with: source, trackId: "audio0")
        self.audioTrack = track
        self.isVideoCall = video
        if video {
            // The video m-line exists from the offer onward whatever the
            // camera permission says — a denied camera negotiates the
            // video receive-only and the far side's picture still shows.
            // Capture itself starts in setCameraEnabled(true), which the
            // manager calls once it knows whether the camera was granted.
            let videoSource = Self.factory.videoSource()
            self.capturer = RTCCameraVideoCapturer(delegate: videoSource)
            self.videoTrack = Self.factory.videoTrack(with: videoSource, trackId: "video0")
        } else {
            self.capturer = nil
            self.videoTrack = nil
        }
        super.init()
        #if os(iOS)
        // WebRTC's audio unit writes its own configuration into the
        // session when it starts — after CallKit's didActivate, and again
        // after an interruption — and every category/mode write drops the
        // output override. The stock configuration is also .voiceChat,
        // which on a VIDEO call put the earpiece back moments after the
        // speaker was chosen (configureAudioSessionForCall now keeps that
        // configuration equal to ours, so the write changes nothing). The
        // category-change notification is the one signal that a write
        // happened; the answer is to say the wanted route again.
        routeObserver = NotificationCenter.default.addObserver(
            forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main
        ) { [weak self] note in
            let raw = note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt
            guard let raw, let reason = AVAudioSession.RouteChangeReason(rawValue: raw) else { return }
            // Category (or mode) writes, and a headset pulled out: both
            // drop the override (Apple: it "remains in effect only until
            // the current route changes"). NEVER on a device plugged IN —
            // forcing the speaker there would defeat the headphones the
            // person just reached for.
            guard reason == .categoryChange || reason == .oldDeviceUnavailable else { return }
            Task { @MainActor [weak self] in self?.reapplySpeakerRouteAfterNotification() }
        }
        #endif
        connection.add(track, streamIds: ["stream0"])
        if let videoTrack { connection.add(videoTrack, streamIds: ["stream0"]) }
        connection.delegate = self
    }

    private var offerAnswerConstraints: RTCMediaConstraints {
        RTCMediaConstraints(
            mandatoryConstraints: [
                kRTCMediaConstraintsOfferToReceiveAudio: kRTCMediaConstraintsValueTrue,
                // On a video call BOTH directions are negotiated — that is
                // what lets a camera-denied side still receive.
                kRTCMediaConstraintsOfferToReceiveVideo:
                    isVideoCall ? kRTCMediaConstraintsValueTrue : kRTCMediaConstraintsValueFalse,
            ],
            optionalConstraints: nil)
    }

    func createOffer() async throws -> String {
        let offer = try await connection.offer(for: offerAnswerConstraints)
        try await connection.setLocalDescription(offer)
        return offer.sdp
    }

    func createAnswer() async throws -> String {
        let answer = try await connection.answer(for: offerAnswerConstraints)
        try await connection.setLocalDescription(answer)
        return answer.sdp
    }

    func setRemoteDescription(sdp: String, isOffer: Bool) async throws {
        let description = RTCSessionDescription(type: isOffer ? .offer : .answer, sdp: sdp)
        try await connection.setRemoteDescription(description)
    }

    func addRemoteCandidate(_ candidate: IceCandidatePayload) async throws {
        let rtcCandidate = RTCIceCandidate(
            sdp: candidate.candidate,
            sdpMLineIndex: candidate.sdpMLineIndex ?? 0,
            sdpMid: candidate.sdpMid)
        try await connection.add(rtcCandidate)
    }

    func setMuted(_ muted: Bool) {
        audioTrack.isEnabled = !muted
    }

    // MARK: - Camera (docs/protocol.md, "Video": cameras toggle, the
    // call's kind does not)

    func setCameraEnabled(_ enabled: Bool) {
        guard let videoTrack else { return }
        // Both halves matter: the disabled track is what the far side
        // sees (the stream stops — no renegotiation, no frame on the
        // wire), and stopping capture is what turns the camera LIGHT off —
        // a "muted" camera that keeps the green dot lit reads as spying.
        // Battery agrees.
        videoTrack.isEnabled = enabled
        if enabled {
            startCapture(front: usingFrontCamera)
        } else {
            capturer?.stopCapture()
        }
    }

    func flipCamera() {
        // On the Mac there is usually one camera and nothing to flip to;
        // guard on the device list rather than the platform, so an iPhone
        // with a broken back camera is a no-op too.
        guard let capturer, RTCCameraVideoCapturer.captureDevices().count > 1 else { return }
        usingFrontCamera.toggle()
        let front = usingFrontCamera
        // Capture restarts on the other device; stop first, and hop back
        // to the main actor because the completion arrives on an
        // AVFoundation queue. The completion can land AFTER the call was
        // closed or the camera turned off (stopCapture waits for the
        // session to really stop, tens of ms) — restarting then would
        // relight the camera indicator for a call that no longer wants
        // it, the exact "muted camera that reads as spying" failure the
        // comment above warns about. So the restart is guarded on the
        // same client (weak self), not closed, and the track still
        // enabled (setCameraEnabled(false) disables it first).
        capturer.stopCapture { [weak self] in
            Task { @MainActor [weak self] in
                guard let self, !self.closed, self.videoTrack?.isEnabled == true else { return }
                self.startCapture(front: front)
            }
        }
    }

    /// The standard capture-start dance: pick the camera by position,
    /// then of the formats RTCCameraVideoCapturer supports for it, the
    /// one whose dimensions are closest to 1280x720, then the highest
    /// frame rate that format offers capped at 30 — 720p30 is the sweet
    /// spot between "looks like a video call" and a phone that stays cool.
    private func startCapture(front: Bool) {
        // The closed check repeats flipCamera's guard on purpose: every
        // path into a capture start must be inert on a closed connection.
        guard !closed, let capturer else { return }
        let devices = RTCCameraVideoCapturer.captureDevices()
        let wanted: AVCaptureDevice.Position = front ? .front : .back
        // The Mac's built-in camera reports .unspecified; falling back to
        // the first device is what makes this work there at all.
        guard let device = devices.first(where: { $0.position == wanted }) ?? devices.first else {
            AppLog.call.warning("No capture device; the call stays receive-only")
            return
        }
        var best: AVCaptureDevice.Format?
        var bestDelta = Int32.max
        for format in RTCCameraVideoCapturer.supportedFormats(for: device) {
            let size = CMVideoFormatDescriptionGetDimensions(format.formatDescription)
            let delta = abs(size.width - 1280) + abs(size.height - 720)
            if delta < bestDelta {
                bestDelta = delta
                best = format
            }
        }
        guard let format = best else { return }
        let maxRate = format.videoSupportedFrameRateRanges.map(\.maxFrameRate).max() ?? 30
        capturer.startCapture(with: device, format: format, fps: min(Int(maxRate), 30))
    }

    // MARK: - Renderers (settable before or after the tracks exist)

    func setLocalVideoRenderer(_ renderer: (any RTCVideoRenderer)?) {
        if let localRenderer { videoTrack?.remove(localRenderer) }
        localRenderer = renderer
        if let renderer { videoTrack?.add(renderer) }
    }

    func detachLocalVideoRenderer(_ renderer: any RTCVideoRenderer) -> Bool {
        guard let localRenderer, localRenderer === renderer else { return false }
        videoTrack?.remove(localRenderer)
        self.localRenderer = nil
        return true
    }

    func setRemoteVideoRenderer(_ renderer: (any RTCVideoRenderer)?) {
        if let remoteRelay { remoteVideoTrack?.remove(remoteRelay) }
        remoteRelay = renderer.map(makeRemoteRelay)
        if let remoteRelay { remoteVideoTrack?.add(remoteRelay) }
        AppLog.call.info(
            "\(AppLog.CallVideo.tag, privacy: .public) client remote renderer=\(AppLog.CallVideo.id(renderer), privacy: .public) trackPresent=\(self.remoteVideoTrack != nil, privacy: .public) attached=\(renderer != nil && self.remoteVideoTrack != nil, privacy: .public)"
        )
    }

    /// Detach only the relay that wraps THIS renderer — a dismantle from
    /// a surface that has already been replaced must not unhook the live
    /// one (see CallVideoView, and issue #38).
    func detachRemoteVideoRenderer(_ renderer: any RTCVideoRenderer) -> Bool {
        guard let relay = remoteRelay, relay.wraps(renderer) else {
            AppLog.call.error(
                "\(AppLog.CallVideo.tag, privacy: .public) client remote detach IGNORED renderer=\(AppLog.CallVideo.id(renderer), privacy: .public) (stale surface; the live one keeps drawing)"
            )
            return false
        }
        remoteVideoTrack?.remove(relay)
        remoteRelay = nil
        AppLog.call.info(
            "\(AppLog.CallVideo.tag, privacy: .public) client remote renderer detached renderer=\(AppLog.CallVideo.id(renderer), privacy: .public)"
        )
        return true
    }

    /// Wraps the screen's remote renderer so the FIRST decoded frame — not
    /// the track's arrival — is what reports the far side's video active.
    /// A fresh relay per attachment: a surface re-attached mid-call (a Mac
    /// window reopened) re-reports on its own next frame, which is again
    /// the honest answer.
    private func makeRemoteRelay(around renderer: any RTCVideoRenderer) -> RemoteFirstFrameRelay {
        RemoteFirstFrameRelay(wrapping: renderer) { [weak self] in
            // Already hopped to the main actor by the relay.
            guard let self, !self.closed else { return }
            self.delegate?.mediaClient(self, remoteVideoActiveChanged: true)
        }
    }

    func setSpeaker(_ enabled: Bool) {
        #if os(iOS)
        speakerOn = enabled
        // An explicit choice opens a fresh budget: the mode write a
        // video-call toggle makes is itself a category change, so each
        // toggle would otherwise spend one of the six on its own echo.
        notifiedReapplies = 0
        applySpeakerRoute()
        #endif
    }

    #if os(iOS)
    private func reapplySpeakerRouteAfterNotification() {
        guard !closed, notifiedReapplies < Self.maxNotifiedReapplies else { return }
        notifiedReapplies += 1
        applySpeakerRoute()
    }

    /// The route the manager wants, said to the session. Idempotent:
    /// called from setSpeaker, and again whenever the session was
    /// re-configured underneath the override (see init).
    private func applySpeakerRoute() {
        let enabled = speakerOn
        let session = RTCAudioSession.sharedInstance()
        session.lockForConfiguration()
        defer { session.unlockForConfiguration() }
        do {
            // `.none` does not mean "the earpiece" — it removes the
            // override and falls back to the session MODE's default
            // route. Under .videoChat that default IS the speaker, so on
            // a video call the mode has to drop to .voiceChat (receiver
            // by default) for "speaker off" to re-route at all; speaker
            // on restores .videoChat so the default and the override
            // agree. Route and mode move together here, in one place —
            // the same pairing Android does in `audio.begin(video)`.
            // Configuration is safe under manual audio: CallKit owns the
            // session's ACTIVATION, and neither setMode nor the override
            // activates anything.
            if isVideoCall {
                let mode: AVAudioSession.Mode = enabled ? .videoChat : .voiceChat
                // Only when it differs: a mode write is itself a
                // category-change notification, which is what brings us
                // here — writing the same mode again would be a loop.
                if session.mode != mode.rawValue {
                    try session.setMode(mode)
                }
                // And WebRTC's own configuration follows, or its next
                // audio-unit start puts the mode — and with it the
                // default route — back.
                Self.syncWebRTCConfiguration(mode: mode)
            }
            try session.overrideOutputAudioPort(enabled ? .speaker : .none)
        } catch {
            AppLog.ui.info("Speaker override failed: \(String(describing: error))")
        }
    }
    #endif

    func close() {
        // Order matters: stop the capturer FIRST, then close the
        // connection. Closing first leaves the capture session delivering
        // frames into a dead source — the camera light stays on for a call
        // that no longer exists.
        closed = true
        #if os(iOS)
        if let routeObserver { NotificationCenter.default.removeObserver(routeObserver) }
        routeObserver = nil
        #endif
        capturer?.stopCapture()
        if let localRenderer { videoTrack?.remove(localRenderer) }
        if let remoteRelay { remoteVideoTrack?.remove(remoteRelay) }
        connection.close()
    }

    func ensureAudioRunning() {
        #if os(iOS)
        // CallKit's `didActivate` is what normally flips this on. When it
        // never fires — a race with another audio session, a watch answer —
        // the call stays silent while perfectly connected. Recover.
        let session = RTCAudioSession.sharedInstance()
        guard session.useManualAudio, !session.isAudioEnabled else { return }
        AppLog.call.warning("Audio was never activated by CallKit; starting it manually")
        Self.configureAudioSessionForCall(video: isVideoCall)
        session.lockForConfiguration()
        do {
            try session.setActive(true)
        } catch {
            AppLog.call.error("Manual audio activation failed: \(String(describing: error))")
        }
        session.unlockForConfiguration()
        session.isAudioEnabled = true
        #endif
    }

    /// One line per decision point: which kind of pair ICE settled on
    /// (host/srflx/relay — never an address). This is the line that tells a
    /// same-network call from a relayed one in a user's `log show`.
    private func logSelectedPair(_ context: StaticString) {
        connection.statistics { report in
            let stats = report.statistics
            var line = "no selected pair"
            let selected = Set(stats.values.filter { $0.type == "transport" }
                .compactMap { $0.values["selectedCandidatePairId"] as? String })
            for (id, s) in stats where s.type == "candidate-pair" {
                let nominated = (s.values["nominated"] as? Bool) == true
                let succeeded = (s.values["state"] as? String) == "succeeded"
                guard selected.contains(id) || (nominated && succeeded) else { continue }
                let local = (s.values["localCandidateId"] as? String).flatMap { stats[$0] }
                let remote = (s.values["remoteCandidateId"] as? String).flatMap { stats[$0] }
                let localType = local?.values["candidateType"] as? String ?? "?"
                let remoteType = remote?.values["candidateType"] as? String ?? "?"
                let proto = local?.values["protocol"] as? String ?? "?"
                line = "local \(localType)/\(proto) -> remote \(remoteType)"
                break
            }
            AppLog.call.info("\(context, privacy: .public): \(line, privacy: .public)")
        }
    }

    #if os(iOS)
    /// CallKit activated the session (CXProviderDelegate
    /// `provider(_:didActivate:)`): hand it to WebRTC, which starts the
    /// audio units now and not before.
    static func audioSessionDidActivate(_ audioSession: AVAudioSession) {
        let session = RTCAudioSession.sharedInstance()
        session.audioSessionDidActivate(audioSession)
        session.isAudioEnabled = true
    }

    static func audioSessionDidDeactivate(_ audioSession: AVAudioSession) {
        let session = RTCAudioSession.sharedInstance()
        session.isAudioEnabled = false
        session.audioSessionDidDeactivate(audioSession)
    }

    /// What a call's session looks like: play-and-record, voice chat mode
    /// (echo cancellation, the earpiece by default), Bluetooth allowed.
    /// Done through RTCAudioSession's lock so WebRTC sees the change.
    static func configureAudioSessionForCall(video: Bool = false) {
        let session = RTCAudioSession.sharedInstance()
        session.lockForConfiguration()
        defer { session.unlockForConfiguration() }
        do {
            // .videoChat differs from .voiceChat in exactly the way a
            // video call wants: the speaker by default (a face on screen
            // means the phone is not at an ear), same echo cancellation.
            try session.setCategory(
                .playAndRecord, mode: video ? .videoChat : .voiceChat,
                options: [.allowBluetoothHFP, .allowBluetoothA2DP])
            syncWebRTCConfiguration(mode: video ? .videoChat : .voiceChat)
        } catch {
            AppLog.ui.info("Call audio session configuration failed: \(String(describing: error))")
        }
    }

    /// What WebRTC's audio unit writes into the session when it starts
    /// (RTCAudioSession.setConfiguration, from its audio thread). Its
    /// stock configuration is .voiceChat, which on a VIDEO call put the
    /// mode — and the speaker the toggle claimed — back to the earpiece
    /// moments after CallKit activated the session. Kept equal to ours,
    /// so that write changes nothing and drops no override.
    private static func syncWebRTCConfiguration(mode: AVAudioSession.Mode) {
        let config = RTCAudioSessionConfiguration.webRTC()
        config.category = AVAudioSession.Category.playAndRecord.rawValue
        config.categoryOptions = [.allowBluetoothHFP, .allowBluetoothA2DP]
        config.mode = mode.rawValue
        RTCAudioSessionConfiguration.setWebRTC(config)
    }
    #endif
}

// MARK: - RTCPeerConnectionDelegate (signalling thread → main actor)

extension WebRTCClient: RTCPeerConnectionDelegate {
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didChange stateChanged: RTCSignalingState) {}
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didAdd stream: RTCMediaStream) {}

    /// Best effort: unified-plan removes the track by deactivating the
    /// transceiver, and this legacy callback is the closest thing the
    /// ObjC API surfaces. A far side that merely DISABLES its camera
    /// stops sending frames without any callback at all — the picture
    /// freezes and there is deliberately no signalling frame to say so.
    /// So while "video active" flips ON only on a really rendered frame
    /// (the relay below), flipping it back OFF stays this loose — there
    /// is nothing better to key it on, and a frozen last frame under the
    /// avatar would be worse than the freeze alone.
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didRemove stream: RTCMediaStream) {
        guard !stream.videoTracks.isEmpty else { return }
        Task { @MainActor in
            self.remoteVideoTrack = nil
            self.delegate?.mediaClient(self, remoteVideoActiveChanged: false)
        }
    }

    /// The far side's tracks, unified-plan style: one call per receiver.
    /// The video one is the picture's PROMISE, not the picture: this
    /// fires when the remote description is applied, before any frame is
    /// decoded — and a camera-denied far side never sends one at all. So
    /// the track is noted and the relay attached, but "video active" is
    /// deliberately NOT reported here; the relay reports it on the first
    /// rendered frame.
    nonisolated func peerConnection(
        _ peerConnection: RTCPeerConnection,
        didAdd rtpReceiver: RTCRtpReceiver,
        streams mediaStreams: [RTCMediaStream]
    ) {
        guard let track = rtpReceiver.track as? RTCVideoTrack else { return }
        Task { @MainActor in
            self.remoteVideoTrack = track
            let relay = self.remoteRelay
            if let relay { track.add(relay) }
            AppLog.call.info(
                "\(AppLog.CallVideo.tag, privacy: .public) didAdd remote video track rendererPresent=\(relay != nil, privacy: .public) path=\(relay != nil ? "track-after-attach" : "track-before-attach", privacy: .public)"
            )
        }
    }
    nonisolated func peerConnectionShouldNegotiate(_ peerConnection: RTCPeerConnection) {}
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceGatheringState) {}
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didRemove candidates: [RTCIceCandidate]) {}
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didOpen dataChannel: RTCDataChannel) {}

    /// Log-only. The call machine keys off `didChangeConnectionState`
    /// below — the AGGREGATE state (ICE and DTLS both), because ICE alone
    /// connecting is a call that LOOKS active while the encryption
    /// handshake may still fail: "connected but silent" on this side,
    /// "connecting…" forever on the other. Android has always used the
    /// aggregate state; this made the two agree.
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceConnectionState) {
        AppLog.call.info("ICE connection state: \(newState.rawValue, privacy: .public)")
    }

    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCPeerConnectionState) {
        let mapped: CallMediaConnectionState
        switch newState {
        case .new: mapped = .new
        case .connecting: mapped = .connecting
        case .connected: mapped = .connected
        case .disconnected: mapped = .disconnected
        case .failed: mapped = .failed
        case .closed: mapped = .closed
        @unknown default: return
        }
        AppLog.call.info("Peer connection state: \(newState.rawValue, privacy: .public)")
        Task { @MainActor in
            if mapped == .connected { self.logSelectedPair("connected") }
            if mapped == .failed { self.logSelectedPair("failed") }
            self.delegate?.mediaClient(self, connectionStateChanged: mapped)
        }
    }

    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didGenerate candidate: RTCIceCandidate) {
        let payload = IceCandidatePayload(
            candidate: candidate.sdp,
            sdpMid: candidate.sdpMid,
            sdpMLineIndex: candidate.sdpMLineIndex)
        Task { @MainActor in
            self.delegate?.mediaClient(self, didGatherLocalCandidate: payload)
        }
    }
}


// MARK: - First-frame relay

/// A pass-through RTCVideoRenderer that reports the FIRST frame it
/// forwards, once per attachment.
///
/// This is what makes `isRemoteVideoActive` mean "frames are rendering":
/// the unified-plan `didAdd` fires at SDP time, before any frame — and a
/// camera-denied far side never sends one — so keying the flag on it left
/// the call screen a black full-bleed surface with no avatar, no name and
/// no timer. A forwarding renderer was chosen over
/// `RTCVideoViewDelegate.didChangeVideoSize` deliberately: that delegate
/// hangs off the concrete Metal view classes (RTCMTLVideoView here,
/// RTCMTLNSVideoView on the Mac), fires on size CHANGES rather than
/// promising one call per first frame on both platforms, and would tie
/// the media client to view types the CallMediaClient seam exists to keep
/// out. The relay works for any renderer — the test fake included — and
/// its semantics are exactly the sentence the flag needs.
///
/// renderFrame arrives on WebRTC's decoder thread; the once-latch is a
/// lock, and the callback hops to the main actor before touching anyone.
nonisolated final class RemoteFirstFrameRelay: NSObject, RTCVideoRenderer, @unchecked Sendable {

    private let wrapped: any RTCVideoRenderer
    private let onFirstFrame: @MainActor @Sendable () -> Void
    private let fired = OSAllocatedUnfairLock(initialState: false)

    /// Whether this relay stands in for `renderer` — how a detach names
    /// the surface it owns rather than clearing whatever is attached.
    nonisolated func wraps(_ renderer: any RTCVideoRenderer) -> Bool {
        wrapped === renderer
    }

    init(wrapping renderer: any RTCVideoRenderer, onFirstFrame: @escaping @MainActor @Sendable () -> Void) {
        self.wrapped = renderer
        self.onFirstFrame = onFirstFrame
    }

    nonisolated func setSize(_ size: CGSize) {
        wrapped.setSize(size)
    }

    nonisolated func renderFrame(_ frame: RTCVideoFrame?) {
        wrapped.renderFrame(frame)
        // A nil frame is a renderer reset, not a picture.
        guard frame != nil else { return }
        let isFirst = fired.withLock { (alreadyFired: inout Bool) -> Bool in
            if alreadyFired { return false }
            alreadyFired = true
            return true
        }
        guard isFirst else { return }
        AppLog.call.info("\(AppLog.CallVideo.tag, privacy: .public) FIRST remote frame")
        let callback = onFirstFrame
        Task { @MainActor in callback() }
    }
}
