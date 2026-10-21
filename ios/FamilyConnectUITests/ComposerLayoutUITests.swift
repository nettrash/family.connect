//
//  ComposerLayoutUITests.swift
//  FamilyConnectUITests
//
//  The message row and the wide layout of #106
//  (docs/composer-and-wide-layout-2026-10-20.md), measured on a live app:
//  the stickers inside the field and staying while typing, no `@ai` button
//  in the row, the assistant first in the `@` list and in the paperclip as
//  "Ask the Assistant", a hardware Return that sends and a Shift+Return that
//  breaks the line, the field on a line of its own at an accessibility text
//  size — and on an iPad, the 1024-pt column with 560-pt bubbles and a
//  composer as wide as the column.
//
//  The pure rules behind all of it are pinned by ComposerRowTests and
//  AttachMenuTests without a server; this is the check that the views draw
//  them. Needs the store fixture (server/scripts/seed-store-screenshots.sh's
//  family, "The Harpers", on a server whose `[ai]` section is filled in so
//  the assistant is advertised) and a session handed in, as
//  ComposerAuditUITests does:
//
//    cd ios && DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
//      TEST_RUNNER_FC_UITEST_SERVER=http://127.0.0.1:8091 \
//      TEST_RUNNER_FC_UITEST_TOKEN=<nora's session token> \
//      xcodebuild test-without-building -project FamilyConnect.xcodeproj \
//        -scheme FamilyConnect -destination 'id=<UDID>' \
//        -only-testing:FamilyConnectUITests/ComposerLayoutUITests
//
//  It SENDS one message (the hardware-Return check) into the fixture's
//  family chat; nothing else is sent.
//

// iOS only: the iPad's orientation and the phone's keyboards.
#if os(iOS)

import XCTest

final class ComposerLayoutUITests: XCTestCase {

    private let familyName = ProcessInfo.processInfo.environment["FC_AUDIT_FAMILY"] ?? "The Harpers"
    private var server = ""
    private var token = ""

    private var isPad: Bool { UIDevice.current.userInterfaceIdiom == .pad }

    override func setUpWithError() throws {
        continueAfterFailure = true
        guard let server = ProcessInfo.processInfo.environment["FC_UITEST_SERVER"],
              let token = ProcessInfo.processInfo.environment["FC_UITEST_TOKEN"]
        else {
            throw XCTSkip("set TEST_RUNNER_FC_UITEST_SERVER and TEST_RUNNER_FC_UITEST_TOKEN")
        }
        self.server = server
        self.token = token
    }

    // MARK: - The row

    @MainActor
    func testTheRow() throws {
        let app = XCUIApplication()
        XCUIDevice.shared.orientation = .portrait
        launch(app, extra: [])
        try XCTSkipUnless(openChat(app, familyName), "the family chat could not be opened")
        capture("row-empty")

        let field = composerField(app)
        let paperclip = app.buttons["Attach a photo, video or file"].firstMatch
        let stickers = app.buttons["Stickers"].firstMatch
        XCTAssertTrue(paperclip.exists, "no paperclip")
        XCTAssertTrue(stickers.exists, "no sticker glyph in the family chat")
        XCTAssertFalse(app.buttons["Ask the assistant"].exists, "the @ai button is still in the row")
        // Inside the capsule, at its trailing edge: the text run (the
        // field's accessibility frame is the TEXT, not the capsule) ends
        // where the glyph begins, and the glyph ends before the slot.
        let slot = app.buttons.matching(NSPredicate(
            format: "label IN {'Record voice message', 'Send'}")).allElementsBoundByIndex
            .max { $0.frame.maxY < $1.frame.maxY }
        XCTAssertEqual(stickers.frame.minX, field.frame.maxX, accuracy: 2,
                       "stickers \(stickers.frame) do not follow the text run \(field.frame)")
        if let slot {
            XCTAssertLessThan(stickers.frame.maxX, slot.frame.minX + 4, "the sticker glyph runs into the slot")
        }
        XCTAssertLessThan(paperclip.frame.maxX, field.frame.minX, "the paperclip is not left of the field")
        let fieldWidthEmpty = field.frame.width

        // Typing: the glyph STAYS, and the row does not move.
        field.tap()
        app.typeText("Hi all")
        sleep(1)
        capture("row-typing")
        XCTAssertTrue(stickers.exists && stickers.isHittable, "the sticker glyph left while typing")
        XCTAssertEqual(focusedField(app).frame.width, fieldWidthEmpty, accuracy: 1,
                       "the field changed width at the first character")

        // `@`: the assistant FIRST, under its display name; picking it writes `@ai `.
        app.typeText(" @")
        let assistant = app.buttons["Mention Assistant"].firstMatch
        XCTAssertTrue(assistant.waitForExistence(timeout: 5), "the assistant is not in the @ list")
        let firstMember = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Mention '"))
            .allElementsBoundByIndex.filter { $0.label != "Mention Assistant" }
            .min { $0.frame.minX < $1.frame.minX }
        if let firstMember {
            XCTAssertLessThan(assistant.frame.minX, firstMember.frame.minX, "the assistant is not first")
        }
        capture("row-at-list")
        assistant.tap()
        sleep(1)
        XCTAssertEqual(focusedField(app).value as? String, "Hi all @ai ", "picking the assistant wrote the wrong token")
        clearFocusedDraft(app)

        // The paperclip's "Ask the Assistant" — the last group's first line.
        if paperclip.isHittable {
            paperclip.tap()
            let ask = app.buttons["Ask the Assistant"].firstMatch
            XCTAssertTrue(ask.waitForExistence(timeout: 5), "no Ask the Assistant in the menu")
            let location = app.buttons["Location"].firstMatch
            if location.exists {
                XCTAssertLessThan(ask.frame.minY, location.frame.minY, "Ask the Assistant is not above Location")
            }
            capture("row-attach-menu")
            ask.tap()
            sleep(1)
            let value = (focusedField(app).value as? String) ?? (composerField(app).value as? String) ?? ""
            XCTAssertTrue(value.hasPrefix("@ai"), "Ask the Assistant typed \(value.debugDescription)")
            clearFocusedDraft(app)
        }
    }

    // MARK: - The keyboard

    @MainActor
    func testHardwareReturn() throws {
        let app = XCUIApplication()
        XCUIDevice.shared.orientation = .portrait
        launch(app, extra: [])
        try XCTSkipUnless(openChat(app, familyName), "the family chat could not be opened")
        composerField(app).tap()
        sleep(1)

        // Does a key event reach the app at all on this simulator?
        app.typeKey("x", modifierFlags: [])
        sleep(1)
        let probe = (focusedField(app).value as? String) ?? ""
        add(note("typeKey probe: \(probe.debugDescription)"))
        clearFocusedDraft(app)
        try XCTSkipUnless(probe.lowercased() == "x", "typeKey reaches nothing here (\(probe.debugDescription))")

        // A hardware Return is `typeKey("\n")` — `XCUIKeyboardKey.return`
        // ("\r") reaches nothing on an iOS simulator, measured.
        // Shift+Return breaks the line and sends nothing.
        app.typeText("line one")
        app.typeKey("\n", modifierFlags: .shift)
        app.typeText("line two")
        sleep(1)
        let draft = (focusedField(app).value as? String) ?? ""
        XCTAssertTrue(draft.contains("\n"), "Shift+Return made no new line: \(draft.debugDescription)")
        clearFocusedDraft(app)

        // Return alone sends.
        let stamp = "Return sends \(Int(Date().timeIntervalSince1970) % 100_000)"
        app.typeText(stamp)
        app.typeKey("\n", modifierFlags: [])
        let sent = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", stamp)).firstMatch
        XCTAssertTrue(sent.waitForExistence(timeout: 10), "a hardware Return did not send")
        sleep(1)
        let after = (composerField(app).value as? String) ?? ""
        XCTAssertFalse(after.contains(stamp), "the draft is still in the field after Return")

        // Return in the empty field: nothing, and no blank line.
        composerField(app).tap()
        app.typeKey("\n", modifierFlags: [])
        sleep(1)
        let empty = (focusedField(app).value as? String) ?? ""
        XCTAssertFalse(empty.contains("\n"), "Return in an empty field left a blank line")
        capture("keyboard-after")
    }

    // MARK: - Large text

    @MainActor
    func testAccessibilityTextPutsTheFieldOnItsOwnLine() throws {
        // An iPad's column has the room for the controls and a long text
        // run even at AX3 — the row stays one line there, by the same rule.
        try XCTSkipIf(isPad, "the row only runs out of room on a phone")
        let app = XCUIApplication()
        XCUIDevice.shared.orientation = .portrait
        launch(app, extra: ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXL"])
        try XCTSkipUnless(openChat(app, familyName), "the family chat could not be opened")
        capture("large-text")
        let field = composerField(app)
        let paperclip = app.buttons["Attach a photo, video or file"].firstMatch
        XCTAssertTrue(field.exists && paperclip.exists)
        XCTAssertLessThanOrEqual(field.frame.maxY, paperclip.frame.minY + 1,
                                 "at AX3 the field \(field.frame) is not above the controls \(paperclip.frame)")
        // Its text run spans the row less the capsule's padding and its
        // glyphs — far more than the controls' line would leave it.
        XCTAssertGreaterThan(field.frame.width, app.windows.firstMatch.frame.width * 0.6,
                             "the field on its own line is not the row's width")
    }

    // MARK: - The iPad's column

    @MainActor
    func testTheColumnOnAWideScreen() throws {
        try XCTSkipUnless(isPad, "the column only binds on an iPad")
        let app = XCUIApplication()
        XCUIDevice.shared.orientation = .landscapeLeft
        launch(app, extra: [])
        try XCTSkipUnless(openChat(app, familyName), "the family chat could not be opened")
        sleep(2)
        capture("column-landscape")

        let paperclip = app.buttons["Attach a photo, video or file"].firstMatch
        let slot = app.buttons.matching(NSPredicate(
            format: "label IN {'Record voice message', 'Send'}")).allElementsBoundByIndex
            .max { $0.frame.maxY < $1.frame.maxY }
        XCTAssertTrue(paperclip.exists)
        guard let slot else { return XCTFail("no slot") }
        // The slot's control overhangs its 36-pt glyph by 4 on each side.
        let column = (slot.frame.maxX - 4) - paperclip.frame.minX
        let window = app.windows.firstMatch.frame
        print("column measured: \(column) — paperclip \(paperclip.frame), slot \(slot.frame), window \(window)")
        add(note("column \(column)  paperclip \(paperclip.frame)  slot \(slot.frame)  window \(window)"))
        XCTAssertEqual(column, 1024, accuracy: 2, "the composer is not the 1024-pt column")

        // No bubble's text is wider than the 560-pt bubble.
        let texts = app.scrollViews.firstMatch.staticTexts.allElementsBoundByIndex
        let widest = texts.map(\.frame.width).max() ?? 0
        add(note("widest bubble text \(widest)"))
        XCTAssertLessThanOrEqual(widest, 560, "a bubble's text runs \(widest) pt")
        XCTAssertGreaterThan(widest, 400, "the long message did not use the wider bubble (\(widest) pt)")
        XCUIDevice.shared.orientation = .portrait
    }

    // MARK: - Helpers (ComposerAuditUITests' own)

    @MainActor
    private func launch(_ app: XCUIApplication, extra: [String]) {
        app.terminate()
        app.launchArguments = ["--uitest-reset",
            "-v1.serverURL", server,
            "-v1.sessionToken", token,
            "-AppleLanguages", "(en)",
            "-AppleLocale", "en_US",
        ] + extra
        app.launch()
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let deadline = Date().addingTimeInterval(8)
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

    @MainActor
    private func openChat(_ app: XCUIApplication, _ chat: String) -> Bool {
        let row = app.collectionViews.staticTexts[chat].firstMatch
        if !row.waitForExistence(timeout: 20) || !row.isHittable {
            for label in ["Show Sidebar", "Toggle Sidebar", "ToggleSidebar", "Chats"] {
                let button = app.buttons[label].firstMatch
                if button.exists, button.isHittable { button.tap(); break }
            }
        }
        guard row.waitForExistence(timeout: 5), row.isHittable else { return false }
        row.tap()
        guard composerField(app).waitForExistence(timeout: 20) else { return false }
        sleep(2)
        return true
    }

    @MainActor
    private func composerField(_ app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(
            format: "placeholderValue == %@ AND (elementType == %d OR elementType == %d)",
            "Message",
            XCUIElement.ElementType.textView.rawValue,
            XCUIElement.ElementType.textField.rawValue)).firstMatch
    }

    @MainActor
    private func focusedField(_ app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any)
            .matching(NSPredicate(format: "hasKeyboardFocus == true")).firstMatch
    }

    @MainActor
    private func clearFocusedDraft(_ app: XCUIApplication) {
        let count = (focusedField(app).value as? String)?.count ?? 0
        guard count > 0 else { return }
        app.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: min(count + 2, 400)))
        usleep(500_000)
    }

    @MainActor
    private func capture(_ name: String) {
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    private func note(_ text: String) -> XCTAttachment {
        let attachment = XCTAttachment(string: text)
        attachment.lifetime = .keepAlways
        return attachment
    }
}

#endif
