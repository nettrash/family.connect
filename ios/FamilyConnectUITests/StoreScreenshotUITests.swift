//
//  StoreScreenshotUITests.swift
//  FamilyConnectUITests
//
//  Captures the App Store screenshot set. Not a regression test — but it
//  now asserts, before every shutter, that the screen it is about to
//  photograph is the screen the file name claims. A store set that comes
//  out wrong is worse than one that does not come out at all: nothing
//  downstream re-reads these images, so a chat list filed as
//  `02-family-chat` ships (#55).
//
//  Run against the seeded fixture, once per device class:
//
//    server/scripts/seed-store-screenshots.sh
//    cd ios && DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
//      TEST_RUNNER_FC_UITEST_SERVER=http://127.0.0.1:8091 \
//      xcodebuild test -project FamilyConnect.xcodeproj -scheme FamilyConnect \
//        -destination 'id=<SIMULATOR UDID>' \
//        -only-testing:FamilyConnectUITests/StoreScreenshotUITests \
//        -resultBundlePath /tmp/shots.xcresult
//    xcrun xcresulttool export attachments \
//      --path /tmp/shots.xcresult --output-path /tmp/shots
//
//  NEVER pass CODE_SIGNING_ALLOWED=NO to this test (#45). An unsigned
//  build carries no application-identifier entitlement, so every keychain
//  write fails with OSStatus -34018 and the login dies as "Something went
//  wrong. Try again." — an auth failure that is really a signing failure.
//
//  ADDRESS SIMULATORS BY UDID, never by name+OS: two runtimes are
//  installed twice on this machine (26.1 and 26.4 each appear under two
//  builds), so `name=…,OS=26.1` resolves ambiguously.
//
//    6.9" iPhone  iPhone 17 Pro Max      1320x2868  C6681051-89A8-4D26-BD29-D9CB20EF4D8B
//    13"  iPad    iPad Pro 13-inch (M5)  2064x2752  B5839D57-056A-4C14-AB0C-A9F40EC50872
//
//  Both live on iOS 26.5, so one runtime covers both required slots.
//
//  ── THE FOUR TRAPS THIS FILE EXISTS TO NOT FALL INTO AGAIN (#55) ──
//
//  1. A LOGIN LEAVES TWO EXTRA WINDOWS BEHIND. One of them eats taps and
//     one does not, and telling them apart is the whole of this issue.
//
//     THE ONE THAT EATS TAPS is the AutoFill "Save Password?" alert. iOS
//     puts it in a UIWindow OF ITS OWN, and while it is up the window
//     holding the app reports `hittable=false` for everything in it —
//     measured on the 13" iPad right after a login:
//
//       app.windows.count = 2
//         [0] Window        hittable=false   ← the chat list, inert
//         [1] Window (Main) hittable=true    ← Alert 'Save Password?'
//
//     Element queries still FIND the chat rows (existence does not care
//     about being covered), so a test asserts its way past this and then
//     taps into a window that is not listening: no navigation, no visible
//     obstruction — the alert is a 320x272 box whose window is
//     full-screen and transparent around it — and no error. That is the
//     "second, empty Window (Main) stacked over the real one" of #55: it
//     is not empty, it is an alert, and the accessibility tree calls it
//     Main because it IS the key window.
//
//     It is a RACE, which is why it defeated five attempts and why it
//     reproduces on one device class and not the other in the same run:
//     the alert can arrive after `dismissSavePassword` has finished
//     polling for it and even after the chat list has appeared.
//
//     THE HARMLESS ONE is UIKit's UITextEffectsWindow, and it is the
//     keyboard's, not the login's. `app.windows.count` goes 1 (auth
//     screen) → 3 the instant a field takes focus (the keyboard's
//     `inputView` window plus the `SystemInputAssistantView` shortcut
//     bar) → 2 once the keyboard leaves. That survivor is full-screen,
//     genuinely EMPTY (two anonymous `Other` nodes and nothing else),
//     not hittable, and never torn down. Neither window is
//     `--uitest-reset`'s doing and neither is a stale SwiftUI scene: a
//     relaunch that skips the login is logged in and has exactly ONE
//     window. (#52 cannot be involved either — that `id:` is inside
//     `#if os(macOS)`.)
//
//     THE FIX IS THE RELAUNCH BELOW. Terminating the process destroys
//     both windows, and nothing types a password afterwards, so no
//     second alert is ever offered. Do not try to win the race instead.
//     CONSEQUENCE FOR THIS FILE: never ask `app.windows.firstMatch` for
//     the screen rectangle after a login — it can be either of them.
//
//  2. `--uitest-reset` DOES NOT CLEAR THE SWIFTDATA MESSAGE CACHE — or
//     rather it did not: FamilyConnectApp now deletes the store under the
//     same flag, for exactly this reason. Before that, a chat id reused
//     by a different fixture rendered the OTHER fixture's messages, and
//     one run photographed 1500 "Message number N" rows inside "The
//     Harpers".
//
//  3. A CHAT-LIST ROW SHOWS THE NEWEST MESSAGE AS ITS PREVIEW, so waiting
//     for that text proves nothing about having navigated — it matches on
//     the list too. Every step below waits on something only its own
//     screen has: the composer's attach button for the conversation, the
//     album/poll bubbles for the photo frame, the nav bar for the rest.
//
//  4. `app.swipeUp()` IS A SILENT NO-OP ON IPHONE IN LANDSCAPE. Twelve of
//     them left every row frame identical to the decimal. Everything here
//     scrolls with a coordinate press-and-drag, which works in both
//     orientations and on the iPad's floating Settings sheet.
//

import XCTest

final class StoreScreenshotUITests: XCTestCase {

    /// The seeded family. Kept here so a change to the seed script fails
    /// this test by name rather than by photographing an empty list.
    private let familyName = "The Harpers"
    /// Two bubbles from the seed that `03-photos-and-poll` is named after.
    /// The album publishes itself as one element ("Album, 1 of 4"); the
    /// poll's LAST option is the witness for the poll, because a poll
    /// whose bars are below the fold is not a screenshot of a poll.
    private let albumLabelPrefix = "Album, 1 of"
    private let pollLastOption = "Café by the park"
    /// The oldest message in the seeded thread — where `02-family-chat` is
    /// anchored, so that frame is the same on every device class instead
    /// of "wherever this screen size happens to open".
    private let firstMessagePrefix = "Half day tomorrow"

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    @MainActor
    func testCaptureStoreScreenshots() throws {
        guard let server = ProcessInfo.processInfo.environment["FC_UITEST_SERVER"] else {
            throw XCTSkip("set TEST_RUNNER_FC_UITEST_SERVER to a seeded server URL")
        }

        let app = XCUIApplication()
        app.launchArguments = [
            "--uitest-reset", "-v1.serverURL", server,
            // Force English, for two reasons. Store screenshots are
            // required only for the PRIMARY localization, so they must not
            // come out in whatever language the simulator happens to hold.
            // And the keyboard follows the language: an iPad left on a
            // Russian keyboard types Cyrillic into the username field, and
            // the only symptom is a login that never happens.
            "-AppleLanguages", "(en)",
            "-AppleLocale", "en_US",
        ]
        app.launch()

        try logIn(app, username: "nora", password: "password123")

        // RELAUNCH IMMEDIATELY AFTER THE LOGIN, and keep the session (no
        // --uitest-reset). This is the fix for #55, not a precaution: a
        // "Save Password?" alert that arrived late is sitting in a window
        // of its own with the whole app inert underneath it, and killing
        // the process is the only way to be rid of it that does not
        // depend on winning a race. It takes the leftover
        // UITextEffectsWindow with it. What comes back is a process that
        // has shown no keyboard, has one window, has been offered no
        // password to save, and is logged in — measured, and the state
        // every step below assumes.
        relaunch(app, server: server)

        // 1 — the chat list.
        let family = familyRow(in: app)
        XCTAssertTrue(family.waitForExistence(timeout: 20), "the seeded family should be listed")
        // THE TRIPWIRE FOR #55. Existence survives being covered;
        // hittability does not. If an alert in its own window is over the
        // list, this is the assertion that says so instead of letting six
        // screenshots come out of a tap that went nowhere.
        XCTAssertTrue(family.isHittable,
                      "the chat list exists but is not hittable — something in another window is over it (#55)")
        // Prove it is the LIST and not a conversation: the composer only
        // exists inside a thread (trap 3).
        XCTAssertFalse(composer(in: app).exists, "01-chats must photograph the list, not a thread")
        capture("01-chats")

        // 2 — the family thread, which carries the album, the poll, the
        //     location and a reacted bubble.
        family.tap()
        XCTAssertTrue(composer(in: app).waitForExistence(timeout: 20),
                      "the family row did not open a conversation — 02 would be the chat list again")
        sleep(3) // let the thread settle: resync, layout, image decode
        // Anchor on the START of the thread. A thread opens on its unread
        // divider, and where that lands depends on the screen: the 13"
        // iPad opens with the album and the poll already in frame, so 02
        // and 03 came out BYTE-IDENTICAL — two slots of a six-slot store
        // set showing the same picture. Pinning 02 to the oldest message
        // makes it the same frame on both device classes and leaves 03
        // with something of its own to say.
        XCTAssertTrue(scrollToThreadStart(in: app),
                      "could not reach the start of the thread for 02")
        assertNoBubbleMenu(in: app, shot: "02-family-chat")
        sleep(2) // let the scroll indicator fade — it is in the frame otherwise
        capture("02-family-chat")

        // 3 — the album and the poll in one frame. They sit BELOW where
        //     the thread opens (it lands on the unread divider), so this
        //     scrolls toward the newest, then backs off until both are on
        //     screen at once.
        XCTAssertTrue(scrollToPhotosAndPoll(in: app),
                      "could not get the album and the poll on screen together — 03 would be some other part of the thread")
        assertNoBubbleMenu(in: app, shot: "03-photos-and-poll")
        sleep(2) // let the scroll indicator fade — it is in the frame otherwise
        capture("03-photos-and-poll")

        back(in: app)

        // 4 — the board.
        if tapFirst(in: app, labels: ["Board"]) {
            XCTAssertTrue(app.staticTexts["Bins go out Tuesday"].waitForExistence(timeout: 10),
                          "the board did not open — 04 would be whatever was behind it")
            sleep(2)
            capture("04-board")
        }

        // RELAUNCH between sections rather than unwinding by hand. A sheet
        // left half-dismissed swallows the next tap, and the symptom is a
        // tap that "succeeds" and navigates nowhere — which cost an hour
        // on iPad. Relaunching without --uitest-reset keeps the session,
        // so this is a fresh navigation stack and a logged-in app.
        relaunch(app, server: server)
        XCTAssertTrue(familyRow(in: app).waitForExistence(timeout: 20),
                      "the session should survive a relaunch")

        // 5 — the family roster, where Safety lives.
        XCTAssertTrue(openSettings(app), "Settings should be on screen")
        capture("06-settings")

        // Scroll INSIDE the sheet. On iPad Settings is a small centred
        // sheet floating over the chat list, so `app.swipeUp()` scrolls
        // the list BEHIND it and the row never arrives (and on iPhone in
        // landscape it does nothing at all — trap 4). A coordinate drag
        // through the middle of the screen lands on the sheet itself.
        var roster = rosterRow(in: app)
        for _ in 0..<4 where !roster.exists {
            drag(in: app, from: 0.62, to: 0.34)
            sleep(1)
            roster = rosterRow(in: app)
        }
        if roster.waitForExistence(timeout: 5) {
            let rosterTitle = roster.label // "Manage Family" or "Family Members"
            roster.tap()
            // The NAV BAR, not a row: "Invite code" is a row on the
            // Settings sheet too, so it cannot tell the two apart.
            XCTAssertTrue(app.navigationBars[rosterTitle].firstMatch.waitForExistence(timeout: 10),
                          "the roster did not open — 05 would be the Settings sheet again")
            sleep(2)
            capture("05-family")
        } else {
            XCTFail("the family roster row was not reachable — the set is incomplete")
        }
    }

    // MARK: - Navigation

    @MainActor
    private func logIn(_ app: XCUIApplication, username: String, password: String) throws {
        let field = app.textFields["Username"]
        XCTAssertTrue(field.waitForExistence(timeout: 20), "should land on the auth screen")
        field.tap()
        field.typeText(username)

        let secure = app.secureTextFields["Password"]
        XCTAssertTrue(secure.waitForExistence(timeout: 5), "password field should exist")
        secure.tap()
        secure.typeText(password)

        // VERIFY BEFORE SUBMITTING. AutoFill can steal focus between the
        // two taps, and the failure mode is silent: the password lands in
        // the username field, the server rejects it, and the only symptom
        // is "Something went wrong" on a screen that looks fine. Reading
        // the field back turns that into a message naming what was typed.
        XCTAssertEqual(field.value as? String, username,
                       "the username field holds something else — AutoFill or focus stole the keystrokes")

        // Two elements carry "Log In" — the mode segment and the submit
        // button. Match order is not visual order, so take the lowest.
        //
        // The keyboard's return key does NOT submit this form: tapping it
        // leaves the screen exactly as it was, which reads as a silent
        // failure three assertions later.
        let submits = app.buttons.matching(NSPredicate(format: "label == 'Log In'"))
            .allElementsBoundByIndex
        XCTAssertFalse(submits.isEmpty, "no Log In button on the auth screen")
        submits.max(by: { $0.frame.minY < $1.frame.minY })?.tap()
        dismissSavePassword(app)

        // Fail here rather than three screens later: an auth error leaves
        // the app on this screen and every later query then misses.
        let stillOnAuth = app.staticTexts["Something went wrong. Try again."]
        if stillOnAuth.waitForExistence(timeout: 3) {
            capture("00-login-failed")
            XCTFail("login was rejected — is the seed fresh and the server on \(app.launchArguments)? "
                    + "(and was this built WITHOUT CODE_SIGNING_ALLOWED=NO — see #45)")
        }

        // The row is the proof the login landed; the chat list is the only
        // screen that has one.
        //
        // EXISTENCE, deliberately, not hittability: a "Save Password?"
        // alert may still be up in its own window, and under it every row
        // reports `hittable=false` (#55). The login has still succeeded,
        // and the relaunch the caller does next is what clears the alert.
        // Asserting hittability here would fail a login that worked.
        XCTAssertTrue(familyRow(in: app).waitForExistence(timeout: 20),
                      "login did not reach the chat list")
    }

    /// Terminate and start again WITHOUT `--uitest-reset`: the keychain
    /// token survives, so the app comes back logged in — see the note at
    /// the first call site for what this is really for.
    @MainActor
    private func relaunch(_ app: XCUIApplication, server: String) {
        app.terminate()
        app.launchArguments = [
            "-v1.serverURL", server,
            "-AppleLanguages", "(en)",
            "-AppleLocale", "en_US",
        ]
        app.launch()
        // Recorded, not asserted: the window count is the number that
        // explains a swallowed tap (#55), and it is not this test's
        // contract. Expect 1 here.
        XCTContext.runActivity(
            named: "after relaunch: \(app.windows.count) window(s), \(app.alerts.count) alert(s)"
        ) { _ in }
    }

    /// iOS offers to save the password it just saw. The alert lives in its
    /// own window and makes the app underneath untappable, so this tries
    /// to be rid of it — but it CANNOT be relied on: the alert can arrive
    /// after this has stopped looking. The relaunch after `logIn` is what
    /// actually guarantees it is gone (#55).
    @MainActor
    private func dismissSavePassword(_ app: XCUIApplication) {
        for host in [app, XCUIApplication(bundleIdentifier: "com.apple.springboard")] {
            let notNow = host.buttons["Not Now"]
            if notNow.waitForExistence(timeout: 4) {
                notNow.tap()
                return
            }
        }
    }

    /// Open Settings, retrying the tap.
    ///
    /// The toolbar button is a real, hittable 36pt button, but a tap sent
    /// while the list is still settling after a launch is swallowed
    /// silently — XCUITest reports the tap as delivered and nothing
    /// happens. Retrying is the difference between a flaky set and a
    /// reproducible one.
    @MainActor
    private func openSettings(_ app: XCUIApplication) -> Bool {
        // Anchor on the NAVIGATION BAR, not on a control further down.
        // "Log Out" sits at the bottom of a long Form and SwiftUI does not
        // render it until it scrolls near the viewport, so waiting for it
        // reports "Settings did not open" about a Settings screen that is
        // plainly open.
        func isOpen() -> Bool {
            app.navigationBars["Settings"].firstMatch.exists
                || app.staticTexts["Privacy"].firstMatch.exists
        }
        for attempt in 0..<3 {
            if isOpen() { return true }
            sleep(2) // let the presenting screen come to rest first
            let button = app.buttons["Settings"].firstMatch
            guard button.waitForExistence(timeout: 5) else { continue }
            button.tap()
            for _ in 0..<6 {
                if isOpen() { return true }
                usleep(500_000)
            }
            XCTContext.runActivity(named: "Settings did not open (attempt \(attempt + 1))") { _ in }
        }
        return false
    }

    /// The row into the roster. Its label depends on who is looking:
    /// an owner sees "Manage Family", a plain member "Family Members".
    @MainActor
    private func rosterRow(in app: XCUIApplication) -> XCUIElement {
        let owner = app.buttons["Manage Family"].firstMatch
        return owner.exists ? owner : app.buttons["Family Members"].firstMatch
    }

    @MainActor
    private func familyRow(in app: XCUIApplication) -> XCUIElement {
        app.collectionViews.staticTexts[familyName].firstMatch
    }

    /// The composer's attach button: present in a conversation and NOWHERE
    /// else, which is what makes it the honest witness for "did the row
    /// actually open" (trap 3).
    @MainActor
    private func composer(in app: XCUIApplication) -> XCUIElement {
        app.buttons["Attach a photo, video or file"].firstMatch
    }

    /// A bubble's long-press menu covering the thread is invisible to
    /// every element query behind it — everything under a context menu
    /// still answers `isHittable`, so no witness in this file can see one.
    /// Only its own buttons can, and "Reply" belongs to nothing else.
    @MainActor
    private func assertNoBubbleMenu(in app: XCUIApplication, shot: String) {
        XCTAssertFalse(app.buttons["Reply"].exists,
                       "a message context menu is covering the thread — \(shot) would photograph the menu")
    }

    @MainActor
    private func back(in app: XCUIApplication) {
        let back = app.navigationBars.buttons.firstMatch
        if back.exists { back.tap() }
        sleep(1)
    }

    // MARK: - Scrolling

    /// One press-and-drag through the middle of the screen, in normalized
    /// coordinates. `from > to` scrolls toward the NEWEST message.
    ///
    /// Never `swipeUp()`/`swipeDown()`: on iPhone in landscape they are
    /// silent no-ops (trap 4), and on the iPad's floating Settings sheet
    /// they scroll the list behind it instead.
    /// KEEP THE PRESS SHORT. The obvious way to creep onto an exact scroll
    /// offset — press for half a second so the list tracks the finger
    /// instead of flicking — cannot be used on a thread: half a second on
    /// a bubble IS the long press, and it opens the reaction capsule and
    /// the Reply/Copy/Share/Safety menu over the very frame being
    /// photographed. Measured: `03-photos-and-poll` came out as a context
    /// menu, and every element behind it still answered `isHittable`, so
    /// nothing in the test noticed. Short presses flick, flicking
    /// overshoots, and overshooting is the price of not doing that.
    @MainActor
    private func drag(in app: XCUIApplication, from: CGFloat, to: CGFloat) {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: from))
        let end = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: to))
        start.press(forDuration: 0.05, thenDragTo: end)
    }

    /// The part of the thread a reader can actually see: below the
    /// navigation bar and above the composer. A bubble that is merely
    /// `isHittable` can still be half-buried under the input bar — which
    /// is how the poll's last option and its "4 of 5 voted" footer came
    /// out sliced off the bottom of the frame.
    @MainActor
    private func contentRect(in app: XCUIApplication) -> CGRect {
        let navBar = app.navigationBars.firstMatch
        let top = navBar.exists ? navBar.frame.maxY : app.frame.minY
        let bar = composer(in: app)
        let bottom = bar.exists ? bar.frame.minY : app.frame.maxY
        return CGRect(x: app.frame.minX, y: top,
                      width: app.frame.width, height: max(0, bottom - top))
    }

    /// Scroll back to the oldest message. Already there on a screen tall
    /// enough to hold the whole opening, which is why it checks first.
    @MainActor
    private func scrollToThreadStart(in app: XCUIApplication) -> Bool {
        let first = app.staticTexts
            .matching(NSPredicate(format: "label BEGINSWITH %@", firstMessagePrefix)).firstMatch
        for _ in 0..<8 {
            // Fully below the navigation bar, not merely `isHittable`: at
            // the offset the thread opens at, the oldest bubble is sliding
            // under the bar and half of it is already unreadable.
            if first.exists && contentRect(in: app).contains(first.frame) { return true }
            drag(in: app, from: 0.35, to: 0.72) // toward the older end
            usleep(700_000)
        }
        return false
    }

    /// Walk the thread until the album bubble and the poll are BOTH on
    /// screen, so `03-photos-and-poll` deserves its name.
    ///
    /// Both directions, because either miss is possible: the poll sits
    /// below the album, so a thread that opens above them scrolls toward
    /// the newest until the poll arrives, and one that has run past them
    /// backs up until the album does.
    ///
    /// "On screen", not "entirely unclipped". Album + caption + reactions
    /// + the message between + a three-option poll is taller than a 6.9"
    /// phone's viewport, so on that device class one end is cropped
    /// whatever the offset — which is what a real thread looks like and
    /// what the frame reports. The activity below records which it got.
    @MainActor
    private func scrollToPhotosAndPoll(in app: XCUIApplication) -> Bool {
        let album = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label BEGINSWITH %@", albumLabelPrefix)).firstMatch
        let poll = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label BEGINSWITH %@", pollLastOption)).firstMatch

        // Phase 1 — coarse, until both are on screen at all.
        var bothOnScreen = false
        for _ in 0..<20 {
            let albumShown = album.exists && album.isHittable
            let pollShown = poll.exists && poll.isHittable
            if albumShown && pollShown { bothOnScreen = true; break }
            // Toward the newest while the poll is missing; back toward the
            // older once it is there but the album is not.
            if pollShown {
                drag(in: app, from: 0.35, to: 0.62)
            } else {
                drag(in: app, from: 0.62, to: 0.35)
            }
            usleep(700_000) // let the deceleration finish before asking again
        }
        guard bothOnScreen else { return false }

        // Phase 2 — nudge until neither end is cut off. The steps are tiny
        // (≈2% of the screen) because a flick's momentum scales with the
        // distance dragged: a 6% step overshoots the whole band of good
        // offsets and oscillates around it forever. Slowing the gesture
        // down instead is what long-presses a bubble — see `drag`.
        for _ in 0..<20 {
            let rect = contentRect(in: app)
            let albumFits = album.exists && rect.contains(album.frame)
            let pollFits = poll.exists && rect.contains(poll.frame)
            if albumFits && pollFits {
                XCTContext.runActivity(named: "03: album and poll both fully in frame") { _ in }
                return true
            }
            if !pollFits {
                drag(in: app, from: 0.425, to: 0.40) // a sliver more of the newer end
            } else {
                drag(in: app, from: 0.40, to: 0.425) // …or of the older end
            }
            usleep(500_000)
        }
        // Not a failure: on a small enough screen the two bubbles cannot
        // both fit whole, and a thread cropped at one edge is what a
        // thread looks like. The activity says which happened.
        XCTContext.runActivity(named: "03: album and poll on screen, one end cropped") { _ in }
        return album.exists && album.isHittable && poll.exists && poll.isHittable
    }

    /// Taps the first of several candidate labels that is actually there.
    /// The same control is a toolbar button on one device class and a row
    /// on another, and its label differs for an owner and a member.
    @MainActor
    @discardableResult
    private func tapFirst(in app: XCUIApplication, labels: [String]) -> Bool {
        for label in labels {
            for element in [app.buttons[label], app.staticTexts[label]] {
                if element.waitForExistence(timeout: 3), element.isHittable {
                    element.tap()
                    return true
                }
            }
        }
        XCTFail("none of \(labels) was reachable — the screenshot set is incomplete")
        return false
    }

    // MARK: - Capture

    /// THE WHOLE DISPLAY, not the app's frame: a store slot expects the
    /// status bar, and `app.screenshot()` crops it out.
    ///
    /// `.keepAlways` is load-bearing. The default lifetime is
    /// `.deleteOnSuccess`, so a screenshot test that PASSES throws its own
    /// attachments away and the result bundle comes back empty.
    @MainActor
    private func capture(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
