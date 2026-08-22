//
//  AppDelegate.swift
//  FamilyConnect
//
//  Minimal UIKit shim for the two things SwiftUI has no surface for:
//  the APNs token callbacks and the UNUserNotificationCenter delegate
//  (which must be installed before didFinishLaunching returns, or a
//  cold-start notification tap is silently dropped). Everything is
//  forwarded straight out — tokens to PushRegistrar, tap routes to
//  AppSession — through static weak references that FamilyConnectApp's
//  init wires before UIApplicationMain delivers any callback. Statics
//  because @UIApplicationDelegateAdaptor constructs this object itself,
//  so there is no initializer to inject through; weak because the app
//  owns both collaborators and the delegate only forwards.
//

// iOS only. macOS would need an NSApplicationDelegate with different
// signatures for the same three callbacks; until the Mac app registers for
// push, there is nothing for it to do — a Mac holds a live socket while it
// is open, which is when its user is looking at it anyway.
#if os(iOS)

import UIKit
import UserNotifications

final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {

    /// Wired by FamilyConnectApp.init (nil only when the SwiftData
    /// container failed, in which case there is nothing to route to).
    static weak var registrar: PushRegistrar?
    static weak var session: AppSession?

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        return true
    }

    // MARK: - APNs token

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        Self.registrar?.handleDeviceToken(deviceToken)
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        Self.registrar?.handleRegistrationFailure(error)
    }

    // MARK: - UNUserNotificationCenterDelegate

    /// Foregrounded app: present nothing. The live socket is already
    /// delivering the same event as a frame — the server never pushes to
    /// a user with an open socket, so anything arriving here is the race
    /// where a push sent while backgrounded lands just after resume, and
    /// showing it would double-notify. No exceptions in v1.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([])
    }

    /// The user tapped a notification (warm or cold start). Parse the
    /// payload here — userInfo is not Sendable, the resulting PushRoute
    /// is — then park the route on AppSession; ChatListView consumes it
    /// once the phase is .active, which is how a cold-start tap survives
    /// bootstrap.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let route = PushRoute.parse(userInfo: response.notification.request.content.userInfo)
        Task { @MainActor in
            AppDelegate.session?.pendingPushRoute = route
            completionHandler()
        }
    }
}

#endif
