//
//  PushRegistrar.swift
//  FamilyConnect
//
//  The push-token lifecycle: authorization prompt → APNs registration →
//  POST /devices → re-POST on token rotation → DELETE /devices/{id} on
//  logout (protocol.md §Push notifications, device lifecycle).
//
//  A separate object rather than more ChatSyncCoordinator, deliberately:
//  the coordinator is the wire ↔ store seam and stays UIKit-free so its
//  tests run without UI machinery, while everything here is OS
//  permission and APNs plumbing (UNUserNotificationCenter,
//  UIApplication.registerForRemoteNotifications). The two meet only
//  through the coordinator's `ensurePushRegistration` closure, wired in
//  the composition root — the same injected-closure pattern AppSession
//  uses for its store side effects.
//
//  Decision rules live in the pure `PushRegistrationLogic` enum below;
//  the class is the stateful shell. Persistence goes through closure
//  seams that default to AppSettings, so the state-machine tests use an
//  in-memory box instead of racing other suites over the process-global
//  UserDefaults.
//

import Foundation
import os
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif
import UserNotifications

// MARK: - Pure decision table

nonisolated enum PushRegistrationLogic {

    /// APNs hands the token as raw bytes; the wire wants lowercase hex.
    static func hexToken(_ deviceToken: Data) -> String {
        deviceToken.map { String(format: "%02x", $0) }.joined()
    }

    /// POST /devices is due exactly when the OS token is one the server
    /// hasn't confirmed: nothing stored yet, a rotated token, or a token
    /// stored without a device_id (impossible via `saveStored`, but the
    /// rule is written to re-register rather than trust half a pair).
    static func needsRegistration(token: String, storedToken: String?, storedDeviceID: Int64?) -> Bool {
        token != storedToken || storedDeviceID == nil
    }

    /// The same question with the VoIP token beside the APNs one: the pair
    /// is re-POSTed when EITHER is news to the server. `voipToken` nil
    /// means PushKit has not spoken yet this launch — which is not news,
    /// so a stored one is left alone (protocol.md: an absent `voip_token`
    /// leaves the row untouched).
    static func needsRegistration(
        token: String, storedToken: String?, storedDeviceID: Int64?,
        voipToken: String?, storedVoIPToken: String?
    ) -> Bool {
        if needsRegistration(token: token, storedToken: storedToken, storedDeviceID: storedDeviceID) {
            return true
        }
        guard let voipToken else { return false }
        return voipToken != storedVoIPToken
    }
}

// MARK: - The stateful shell

@MainActor
final class PushRegistrar {

    /// What this build calls itself to the server (protocol.md, POST
    /// /devices).
    static var platform: String {
        #if os(macOS)
        "macos"
        #else
        "ios"
        #endif
    }

    private let api: APIClient

    /// Serializes POSTs so a token callback racing a resync-triggered
    /// re-registration can't create two requests for the same token.
    private var registrationInFlight = false

    /// Storage seams — AppSettings in the app, an in-memory box in tests
    /// (see file header). Token and device_id are one pair: always read
    /// and written together.
    var loadStored: () -> (token: String?, deviceID: Int64?) = {
        (AppSettings.pushToken, AppSettings.pushDeviceID)
    }
    var saveStored: (_ token: String?, _ deviceID: Int64?) -> Void = { token, deviceID in
        AppSettings.pushToken = token
        AppSettings.pushDeviceID = deviceID
    }
    /// The VoIP token the server has confirmed, beside the pair above.
    var loadStoredVoIP: () -> String? = { AppSettings.voipToken }
    var saveStoredVoIP: (String?) -> Void = { AppSettings.voipToken = $0 }

    /// What PushKit said this launch: nil until it speaks, and it speaks
    /// on every launch. `.some(nil)` after it INVALIDATED the token — that
    /// is news too, and clears the server's copy.
    private var voipToken: String??

    init(api: APIClient) {
        self.api = api
    }

    // MARK: - Authorization + APNs registration

    /// Called at the end of every coordinator resync — i.e. when the
    /// session first reaches .active and again on every reconnect and
    /// foregrounding. First pass asks for permission; once granted (now
    /// or on an earlier launch) it (re)requests the APNs token, whose
    /// delegate callback drives the actual POST. Denial is final and
    /// silent — checked via settings first so this never becomes a nag,
    /// and a denied user simply keeps the live-socket experience.
    func ensureRegistered() async {
        let center = UNUserNotificationCenter.current()
        let settings = await center.notificationSettings()
        switch settings.authorizationStatus {
        case .notDetermined:
            let granted = (try? await center.requestAuthorization(options: [.alert, .sound, .badge])) ?? false
            guard granted else {
                AppLog.push.info("Notification authorization denied")
                return
            }
        case .denied:
            return
        default:
            break
        }
        // Cheap and idempotent: iOS re-delivers the current token to the
        // app delegate even when it hasn't changed — which is exactly the
        // retry path for a POST that failed on an earlier pass.
        // Both platforms register the same way through a differently
        // spelled application object; everything after this — the token
        // callback, the POST /devices — is shared.
        #if os(iOS)
        UIApplication.shared.registerForRemoteNotifications()
        #elseif os(macOS)
        NSApplication.shared.registerForRemoteNotifications()
        #endif
    }

    // MARK: - Token callbacks (forwarded by AppDelegate)

    func handleDeviceToken(_ deviceToken: Data) {
        let tokenHex = PushRegistrationLogic.hexToken(deviceToken)
        Task { await self.register(tokenHex: tokenHex) }
    }

    nonisolated func handleRegistrationFailure(_ error: Error) {
        // Non-fatal (no network, simulator, …): the next resync retries.
        AppLog.push.info("APNs registration failed: \(String(describing: error))")
    }

    /// PushKit delivered (or invalidated) the VoIP token. Folded into the
    /// same POST /devices as the APNs token — the row is one device — and
    /// POSTed now when the APNs half is already known, otherwise when it
    /// arrives (docs/protocol.md, "Incoming calls").
    func handleVoIPToken(_ hex: String?) {
        voipToken = .some(hex)
        if let apnsToken = loadStored().token {
            pendingRegistration = Task { await self.register(tokenHex: apnsToken) }
        }
    }

    /// The re-POST started by the most recent `handleVoIPToken`. Test
    /// seam, the `pendingDelivery` idiom: nothing in the app reads it.
    private(set) var pendingRegistration: Task<Void, Never>?

    /// POST the token when it's news to the server (see the logic table
    /// above); persist the {token, device_id} pair only after a 2xx so a
    /// failed attempt naturally retries on the next token delivery.
    func register(tokenHex: String) async {
        let stored = loadStored()
        let storedVoIP = loadStoredVoIP()
        let voip = voipToken
        guard PushRegistrationLogic.needsRegistration(
            token: tokenHex, storedToken: stored.token, storedDeviceID: stored.deviceID,
            voipToken: voip.flatMap { $0 }, storedVoIPToken: storedVoIP)
            || (voip == .some(nil) && storedVoIP != nil) else { return }
        guard !registrationInFlight else { return }
        registrationInFlight = true
        defer { registrationInFlight = false }
        do {
            // A Mac says so, even though its token goes out over the same
            // APNs connection as an iPhone's: the server routes on this
            // field, and a Mac claiming to be an iPhone is the kind of
            // small untruth that costs an hour the first time somebody
            // debugs "why did only one device get it".
            //
            // The VoIP token rides along only when PushKit has spoken —
            // absent otherwise, so a launch that has only the APNs token
            // does not wipe the server's copy (protocol.md, POST /devices).
            let deviceID = try await api.registerDevice(
                platform: Self.platform, pushToken: tokenHex, voipToken: voip)
            saveStored(tokenHex, deviceID)
            if let voip { saveStoredVoIP(voip) }
            AppLog.push.info("Registered device \(deviceID, privacy: .public) for push")
        } catch {
            AppLog.push.info("Device registration failed: \(String(describing: error))")
        }
    }

    // MARK: - Logout

    /// Best-effort server-side removal. AppSession.logout calls this
    /// BEFORE /auth/logout, because the DELETE authenticates with the
    /// very token logout revokes. Local state clears regardless — a row
    /// left behind dies server-side on the first push APNs rejects as
    /// unregistered.
    func deregister() async {
        let stored = loadStored()
        if let deviceID = stored.deviceID {
            try? await api.deleteDevice(id: deviceID)
        }
        saveStored(nil, nil)
        saveStoredVoIP(nil)
    }
}
