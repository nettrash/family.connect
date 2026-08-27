//
//  CallMediaClient.swift
//  FamilyConnect
//
//  The seam between the call state machine and WebRTC. CallManager talks
//  to this protocol and nothing else about media, which is what lets its
//  state machine be tested with a fake that never opens a socket or a
//  microphone. WebRTCClient is the real one.
//

import Foundation
// WebRTC here is for RTCVideoRenderer alone — a two-method protocol
// (setSize/renderFrame) a test fake conforms to trivially. The rest of
// WebRTC stays behind WebRTCClient.
import WebRTC

/// Where the peer connection is, reduced to what the call machine acts on.
nonisolated enum CallMediaConnectionState: Equatable, Sendable {
    case new
    case connecting
    case connected
    /// A blip: WebRTC may recover on its own, so the call is not over.
    case disconnected
    /// It will not recover. The call is reported as `failed`.
    case failed
    case closed
}

@MainActor
protocol CallMediaClientDelegate: AnyObject {
    func mediaClient(_ client: any CallMediaClient, didGatherLocalCandidate candidate: IceCandidatePayload)
    func mediaClient(_ client: any CallMediaClient, connectionStateChanged state: CallMediaConnectionState)
    /// The far side's video appeared (or, best effort, went away). On a
    /// video call this is what lets the UI swap the avatar for the
    /// picture. Never fires on a voice call.
    func mediaClient(_ client: any CallMediaClient, remoteVideoActiveChanged active: Bool)
}

extension CallMediaClientDelegate {
    func mediaClient(_ client: any CallMediaClient, remoteVideoActiveChanged active: Bool) {}
}

@MainActor
protocol CallMediaClient: AnyObject {
    var delegate: (any CallMediaClientDelegate)? { get set }

    /// Create and apply the local offer; the SDP goes in `call_offer`.
    func createOffer() async throws -> String
    /// Create and apply the local answer to a remote offer already set.
    func createAnswer() async throws -> String
    /// The peer's description. `isOffer` distinguishes the callee's side
    /// (an offer to answer) from the caller's (an answer to accept).
    func setRemoteDescription(sdp: String, isOffer: Bool) async throws
    func addRemoteCandidate(_ candidate: IceCandidatePayload) async throws
    func setMuted(_ muted: Bool)
    /// Called shortly after the call goes active: if the platform's audio
    /// units are not running by then, start them. On iOS CallKit normally
    /// activates the audio session and this is a no-op; when its
    /// `didActivate` never fires — the classic connected-but-silent call —
    /// this is the fallback that makes the call audible. A no-op on macOS
    /// (audio is automatic) and in tests.
    func ensureAudioRunning()
    /// iOS only; a no-op elsewhere.
    func setSpeaker(_ enabled: Bool)

    // MARK: Video (docs/protocol.md, "Video") — all four have no-op
    // defaults below, so an audio-only client (and the test fake) need
    // not know video exists.

    /// Turn the local camera on or off. The call's KIND never changes —
    /// this toggles the track and the capture session, no renegotiation,
    /// no frame on the wire.
    func setCameraEnabled(_ enabled: Bool)
    /// Front ↔ back. A no-op where there is only one camera (the Mac).
    func flipCamera()
    /// Where the local preview draws. nil detaches. Settable before or
    /// after the track exists — the client attaches late.
    func setLocalVideoRenderer(_ renderer: (any RTCVideoRenderer)?)
    /// Where the far side's picture draws. Same rules.
    func setRemoteVideoRenderer(_ renderer: (any RTCVideoRenderer)?)

    func close()
}

extension CallMediaClient {
    func ensureAudioRunning() {}
    func setCameraEnabled(_ enabled: Bool) {}
    func flipCamera() {}
    func setLocalVideoRenderer(_ renderer: (any RTCVideoRenderer)?) {}
    func setRemoteVideoRenderer(_ renderer: (any RTCVideoRenderer)?) {}
}
