//
//  VoIPPushRegistrar.swift
//  FamilyConnect
//
//  PushKit: the VoIP token, and the push that rings a phone whose app is
//  not running (docs/protocol.md, "Incoming calls"). Separate from
//  PushRegistrar because it is a different registry with a different
//  token, delivered on its own schedule — and because the incoming-push
//  callback has a rule the alert path never has: the app MUST report a
//  call to CallKit before the completion handler runs, or iOS terminates
//  it. Everything here is arranged around that sentence.
//
//  The token goes to PushRegistrar, which folds it into POST /devices
//  beside the APNs token. The push goes to CallKit first, then to
//  CallManager, and then the socket is brought up so the server's replay
//  of the offer has somewhere to land.
//

#if os(iOS)

import Foundation
import os
import PushKit

@MainActor
final class VoIPPushRegistrar: NSObject {

    weak var pushRegistrar: PushRegistrar?
    weak var callManager: CallManager?
    weak var callKit: CallKitController?

    private let registry = PKPushRegistry(queue: .main)

    /// Ask for the VoIP token. PushKit delivers it on every launch, so
    /// this is also how a token the server never confirmed gets retried.
    func start() {
        registry.delegate = self
        registry.desiredPushTypes = [.voIP]
    }
}

extension VoIPPushRegistrar: PKPushRegistryDelegate {
    nonisolated func pushRegistry(_ registry: PKPushRegistry, didUpdate pushCredentials: PKPushCredentials, for type: PKPushType) {
        guard type == .voIP else { return }
        let hex = PushRegistrationLogic.hexToken(pushCredentials.token)
        Task { @MainActor in
            self.pushRegistrar?.handleVoIPToken(hex)
        }
    }

    nonisolated func pushRegistry(_ registry: PKPushRegistry, didInvalidatePushTokenFor type: PKPushType) {
        guard type == .voIP else { return }
        Task { @MainActor in
            self.pushRegistrar?.handleVoIPToken(nil)
        }
    }

    nonisolated func pushRegistry(
        _ registry: PKPushRegistry,
        didReceiveIncomingPushWith payload: PKPushPayload,
        for type: PKPushType,
        completion: @escaping () -> Void
    ) {
        guard type == .voIP else {
            completion()
            return
        }
        let parsed = IncomingCallPush.parse(payload.dictionaryPayload)
        // The registry's queue is main, so this is a hop in name only; it
        // keeps the compiler honest about where the manager lives.
        MainActor.assumeIsolated {
            self.handleIncoming(parsed, completion: completion)
        }
    }

    private func handleIncoming(_ push: IncomingCallPush?, completion: @escaping () -> Void) {
        guard let callKit else {
            completion()
            return
        }
        guard let push else {
            // Nothing to ring, but a call has to be reported anyway (see the
            // file header). Report one and end it in the same breath.
            AppLog.push.error("VoIP push with an unreadable payload")
            let uuid = UUID()
            callKit.reportIncoming(callID: uuid, peerName: String(localized: "Unknown caller"), hasVideo: false) { _ in
                completion()
            }
            callKit.reportEnded(callID: uuid, reason: .failed)
            return
        }
        let uuid = UUID(uuidString: push.callID) ?? UUID()
        let peer = callManager?.resolvePeer(push.fromUserID)
        let name = peer.map { $0.name == String(localized: "Someone") && !push.callerName.isEmpty ? push.callerName : $0.name }
            ?? push.callerName
        callKit.reportIncoming(callID: uuid, peerName: name, hasVideo: push.video) { _ in
            completion()
        }
        if callManager?.handleIncomingPush(push) != true {
            // Busy, or the same call already ended: the system was shown a
            // call it must now be shown the end of.
            callKit.reportEnded(callID: uuid, reason: .busy)
        }
    }
}

#endif
