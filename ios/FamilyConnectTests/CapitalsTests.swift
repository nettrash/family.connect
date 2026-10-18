//
//  CapitalsTests.swift
//  FamilyConnectTests
//
//  #97: the desktop's capital letters. The table is the design doc's
//  (docs/capitals-and-rich-markdown-2026-10-17.md, "The table every port's
//  tests hold"), VERBATIM and in its order — Windows' CapitalsTests.cs and
//  the web's composer.rs hold the same rows, so a port that drifts fails
//  here rather than on somebody's keyboard.
//

import Testing
@testable import FamilyConnect
#if os(macOS)
import AppKit
import SwiftUI
#endif

struct CapitalsTests {

    struct Case: CustomTestStringConvertible, Sendable {
        let before: String
        let typed: Character
        let capitalizes: Bool

        var testDescription: String {
            "\(before.debugDescription) + \(String(typed).debugDescription) → \(capitalizes ? "yes" : "no")"
        }
    }

    /// `⏎` in the doc is "\n", `⇥` is "\t".
    nonisolated static let table: [Case] = [
        Case(before: "", typed: "h", capitalizes: true),
        Case(before: "Hi. ", typed: "h", capitalizes: true),
        Case(before: "Hi.", typed: "h", capitalizes: false),  // the word is not starting
        Case(before: "hi ", typed: "h", capitalizes: false),
        Case(before: "Really? ", typed: "y", capitalizes: true),
        Case(before: "Wow! ", typed: "s", capitalizes: true),
        Case(before: "Wait... ", typed: "w", capitalizes: false),  // an ellipsis
        Case(before: "See e.g. ", typed: "t", capitalizes: false),  // an abbreviation
        Case(before: "It's at example.com. ", typed: "t", capitalizes: true),
        Case(before: "Line\n", typed: "n", capitalizes: true),
        Case(before: "He said \"stop.\" ", typed: "t", capitalizes: true),
        Case(before: "(Done.) ", typed: "n", capitalizes: true),
        Case(before: "\"", typed: "h", capitalizes: true),
        Case(before: "Hi. (", typed: "s", capitalizes: true),
        Case(before: "- ", typed: "m", capitalizes: true),
        Case(before: "List:\n1. ", typed: "b", capitalizes: true),
        Case(before: "12) ", typed: "x", capitalizes: true),
        Case(before: "# ", typed: "t", capitalizes: true),
        Case(before: "> ", typed: "q", capitalizes: true),
        Case(before: "```\n", typed: "l", capitalizes: false),  // inside a fence
        Case(before: "```\ncode\n```\n", typed: "n", capitalizes: true),  // the fence closed
        Case(before: "Use `x. ", typed: "y", capitalizes: false),  // inside a code span
        Case(before: "Hi. ", typed: "H", capitalizes: false),  // already upper case
        Case(before: "Hi. ", typed: "1", capitalizes: false),  // not a letter
        Case(before: "", typed: "ß", capitalizes: false),  // its upper case is two letters
        Case(before: "Hi.  ", typed: "h", capitalizes: true),  // two spaces
        Case(before: "Hi.\t", typed: "h", capitalizes: true),  // a tab
        Case(before: "", typed: "é", capitalizes: true),
        Case(before: "Привет. ", typed: "к", capitalizes: true),
        Case(before: "Ok.\n\n", typed: "n", capitalizes: true),
        Case(before: "@ann ", typed: "h", capitalizes: false),
    ]

    @Test("the design doc's table, row by row", arguments: table)
    func row(_ row: Case) {
        #expect(Capitals.capitalizes(before: row.before, typed: row.typed) == row.capitalizes)
    }

    @Test("the capital is one scalar or nothing")
    func upperCase() {
        #expect(Capitals.upperCase(of: "h") == "H")
        #expect(Capitals.upperCase(of: "к") == "К")
        #expect(Capitals.upperCase(of: "é") == "É")
        #expect(Capitals.upperCase(of: "ß") == nil)
        // A J and a combining caron: one grapheme, two scalars.
        #expect(Capitals.upperCase(of: "ǰ") == nil)
        #expect(Capitals.upperCase(of: "H") == nil)
        #expect(Capitals.upperCase(of: "7") == nil)
        #expect(Capitals.upperCase(of: "😀") == nil)
    }

    @Test("edges the table leaves implicit")
    func edges() {
        // An indented marker is still a marker: a nested item.
        #expect(Capitals.capitalizes(before: "  - ", typed: "a"))
        #expect(Capitals.capitalizes(before: "List:\n\t2. ", typed: "a"))
        // Seven hashes are not a heading.
        #expect(!Capitals.capitalizes(before: "####### ", typed: "a"))
        #expect(Capitals.capitalizes(before: "###### ", typed: "a"))
        // Abbreviation pieces are ASCII: a Cyrillic "т.е." still ends a
        // sentence — the rule is ASCII on purpose, so four ports agree.
        #expect(Capitals.capitalizes(before: "т.е. ", typed: "a"))
        #expect(!Capitals.capitalizes(before: "a.m. ", typed: "a"))
        // A closed code span on the line is not code any more.
        #expect(Capitals.capitalizes(before: "Use `x`. ", typed: "y"))
        // Several opening marks are all set aside.
        #expect(Capitals.capitalizes(before: "Hi. (\"", typed: "s"))
        // Windows line ends are line breaks too.
        #expect(Capitals.capitalizes(before: "Line\r\n", typed: "n"))
    }

    #if os(macOS)
    /// A multi-line SwiftUI field in a real window, so the field editor is
    /// AppKit's own — what MacCapitalKeys meets in the app.
    @MainActor
    private final class Window {
        struct Fields: View {
            @State var many = "Hi."
            @State var one = "search"
            var body: some View {
                VStack {
                    TextField("Message", text: $many, axis: .vertical).lineLimit(1...8)
                    TextField("Search", text: $one)
                }
                .frame(width: 300, height: 200)
            }
        }

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 300, height: 200),
            styleMask: [.titled], backing: .buffered, defer: false)

        init() {
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(rootView: Fields())
            window.layoutIfNeeded()
        }

        func focus(_ text: String) throws -> NSTextView {
            func fields(_ view: NSView) -> [NSTextField] {
                ((view as? NSTextField).map { [$0] } ?? []) + view.subviews.flatMap(fields)
            }
            let content = try #require(window.contentView)
            let field = try #require(fields(content).first { $0.stringValue == text })
            #expect(window.makeFirstResponder(field))
            return try #require(window.firstResponder as? NSTextView)
        }

        func key(_ characters: String, keyCode: UInt16, held: NSEvent.ModifierFlags = []) throws -> NSEvent {
            try #require(NSEvent.keyEvent(
                with: .keyDown, location: .zero, modifierFlags: held, timestamp: 0,
                windowNumber: window.windowNumber, context: nil, characters: characters,
                charactersIgnoringModifiers: characters, isARepeat: false, keyCode: keyCode))
        }

        func backspace(held: NSEvent.ModifierFlags = []) throws -> NSEvent {
            try key("\u{7F}", keyCode: 51, held: held)
        }
    }

    @MainActor
    @Test("a sentence's first letter goes in as a capital, and Backspace takes it back")
    func macCapitalAndTakeBack() throws {
        let host = Window()
        let editor = try host.focus("Hi.")
        editor.setSelectedRange(NSRange(location: 3, length: 0))
        editor.insertText(" ", replacementRange: editor.selectedRange())
        #expect(MacCapitalKeys.take(try host.key("t", keyCode: 17), enabled: true, plainLayout: true))
        #expect(editor.string == "Hi. T")
        // Straight after: the lower-case letter comes back instead of the
        // capital going away.
        #expect(MacCapitalKeys.take(try host.backspace(), enabled: true, plainLayout: true))
        #expect(editor.string == "Hi. t")
        // Once only: the next Backspace is an ordinary one.
        #expect(!MacCapitalKeys.take(try host.backspace(), enabled: true, plainLayout: true))
    }

    @MainActor
    @Test("any other key forgets the take-back")
    func macTakeBackForgotten() throws {
        let host = Window()
        let editor = try host.focus("Hi.")
        editor.setSelectedRange(NSRange(location: 3, length: 0))
        editor.insertText(" ", replacementRange: editor.selectedRange())
        #expect(MacCapitalKeys.take(try host.key("t", keyCode: 17), enabled: true, plainLayout: true))
        // Another letter, mid-word: not capitalised, and it ends the offer.
        #expect(!MacCapitalKeys.take(try host.key("o", keyCode: 31), enabled: true, plainLayout: true))
        editor.insertText("o", replacementRange: editor.selectedRange())
        editor.deleteBackward(nil)
        #expect(editor.string == "Hi. T")
        #expect(!MacCapitalKeys.take(try host.backspace(), enabled: true, plainLayout: true))
    }

    @MainActor
    @Test("left alone: the setting off, an input method, ⌘/⌃, mid-sentence, a one-line field")
    func macLeftAlone() throws {
        let host = Window()
        let editor = try host.focus("Hi.")
        editor.setSelectedRange(NSRange(location: 3, length: 0))
        editor.insertText(" ", replacementRange: editor.selectedRange())
        #expect(!MacCapitalKeys.take(try host.key("t", keyCode: 17), enabled: false, plainLayout: true))
        #expect(!MacCapitalKeys.take(try host.key("t", keyCode: 17), enabled: true, plainLayout: false))
        #expect(!MacCapitalKeys.take(try host.key("t", keyCode: 17, held: .command), enabled: true, plainLayout: true))
        #expect(!MacCapitalKeys.take(try host.key("t", keyCode: 17, held: .control), enabled: true, plainLayout: true))
        editor.setMarkedText("か", selectedRange: NSRange(location: 1, length: 0),
                             replacementRange: NSRange(location: NSNotFound, length: 0))
        #expect(!MacCapitalKeys.take(try host.key("t", keyCode: 17), enabled: true, plainLayout: true))
        editor.unmarkText()
        #expect(editor.string == "Hi. か")
        let search = try host.focus("search")
        search.setSelectedRange(NSRange(location: 0, length: 0))
        #expect(!MacCapitalKeys.take(try host.key("t", keyCode: 17), enabled: true, plainLayout: true))
        #expect(search.string == "search")
    }
    #endif
}
