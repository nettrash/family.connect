//
//  CallKitController.swift
//  FamilyConnect
//
//  The iPhone's half of a call's system UI: CallKit shows the incoming
//  call on the lock screen, puts an outgoing one in the status bar, lets
//  the person answer from their watch, and — the part that is not
//  cosmetic — owns the audio session's activation, which is what lets the
//  microphone run with the app in the background.
//
//  Two directions, exactly as CallSystemBridge says: CallManager reports
//  what happened and requests what the user asked for in the app's UI;
//  CallKit calls back into `perform` with an action, and THAT is what
//  moves the state machine — so the system's call and the app's are one.
//

#if os(iOS)

import AVFAudio
import CallKit
import Foundation
import os

@MainActor
final class CallKitController: NSObject, CallSystemBridge {

    weak var manager: CallManager?

    private let provider: CXProvider
    private let callController = CXCallController()

    override init() {
        let configuration = CXProviderConfiguration()
        configuration.supportsVideo = false
        configuration.maximumCallGroups = 1
        configuration.maximumCallsPerCallGroup = 1
        configuration.supportedHandleTypes = [.generic]
        configuration.includesCallsInRecents = true
        provider = CXProvider(configuration: configuration)
        super.init()
        // nil queue = the main queue, which is where the manager lives.
        provider.setDelegate(self, queue: nil)
    }

    // MARK: - CallSystemBridge

    func reportOutgoing(callID: UUID, peerName: String) {
        let action = CXStartCallAction(call: callID, handle: CXHandle(type: .generic, value: peerName))
        action.isVideo = false
        request(action)
    }

    func reportOutgoingConnecting(callID: UUID) {
        provider.reportOutgoingCall(with: callID, startedConnectingAt: nil)
    }

    func reportOutgoingConnected(callID: UUID) {
        provider.reportOutgoingCall(with: callID, connectedAt: nil)
    }

    func reportIncoming(callID: UUID, peerName: String) {
        reportIncoming(callID: callID, peerName: peerName) { _ in }
    }

    /// The push path's version, whose completion the PushKit delegate
    /// waits on. Never skipped on an error: iOS terminates an app that
    /// receives a VoIP push and reports no call.
    func reportIncoming(callID: UUID, peerName: String, completion: @escaping @Sendable (Error?) -> Void) {
        let update = CXCallUpdate()
        update.remoteHandle = CXHandle(type: .generic, value: peerName)
        update.localizedCallerName = peerName
        update.hasVideo = false
        update.supportsGrouping = false
        update.supportsUngrouping = false
        update.supportsHolding = false
        update.supportsDTMF = false
        provider.reportNewIncomingCall(with: callID, update: update) { error in
            if let error {
                AppLog.push.error("CallKit refused the incoming call: \(String(describing: error))")
            }
            completion(error)
        }
    }

    func reportEnded(callID: UUID, reason: CallEndReason) {
        let mapped: CXCallEndedReason
        switch reason {
        case .hangup, .cancel, .decline:
            mapped = .remoteEnded
        case .timeout:
            mapped = .unanswered
        case .answeredElsewhere:
            mapped = .answeredElsewhere
        case .failed, .busy, .unreachable, .unavailable, .microphoneDenied:
            mapped = .failed
        }
        provider.reportCall(with: callID, endedAt: nil, reason: mapped)
    }

    func requestAnswer(callID: UUID) {
        request(CXAnswerCallAction(call: callID))
    }

    func requestEnd(callID: UUID) {
        request(CXEndCallAction(call: callID))
    }

    private func request(_ action: CXAction) {
        callController.request(CXTransaction(action: action)) { error in
            if let error {
                AppLog.push.error("CallKit transaction failed: \(String(describing: error))")
            }
        }
    }
}

// MARK: - CXProviderDelegate (main queue → main actor)

extension CallKitController: CXProviderDelegate {
    nonisolated func providerDidReset(_ provider: CXProvider) {
        Task { @MainActor in
            self.manager?.systemDidEnd()
        }
    }

    nonisolated func provider(_ provider: CXProvider, perform action: CXStartCallAction) {
        Task { @MainActor in
            WebRTCClient.configureAudioSessionForCall()
            action.fulfill()
        }
    }

    nonisolated func provider(_ provider: CXProvider, perform action: CXAnswerCallAction) {
        Task { @MainActor in
            WebRTCClient.configureAudioSessionForCall()
            self.manager?.systemDidAnswer()
            action.fulfill()
        }
    }

    nonisolated func provider(_ provider: CXProvider, perform action: CXEndCallAction) {
        Task { @MainActor in
            self.manager?.systemDidEnd()
            action.fulfill()
        }
    }

    nonisolated func provider(_ provider: CXProvider, perform action: CXSetMutedCallAction) {
        let muted = action.isMuted
        Task { @MainActor in
            self.manager?.systemDidSetMuted(muted)
            action.fulfill()
        }
    }

    nonisolated func provider(_ provider: CXProvider, didActivate audioSession: AVAudioSession) {
        Task { @MainActor in
            WebRTCClient.audioSessionDidActivate(audioSession)
        }
    }

    nonisolated func provider(_ provider: CXProvider, didDeactivate audioSession: AVAudioSession) {
        Task { @MainActor in
            WebRTCClient.audioSessionDidDeactivate(audioSession)
        }
    }
}

#endif
