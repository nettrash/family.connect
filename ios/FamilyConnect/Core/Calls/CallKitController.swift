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
        configuration.supportsVideo = true
        configuration.maximumCallGroups = 1
        configuration.maximumCallsPerCallGroup = 1
        // Our own calls carry a `.generic` handle (CallHandle). The other
        // two are declared so the Phone app offers Family on a contact's
        // phone numbers and e-mails — in Favorites and on the contact card
        // — and hands the app that contact to resolve (CallIntents,
        // ContactLinks); nothing here dials a number.
        configuration.supportedHandleTypes = [.generic, .phoneNumber, .emailAddress]
        // A Family call is a call: it belongs in Recents like any other,
        // named after the person (localizedCallerName), and tapping it
        // there calls them again (CallIntents).
        configuration.includesCallsInRecents = true
        provider = CXProvider(configuration: configuration)
        super.init()
        // nil queue = the main queue, which is where the manager lives.
        provider.setDelegate(self, queue: nil)
    }

    // MARK: - CallSystemBridge

    func reportOutgoing(callID: UUID, peerUserID: Int64, peerName: String, isVideo: Bool) {
        let link = ContactLinks.shared.link(for: peerUserID)
        let action = CXStartCallAction(call: callID, handle: Self.handle(peerName: peerName, link: link))
        action.isVideo = isVideo
        request(action)
        CallDonation.donate(peerUserID: peerUserID, peerName: peerName, video: isVideo, contactIdentifier: link?.contactIdentifier)
    }

    /// The system's handle for an OUTGOING call. The Recents row shows a
    /// handle's raw value unless the system can map it to a contact, and
    /// it maps phone numbers and e-mails — so a member linked to a device
    /// contact (ContactLinks) is reported by that contact's number (or
    /// e-mail), and the row carries the contact's name and photo like any
    /// call. An unlinked member is reported by their display name — a
    /// deliberate choice over the id (CallHandle): names are write-once
    /// in this protocol, the row stays readable, and tapping it comes
    /// back through CallIntents, where CallRequestRouter turns the number
    /// or the name back into the member (two members with one name are
    /// asked about; linking a contact ends that).
    static func handle(peerName: String, link: ContactLink?) -> CXHandle {
        if let number = link?.phoneNumbers.first, !number.isEmpty {
            return CXHandle(type: .phoneNumber, value: number)
        }
        if let email = link?.emailAddresses.first, !email.isEmpty {
            return CXHandle(type: .emailAddress, value: email)
        }
        return CXHandle(type: .generic, value: peerName)
    }

    func reportOutgoingConnecting(callID: UUID) {
        provider.reportOutgoingCall(with: callID, startedConnectingAt: nil)
    }

    func reportOutgoingConnected(callID: UUID) {
        provider.reportOutgoingCall(with: callID, connectedAt: nil)
    }

    func reportIncoming(callID: UUID, peerUserID: Int64?, peerName: String, hasVideo: Bool) {
        reportIncoming(callID: callID, peerUserID: peerUserID, peerName: peerName, hasVideo: hasVideo) { _ in }
    }

    /// The push path's version, whose completion the PushKit delegate
    /// waits on. Never skipped on an error: iOS terminates an app that
    /// receives a VoIP push and reports no call.
    func reportIncoming(callID: UUID, peerUserID: Int64?, peerName: String, hasVideo: Bool, completion: @escaping @Sendable (Error?) -> Void) {
        let update = CXCallUpdate()
        // An INCOMING call keeps the generic name handle even for a linked
        // member: a phone-number handle is run through the block list and
        // Focus rules ("calls from Favorites only") before it is allowed
        // to ring, and a Family call must not become LESS likely to ring
        // because the caller was linked. The name is what the row shows.
        update.remoteHandle = CXHandle(type: .generic, value: peerName)
        update.localizedCallerName = peerName
        update.hasVideo = hasVideo
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
        case .failed, .busy, .unreachable, .unavailable, .videoUnavailable, .microphoneDenied:
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
            // The manager knows the call's kind; a video call's session is
            // mode .videoChat (speaker by default).
            WebRTCClient.configureAudioSessionForCall(video: self.manager?.isVideo ?? false)
            action.fulfill()
            // The person's NAME for the system UI, said once the call is
            // the provider's (after the fulfil — an update before it may
            // be dropped). Incoming calls carry it on the CXCallUpdate
            // that reports them; an outgoing one has to be told.
            if let name = self.manager?.peerName, !name.isEmpty {
                let update = CXCallUpdate()
                update.remoteHandle = action.handle
                update.localizedCallerName = name
                update.hasVideo = action.isVideo
                update.supportsGrouping = false
                update.supportsUngrouping = false
                update.supportsHolding = false
                update.supportsDTMF = false
                provider.reportCall(with: action.callUUID, updated: update)
            }
        }
    }

    nonisolated func provider(_ provider: CXProvider, perform action: CXAnswerCallAction) {
        Task { @MainActor in
            WebRTCClient.configureAudioSessionForCall(video: self.manager?.isVideo ?? false)
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
            // The route, now that there is a live session to route.
            self.manager?.systemDidActivateAudio()
        }
    }

    nonisolated func provider(_ provider: CXProvider, didDeactivate audioSession: AVAudioSession) {
        Task { @MainActor in
            WebRTCClient.audioSessionDidDeactivate(audioSession)
            self.manager?.systemDidDeactivateAudio()
        }
    }
}

#endif
