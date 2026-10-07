//
//  MenuBarRules.swift
//  FamilyConnect
//
//  The Mac in the menu bar (issue #80, docs/mac-menu-bar-2026-10-07.md):
//  every DECISION, with none of AppKit — so it is tested on both platforms'
//  runs. MacMenuBar, MacHotKey and MacLoginItem are the wiring around it.
//

import Foundation

nonisolated enum MenuBarRules {

    /// The three facts about a window the policy reads.
    struct Window: Equatable, Sendable {
        var isVisible: Bool
        var isMiniaturized: Bool
        /// AppKit's own furniture — the status item, the menu bar, tooltips
        /// and popovers — cannot become main, and is never "a window open".
        var canBecomeMain: Bool
    }

    /// What the app is in the Dock and ⌘-Tab.
    enum Policy: Equatable, Sendable {
        /// A Dock icon, a menu bar of its own, ⌘-Tab.
        case regular
        /// The menu bar icon and nothing else.
        case accessory
    }

    /// The Dock icon only while a window is open: with the menu bar icon kept
    /// and nothing on the screen, the icon IS the app. A minimised window is
    /// open — the Dock is how it comes back — and a hidden app (⌘H) keeps
    /// whatever it was, because every window of a hidden app reads as not
    /// visible and the Dock icon is how a hidden app is brought back.
    static func policy(keepsMenuBar: Bool, appHidden: Bool, current: Policy, windows: [Window]) -> Policy {
        guard keepsMenuBar else { return .regular }
        if appHidden { return current }
        let open = windows.contains { $0.canBecomeMain && ($0.isVisible || $0.isMiniaturized) }
        return open ? .regular : .accessory
    }

    /// The main window's close button and ⌘W hide it rather than close it
    /// while the menu bar icon is kept — so the view that opens the call
    /// window and holds the scene stays alive. Never on the way out, and
    /// only the LAST main window: a second one (File ▸ New Window) closes as
    /// it always did, so the menu bar never holds two hidden copies.
    static func closeHides(
        keepsMenuBar: Bool, isMainWindow: Bool, quitting: Bool, anotherMainOpen: Bool = false
    ) -> Bool {
        keepsMenuBar && isMainWindow && !quitting && !anotherMainOpen
    }

    /// A launch macOS made at login starts in the menu bar, its window hidden
    /// — and only when there IS a menu bar icon to come back from (Windows'
    /// `StartupSetting.StartsHidden`).
    static func startsHidden(launchedAtLogin: Bool, keepsMenuBar: Bool) -> Bool {
        launchedAtLogin && keepsMenuBar
    }

    /// What the global shortcut does.
    enum HotKeyAction: Equatable, Sendable {
        case show
        case hide
    }

    /// From any app it brings the window forward; pressed while the main
    /// window is the key window of the active app, it puts it back in the
    /// menu bar — or, with no menu bar icon to go back to, does nothing new
    /// and simply shows it.
    static func hotKeyAction(appActive: Bool, mainWindowKey: Bool, keepsMenuBar: Bool) -> HotKeyAction {
        appActive && mainWindowKey && keepsMenuBar ? .hide : .show
    }

    /// The text beside the menu bar icon: the unread count, nothing at zero,
    /// and never wider than "99+".
    static func countText(_ unread: Int) -> String? {
        guard unread > 0 else { return nil }
        return unread > 99 ? "99+" : String(unread)
    }

    /// Where Open at Login stands, from `SMAppService.Status` — kept as plain
    /// cases here so the rule is testable without the framework.
    enum LoginItem: Equatable, Sendable {
        /// Registered and allowed.
        case on
        /// Not registered.
        case off
        /// Registered, but switched off in System Settings ▸ General ▸ Login
        /// Items — the app cannot turn it back on (Windows' DisabledByUser).
        case needsApproval
        /// The service is not there at all (an unbundled or broken copy).
        case unavailable
    }

    /// Whether the switch reads on: needing approval is on in intent, and
    /// the note under it says what is missing.
    static func loginSwitchOn(_ state: LoginItem) -> Bool {
        state == .on || state == .needsApproval
    }

    /// Whether the switch can be flipped from here.
    static func loginSwitchEnabled(_ state: LoginItem) -> Bool {
        state != .unavailable
    }
}
