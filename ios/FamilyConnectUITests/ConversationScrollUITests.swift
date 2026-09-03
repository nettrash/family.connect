//
//  ConversationScrollUITests.swift
//  FamilyConnectUITests
//
//  Manual regression: opening a long chat must land at the NEWEST
//  message, not scrolled into history or blank estimated space.
//
//  Runs on both shapes of the chat screen (issue #43). On a phone the
//  thread is PUSHED onto a NavigationStack; on an iPad it lives in a
//  NavigationSplitView's detail column beside a permanent sidebar, where
//  there is no back button to press and nothing to pop. `leaveThread`
//  below is where that difference is absorbed — see the note on it; the
//  phone's path through this test is unchanged, line for line.
//
//  Needs a running server seeded with the fixture this asserts against
//  (user "junior"/"password123" in a family whose chat holds messages
//  "Message number 1"…"Message number 1500", some with reactions), so it
//  skips unless the runner passes the server URL:
//
//    TEST_RUNNER_FC_UITEST_SERVER=http://127.0.0.1:8091 xcodebuild test \
//      -only-testing:FamilyConnectUITests/ConversationScrollUITests …
//

import XCTest

final class ConversationScrollUITests: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    @MainActor
    func testFamilyChatOpensAtTheNewestMessage() throws {
        guard let server = ProcessInfo.processInfo.environment["FC_UITEST_SERVER"] else {
            throw XCTSkip("set TEST_RUNNER_FC_UITEST_SERVER to a seeded server URL")
        }

        let app = XCUIApplication()
        // Fresh install state, but with the server preseeded through the
        // UserDefaults argument domain so setup is skipped.
        app.launchArguments = ["--uitest-reset", "-v1.serverURL", server]
        app.launch()

        // Log in as the seeded member (mode defaults to Log In).
        let username = app.textFields["Username"]
        XCTAssertTrue(username.waitForExistence(timeout: 15), "should land on the auth screen")
        username.tap()
        username.typeText("junior")
        let password = app.secureTextFields["Password"]
        password.tap()
        password.typeText("password123")
        // Two elements carry the "Log In" label — the mode segment and the
        // submit button. Match order is not visual order, so pick the one
        // lowest on screen (the submit lives further down the form).
        let submits = app.buttons.matching(NSPredicate(format: "label == 'Log In'"))
            .allElementsBoundByIndex
        submits.max(by: { $0.frame.minY < $1.frame.minY })?.tap()

        // iOS offers to save the freshly used password; the sheet blocks
        // every later tap. It may belong to the app or to SpringBoard.
        for host in [app, XCUIApplication(bundleIdentifier: "com.apple.springboard")] {
            let notNow = host.buttons["Not Now"]
            if notNow.waitForExistence(timeout: 4) {
                notNow.tap()
                break
            }
        }

        // Phase 1 — first open after a fresh login (full resync just ran).
        openFamilyChat(in: app)
        let newest = ProcessInfo.processInfo.environment["FC_UITEST_NEWEST"] ?? "Message number 1500"
        assertNewestVisible(newest, in: app, phase: "fresh-login-open")
        leaveThread(in: app, server: server)

        // Phase 2 — family activity while this device sits on the chat
        // list: the other member posts a new message AND reacts to an old
        // one (a row far above the viewport resizes on next open).
        let olive = try apiLogin(server: server, username: "olive")
        try api(server: server, token: olive, method: "POST", path: "/chats/1/messages",
                body: ["client_msg_id": UUID().uuidString, "body": "Fresh message A"])
        try api(server: server, token: olive, method: "PUT", path: "/chats/1/messages/3/reaction",
                body: ["emoji": "😂"])
        sleep(2) // let the frames land
        openFamilyChat(in: app)
        assertNewestVisible("Fresh message A", in: app, phase: "reopen-after-activity")
        leaveThread(in: app, server: server)

        // Phase 3 — plain reopen with no new activity.
        openFamilyChat(in: app)
        assertNewestVisible("Fresh message A", in: app, phase: "plain-reopen")

        // Phase 3b — read some history (several pages up), leave, reopen:
        // the fresh thread must re-anchor at the bottom, not restore or
        // mis-estimate the old offset. Skippable for the tiny fixture: a
        // FULLY materialized thread of screen-tall bubbles livelocks the
        // test harness's accessibility snapshots (app is fine — verified
        // by sampling: the churn is AX bundle/prefs machinery, and the
        // long-fixture run covers this phase).
        if ProcessInfo.processInfo.environment["FC_UITEST_SKIP_HISTORY"] == nil {
        for _ in 0..<6 {
            app.swipeDown()
            usleep(400_000) // let each fling settle: chained decelerations
                            // through screen-tall bubbles keep the app
                            // non-quiescent and starve the test harness
        }
        sleep(2)
        add(screenshot(named: "scrolled-into-history", of: app))
        leaveThread(in: app, server: server)
        openFamilyChat(in: app)
        assertNewestVisible("Fresh message A", in: app, phase: "reopen-after-history-read")
        leaveThread(in: app, server: server)
        } else {
            leaveThread(in: app, server: server)
        }

        // Phase 4 — cold relaunch with the full local store (no reset:
        // session and cache survive), straight into the chat.
        app.terminate()
        app.launchArguments = ["-v1.serverURL", server]
        app.launch()
        openFamilyChat(in: app)
        assertNewestVisible("Fresh message A", in: app, phase: "cold-relaunch-open")

        // Phase 5 — the keyboard: focusing the input bar raises the
        // bottom inset by the keyboard height, and growing a multi-line
        // draft raises it further — the newest messages must stay pinned
        // above it the whole way (the reported bug: they scroll away).
        let field = app.textFields["Message"]
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.tap()
        sleep(1) // keyboard animation
        assertNewestVisible("Fresh message A", in: app, phase: "keyboard-up")
        field.typeText("A reply that is long enough to wrap over several lines of the input bar so the bar itself grows while the keyboard stays up")
        sleep(1)
        assertNewestVisible("Fresh message A", in: app, phase: "keyboard-multiline-draft")
    }

    // MARK: - Steps

    /// The chat-list ROW, not the nav-bar title: the list's navigation
    /// title is the family name too, so an unscoped "The Smiths" query is
    /// ambiguous — the List's collection view holds only the row.
    private func familyRow(in app: XCUIApplication) -> XCUIElement {
        app.collectionViews.staticTexts["The Smiths"].firstMatch
    }

    @MainActor
    private func openFamilyChat(in app: XCUIApplication) {
        dismissSavePasswordIfPresent(app) // can appear late and eat taps
        let row = familyRow(in: app)
        XCTAssertTrue(row.waitForExistence(timeout: 15), "family chat should be listed")
        if !row.isHittable { dismissSavePasswordIfPresent(app) }
        row.tap()
        sleep(2) // let the thread settle (layout, resync, any misguided pagination)
    }

    @MainActor
    private func dismissSavePasswordIfPresent(_ app: XCUIApplication) {
        for host in [app, XCUIApplication(bundleIdentifier: "com.apple.springboard")] {
            let notNow = host.buttons["Not Now"]
            if notNow.exists {
                notNow.tap()
                return
            }
        }
    }

    /// Leave the thread, so that the NEXT open is a fresh one.
    ///
    /// On a phone the thread is a pushed screen: the nav bar's back button
    /// pops it and the chat list is what remains. That is what this test
    /// has always done and it still does exactly that.
    ///
    /// On an iPad the thread is a NavigationSplitView's detail column
    /// (issue #43), and there is no back button, because there is nothing
    /// to pop: the list is permanently beside the thread. Deleting the
    /// assertion would quietly delete the phases too — every one of them
    /// depends on the NEXT open being a genuinely new ConversationView,
    /// and re-tapping an already-selected sidebar row builds nothing
    /// (`.id(chatID)` is unchanged, so SwiftUI reuses the view). So the
    /// assertion is REPLACED by the two facts that mean the same thing
    /// on this shape, and neither is weaker:
    ///
    /// 1. The split view's own invariant — the chat list is on screen
    ///    WITH the thread, which on a stack is never true.
    /// 2. The thread is genuinely closed and genuinely reopened, by
    ///    relaunching with the session and cache intact. A launch lands on
    ///    the chat list with NOTHING selected (ChatListView deliberately
    ///    does not auto-select: ChatPresence would then mark the family
    ///    chat read on every launch), which is precisely where the phone's
    ///    back button leaves the app — so the phases that follow measure
    ///    what they always measured, including the one that wants this
    ///    device sitting on the chat list while another member posts.
    @MainActor
    private func leaveThread(in app: XCUIApplication, server: String) {
        if isSplitView(in: app) {
            XCTAssertTrue(
                familyRow(in: app).exists,
                "[split] the chat list must stay on screen beside the thread")
            app.terminate()
            app.launchArguments = ["-v1.serverURL", server]
            app.launch()
            XCTAssertTrue(
                app.staticTexts["No conversation selected"].waitForExistence(timeout: 20),
                "[split] a launch must land on the chat list with no chat selected")
        } else {
            app.navigationBars.buttons.firstMatch.tap()
        }
        XCTAssertTrue(familyRow(in: app).waitForExistence(timeout: 10), "should be back on the chat list")
    }

    /// The chat list and the thread on screen at the same time — the one
    /// thing that is true of the split view and of nothing else.
    @MainActor
    private func isSplitView(in app: XCUIApplication) -> Bool {
        app.textFields["Message"].exists && familyRow(in: app).exists
    }

    @MainActor
    private func assertNewestVisible(_ bodyPrefix: String, in app: XCUIApplication, phase: String) {
        // Prefix match: seeded bodies carry varied-length suffixes.
        let newest = app.staticTexts
            .matching(NSPredicate(format: "label BEGINSWITH %@", bodyPrefix)).firstMatch
        _ = newest.waitForExistence(timeout: 6)
        add(screenshot(named: phase, of: app))
        logVisibleMessages(in: app)
        XCTAssertTrue(newest.exists, "[\(phase)] the newest message should be materialized on open")
        XCTAssertTrue(newest.isHittable, "[\(phase)] the newest message should be VISIBLE on open — not scrolled away")
        // Fully above the input bar: a bubble half-buried under the bar
        // still counts as hittable, but reads as "my messages scrolled
        // away" — the exact reported bug when the keyboard or a growing
        // multi-line draft raises the bar.
        let barTop = app.textFields.firstMatch.frame.minY
        XCTAssertLessThanOrEqual(
            newest.frame.maxY, barTop + 1,
            "[\(phase)] the newest message must sit fully above the input bar")
    }

    // MARK: - Server-side actors (the runner plays the other family member)

    private func apiLogin(server: String, username: String) throws -> String {
        let response = try api(server: server, token: nil, method: "POST", path: "/auth/login",
                               body: ["username": username, "password": "password123"])
        guard let token = response["token"] as? String else {
            throw NSError(domain: "uitest", code: 1, userInfo: [NSLocalizedDescriptionKey: "no token in login response"])
        }
        return token
    }

    @discardableResult
    private func api(server: String, token: String?, method: String, path: String,
                     body: [String: Any]) throws -> [String: Any] {
        var request = URLRequest(url: URL(string: server + "/api/v1" + path)!)
        request.httpMethod = method
        request.addValue("application/json", forHTTPHeaderField: "Content-Type")
        if let token { request.addValue("Bearer \(token)", forHTTPHeaderField: "Authorization") }
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        let semaphore = DispatchSemaphore(value: 0)
        var result: [String: Any] = [:]
        var failure: Error?
        URLSession.shared.dataTask(with: request) { data, response, error in
            defer { semaphore.signal() }
            if let error { failure = error; return }
            guard let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode) else {
                failure = NSError(domain: "uitest", code: (response as? HTTPURLResponse)?.statusCode ?? -1)
                return
            }
            if let data, let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                result = json
            }
        }.resume()
        semaphore.wait()
        if let failure { throw failure }
        return result
    }

    // MARK: - Diagnostics

    private func screenshot(named name: String, of app: XCUIApplication) -> XCTAttachment {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        return attachment
    }

    /// Prints which seeded messages are materialized and which are truly
    /// on screen — the failure log then shows WHERE the view landed.
    private func logVisibleMessages(in app: XCUIApplication) {
        let window = app.windows.firstMatch.frame
        var materialized: [Int] = []
        var visible: [Int] = []
        for n in stride(from: 1, through: 1500, by: 75) {
            let element = app.staticTexts["Message number \(n)"]
            if element.exists {
                materialized.append(n)
                if window.intersects(element.frame) && element.isHittable {
                    visible.append(n)
                }
            }
        }
        NSLog("FC-SCROLL-DIAG materialized=\(materialized) visible=\(visible)")
    }
}
