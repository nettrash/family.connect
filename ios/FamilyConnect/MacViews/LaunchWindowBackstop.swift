//
//  LaunchWindowBackstop.swift
//  FamilyConnect
//
//  ONE INVARIANT: after any launch, this app has at least one window (#52).
//
//  WHY IT IS NOT ENOUGH TO FIX THE CAUSE. The cause of #52 was an unstable
//  window restoration identifier, and FamilyConnectApp fixes that by giving
//  the main WindowGroup a stable `id:`. But the failure it produced —
//  AppKit asks SwiftUI to restore a window, SwiftUI answers nil, and AppKit
//  puts NOTHING in its place — is a shape this app cannot detect from
//  inside SwiftUI and cannot recover from at all: with no window there is
//  no view to draw a button in, and the Dock icon is already running, so
//  clicking it is a no-op. Saved state also goes stale for reasons that
//  have nothing to do with us (an OS update, a crash mid-write, a future
//  scene added to the App body — this app has gained two already). So the
//  window count is checked once per launch and the app repairs itself.
//
//  MEASURED, so the timing is not a guess. On this machine, in both healthy
//  launch paths — a restore that succeeds, and a first launch with no saved
//  state at all — the window is ALREADY in NSApp.windows by the time
//  applicationDidFinishLaunching runs, and it is still the only window three
//  seconds later. The grace period below is therefore pure margin.
//
//  WHY THE PIECES ARE SPLIT THE WAY THEY ARE. Everything that decides
//  anything takes its world as a parameter — the window list, the menu, the
//  two ways of opening a window — so the decision is unit-testable without
//  a running NSApplication. `start()` is the only part that touches the
//  live app, and it is six lines.
//

#if os(macOS)

import AppKit
import os

/// The three facts about a window this file actually reads.
///
/// A protocol rather than plain `NSWindow` for one reason: a test can then
/// state the interesting cases — a closed window, a miniaturized one, a
/// status-bar window — as three booleans, instead of trying to drive a real
/// NSWindow into each of those states through the window server.
protocol BackstopWindow {
    var isVisible: Bool { get }
    var isMiniaturized: Bool { get }
    var canBecomeMain: Bool { get }
}

extension NSWindow: BackstopWindow {}

@MainActor
enum LaunchWindowBackstop {

    /// How long after launch the window count is checked. Measured latency
    /// for a healthy launch is zero (see the file comment); this is margin
    /// for a machine under load, and the only cost of it being generous is
    /// that a launch which IS broken stays broken for two more seconds.
    static let grace: TimeInterval = 2.0

    /// What the check decided, returned so a test can assert it and so the
    /// log says which door the app had to use.
    enum Outcome: Equatable {
        /// The normal path: a window was already there, nothing was done.
        case windowAlreadyOpen
        /// Recovered through the File ▸ New Window menu item SwiftUI
        /// installs for the main WindowGroup. In-process, so it works under
        /// the App Sandbox and does not go near LaunchServices.
        case openedViaMenu
        /// Recovered by asking LaunchServices to reopen us, which is
        /// literally what clicking the Dock icon does. The fallback for a
        /// day when the menu item is not there.
        case openedViaReopen
        /// Neither door worked. Nothing more this process can do; the log
        /// line is the only evidence anyone will ever get.
        case couldNotOpen
    }

    /// Is there a window a person can actually get to?
    ///
    /// NOT `windows.isEmpty`: measured, a closed NSWindow STAYS in
    /// `NSApp.windows` — closing the only window leaves a count of 1, with
    /// `isVisible` false. A miniaturized window counts too, because it is
    /// one click away in the Dock, and it is checked first because a
    /// miniaturized window reports `isVisible == false`.
    ///
    /// `canBecomeMain` is what keeps AppKit's own furniture — status items,
    /// the menu bar's own windows, tooltip and popover backing windows —
    /// from reading as "the user has a window".
    static func hasUsableWindow(_ windows: [some BackstopWindow]) -> Bool {
        windows.contains { $0.isMiniaturized || ($0.isVisible && $0.canBecomeMain) }
    }

    /// The File ▸ New Window item, found by its SHORTCUT rather than its
    /// title: the title is localized into nine languages here and the
    /// action selector is SwiftUI's private `menuAction:`, so ⌘N is the only
    /// part of it this app is entitled to recognise. Returns nil rather
    /// than guessing if the app ever grows a second ⌘N.
    static func newWindowMenuItem(in menu: NSMenu?) -> NSMenuItem? {
        guard let menu else { return nil }
        var found: NSMenuItem?
        func walk(_ menu: NSMenu) {
            for item in menu.items {
                if item.keyEquivalent == "n",
                    item.keyEquivalentModifierMask == [.command],
                    item.action != nil
                {
                    // A second match means the shortcut is no longer a
                    // reliable name for "open a new main window".
                    if found != nil {
                        found = nil
                        return
                    }
                    found = item
                }
                if let submenu = item.submenu { walk(submenu) }
            }
        }
        walk(menu)
        return found
    }

    /// The decision, with every effect injected.
    ///
    /// `hidden` short-circuits it, and that is the "do not open a window it
    /// should not" guard: an app launched hidden (a login item, `open -j`)
    /// has a real window that simply is not on screen, and forcing one open
    /// would drag it in front of whatever the person is doing.
    @discardableResult
    static func recover(
        windows: [some BackstopWindow],
        hidden: Bool,
        menuItem: NSMenuItem?,
        perform: (NSMenuItem) -> Bool,
        reopen: () -> Bool
    ) -> Outcome {
        if hidden || hasUsableWindow(windows) { return .windowAlreadyOpen }
        if let menuItem, perform(menuItem) { return .openedViaMenu }
        if reopen() { return .openedViaReopen }
        return .couldNotOpen
    }

    /// Wire the check to the live app. Call once, from
    /// applicationDidFinishLaunching.
    static func start() {
        DispatchQueue.main.asyncAfter(deadline: .now() + grace) {
            MainActor.assumeIsolated {
                let outcome = recover(
                    windows: NSApp.windows,
                    hidden: NSApp.isHidden,
                    menuItem: newWindowMenuItem(in: NSApp.mainMenu),
                    perform: { item in
                        // sendAction returns false when nothing in the
                        // responder chain takes it, which is the answer the
                        // fallback below needs.
                        guard let action = item.action else { return false }
                        return NSApp.sendAction(action, to: item.target, from: item)
                    },
                    // Sending ourselves a reopen through LaunchServices: the
                    // same event a Dock click delivers, and measured to open
                    // a window from a windowless sandboxed app.
                    reopen: { NSWorkspace.shared.open(Bundle.main.bundleURL) }
                )
                guard outcome != .windowAlreadyOpen else { return }
                AppLog.app.error(
                    "Launched with no window; opened one (\(String(describing: outcome), privacy: .public))")
            }
        }
    }
}

#endif
