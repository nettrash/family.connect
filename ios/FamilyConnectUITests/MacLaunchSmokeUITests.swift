//
//  MacLaunchSmokeUITests.swift
//  FamilyConnectUITests
//
//  The Mac's launch smoke test (#31).
//
//  WHY IT EXISTS. Until this file, a whole-scheme macOS test run printed
//
//      Cannot test target "FamilyConnectUITests" on "My Mac": …
//
//  and exited 0. That notice reads like a skip and scores like a pass, so
//  the Mac app — a shipping target with a 4,500-line MacViews tree — had
//  no evidence that it could reach its first screen. A green BUILD is not
//  that evidence: the Mac's failure class is the launch itself, and this
//  test found a live example of it on the first run (see the launch
//  arguments below).
//
//  WHAT IT ASSERTS, AND WHY NOT MORE. One thing: a cold launch with a
//  wiped slate paints the server-setup screen. That is the first stop of
//  the phase machine (RootView's `.needsServer`) and it is reachable with
//  NO SERVER ANYWHERE — `--uitest-reset` clears the stored URL in
//  FamilyConnectApp.init, and Debug/Release leave FC_DEFAULT_SERVER_URL
//  empty, so `bootstrap()` finds neither a stored URL nor a compiled
//  default and stops there. Nothing here opens a socket, which is the
//  point: it must run on a CI box with no server, no seed and no fixture.
//
//  (Release-nettrash DOES carry a compiled default, so the same launch
//  lands on the auth screen instead. Test actions use Debug; if somebody
//  points this at Release-nettrash, that mismatch is what they will see.)
//
//  WHY THE ASSERTIONS ARE NOT THE iOS ONES. FamilyConnectUITests.swift
//  waits on `app.navigationBars["Family Connect"]`. A Mac has no
//  navigation bar: `.navigationTitle` on a NavigationStack inside a
//  WindowGroup becomes the WINDOW's title, so that query matches nothing
//  here however correct the screen is. The durable anchors on this
//  platform are the window itself, the "Connect" button and the "Server
//  address" section header.
//
//  THERE IS DELIBERATELY NO SCREENSHOT. `app.screenshot()` on macOS
//  returns THE WHOLE DISPLAY when the app has no window of its own — the
//  first run of this test attached a picture of the developer's entire
//  desktop (mail, calendar, other terminals) to the result bundle, which
//  CI then uploads as an artifact. A launch smoke test is worth far less
//  than that, so it asserts and says nothing.
//

#if os(macOS)

import XCTest

final class MacLaunchSmokeUITests: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    @MainActor
    func testFreshLaunchReachesServerSetup() throws {
        let app = XCUIApplication()
        app.launchArguments = [
            // NOT optional, and the reason is the whole of #31's flake.
            //
            // macOS restores an app's windows from saved state kept per
            // bundle id (…/Data/tmp/me.nettrash.FamilyConnect.savedState).
            // A SwiftUI WindowGroup's window is restored through that path,
            // and when the restore fails AppKit opens NOTHING in its place:
            //
            //   _reopenWindowsAsNecessary… hasPersistentStateToRestore=1
            //   restoreWindowWithIdentifier:…-AppWindow-1
            //       className=SwiftUI.AppWindowsController
            //   …_block_invoke … window=0x0 error=(null)
            //
            // The result is a live app with a menu bar, an idle run loop
            // and zero windows. Measured on this tree: 8 of 8 plain
            // launches came up windowless, 8 of 8 with this flag came up
            // with the window. It is self-perpetuating in both directions,
            // because each exit saves whatever window count it had — which
            // is exactly the shape of a test that is green for a week and
            // then red forever, on a machine nobody changed.
            //
            // A UI test inherits that state from every earlier run of the
            // app on the same box, including runs XCUITest killed. Ignoring
            // it is the only way this test measures the app rather than the
            // residue of the last run.
            "-ApplePersistenceIgnoreState", "YES",
            "--uitest-reset",
        ]
        app.launch()

        // Generous timeouts, deliberately. Nothing here waits on a network;
        // it waits on ModelContainer opening the SwiftData store, which on
        // this platform lives in the App Group container shared by every
        // copy of the app on the machine (ModelConfiguration defaults to
        // `groupContainer: .automatic` and the entitlement names one). A
        // developer's own running /Applications/FamilyConnect.app holds
        // that file open, and a contended open is slow long before it is
        // fatal — audit F124 caught one wedged in __guarded_open_np. Ten
        // seconds would make that a red build on a green machine.
        XCTAssertTrue(
            app.windows.firstMatch.waitForExistence(timeout: 60),
            "the Mac app opened no window at all — check window restoration and the store open, not the view code")

        XCTAssertTrue(
            app.buttons["Connect"].waitForExistence(timeout: 60),
            "a wiped launch should paint the server-setup screen (RootView's .needsServer)")
        XCTAssertTrue(
            app.staticTexts["Server address"].exists,
            "the setup form's section header should be on screen with the button")
    }

    /// The invariant from #52: AFTER ANY LAUNCH, AT LEAST ONE WINDOW EXISTS.
    ///
    /// THE MISSING FLAG IS THE POINT. The test above passes
    /// `-ApplePersistenceIgnoreState YES` so that what it measures is the
    /// app rather than the residue of the last run — but that flag also
    /// switched off the very mechanism #52 lives in, which is how the bug
    /// went on being real while the suite was green. This test deliberately
    /// does NOT pass it: it launches with whatever saved state this machine
    /// has, which is what a person's Mac does every morning.
    ///
    /// It asserts nothing about WHICH screen, on purpose. Restoration can
    /// legitimately bring back a conversation window instead of the main
    /// one, and a store that will not open legitimately shows StoreErrorView
    /// — all of those are a window, and a window is the whole claim.
    ///
    /// `--uitest-reset` for the same reason the other test passes it: a
    /// launch that lands on server setup needs no server, no seed and no
    /// fixture, so this runs on a CI box with nothing on it.
    @MainActor
    func testLaunchWithWhateverSavedStateExistsStillHasAWindow() throws {
        let app = XCUIApplication()
        app.launchArguments = ["--uitest-reset"]
        app.launch()

        XCTAssertTrue(
            app.windows.firstMatch.waitForExistence(timeout: 60),
            """
            the Mac app reached a live run loop with no window — there is no way \
            out of that from inside the app, so this is a launch nobody can use. \
            Check the main WindowGroup's restoration id and LaunchWindowBackstop.
            """)
    }
}

#endif
