//
//  MacAppDelegate.swift
//  FamilyConnect
//
//  The Mac's half of push: the same three callbacks the iOS delegate
//  handles, spelled the way AppKit spells them.
//
//  It exists as its own type rather than as conditionals inside
//  AppDelegate because the protocols genuinely differ —
//  `application(_:didFinishLaunchingWithOptions:)` versus
//  `applicationDidFinishLaunching(_:)`, a Notification instead of a launch
//  dictionary. Everything downstream is shared: the token goes to the same
//  PushRegistrar, the tap parks the same PushRoute on the same AppSession.
//
//  A Mac holds a live socket the whole time it is open, and the server
//  never pushes to a user with an open socket — so what push buys here is
//  the case the phone has too: something arriving while the app is closed.
//

#if os(macOS)

import AppKit
import UserNotifications

final class MacAppDelegate: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate {

    /// Wired by FamilyConnectApp.init, exactly as on iOS.
    static weak var registrar: PushRegistrar?
    static weak var session: AppSession?

    func applicationDidFinishLaunching(_ notification: Notification) {
        UNUserNotificationCenter.current().delegate = self
    }

    // MARK: - APNs token

    func application(
        _ application: NSApplication,
        didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
        Self.registrar?.handleDeviceToken(deviceToken)
    }

    func application(
        _ application: NSApplication,
        didFailToRegisterForRemoteNotificationsWithError error: Error
    ) {
        Self.registrar?.handleRegistrationFailure(error)
    }

    // MARK: - UNUserNotificationCenterDelegate

    /// Foreground: present nothing, for the same reason iOS does not — the
    /// socket already delivered it, and a banner would double-notify.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([])
    }

    /// Clicked a notification. Parse here (userInfo is not Sendable, the
    /// PushRoute is), then park it on AppSession for the window to pick up
    /// once the session is active — which is how a click that launched the
    /// app survives bootstrap.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let route = PushRoute.parse(userInfo: response.notification.request.content.userInfo)
        Task { @MainActor in
            MacAppDelegate.session?.pendingPushRoute = route
            completionHandler()
        }
    }
}

#endif
