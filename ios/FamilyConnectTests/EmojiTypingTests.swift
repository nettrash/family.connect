//
//  EmojiTypingTests.swift
//  FamilyConnectTests
//
//  AN ORACLE, NOT FOUR READINGS — emoji as you type (#101, docs/emoji-as-
//  you-type-2026-10-18.md) is written once, in `web/text/src/emoji_typing.rs`,
//  and printed by `win/tools/board-oracle` (`cargo run -- emoji-typing`)
//  into Fixtures/emoji-typing-vectors.json: the emoticon table, and 1,600
//  key sequences typed into an empty field with what it then holds. CI
//  prints it again and compares the copies. The Swift port (`EmojiTyping`)
//  answers every case exactly — compared by code units, not canonically.
//
//  Then the two wirings around it: iOS's binding (EmojiTypingField), driven
//  by the same vectors and through a real UIKit field, and the Mac's key
//  monitor (MacCapitalKeys) in a real window with AppKit's field editor.
//

import Foundation
import SwiftUI
import Testing
@testable import FamilyConnect

#if os(macOS)
import AppKit
#else
import UIKit
#endif

@Suite("Emoji as you type")
struct EmojiTypingTests {

    // MARK: - The vectors

    struct Vectors: Decodable {
        struct Emoticon: Decodable {
            let text: String
            let emoji: String
        }

        struct Typing: Decodable {
            let keys: String
            let text: String
        }

        let emoticons: [Emoticon]
        let typing: [Typing]
    }

    /// Bundle(for:) needs a class; a Swift Testing suite is a struct.
    private final class Anchor {}

    /// The synchronized group flattens Fixtures/ into the bundle's root,
    /// with the source tree as the fallback the other vector tests use.
    private static func vectors() throws -> Vectors {
        let url = Bundle(for: Anchor.self).url(forResource: "emoji-typing-vectors", withExtension: "json")
            ?? URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent()
                .appending(path: "Fixtures")
                .appending(path: "emoji-typing-vectors.json")
        return try JSONDecoder().decode(Vectors.self, from: Data(contentsOf: url))
    }

    /// Code unit for code unit: String's `==` is canonical, the Rust's is not.
    private static func same(_ lhs: String, _ rhs: String) -> Bool {
        lhs.utf16.elementsEqual(rhs.utf16)
    }

    /// The Rust's `typing`, line for line: the caret always at the end; `⌫`
    /// a Backspace — the live swap taken back, else the last Unicode scalar
    /// removed; `→` "something else happened" (the live swap dropped); any
    /// other character typed.
    static func typing(_ keys: String) -> String {
        var text = ""
        var live: EmojiTyping.Swap?
        for key in keys.unicodeScalars {
            switch key {
            case "⌫":
                if let undone = live?.backspace(text: text, caret: text.utf16.count) {
                    text = undone.text
                } else if !text.unicodeScalars.isEmpty {
                    text.unicodeScalars.removeLast()
                }
                live = nil
            case "→":
                live = nil
            default:
                if let swap = EmojiTyping.replace(before: text, typed: key, live: live) {
                    text = swap.apply(before: text, after: "").text
                    live = swap
                } else {
                    text.unicodeScalars.append(key)
                    live = nil
                }
            }
        }
        return text
    }

    @Test("the emoticon table is the Rust's, entry for entry, in its order")
    func emoticonTable() throws {
        let expected = try Self.vectors().emoticons
        #expect(EmojiTyping.emoticons.count == expected.count)
        for (index, (ours, theirs)) in zip(EmojiTyping.emoticons, expected).enumerated() {
            #expect(Self.same(ours.text, theirs.text), "emoticon \(index): \(ours.text) is not \(theirs.text)")
            #expect(Self.same(ours.emoji, theirs.emoji), "emoticon \(index) (\(ours.text)): wrong emoji")
        }
    }

    @Test("every key sequence in the vectors types what the Rust types")
    func typingVectors() throws {
        let cases = try Self.vectors().typing
        #expect(cases.count > 1000, "only \(cases.count) vectors")
        var failures = 0
        for vector in cases {
            let typed = Self.typing(vector.keys)
            if !Self.same(typed, vector.text) {
                failures += 1
                Issue.record("typing \(vector.keys.debugDescription) gave \(typed.debugDescription), not \(vector.text.debugDescription)")
            }
        }
        #expect(failures == 0)
    }

    @Test("the names load from the app bundle, exactly")
    func names() {
        #expect(EmojiTyping.nameCount > 6000, "\(EmojiTyping.nameCount) names")
        #expect(EmojiTyping.emoji(named: "fire") == "🔥")
        #expect(EmojiTyping.emoji(named: "like") == "👍")
        #expect(EmojiTyping.emoji(named: "") == nil)
        #expect(EmojiTyping.emoji(named: ":fire:") == nil)
        #expect(EmojiTyping.emoji(named: "Fire") == nil)
        // Byte for byte: the table's ñ is the composed one, and the
        // decomposed spelling is a different name, as it is to the Rust.
        #expect(EmojiTyping.emoji(named: "pi\u{F1}ata") == "🪅")
        #expect(EmojiTyping.emoji(named: "pin\u{303}ata") == nil)
    }

    @Test("the name file is read as the Rust's lines() reads it")
    func nameParsing() {
        let tsv = "# a comment\tthat has a tab\r\nfire\t🔥\r\n\nboth\t1\nboth\t2\nno tab here\n#x\tno"
        let table = EmojiTyping.parseNames(Data(tsv.utf8))
        #expect(table.count == 2)
        #expect(table[Array("fire".utf8)] == "🔥")
        #expect(table[Array("both".utf8)] == "2")
    }

    @Test("canonical twins of ASCII are not the ASCII: U+037E is not a semicolon")
    func canonicalTwins() {
        #expect(EmojiTyping.replace(before: "\u{37E}", typed: ")", live: nil) == nil)
        #expect(EmojiTyping.replace(before: ";", typed: ")", live: nil) != nil)
        // GREEK VARIA is canonically a backtick, but it opens no code span.
        #expect(EmojiTyping.replace(before: "\u{1FEF}:", typed: ")", live: nil) != nil)
        #expect(EmojiTyping.replace(before: "`:", typed: ")", live: nil) == nil)
    }

    // MARK: - The swap, in the middle of a text

    @Test("a swap lands between before and after, and its Backspace puts the typing back")
    func swapInTheMiddle() throws {
        let swap = try #require(EmojiTyping.replace(before: "ok :", typed: ")", live: nil))
        #expect(swap == EmojiTyping.Swap(at: 3, put: "😊", typed: ":)"))
        let applied = swap.apply(before: "ok :", after: " bye")
        #expect(Self.same(applied.text, "ok 😊 bye"))
        #expect(applied.caret == 3 + "😊".utf16.count)
        let undone = try #require(swap.backspace(text: applied.text, caret: applied.caret))
        #expect(Self.same(undone.text, "ok :) bye"))
        #expect(undone.caret == 5)
        // Not right behind it: an ordinary Backspace.
        #expect(swap.backspace(text: applied.text, caret: applied.caret + 1) == nil)
        #expect(swap.backspace(text: applied.text, caret: applied.caret - 1) == nil)
        // The replacement no longer there.
        #expect(swap.backspace(text: "ok x bye", caret: applied.caret) == nil)
        #expect(swap.backspace(text: "ok", caret: applied.caret) == nil)
    }

    @Test("offsets are UTF-16, behind astral emoji and Cyrillic alike")
    func swapAfterWideText() throws {
        let before = "Привет 🔥 :fire"
        let swap = try #require(EmojiTyping.replace(before: before, typed: ":", live: nil))
        #expect(swap.at == (before as NSString).length - 5)
        #expect(Self.same(swap.typed, ":fire:"))
        let applied = swap.apply(before: before, after: "!")
        #expect(Self.same(applied.text, "Привет 🔥 🔥!"))
        let undone = try #require(swap.backspace(text: applied.text, caret: applied.caret))
        #expect(Self.same(undone.text, "Привет 🔥 :fire:!"))
        #expect(undone.caret == (before as NSString).length + 1)
    }

    @Test("a space-ended emoticon keeps its space, and takes it back with the rest")
    func spaceEnded() throws {
        let swap = try #require(EmojiTyping.replace(before: "so :o", typed: " ", live: nil))
        let applied = swap.apply(before: "so :o", after: "end")
        #expect(Self.same(applied.text, "so 😨 end"))
        let undone = try #require(swap.backspace(text: applied.text, caret: applied.caret))
        #expect(Self.same(undone.text, "so :o end"))
    }

    @Test("a swap grows only from where it started, and only right behind it")
    func growing() throws {
        let sad = try #require(EmojiTyping.replace(before: "a :", typed: "(", live: nil))
        let text = sad.apply(before: "a :", after: "").text
        let grown = try #require(EmojiTyping.replace(before: text, typed: "(", live: sad))
        #expect(grown == EmojiTyping.Swap(at: 2, put: "😩", typed: ":(("))
        // Not right behind it any more: no growing.
        #expect(EmojiTyping.replace(before: text + "x", typed: "(", live: sad) == nil)
        // A shortcode's closing colon and a `(` are not a `:(`.
        let fire = try #require(EmojiTyping.replace(before: ":fire", typed: ":", live: nil))
        #expect(EmojiTyping.replace(before: fire.apply(before: ":fire", after: "").text, typed: "(", live: fire) == nil)
    }

    // MARK: - iOS's binding

    /// The vectors through EmojiTypingField, as a phone's field would write
    /// them: each key one scalar more at the end, each Backspace the last
    /// character fewer. `→` has no counterpart — the field never says
    /// "something else happened" — so those sequences are left out.
    @MainActor
    @Test("the iOS field's binding types every vector as the rule does")
    func fieldTypesTheVectors() throws {
        var checked = 0
        for vector in try Self.vectors().typing where !vector.keys.contains("→") {
            let field = EmojiTypingField()
            var text = ""
            for key in vector.keys.unicodeScalars {
                var written = text
                if key == "⌫" {
                    if !written.isEmpty { written.removeLast() }
                } else {
                    written.unicodeScalars.append(key)
                }
                text = field.typed(written, over: text)
            }
            #expect(Self.same(text, vector.text), "typing \(vector.keys.debugDescription) gave \(text.debugDescription)")
            checked += 1
        }
        #expect(checked > 1000)
    }

    @MainActor
    @Test("the iOS field leaves alone what is not one character typed at the end")
    func fieldLeavesOtherChangesAlone() {
        let field = EmojiTypingField()
        // A paste, a suggestion: more than one character.
        #expect(Self.same(field.typed("Hi :)", over: "Hi "), "Hi :)"))
        // A character typed in the middle.
        #expect(Self.same(field.typed(":)x", over: ":x"), ":)x"))
        // One character typed at the end is the rule's.
        #expect(Self.same(field.typed("Hi :)", over: "Hi :"), "Hi 😊"))
        // The field writing back what it holds keeps the swap…
        #expect(Self.same(field.typed("Hi 😊", over: "Hi 😊"), "Hi 😊"))
        // …and a Backspace right behind it takes it back.
        #expect(Self.same(field.typed("Hi ", over: "Hi 😊"), "Hi :)"))
        // Once: the next Backspace is an ordinary one.
        #expect(Self.same(field.typed("Hi :", over: "Hi :)"), "Hi :"))
    }

    @MainActor
    @Test("the iOS field drops the swap when anything else changes, or another field types")
    func fieldDropsTheSwap() {
        let field = EmojiTypingField()
        #expect(Self.same(field.typed(":)", over: ":", in: "question"), "😊"))
        // Typing in another field: the swap is gone.
        #expect(Self.same(field.typed("a", over: "", in: "option"), "a"))
        #expect(Self.same(field.typed("", over: "😊", in: "question"), ""))
        // The text changed from elsewhere since the swap — here an edit
        // before it — even though the emoji is still right at the end.
        #expect(Self.same(field.typed("a :(", over: "a :", in: "question"), "a 😞"))
        #expect(Self.same(field.typed("b ", over: "b 😞", in: "question"), "b "))
        // A change that is not a Backspace right behind it.
        #expect(Self.same(field.typed(":D", over: ":", in: "question"), "😃"))
        #expect(Self.same(field.typed("😃ab", over: "😃", in: "question"), "😃ab"))
    }

    #if os(iOS)
    /// A SwiftUI field in a real window, written to through UIKit's own
    /// text input — what the keyboard does — so the round trip through the
    /// binding and back into the field is the real one.
    @MainActor
    fileprivate final class PhoneHost {
        @Observable final class Model {
            var single = ""
            var many = ""
        }

        struct Fields: View {
            let model: Model
            @State private var emoji = EmojiTypingField()
            var body: some View {
                @Bindable var model = model
                VStack {
                    TextField("One", text: emoji.binding($model.single, field: "single"))
                        .proseTextEntry()
                    TextField("Many", text: emoji.binding($model.many, field: "many"), axis: .vertical)
                        .proseTextEntry()
                }
            }
        }

        let model = Model()
        let window: UIWindow

        init() {
            let canvas = CGRect(x: 0, y: 0, width: 390, height: 300)
            let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
            window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: canvas)
            window.frame = canvas
            window.rootViewController = UIHostingController(rootView: Fields(model: model))
            window.isHidden = false
        }

        /// Layout, and time for the main actor to run what is queued — the
        /// swap is written a turn after the field's own write, so the test
        /// SUSPENDS rather than spinning the run loop inside its own turn.
        func settle() async throws {
            for _ in 0..<3 {
                window.layoutIfNeeded()
                try await Task.sleep(for: .milliseconds(50))
            }
        }

        func inputs() -> [UIView & UIKeyInput] {
            func walk(_ view: UIView) -> [UIView & UIKeyInput] {
                let own: [UIView & UIKeyInput] =
                    if let field = view as? UITextField { [field] } else if let text = view as? UITextView { [text] } else { [] }
                return own + view.subviews.flatMap(walk)
            }
            return walk(window)
        }

        func text(of input: UIView) -> String {
            (input as? UITextField)?.text ?? (input as? UITextView)?.text ?? ""
        }

        func type(_ keys: String, into input: UIView & UIKeyInput) async throws {
            for key in keys {
                if key == "⌫" { input.deleteBackward() } else { input.insertText(String(key)) }
                try await settle()
            }
        }
    }

    @MainActor
    @Test("typed through UIKit, the phone's fields turn :) into 😊 and take it back")
    func phoneFieldsRoundTrip() async throws {
        let host = PhoneHost()
        defer { host.window.isHidden = true }
        try await host.settle()
        let inputs = host.inputs()
        try #require(inputs.count == 2, "found \(inputs.count) text inputs")
        for (input, read) in zip(inputs, [{ host.model.single }, { host.model.many }]) {
            _ = input.becomeFirstResponder()
            try await host.type("Hi :)", into: input)
            #expect(Self.same(read(), "Hi 😊"))
            #expect(Self.same(host.text(of: input), "Hi 😊"))
            try await host.type("⌫", into: input)
            #expect(Self.same(read(), "Hi :)"))
            #expect(Self.same(host.text(of: input), "Hi :)"))
            try await host.type(" :fire:", into: input)
            #expect(Self.same(read(), "Hi :) 🔥"))
            #expect(Self.same(host.text(of: input), "Hi :) 🔥"))
        }
    }
    #endif

    // MARK: - The Mac's key monitor

    #if os(macOS)
    /// Three SwiftUI fields in a real window — a multi-line one, a
    /// single-line PROSE one, a single-line plain one — so the field editor
    /// is AppKit's own, as MacCapitalKeys meets it in the app.
    @MainActor
    private final class MacHost {
        struct Fields: View {
            @State var many = "note"
            @State var option = "option"
            @State var search = "search"
            var body: some View {
                VStack {
                    TextField("Message", text: $many, axis: .vertical).lineLimit(1...8)
                    TextField("Option", text: $option).proseTextEntry()
                    TextField("Search", text: $search)
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

        /// The field holding `text`, focused and emptied, its editor.
        func focus(_ text: String) throws -> NSTextView {
            func fields(_ view: NSView) -> [NSTextField] {
                ((view as? NSTextField).map { [$0] } ?? []) + view.subviews.flatMap(fields)
            }
            let content = try #require(window.contentView)
            let field = try #require(fields(content).first { $0.stringValue == text })
            #expect(window.makeFirstResponder(field))
            let editor = try #require(window.firstResponder as? NSTextView)
            editor.selectAll(nil)
            editor.insertText("", replacementRange: editor.selectedRange())
            return editor
        }

        func key(_ characters: String, keyCode: UInt16 = 0) throws -> NSEvent {
            try #require(NSEvent.keyEvent(
                with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
                windowNumber: window.windowNumber, context: nil, characters: characters,
                charactersIgnoringModifiers: characters, isARepeat: false, keyCode: keyCode))
        }

        /// Each key offered to the monitor first, as the app's local
        /// monitor does, and typed (or deleted) the ordinary way when it is
        /// not taken. `⌫` is Backspace.
        func type(_ keys: String, into editor: NSTextView, capitals: Bool = false) throws {
            for key in keys {
                if key == "⌫" {
                    if !MacCapitalKeys.take(try self.key("\u{7F}", keyCode: 51), enabled: capitals, plainLayout: true) {
                        editor.deleteBackward(nil)
                    }
                } else if !MacCapitalKeys.take(try self.key(String(key)), enabled: capitals, plainLayout: true) {
                    // As AppKit's typing passes it: over the selection.
                    editor.insertText(String(key), replacementRange: NSRange(location: NSNotFound, length: 0))
                }
            }
        }
    }

    @MainActor
    @Test("multi-line and single-line prose fields turn :) into 😊; Backspace takes it back")
    func macProseFields() throws {
        let host = MacHost()
        for name in ["note", "option"] {
            let editor = try host.focus(name)
            try host.type("Hi :)", into: editor)
            #expect(Self.same(editor.string, "Hi 😊"), "\(name)")
            try host.type("⌫", into: editor)
            #expect(Self.same(editor.string, "Hi :)"), "\(name)")
            // Once: the next Backspace is an ordinary one.
            try host.type("⌫", into: editor)
            #expect(Self.same(editor.string, "Hi :"), "\(name)")
            try host.type(") :((", into: editor)
            #expect(Self.same(editor.string, "Hi 😊 😩"), "\(name)")
            try host.type("⌫", into: editor)
            #expect(Self.same(editor.string, "Hi 😊 :(("), "\(name)")
            editor.selectAll(nil)
            editor.insertText("", replacementRange: editor.selectedRange())
        }
    }

    @MainActor
    @Test("a single-line field that is not prose — a search — is left alone")
    func macPlainFieldLeftAlone() throws {
        let host = MacHost()
        let editor = try host.focus("search")
        try host.type(":) :fire:", into: editor)
        #expect(Self.same(editor.string, ":) :fire:"))
    }

    @MainActor
    @Test("in the middle of a text the caret lands right behind the emoji, and takes it back there")
    func macMiddleOfText() throws {
        let host = MacHost()
        let editor = try host.focus("note")
        editor.insertText("ok  bye", replacementRange: editor.selectedRange())
        editor.setSelectedRange(NSRange(location: 3, length: 0))
        try host.type(":)", into: editor)
        #expect(Self.same(editor.string, "ok 😊 bye"))
        #expect(editor.selectedRange() == NSRange(location: 3 + "😊".utf16.count, length: 0))
        try host.type("⌫", into: editor)
        #expect(Self.same(editor.string, "ok :) bye"))
        #expect(editor.selectedRange() == NSRange(location: 5, length: 0))
    }

    @MainActor
    @Test("capitals run first: the emoji rule sees the capital, and emoji ignore the capitals switch")
    func macCapitalsThenEmoji() throws {
        let host = MacHost()
        let editor = try host.focus("note")
        // At a sentence start `x` goes in as `X`, so `xD ` is `XD `.
        try host.type("Hi. xD ", into: editor, capitals: true)
        #expect(Self.same(editor.string, "Hi. 😆 "))
        // With the capitals switch off, emoji still come.
        try host.type(":fire:", into: editor, capitals: false)
        #expect(Self.same(editor.string, "Hi. 😆 🔥"))
    }

    @MainActor
    @Test("any other key forgets the swap; code is typed as it is")
    func macSwapForgotten() throws {
        let host = MacHost()
        let editor = try host.focus("note")
        try host.type(":)a⌫⌫", into: editor)
        #expect(Self.same(editor.string, ""))
        try host.type("`:)", into: editor)
        #expect(Self.same(editor.string, "`:)"))
    }

    @MainActor
    @Test("⌘Z after an emoji takes it away as one step, and ⌘⇧Z brings it back")
    func macUndo() throws {
        let host = MacHost()
        let editor = try host.focus("note")
        let undo = try #require(editor.undoManager)
        // One key, one undo group, as one key is one event in the app: a
        // test sends no events, so the groups are opened and closed here.
        undo.groupsByEvent = false
        defer { undo.groupsByEvent = true }
        while undo.groupingLevel > 0 { undo.endUndoGrouping() }
        undo.removeAllActions()
        for key in "ok :)" {
            undo.beginUndoGrouping()
            try host.type(String(key), into: editor)
            undo.endUndoGrouping()
        }
        #expect(Self.same(editor.string, "ok 😊"))
        // One ⌘Z takes the emoji away and leaves the typing. In the app the
        // `)` joins the typing run before it, so what comes back is `ok :)`;
        // a test's insertText never coalesces across groups (measured: "ok
        // :x" undoes one key at a time here), so the `)` goes with the swap.
        undo.undo()
        #expect(Self.same(editor.string, "ok :)") || Self.same(editor.string, "ok :"), "\(editor.string)")
        undo.redo()
        #expect(Self.same(editor.string, "ok 😊"))
    }
    #endif
}
