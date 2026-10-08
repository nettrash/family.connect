//
//  MacNotificationAccess.swift
//  FamilyConnect
//
//  What macOS lets Family Connect's notifications do (#84): the answer
//  Settings shows under "Tell me when a message arrives". A Mac where the
//  app was never allowed, or where its style is None, raises every banner
//  into nothing — with no error anywhere a person could see.
//

#if os(macOS)

import AppKit
import UserNotifications

enum MacNotificationAccess: Equatable {
    /// Not read yet.
    case unknown
    /// Allowed, with banners or alerts.
    case allowed
    /// Turned off for this app in System Settings (or declined when asked).
    case denied
    /// Allowed, but shown as None: they go to Notification Center unseen.
    case noBanners

    /// Whether the fix is in System Settings rather than in this app.
    var needsSystemSettings: Bool { self == .denied || self == .noBanners }

    /// The pure reading of macOS's own answer, so it is testable.
    static func reading(status: UNAuthorizationStatus, alertStyle: UNAlertStyle) -> MacNotificationAccess {
        switch status {
        case .denied: .denied
        case .notDetermined: .unknown
        default: alertStyle == .none ? .noBanners : .allowed
        }
    }

    @MainActor
    static func current() async -> MacNotificationAccess {
        let settings = await UNUserNotificationCenter.current().notificationSettings()
        return reading(status: settings.authorizationStatus, alertStyle: settings.alertStyle)
    }

    /// System Settings ▸ Notifications, opened on this app.
    @MainActor
    static func openSystemSettings() {
        let id = Bundle.main.bundleIdentifier ?? ""
        if let url = URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id=\(id)") {
            NSWorkspace.shared.open(url)
        }
    }
}

#endif
