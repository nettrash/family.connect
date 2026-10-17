//
//  NewLineKeysTests.swift
//  FamilyConnectTests
//
//  #95: Return sends; Return with ⌥ ⌘ ⌃ or ⇧ is a new line.
//

import Testing
@testable import FamilyConnect
#if os(macOS)
import AppKit
import SwiftUI
#endif

struct NewLineKeysTests {

    @Test("Return alone is not a new line; with any modifier it is")
    func rule() {
        #expect(!NewLineKeys.returnMakesLine(shift: false, option: false, command: false, control: false))
        #expect(NewLineKeys.returnMakesLine(shift: true, option: false, command: false, control: false))
        #expect(NewLineKeys.returnMakesLine(shift: false, option: true, command: false, control: false))
        #expect(NewLineKeys.returnMakesLine(shift: false, option: false, command: true, control: false))
        #expect(NewLineKeys.returnMakesLine(shift: false, option: false, command: false, control: true))
        #expect(NewLineKeys.returnMakesLine(shift: true, option: true, command: true, control: true))
    }

    #if os(macOS)
    /// Both kinds of SwiftUI field, in a real window, so the field editor
    /// is AppKit's own — what MacNewLineKeys meets in the app.
    @MainActor
    private final class Window {
        struct Fields: View {
            @State var many = "hello world"
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

        /// The field showing `text`, made the one being typed into.
        func focus(_ text: String) throws -> NSTextView {
            func fields(_ view: NSView) -> [NSTextField] {
                ((view as? NSTextField).map { [$0] } ?? []) + view.subviews.flatMap(fields)
            }
            let content = try #require(window.contentView)
            let field = try #require(fields(content).first { $0.stringValue == text })
            #expect(window.makeFirstResponder(field))
            return try #require(window.firstResponder as? NSTextView)
        }

        func returnKey(_ held: NSEvent.ModifierFlags, keyCode: UInt16 = 36) throws -> NSEvent {
            try #require(NSEvent.keyEvent(
                with: .keyDown, location: .zero, modifierFlags: held, timestamp: 0,
                windowNumber: window.windowNumber, context: nil, characters: "\r",
                charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: keyCode))
        }
    }

    @MainActor
    @Test("the message field takes lines; a one-line field does not")
    func whichFields() throws {
        let host = Window()
        #expect(MacNewLineKeys.takesLines(try host.focus("hello world")))
        #expect(!MacNewLineKeys.takesLines(try host.focus("search")))
        #expect(MacNewLineKeys.takesLines(NSTextView()), "a TextEditor's own text view")
    }

    @MainActor
    @Test("⇧ ⌥ ⌘ ⌃ Return and keypad Enter put a \\n at the caret, over the selection",
          arguments: [NSEvent.ModifierFlags.shift, .option, .command, .control])
    func newLine(held: NSEvent.ModifierFlags) throws {
        let host = Window()
        let editor = try host.focus("hello world")
        editor.setSelectedRange(NSRange(location: 5, length: 1))
        #expect(MacNewLineKeys.take(try host.returnKey(held)))
        #expect(editor.string == "hello\nworld")
        #expect(editor.selectedRange() == NSRange(location: 6, length: 0))
        #expect(MacNewLineKeys.take(try host.returnKey(held, keyCode: 76)))
        #expect(editor.string == "hello\n\nworld")
    }

    @MainActor
    @Test("plain Return, a one-line field and marked text are left to the field")
    func leftAlone() throws {
        let host = Window()
        let editor = try host.focus("hello world")
        #expect(!MacNewLineKeys.take(try host.returnKey([])), "Return alone is Send")
        editor.setMarkedText("か", selectedRange: NSRange(location: 1, length: 0),
                             replacementRange: NSRange(location: NSNotFound, length: 0))
        #expect(!MacNewLineKeys.take(try host.returnKey(.shift)), "it confirms the input method")
        editor.unmarkText()
        let search = try host.focus("search")
        #expect(!MacNewLineKeys.take(try host.returnKey(.shift)))
        #expect(search.string == "search")
    }
    #endif
}
