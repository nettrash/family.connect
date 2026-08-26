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
    /// iOS only; a no-op elsewhere.
    func setSpeaker(_ enabled: Bool)
    func close()
}
