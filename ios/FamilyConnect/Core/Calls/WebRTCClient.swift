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
        return RTCPeerConnectionFactory(encoderFactory: nil, decoderFactory: nil)
    }()

    weak var delegate: (any CallMediaClientDelegate)?

    private let connection: RTCPeerConnection
    private let audioTrack: RTCAudioTrack

    /// Fails only when libwebrtc refuses the configuration, which with an
    /// audio-only unified-plan connection it does not.
    init?(iceServers: [IceServerDTO]) {
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
        super.init()
        connection.add(track, streamIds: ["stream0"])
        connection.delegate = self
    }

    private var offerAnswerConstraints: RTCMediaConstraints {
        RTCMediaConstraints(
            mandatoryConstraints: [
                kRTCMediaConstraintsOfferToReceiveAudio: kRTCMediaConstraintsValueTrue,
                kRTCMediaConstraintsOfferToReceiveVideo: kRTCMediaConstraintsValueFalse,
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

    func setSpeaker(_ enabled: Bool) {
        #if os(iOS)
        let session = RTCAudioSession.sharedInstance()
        session.lockForConfiguration()
        defer { session.unlockForConfiguration() }
        do {
            try session.overrideOutputAudioPort(enabled ? .speaker : .none)
        } catch {
            AppLog.ui.info("Speaker override failed: \(String(describing: error))")
        }
        #endif
    }

    func close() {
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
        Self.configureAudioSessionForCall()
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
    static func configureAudioSessionForCall() {
        let session = RTCAudioSession.sharedInstance()
        session.lockForConfiguration()
        defer { session.unlockForConfiguration() }
        do {
            try session.setCategory(.playAndRecord, mode: .voiceChat, options: [.allowBluetoothHFP, .allowBluetoothA2DP])
        } catch {
            AppLog.ui.info("Call audio session configuration failed: \(String(describing: error))")
        }
    }
    #endif
}

// MARK: - RTCPeerConnectionDelegate (signalling thread → main actor)

extension WebRTCClient: RTCPeerConnectionDelegate {
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didChange stateChanged: RTCSignalingState) {}
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didAdd stream: RTCMediaStream) {}
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didRemove stream: RTCMediaStream) {}
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
