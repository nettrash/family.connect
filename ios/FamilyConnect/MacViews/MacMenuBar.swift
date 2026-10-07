//
//  MacMenuBar.swift
//  FamilyConnect
//
//  The Mac in the menu bar (issue #80, docs/mac-menu-bar-2026-10-07.md):
//  an icon that opens the window on a click and offers Open, Settings… and
//  Quit on a right-click; the main window HIDDEN, not closed, by its close
//  button and ⌘W, so RootView — which opens the call window, applies a
//  notification's route and holds the scene — lives on; and the Dock icon
//  only while a window is open. Every decision is MenuBarRules; this file is
//  the AppKit around it.
//
//  An NSStatusItem, not a SwiftUI MenuBarExtra: a MenuBarExtra's click can
//  only open its menu or its panel, and the click here opens the window, as
//  the Windows icon's does.
//

#if os(macOS)

import AppKit
import os

@MainActor
final class MacMenuBar: NSObject {

    static let shared = MacMenuBar()

    private var item: NSStatusItem?
    private var unread = 0
    /// What the app is now — set only here, so it is never asked of AppKit.
    private var policy: MenuBarRules.Policy = .regular
    /// The main window this file hid, to bring back exactly that one. A
    /// window that was really closed stays in NSApp.windows (measured, see
    /// LaunchWindowBackstop) and must never be ordered front again.
    private weak var hiddenMain: NSWindow?
    private var quitting = false
    private var keyMonitor: Any?
    private var observers: [NSObjectProtocol] = []
    private var recomputeQueued = false

    /// The main WindowGroup's windows: SwiftUI names them after the group's
    /// id ("main-AppWindow-1"). The conversation, board, attachment, call and
    /// Settings windows close as they always did.
    static func isMain(_ window: NSWindow) -> Bool {
        guard let id = window.identifier?.rawValue else { return false }
        return id == MacWindow.main || id.hasPrefix(MacWindow.main + "-")
    }

    // MARK: - Starting, and the setting

    /// Once, from applicationDidFinishLaunching. `hidden` is a launch at
    /// login that starts in the menu bar (MenuBarRules.startsHidden).
    func start(hidden: Bool) {
        watchWindows()
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            // Delivered on the main thread; only the two value-type facts
            // cross into the main actor, never the event itself.
            let flags = event.modifierFlags
            let characters = event.charactersIgnoringModifiers
            let consumed = MainActor.assumeIsolated {
                MacMenuBar.shared.closeShortcut(flags: flags, characters: characters)
            }
            return consumed ? nil : event
        }
        MacHotKey.shared.onPress = { MacMenuBar.shared.hotKeyPressed() }
        apply()
        if hidden {
            // The window is already in NSApp.windows by now (measured, see
            // LaunchWindowBackstop) — and once more a beat later, for a
            // machine under load at login.
            hideAllMain()
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                MainActor.assumeIsolated { MacMenuBar.shared.hideAllMain() }
            }
        }
    }

    /// The settings changed (or the app started): the icon there or not,
    /// the shortcut registered or not, the policy for what is on the screen.
    func apply() {
        if AppSettings.keepsRunningInMenuBar {
            if item == nil { makeItem() }
        } else {
            if let item {
                NSStatusBar.system.removeStatusItem(item)
                self.item = nil
            }
            // No icon to come back from: a window hidden into it comes back
            // now, behind whatever turned the setting off.
            if let window = hiddenMain {
                hiddenMain = nil
                window.orderBack(nil)
            }
        }
        MacHotKey.shared.apply(AppSettings.opensWithHotKey)
        recompute()
    }

    /// Quit chosen: nothing hides on the way out.
    func willQuit() {
        quitting = true
    }

    // MARK: - The icon

    private func makeItem() {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        if let button = item.button {
            let image = NSImage(
                systemSymbolName: "bubble.left.and.bubble.right.fill",
                accessibilityDescription: "Family Connect")
            image?.isTemplate = true
            button.image = image
            button.imagePosition = .imageLeading
            button.target = self
            button.action = #selector(clicked(_:))
            button.sendAction(on: [.leftMouseUp, .rightMouseUp])
        }
        self.item = item
        drawCount()
    }

    /// The unread total, from UnreadBadge — beside the icon, in its tooltip
    /// ("(3) Family Connect", the window title's form), and for VoiceOver.
    func setUnread(_ count: Int) {
        unread = max(0, count)
        drawCount()
    }

    private func drawCount() {
        guard let button = item?.button else { return }
        let text = MenuBarRules.countText(unread)
        button.title = text.map { " " + $0 } ?? ""
        button.toolTip = text.map { "(\($0)) Family Connect" } ?? "Family Connect"
        button.setAccessibilityLabel(
            unread > 0
                ? String(localized: "Family Connect, \(unread) unread")
                : "Family Connect")
    }

    @objc private func clicked(_ sender: NSStatusBarButton) {
        let event = NSApp.currentEvent
        if event?.type == .rightMouseUp || event?.modifierFlags.contains(.control) == true {
            showMenu()
        } else {
            showMainWindow()
        }
    }

    private func showMenu() {
        guard let item else { return }
        let menu = NSMenu()
        menu.addItem(withTitle: String(localized: "Open Family Connect"), action: #selector(openChosen), keyEquivalent: "")
            .target = self
        menu.addItem(withTitle: String(localized: "Settings…"), action: #selector(settingsChosen), keyEquivalent: "")
            .target = self
        menu.addItem(.separator())
        menu.addItem(withTitle: String(localized: "Quit Family Connect"), action: #selector(quitChosen), keyEquivalent: "")
            .target = self
        // The status item's own way to drop a menu: attached for one click.
        item.menu = menu
        item.button?.performClick(nil)
        item.menu = nil
    }

    @objc private func openChosen() { showMainWindow() }

    @objc private func settingsChosen() {
        become(.regular)
        comeForward(nil)
        let before = Set(NSApp.windows.filter(\.isVisible).map(ObjectIdentifier.init))
        let settings = LaunchWindowBackstop.commandItem(key: ",", in: NSApp.mainMenu)
        if let settings, let action = settings.action {
            NSApp.sendAction(action, to: settings.target, from: settings)
        } else {
            AppLog.app.error("The menu bar icon found no Settings… item to open")
            return
        }
        // SwiftUI opens the Settings window a moment AFTER the action — behind
        // the app in front, measured on the owner's Mac — so it is brought
        // forward itself once it is there, and again when the policy change
        // has landed (as showMainWindow does for the main window).
        for delay in [0.05, 0.2] {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                MainActor.assumeIsolated {
                    let bar = MacMenuBar.shared
                    bar.comeForward(bar.settingsWindow(notIn: before))
                }
            }
        }
    }

    /// The Settings scene's window: SwiftUI names it
    /// "com_apple_SwiftUI_Settings_window"; failing that, the one window that
    /// came up since `before`.
    private func settingsWindow(notIn before: Set<ObjectIdentifier>) -> NSWindow? {
        let visible = NSApp.windows.filter { $0.isVisible && $0.canBecomeKey }
        return visible.first { $0.identifier?.rawValue.contains("Settings") == true }
            ?? visible.first { !before.contains(ObjectIdentifier($0)) && !Self.isMain($0) }
    }

    @objc private func quitChosen() {
        quitting = true
        NSApp.terminate(nil)
        // ⌘Q over a video message can say Keep (applicationShouldTerminate):
        // then the app runs on, and hides as before.
        DispatchQueue.main.async { MainActor.assumeIsolated { MacMenuBar.shared.quitting = false } }
    }

    // MARK: - The main window

    /// The icon, the shortcut, a notification, a Dock click: the main window
    /// in front — the one hidden, or a new one when it was really closed.
    func showMainWindow() {
        become(.regular)
        if let window = visibleMain() ?? hiddenMain {
            if window.isMiniaturized { window.deminiaturize(nil) }
            hiddenMain = nil
            comeForward(window)
        } else {
            openNewMain()
            comeForward(nil)
        }
        // An app that was an accessory a moment ago is not yet allowed in front:
        // measured on the owner's Mac, the window came up BEHIND the app in
        // front. So once more when the policy change has landed — the window
        // SwiftUI just made included.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) {
            MainActor.assumeIsolated { MacMenuBar.shared.comeForward(MacMenuBar.shared.visibleMain()) }
        }
    }

    /// The app active and the window in front of every app's. Cooperative
    /// activation (macOS 14) lets a request through only when the person just
    /// interacted with this app — a click on its icon, its shortcut, its
    /// notification are exactly that — and `ignoringOtherApps` is what still
    /// asks for it outright on the systems that honour it. `orderFrontRegardless`
    /// puts the window in front even in the moment before the app is active.
    private func comeForward(_ window: NSWindow?) {
        NSApp.activate(ignoringOtherApps: true)
        guard let window else { return }
        window.makeKeyAndOrderFront(nil)
        window.orderFrontRegardless()
    }

    /// The main window off the screen and into the menu bar: a window that
    /// went away, for every recording in it (S8.3 of #79), and the Dock icon
    /// gone if nothing else is open.
    func hide(_ window: NSWindow) {
        VoiceRecordingArbiter.shared.windowWentAway(window)
        window.orderOut(nil)
        hiddenMain = window
        recompute()
    }

    private func hideAllMain() {
        for window in NSApp.windows where Self.isMain(window) && window.isVisible {
            hide(window)
        }
    }

    /// Another main window on the screen (or in the Dock) besides this one.
    private func anotherMain(than window: NSWindow) -> Bool {
        NSApp.windows.contains { $0 !== window && Self.isMain($0) && ($0.isVisible || $0.isMiniaturized) }
    }

    private func visibleMain() -> NSWindow? {
        NSApp.windows.first { Self.isMain($0) && ($0.isVisible || $0.isMiniaturized) }
    }

    /// No main window to bring back: SwiftUI's own File ▸ New Window, as the
    /// launch backstop opens one, and a reopen through LaunchServices if the
    /// menu ever lacks it.
    private func openNewMain() {
        if let item = LaunchWindowBackstop.newWindowMenuItem(in: NSApp.mainMenu), let action = item.action,
            NSApp.sendAction(action, to: item.target, from: item)
        {
            return
        }
        NSWorkspace.shared.open(Bundle.main.bundleURL)
    }

    private func hotKeyPressed() {
        let key = NSApp.keyWindow.map(Self.isMain) ?? false
        switch MenuBarRules.hotKeyAction(
            appActive: NSApp.isActive, mainWindowKey: key, keepsMenuBar: AppSettings.keepsRunningInMenuBar)
        {
        case .show:
            showMainWindow()
        case .hide:
            if let window = NSApp.keyWindow { hide(window) }
        }
    }

    // MARK: - Close means hide

    /// ⌘W on the main window, while the icon is kept: hidden, and the key
    /// press consumed. A sheet over it is the key window, and keeps its own ⌘W.
    private func closeShortcut(flags: NSEvent.ModifierFlags, characters: String?) -> Bool {
        guard flags.intersection(.deviceIndependentFlagsMask) == .command,
            characters == "w",
            let window = NSApp.keyWindow, Self.isMain(window),
            MenuBarRules.closeHides(
                keepsMenuBar: AppSettings.keepsRunningInMenuBar, isMainWindow: true, quitting: quitting,
                anotherMainOpen: anotherMain(than: window))
        else { return false }
        hide(window)
        return true
    }

    /// The main window's red button, taken over: it hides while the icon is
    /// kept and closes otherwise — exactly what it did before. Put back each
    /// time the window comes forward, in case SwiftUI rebuilt the title bar.
    private func takeCloseButton(of window: NSWindow) {
        guard Self.isMain(window), let button = window.standardWindowButton(.closeButton),
            button.target !== self
        else { return }
        button.target = self
        button.action = #selector(closePressed(_:))
    }

    @objc private func closePressed(_ sender: NSButton) {
        guard let window = sender.window else { return }
        if MenuBarRules.closeHides(
            keepsMenuBar: AppSettings.keepsRunningInMenuBar, isMainWindow: Self.isMain(window), quitting: quitting,
            anotherMainOpen: anotherMain(than: window))
        {
            hide(window)
        } else {
            window.performClose(sender)
        }
    }

    // MARK: - The Dock icon

    private func watchWindows() {
        let center = NotificationCenter.default
        let names: [Notification.Name] = [
            NSWindow.didBecomeKeyNotification, NSWindow.didBecomeMainNotification,
            NSWindow.didMiniaturizeNotification, NSWindow.didDeminiaturizeNotification,
            NSWindow.didChangeOcclusionStateNotification, NSWindow.willCloseNotification,
            NSApplication.didUnhideNotification,
        ]
        for name in names {
            observers.append(
                center.addObserver(forName: name, object: nil, queue: .main) { note in
                    let window = note.object as? NSWindow
                    let closing = note.name == NSWindow.willCloseNotification
                    MainActor.assumeIsolated {
                        let bar = MacMenuBar.shared
                        if let window {
                            if closing, window === bar.hiddenMain { bar.hiddenMain = nil }
                            if !closing { bar.takeCloseButton(of: window) }
                        }
                        // After the close has happened: a closing window is
                        // still visible while its notification is delivered.
                        bar.recompute()
                    }
                })
        }
    }

    /// The policy for what is on the screen now, worked out on the next turn
    /// of the run loop so a burst of window notifications is one decision.
    private func recompute() {
        guard !recomputeQueued else { return }
        recomputeQueued = true
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                let bar = MacMenuBar.shared
                bar.recomputeQueued = false
                let windows = NSApp.windows.map {
                    MenuBarRules.Window(
                        isVisible: $0.isVisible, isMiniaturized: $0.isMiniaturized,
                        canBecomeMain: $0.canBecomeMain)
                }
                bar.become(
                    MenuBarRules.policy(
                        keepsMenuBar: AppSettings.keepsRunningInMenuBar, appHidden: NSApp.isHidden,
                        current: bar.policy, windows: windows))
            }
        }
    }

    private func become(_ next: MenuBarRules.Policy) {
        guard next != policy else { return }
        policy = next
        NSApp.setActivationPolicy(next == .regular ? .regular : .accessory)
    }
}

#endif
