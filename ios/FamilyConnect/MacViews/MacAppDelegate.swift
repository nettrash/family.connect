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
//  never pushes to a device whose session is live — so what push buys here
//  is the case the phone has too: something arriving while the app is
//  closed. Everything that arrives while it is OPEN comes over the socket
//  and is announced locally instead (ChatNotifier), which is why the
//  presentation rule below has to tell the two apart.
//

#if os(macOS)

import AppKit
import UserNotifications

final class MacAppDelegate: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate {

    /// Wired by FamilyConnectApp.init, exactly as on iOS.
    static weak var registrar: PushRegistrar?
    static weak var session: AppSession?
    /// For the Answer / Decline buttons on an incoming-call notification.
    static weak var callManager: CallManager?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        center.setNotificationCategories([ChatNotifier.incomingCallCategory()])
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

    /// Foreground: present a notification this app raised itself, and
    /// nothing else.
    ///
    /// A REMOTE one arriving while the app is frontmost is the race where a
    /// push sent to a closed app lands just after launch — the socket
    /// already delivered the same event, and a banner would double-notify.
    /// A LOCAL one is the opposite case by construction: ChatNotifier
    /// raises it only when the user is NOT looking at that chat, which on a
    /// Mac includes "frontmost, with a different conversation selected".
    /// Suppressing those would leave the app back where it started, telling
    /// its owner nothing.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        let isLocal = notification.request.content.userInfo[ChatNotifier.localKey] as? Bool ?? false
        completionHandler(isLocal ? [.banner, .sound, .list] : [])
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
        let content = response.notification.request.content
        // An incoming call's banner: Answer and Decline are its buttons,
        // and a plain click brings the call window forward — which RootView
        // does on its own once the app is active.
        if content.categoryIdentifier == ChatNotifier.incomingCallCategoryID {
            let action = response.actionIdentifier
            Task { @MainActor in
                switch action {
                case ChatNotifier.answerActionID:
                    MacAppDelegate.callManager?.acceptIncoming()
                case ChatNotifier.declineActionID:
                    MacAppDelegate.callManager?.declineIncoming()
                default:
                    NSApp.activate()
                }
                completionHandler()
            }
            return
        }
        let route = PushRoute.parse(userInfo: content.userInfo)
        Task { @MainActor in
            MacAppDelegate.session?.pendingPushRoute = route
            completionHandler()
        }
    }
}

#endif
