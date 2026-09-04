//
//  LaunchWindowBackstopTests.swift
//  FamilyConnectTests
//
//  #52: a Mac launch that ends with zero windows, a live run loop and no way
//  back — no view to click, and a Dock icon that is already running.
//
//  The invariant is "after any launch, at least one window exists", and the
//  end-to-end proof of it is MacLaunchSmokeUITests, which launches the real
//  app with NO -ApplePersistenceIgnoreState and waits for a window. What is
//  pinned HERE is the judgement that test cannot see: which windows count as
//  a window, which menu item is the door out, and the order the two doors
//  are tried in — every one of which was a measurement, and every one of
//  which is silently wrong-able.
//
//  THE ONE THAT COST THE MOST TO LEARN is `closedWindowStillCounted`. A
//  closed NSWindow does NOT leave `NSApp.windows`: closing an app's only
//  window leaves a count of exactly 1, with `isVisible` false. An
//  implementation that asks `NSApp.windows.isEmpty` therefore NEVER fires,
//  on the very launch it exists for.
//

#if os(macOS)

import AppKit
import Testing
@testable import FamilyConnect

@MainActor
struct LaunchWindowBackstopTests {

    /// A window as the backstop sees it. Three booleans instead of a real
    /// NSWindow, so "miniaturized" and "closed" are states this test can
    /// simply assert rather than drive through the window server.
    private struct Window: BackstopWindow {
        var isVisible = false
        var isMiniaturized = false
        var canBecomeMain = false
    }

    // MARK: - What counts as a window

    @Test("no windows at all is the failure the backstop exists for")
    func noWindows() {
        #expect(LaunchWindowBackstop.hasUsableWindow([Window]()) == false)
    }

    @Test("a CLOSED window is still in NSApp.windows and must not count")
    func closedWindowStillCounted() {
        // Measured on the built Mac app: after closing its only window,
        // NSApp.windows was still [AppKitWindow/vis=false/mini=false].
        let closed = Window(isVisible: false, isMiniaturized: false, canBecomeMain: false)
        #expect(LaunchWindowBackstop.hasUsableWindow([closed]) == false,
                "asking NSApp.windows.isEmpty would report a window here and never recover")
    }

    @Test("an ordinary open window counts")
    func openWindowCounts() {
        let open = Window(isVisible: true, isMiniaturized: false, canBecomeMain: true)
        #expect(LaunchWindowBackstop.hasUsableWindow([open]))
    }

    @Test("a miniaturized window counts even though it is not visible")
    func miniaturizedCounts() {
        // isVisible is false for a window in the Dock. It is still one click
        // from the person, so opening a second one would be wrong.
        let dock = Window(isVisible: false, isMiniaturized: true, canBecomeMain: false)
        #expect(LaunchWindowBackstop.hasUsableWindow([dock]))
    }

    @Test("AppKit's own furniture is not a window")
    func furnitureDoesNotCount() {
        // Status items, tooltip and popover backing windows are visible and
        // in NSApp.windows, and none of them can become main. Counting them
        // is how a "recovered" app still shows nothing.
        let furniture = Window(isVisible: true, isMiniaturized: false, canBecomeMain: false)
        #expect(LaunchWindowBackstop.hasUsableWindow([furniture]) == false)
    }

    @Test("one real window among the furniture is enough")
    func mixedList() {
        let list = [
            Window(isVisible: true, canBecomeMain: false),
            Window(isVisible: false),
            Window(isVisible: true, canBecomeMain: true),
        ]
        #expect(LaunchWindowBackstop.hasUsableWindow(list))
    }

    // MARK: - Finding the door out

    private func menu(_ items: [NSMenuItem]) -> NSMenu {
        let root = NSMenu()
        for item in items { root.addItem(item) }
        return root
    }

    private func item(
        _ title: String, key: String, modifiers: NSEvent.ModifierFlags = [.command],
        action: Selector? = #selector(NSApplication.stop(_:))
    ) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
        item.keyEquivalentModifierMask = modifiers
        return item
    }

    @Test("⌘N is found however deep the submenu, and whatever it is called")
    func findsNewWindowItem() {
        // Matched by shortcut, not by title: this app is localized into nine
        // languages and SwiftUI's action selector is private.
        let file = NSMenuItem(title: "Datei", action: nil, keyEquivalent: "")
        let submenu = menu([item("Neues Fenster", key: "n"), item("Schließen", key: "w")])
        file.submenu = submenu
        let root = menu([NSMenuItem(title: "App", action: nil, keyEquivalent: ""), file])

        #expect(LaunchWindowBackstop.newWindowMenuItem(in: root)?.title == "Neues Fenster")
    }

    @Test("a shortcut with extra modifiers is a different command")
    func ignoresOtherModifiers() {
        let root = menu([item("New Window in Tab", key: "n", modifiers: [.command, .shift])])
        #expect(LaunchWindowBackstop.newWindowMenuItem(in: root) == nil)
    }

    @Test("an item with no action cannot open anything")
    func ignoresActionlessItem() {
        let root = menu([item("New Window", key: "n", action: nil)])
        #expect(LaunchWindowBackstop.newWindowMenuItem(in: root) == nil)
    }

    @Test("two ⌘N items mean the shortcut no longer names one command")
    func refusesToGuessBetweenTwo() {
        // Better to fall through to the LaunchServices reopen than to fire
        // whichever command happened to be added first.
        let root = menu([item("New Window", key: "n"), item("New Note", key: "n")])
        #expect(LaunchWindowBackstop.newWindowMenuItem(in: root) == nil)
    }

    @Test("no menu bar yet, no item")
    func noMenu() {
        #expect(LaunchWindowBackstop.newWindowMenuItem(in: nil) == nil)
    }

    // MARK: - The decision

    private var open: [Window] { [Window(isVisible: true, canBecomeMain: true)] }

    @Test("the normal launch does nothing at all")
    func normalLaunchIsUntouched() {
        var performed = false
        var reopened = false
        let outcome = LaunchWindowBackstop.recover(
            windows: open, hidden: false, menuItem: item("New Window", key: "n"),
            perform: { _ in performed = true; return true },
            reopen: { reopened = true; return true })

        #expect(outcome == .windowAlreadyOpen)
        #expect(performed == false)
        #expect(reopened == false)
    }

    @Test("a hidden app is left hidden")
    func hiddenLaunchIsLeftAlone() {
        // A login item or `open -j` launch: the window exists, it is just not
        // on screen, and forcing one open would shove the app in front of
        // whatever the person is actually doing.
        var acted = false
        let outcome = LaunchWindowBackstop.recover(
            windows: [Window](), hidden: true, menuItem: item("New Window", key: "n"),
            perform: { _ in acted = true; return true },
            reopen: { acted = true; return true })

        #expect(outcome == .windowAlreadyOpen)
        #expect(acted == false)
    }

    @Test("windowless: the menu item is tried first and LaunchServices is not")
    func windowlessUsesTheMenuFirst() {
        // In-process, so it needs no entitlement, does not bounce the Dock
        // icon and does not depend on the bundle still being where
        // LaunchServices last saw it.
        var reopened = false
        let outcome = LaunchWindowBackstop.recover(
            windows: [Window()], hidden: false, menuItem: item("New Window", key: "n"),
            perform: { _ in true },
            reopen: { reopened = true; return true })

        #expect(outcome == .openedViaMenu)
        #expect(reopened == false)
    }

    @Test("a menu item that refuses falls through to the reopen")
    func fallsThroughWhenSendActionFails() {
        // NSApp.sendAction returns false when nothing in the responder chain
        // takes it — which is exactly the state a broken launch is in.
        let outcome = LaunchWindowBackstop.recover(
            windows: [Window()], hidden: false, menuItem: item("New Window", key: "n"),
            perform: { _ in false },
            reopen: { true })

        #expect(outcome == .openedViaReopen)
    }

    @Test("no menu item at all still reopens")
    func reopensWithoutAMenu() {
        let outcome = LaunchWindowBackstop.recover(
            windows: [Window](), hidden: false, menuItem: nil,
            perform: { _ in Issue.record("perform must not be called with no item"); return true },
            reopen: { true })

        #expect(outcome == .openedViaReopen)
    }

    @Test("both doors shut is reported, not silently swallowed")
    func bothDoorsShut() {
        // The log line this produces is the only evidence anyone will get.
        let outcome = LaunchWindowBackstop.recover(
            windows: [Window()], hidden: false, menuItem: nil,
            perform: { _ in true },
            reopen: { false })

        #expect(outcome == .couldNotOpen)
    }
}

#endif
