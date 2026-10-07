//
//  MenuBarRulesTests.swift
//  FamilyConnectTests
//
//  The Mac in the menu bar (#80, docs/mac-menu-bar-2026-10-07.md): every
//  decision MacMenuBar, MacHotKey and MacLoginItem wire to AppKit — the Dock
//  icon for what is on the screen, what a close does, whether a launch
//  starts hidden, the shortcut, the icon's count and Open at Login's switch.
//

import Testing
@testable import FamilyConnect

struct MenuBarRulesTests {

    private typealias W = MenuBarRules.Window

    private static let shown = W(isVisible: true, isMiniaturized: false, canBecomeMain: true)
    private static let gone = W(isVisible: false, isMiniaturized: false, canBecomeMain: true)
    private static let docked = W(isVisible: false, isMiniaturized: true, canBecomeMain: true)
    /// The status item's own window, a tooltip, the menu bar: never "a window open".
    private static let furniture = W(isVisible: true, isMiniaturized: false, canBecomeMain: false)

    @Test("with no window on the screen the app is its menu bar icon; with one, a Dock app")
    func policyFollowsTheWindows() {
        #expect(MenuBarRules.policy(keepsMenuBar: true, appHidden: false, current: .regular, windows: []) == .accessory)
        #expect(MenuBarRules.policy(keepsMenuBar: true, appHidden: false, current: .regular,
                                    windows: [Self.gone, Self.furniture]) == .accessory)
        #expect(MenuBarRules.policy(keepsMenuBar: true, appHidden: false, current: .accessory,
                                    windows: [Self.gone, Self.shown]) == .regular)
    }

    @Test("a minimised window is open: the Dock is how it comes back")
    func minimisedKeepsTheDockIcon() {
        #expect(MenuBarRules.policy(keepsMenuBar: true, appHidden: false, current: .regular,
                                    windows: [Self.docked]) == .regular)
    }

    @Test("a hidden app (⌘H) keeps what it was: its windows only read as not visible")
    func hiddenAppKeepsItsPolicy() {
        #expect(MenuBarRules.policy(keepsMenuBar: true, appHidden: true, current: .regular, windows: [Self.gone]) == .regular)
        #expect(MenuBarRules.policy(keepsMenuBar: true, appHidden: true, current: .accessory, windows: [Self.gone]) == .accessory)
    }

    @Test("without the menu bar icon it is the Dock app it always was")
    func noMenuBarIsAlwaysRegular() {
        #expect(MenuBarRules.policy(keepsMenuBar: false, appHidden: false, current: .accessory, windows: []) == .regular)
        #expect(MenuBarRules.policy(keepsMenuBar: false, appHidden: true, current: .accessory, windows: []) == .regular)
    }

    @Test("only the main window hides on close, only with the icon kept, never on the way out")
    func closeHides() {
        #expect(MenuBarRules.closeHides(keepsMenuBar: true, isMainWindow: true, quitting: false))
        #expect(!MenuBarRules.closeHides(keepsMenuBar: true, isMainWindow: false, quitting: false))
        #expect(!MenuBarRules.closeHides(keepsMenuBar: false, isMainWindow: true, quitting: false))
        #expect(!MenuBarRules.closeHides(keepsMenuBar: true, isMainWindow: true, quitting: true))
        // A second main window closes: the menu bar holds one hidden window, never two.
        #expect(!MenuBarRules.closeHides(keepsMenuBar: true, isMainWindow: true, quitting: false, anotherMainOpen: true))
    }

    @Test("a launch at login starts hidden only when there is an icon to come back from")
    func startsHidden() {
        #expect(MenuBarRules.startsHidden(launchedAtLogin: true, keepsMenuBar: true))
        #expect(!MenuBarRules.startsHidden(launchedAtLogin: true, keepsMenuBar: false))
        #expect(!MenuBarRules.startsHidden(launchedAtLogin: false, keepsMenuBar: true))
        #expect(!MenuBarRules.startsHidden(launchedAtLogin: false, keepsMenuBar: false))
    }

    @Test("the shortcut shows the window from anywhere, and hides it when it is the one in front")
    func hotKey() {
        #expect(MenuBarRules.hotKeyAction(appActive: false, mainWindowKey: false, keepsMenuBar: true) == .show)
        // The app in front, but on Settings or a conversation window: show the main one.
        #expect(MenuBarRules.hotKeyAction(appActive: true, mainWindowKey: false, keepsMenuBar: true) == .show)
        #expect(MenuBarRules.hotKeyAction(appActive: true, mainWindowKey: true, keepsMenuBar: true) == .hide)
        // Nowhere to hide it to.
        #expect(MenuBarRules.hotKeyAction(appActive: true, mainWindowKey: true, keepsMenuBar: false) == .show)
    }

    @Test("the count beside the icon: nothing at zero, never wider than 99+")
    func countText() {
        #expect(MenuBarRules.countText(0) == nil)
        #expect(MenuBarRules.countText(-3) == nil)
        #expect(MenuBarRules.countText(1) == "1")
        #expect(MenuBarRules.countText(99) == "99")
        #expect(MenuBarRules.countText(100) == "99+")
    }

    @Test("Open at Login: switched off in System Settings still reads on, and says why")
    func loginSwitch() {
        #expect(MenuBarRules.loginSwitchOn(.on))
        #expect(MenuBarRules.loginSwitchOn(.needsApproval))
        #expect(!MenuBarRules.loginSwitchOn(.off))
        #expect(!MenuBarRules.loginSwitchOn(.unavailable))
        #expect(MenuBarRules.loginSwitchEnabled(.needsApproval))
        #expect(!MenuBarRules.loginSwitchEnabled(.unavailable))
    }
}
