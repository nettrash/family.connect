//
//  ComposerAuditUITests.swift
//  FamilyConnectUITests
//
//  A UX/UI AUDIT walk of the MESSAGE COMPOSER (#106), not a regression
//  test: hands a session in, opens the seeded fixture's chats
//  (server/scripts/seed-store-screenshots.sh) and photographs the composer
//  in every state it has — empty, focused, a short and a long draft, the
//  attach menu, the sticker panel, the bubble menu and the reply and edit
//  banners it leads to, a hands-free voice recording, the direct and the
//  assistant chats, a thread — in portrait and, where the review asked for
//  it, landscape. Nothing here asserts a layout; the pictures are the
//  deliverable.
//
//    cd ios && DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
//      TEST_RUNNER_FC_UITEST_SERVER=http://127.0.0.1:8091 \
//      TEST_RUNNER_FC_UITEST_TOKEN=<nora's session token> \
//      xcodebuild test-without-building -project FamilyConnect.xcodeproj \
//        -scheme FamilyConnect -destination 'id=<UDID>' \
//        -only-testing:FamilyConnectUITests/ComposerAuditUITests \
//        -resultBundlePath /tmp/composer.xcresult
//    ios/scripts/export-screenshots.sh /tmp/composer.xcresult /tmp/composer
//
//  Every capture is named NN-<state>-<orientation> so export-screenshots.sh
//  keeps it. Soft everywhere: a state that cannot be reached is logged and
//  skipped, never a failure that hides the rest of the set.
//
//  NOTHING IS SENT. Drafts are typed and left behind, menus and panels are
//  opened and the app relaunched out of them, the voice recording is
//  stopped and deleted before the walk moves on. The one thing this must
//  never do is tap a sticker inside the panel: there, one tap sends.
//

// iOS only: the walk rotates the device and the Mac has no orientation.
#if os(iOS)

import XCTest

final class ComposerAuditUITests: XCTestCase {

    private let familyName = ProcessInfo.processInfo.environment["FC_AUDIT_FAMILY"] ?? "The Harpers"
    private let directName = ProcessInfo.processInfo.environment["FC_AUDIT_DIRECT"] ?? "Ellie"
    private let assistantName = ProcessInfo.processInfo.environment["FC_AUDIT_ASSISTANT"] ?? "Assistant"

    /// The session, handed in rather than typed (see CallEndUITests).
    private var server = ""
    private var token = ""

    private var isPad: Bool { UIDevice.current.userInterfaceIdiom == .pad }

    /// A draft that wraps to three lines or more in either composer width:
    /// the phone's field and the iPad's 560pt column alike.
    private let longDraft =
        "Running late — the big shop took longer than it should have, so start the potatoes "
        + "without me. I will bring the bread, the good gravy boat and the little oranges Mae "
        + "wanted, and whoever gets there first please put the kettle on and open a window."

    /// What the sidebar's toggle has been called, newest first.
    private let sidebarToggles = ["Hide Sidebar", "Toggle Sidebar", "ToggleSidebar", "Show Sidebar"]

    override func setUpWithError() throws {
        continueAfterFailure = true
    }

    @MainActor
    func testComposerWalk() throws {
        guard let server = ProcessInfo.processInfo.environment["FC_UITEST_SERVER"],
              let token = ProcessInfo.processInfo.environment["FC_UITEST_TOKEN"]
        else {
            throw XCTSkip("set TEST_RUNNER_FC_UITEST_SERVER and TEST_RUNNER_FC_UITEST_TOKEN")
        }
        self.server = server
        self.token = token
        let app = XCUIApplication()
        XCUIDevice.shared.orientation = .portrait

        // Landscape for every family-chat state on the iPad; on the phone
        // only the empty and typing states turn.
        let turning: [UIDeviceOrientation] = isPad ? [.portrait, .landscapeLeft] : [.portrait]

        // 1 — the family chat: empty, focused, a short and a long draft.
        //     Both orientations on both devices.
        launch(app, reset: true)
        guard openChat(app, familyName) else {
            note("the family chat could not be opened; nothing else is reachable")
            return
        }
        for orientation in [UIDeviceOrientation.portrait, .landscapeLeft] {
            let suffix = orientationName(orientation)
            rotate(orientation); sleep(2)
            capture("10-family-empty-\(suffix)")
            // The iPad's wider column: the sidebar toggled away.
            if isPad, orientation == .landscapeLeft {
                if tapFirst(app, sidebarToggles) {
                    sleep(1); capture("11-family-empty-landscape-nosidebar")
                    if focusComposer(app) {
                        app.typeText(longDraft); sleep(1)
                        capture("15-family-draft-long-landscape-nosidebar")
                    }
                    fresh(app, familyName, orientation)
                } else {
                    dumpTree(app, "sidebar-toggle")
                }
            }
            if focusComposer(app) {
                sleep(1); capture("12-family-focused-empty-\(suffix)")
                app.typeText("Hi all"); sleep(1)
                capture("13-family-draft-short-\(suffix)")
                clearFocusedDraft(app)
                app.typeText(longDraft); sleep(1)
                capture("14-family-draft-long-\(suffix)")
            }
            // Unfocused and empty again for the next orientation.
            fresh(app, familyName, .portrait)
        }

        // 2 — the family chat's transient states, each from a fresh thread
        //     so one cannot bleed into the next.
        for orientation in turning {
            let suffix = orientationName(orientation)

            // The attach menu.
            fresh(app, familyName, orientation)
            if tapIfHittable(attachButton(app), "the paperclip") {
                sleep(1); capture("16-family-attach-menu-\(suffix)")
            }

            // The sticker panel — looked at, never touched.
            fresh(app, familyName, orientation)
            if tapIfHittable(app.buttons["Stickers"].firstMatch, "the sticker button") {
                sleep(2); capture("17-family-sticker-panel-\(suffix)")
            }

            // The bubble menu, then Reply, then a draft under the banner.
            fresh(app, familyName, orientation)
            if longPress(app, bubbleBeginning: "We're by the ducks") {
                sleep(1); capture("18-family-bubble-menu-\(suffix)")
                if tapFirst(app, ["Reply"]) {
                    sleep(1); capture("19-family-reply-banner-\(suffix)")
                    if focusedField(app).exists || focusComposer(app) {
                        app.typeText("On our way"); sleep(1)
                        capture("20-family-reply-draft-\(suffix)")
                    }
                }
            }

            // Edit: one of nora's own bubbles, which sits above the album.
            fresh(app, familyName, orientation)
            if scrollUp(app, until: bubble(app, beginning: "Got it. Rob")),
               longPress(app, bubbleBeginning: "Got it. Rob") {
                if tapFirst(app, ["Edit"]) {
                    sleep(1); capture("21-family-edit-banner-\(suffix)")
                }
            }

            // A hands-free voice recording: started, photographed, stopped,
            // photographed again, deleted — nothing not-sent is left behind.
            fresh(app, familyName, orientation)
            recordAndDiscard(app, suffix: suffix)
        }

        // 3 — the direct chat: empty, a draft, and an edit of nora's own line.
        for orientation in turning {
            let suffix = orientationName(orientation)
            fresh(app, directName, orientation)
            sleep(1); capture("30-direct-empty-\(suffix)")
            if focusComposer(app) {
                app.typeText("Dinner is at seven"); sleep(1)
                capture("31-direct-draft-\(suffix)")
            }
        }
        fresh(app, directName, .portrait)
        if longPress(app, bubbleBeginning: "Yes — home by lunch"), tapFirst(app, ["Edit"]) {
            sleep(1); capture("32-direct-edit-banner-portrait")
        }

        // 4 — the assistant's chat: no microphone, a paintbrush instead.
        for orientation in turning {
            let suffix = orientationName(orientation)
            fresh(app, assistantName, orientation)
            sleep(1); capture("40-assistant-empty-\(suffix)")
            if focusComposer(app) {
                app.typeText("What should we cook on Sunday?"); sleep(1)
                capture("41-assistant-draft-\(suffix)")
            }
        }
        fresh(app, assistantName, .portrait)
        if tapIfHittable(app.buttons["Ask for a picture"].firstMatch, "the paintbrush") {
            sleep(1); capture("42-assistant-picture-token-portrait")
        }

        // 5 — a thread's composer, under the root with eight replies.
        for orientation in turning {
            let suffix = orientationName(orientation)
            fresh(app, familyName, orientation)
            let replies = app.descendants(matching: .any)
                .matching(NSPredicate(format: "label == '8 replies'")).firstMatch
            guard scrollUp(app, until: replies), tapIfHittable(replies, "the replies affordance") else {
                dumpTree(app, "replies-\(suffix)")
                continue
            }
            let field = threadField(app)
            guard field.waitForExistence(timeout: 10) else {
                note("the thread opened without its composer")
                sleep(1); capture("50-thread-no-composer-\(suffix)")
                continue
            }
            sleep(2); capture("50-thread-empty-\(suffix)")
            if tapIfHittable(field, "the thread's field") {
                sleep(1)
                if focusedField(app).exists {
                    app.typeText("Sounds lovely, see you all at one"); sleep(1)
                    capture("51-thread-draft-\(suffix)")
                }
            }
        }

        // 6 — the thread scrolled back to the album and the poll, so the
        //     review can see bubble widths in the iPad's column (and, for
        //     comparison, on the phone).
        let wide: UIDeviceOrientation = isPad ? .landscapeLeft : .portrait
        fresh(app, familyName, wide)
        let suffix = orientationName(wide)
        if scrollUp(app, until: bubble(app, beginning: "Sunday lunch"), maxDrags: 6) {
            sleep(1); capture("60-family-poll-\(suffix)")
        }
        // The album by its caption: its own "Album" label answers hittable
        // before the stack is on screen, and the caption does not.
        if scrollUp(app, until: bubble(app, beginning: "Sunday at the lake"), maxDrags: 6) {
            sleep(1); capture("61-family-album-\(suffix)")
            // The album arrives bottom first; one more nudge shows it whole.
            drag(app, from: 0.4, to: 0.65)
            sleep(1); capture("61-family-album-whole-\(suffix)")
        }
        for _ in 0..<4 { drag(app, from: 0.3, to: 0.8) }
        sleep(1); capture("62-family-top-\(suffix)")
    }

    // MARK: - The voice recording

    /// Tap the microphone, photograph the recording row, stop it,
    /// photograph the review, then delete — through the row's own trash
    /// and the "Delete this recording?" it may ask — and say so if a
    /// "Voice message not sent" row is still there afterwards.
    @MainActor
    private func recordAndDiscard(_ app: XCUIApplication, suffix: String) {
        // The slot's label is the row's ("Record voice message"); the
        // longer form is what its hint and Voice Control know it by.
        let microphone = app.buttons.matching(NSPredicate(
            format: "label == 'Record voice message' OR label == 'Record a voice message'")).firstMatch
        guard tapIfHittable(microphone, "the microphone slot") else {
            dumpTree(app, "microphone-\(suffix)")
            return
        }
        // The microphone prompt belongs to SpringBoard. The runner grants
        // the privilege beforehand where it can; this is the fallback.
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        for host in [springboard, app] where host.alerts.firstMatch.waitForExistence(timeout: 2) {
            _ = tapFirst(host, ["Allow", "OK"])
            break
        }
        let stop = app.buttons["Stop recording"].firstMatch
        let recording = stop.waitForExistence(timeout: 6)
        sleep(3)
        capture(recording ? "22-family-voice-recording-\(suffix)" : "22-family-voice-attempt-\(suffix)")
        guard recording else {
            note("no recording started from the microphone slot")
            return
        }
        stop.tap()
        sleep(2)
        capture("23-family-voice-stopped-\(suffix)")
        // Delete from wherever the review put it: the recording row's
        // trash or the not-sent row's — both are "Delete recording".
        if tapFirst(app, ["Delete recording"]) {
            if app.buttons["Delete"].firstMatch.waitForExistence(timeout: 3) {
                app.buttons["Delete"].firstMatch.tap()
            }
            sleep(2)
            capture("24-family-voice-deleted-\(suffix)")
        } else {
            note("no Delete recording control after Stop; the recording may be left not-sent")
        }
        let leftover = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label BEGINSWITH 'Voice message not sent'")).firstMatch
        if leftover.exists {
            note("a 'Voice message not sent' row remains after the delete")
        }
    }

    // MARK: - Launching and opening

    @MainActor
    private func launch(_ app: XCUIApplication, reset: Bool) {
        app.terminate()
        app.launchArguments = (reset ? ["--uitest-reset"] : []) + [
            "-v1.serverURL", server,
            "-v1.sessionToken", token,
            "-AppleLanguages", "(en)",
            "-AppleLocale", "en_US",
        ]
        app.launch()
        // The notification prompt blocks every later tap, and it belongs
        // to SpringBoard rather than to the app; "Not Now" is the app's.
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let deadline = Date().addingTimeInterval(reset ? 8 : 3)
        while Date() < deadline {
            var tapped = false
            for host in [app, springboard] {
                for label in ["Allow", "Not Now"] {
                    let button = host.buttons[label].firstMatch
                    if button.exists, button.isHittable {
                        button.tap()
                        tapped = true
                    }
                }
            }
            if tapped { sleep(1) } else { usleep(500_000) }
        }
    }

    /// A clean thread in the wanted orientation: relaunched, opened, turned.
    @MainActor
    private func fresh(_ app: XCUIApplication, _ chat: String, _ orientation: UIDeviceOrientation) {
        XCUIDevice.shared.orientation = .portrait
        launch(app, reset: false)
        _ = openChat(app, chat)
        rotate(orientation)
        sleep(1)
    }

    /// Open a chat from the list and wait for its composer. On a portrait
    /// iPad the list may be the hidden column, so its toggle is tried
    /// before giving up.
    @MainActor
    private func openChat(_ app: XCUIApplication, _ chat: String) -> Bool {
        let row = app.collectionViews.staticTexts[chat].firstMatch
        if !row.waitForExistence(timeout: 20) || !row.isHittable {
            _ = tapFirst(app, ["Show Sidebar", "Toggle Sidebar", "ToggleSidebar", "Chats"])
        }
        guard row.waitForExistence(timeout: 5), row.isHittable else {
            note("no row for \(chat) in the chat list")
            return false
        }
        row.tap()
        // The field is the composer; the paperclip is not in a thread
        // and the assistant's chat has no microphone, so wait on the field.
        guard composerField(app).waitForExistence(timeout: 20) else {
            note("\(chat) opened without a composer")
            return false
        }
        sleep(2)
        return true
    }

    // MARK: - The composer

    /// The message field. A multi-line SwiftUI field is a text view to
    /// UIKit but may read as a text field to the accessibility snapshot;
    /// either will do (see EmojiKeyboardUITests).
    @MainActor
    private func composerField(_ app: XCUIApplication) -> XCUIElement {
        field(app, placeholder: "Message")
    }

    @MainActor
    private func threadField(_ app: XCUIApplication) -> XCUIElement {
        field(app, placeholder: "Reply in thread")
    }

    @MainActor
    private func field(_ app: XCUIApplication, placeholder: String) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(
            format: "placeholderValue == %@ AND (elementType == %d OR elementType == %d)",
            placeholder,
            XCUIElement.ElementType.textView.rawValue,
            XCUIElement.ElementType.textField.rawValue)).firstMatch
    }

    /// The field once it holds text: its placeholder gives way to its
    /// value, so from then on it is found by focus.
    @MainActor
    private func focusedField(_ app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any)
            .matching(NSPredicate(format: "hasKeyboardFocus == true")).firstMatch
    }

    @MainActor
    private func attachButton(_ app: XCUIApplication) -> XCUIElement {
        app.buttons["Attach a photo, video or file"].firstMatch
    }

    /// Tap the field and wait for the focus the keyboard needs.
    @MainActor
    private func focusComposer(_ app: XCUIApplication) -> Bool {
        let field = composerField(app)
        guard tapIfHittable(field, "the message field") else { return false }
        for _ in 0..<6 where !focusedField(app).exists { usleep(500_000) }
        guard focusedField(app).exists else {
            note("the message field took no keyboard focus")
            return false
        }
        return true
    }

    /// Backspace over whatever the focused field holds.
    @MainActor
    private func clearFocusedDraft(_ app: XCUIApplication) {
        let count = (focusedField(app).value as? String)?.count ?? 0
        guard count > 0 else { return }
        app.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: min(count + 2, 400)))
        usleep(500_000)
    }

    // MARK: - Bubbles

    @MainActor
    private func bubble(_ app: XCUIApplication, beginning prefix: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix)).firstMatch
    }

    /// Long-press a bubble for its menu; false when it is not on screen.
    @MainActor
    private func longPress(_ app: XCUIApplication, bubbleBeginning prefix: String) -> Bool {
        let target = bubble(app, beginning: prefix)
        guard target.waitForExistence(timeout: 5), target.isHittable else {
            note("no bubble beginning \"\(prefix)\" to long-press")
            return false
        }
        target.press(forDuration: 1.0)
        sleep(1)
        return true
    }

    /// Drag the thread down towards older messages until the element is
    /// on screen, or give up after the drags allowed.
    @MainActor
    private func scrollUp(_ app: XCUIApplication, until element: XCUIElement, maxDrags: Int = 8) -> Bool {
        for _ in 0..<maxDrags {
            if element.exists, element.isHittable { return true }
            drag(app, from: 0.35, to: 0.7)
            sleep(1)
        }
        if element.exists, element.isHittable { return true }
        note("not reached by scrolling: \(element)")
        return false
    }

    // MARK: - Helpers

    @MainActor
    private func rotate(_ orientation: UIDeviceOrientation) {
        XCUIDevice.shared.orientation = orientation
        sleep(1)
    }

    private func orientationName(_ orientation: UIDeviceOrientation) -> String {
        orientation == .portrait ? "portrait" : "landscape"
    }

    @MainActor
    private func drag(_ app: XCUIApplication, from: CGFloat, to: CGFloat) {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: from))
        let end = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: to))
        start.press(forDuration: 0.05, thenDragTo: end)
    }

    @MainActor
    private func tapFirst(_ app: XCUIApplication, _ labels: [String]) -> Bool {
        for label in labels {
            for element in [app.buttons[label].firstMatch, app.staticTexts[label].firstMatch] {
                if element.waitForExistence(timeout: 2), element.isHittable {
                    element.tap()
                    return true
                }
            }
        }
        note("none of \(labels) was reachable")
        return false
    }

    /// A tap that is skipped, and said so, rather than failed.
    @MainActor
    private func tapIfHittable(_ element: XCUIElement, _ what: String) -> Bool {
        guard element.waitForExistence(timeout: 5), element.isHittable else {
            note("\(what) was not reachable")
            return false
        }
        element.tap()
        return true
    }

    private func note(_ message: String) {
        XCTContext.runActivity(named: message) { _ in }
    }

    /// The element tree, for the states this walk could not find its way
    /// to: a label that has changed is read off it, not guessed at. Named
    /// without the NN- prefix so export-screenshots.sh leaves it alone.
    @MainActor
    private func dumpTree(_ app: XCUIApplication, _ name: String) {
        let dump = XCTAttachment(string: app.debugDescription)
        dump.name = "tree-\(name)"
        dump.lifetime = .keepAlways
        add(dump)
    }

    @MainActor
    private func capture(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}

#endif
