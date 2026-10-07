//
//  MacLoginItem.swift
//  FamilyConnect
//
//  Open at Login (#80, docs/mac-menu-bar-2026-10-07.md): SMAppService's
//  main-app login item, which needs no helper and works in the sandbox, and
//  the one fact about a launch that decides whether it starts in the menu
//  bar — whether macOS made it at login.
//

#if os(macOS)

import AppKit
import Carbon
import os
import ServiceManagement

@MainActor
enum MacLoginItem {

    /// Set once, in applicationWillFinishLaunching, while the open event
    /// that launched the app is still the current one.
    private(set) static var launchedAtLogin = false

    /// A login item's launch says so in its open-application event — the
    /// event's `keyAEPropData` is `keyAELaunchedAsLogInItem`.
    static func noteLaunch() {
        guard let event = NSAppleEventManager.shared().currentAppleEvent else { return }
        launchedAtLogin =
            event.eventID == AEEventID(kAEOpenApplication)
            && event.paramDescriptor(forKeyword: AEKeyword(keyAEPropData))?.enumCodeValue
                == OSType(keyAELaunchedAsLogInItem)
    }

    static var state: MenuBarRules.LoginItem {
        switch SMAppService.mainApp.status {
        case .enabled: .on
        case .notRegistered: .off
        case .requiresApproval: .needsApproval
        case .notFound: .unavailable
        @unknown default: .unavailable
        }
    }

    /// Turn it on or off; what it is afterwards is `state`, which is what the
    /// switch then shows — a register macOS refused leaves it off.
    static func set(_ on: Bool) {
        do {
            if on {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            AppLog.app.error("Open at Login could not be changed: \(error.localizedDescription, privacy: .public)")
        }
    }

    /// System Settings ▸ General ▸ Login Items, where a login item somebody
    /// switched off there is switched back on.
    static func openSystemSettings() {
        SMAppService.openSystemSettingsLoginItems()
    }
}

#endif
