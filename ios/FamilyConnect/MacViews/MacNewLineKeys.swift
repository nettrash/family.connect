//
//  MacNewLineKeys.swift
//  FamilyConnect
//
//  Return with ⌥, ⌘, ⌃ or ⇧ puts a new line into whichever MULTI-LINE
//  field has the keyboard (issue #95, NewLineKeys).
//
//  Left to AppKit, the four did four different things in a SwiftUI
//  multi-line TextField (measured, through the field editor's own
//  `interpretKeyEvents`): ⌥↩ a new line, ⇧↩ Send (the field editor's
//  `insertNewline:` is its Return), ⌃↩ `insertLineBreak:` — a U+2028 LINE
//  SEPARATOR where every other client writes a line feed — and ⌘↩
//  nothing at all, or the thread's Send. So the press is taken before any
//  of that, from a local key monitor, and answered with the one thing ⌥↩
//  already did: `insertNewlineIgnoringFieldEditor`, a real "\n" at the
//  caret, replacing the selection.
//
//  A single-line field (search, sign-in, a name) is left alone, and so is
//  a press that confirms an input method's marked text.
//

#if os(macOS)

import AppKit

@MainActor
enum MacNewLineKeys {

    private static var monitor: Any?

    static func install() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            take(event) ? nil : event
        }
    }

    /// Return and the keypad's Enter.
    private static let returnKeys: Set<UInt16> = [36, 76]

    /// Whether the press was taken — a new line put into the focused field.
    @discardableResult
    static func take(_ event: NSEvent) -> Bool {
        guard event.type == .keyDown, returnKeys.contains(event.keyCode) else { return false }
        let held = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        guard NewLineKeys.returnMakesLine(
            shift: held.contains(.shift), option: held.contains(.option),
            command: held.contains(.command), control: held.contains(.control))
        else { return false }
        guard let editor = (event.window ?? NSApp.keyWindow)?.firstResponder as? NSTextView,
              editor.isEditable, !editor.hasMarkedText(), takesLines(editor)
        else { return false }
        editor.insertNewlineIgnoringFieldEditor(nil)
        return true
    }

    /// A text view of its own (a TextEditor) always takes lines; a field
    /// editor only for a field that WRAPS — measured: SwiftUI's
    /// `TextField(axis: .vertical)` is a wrapping AppKitTextField, its
    /// one-line TextField the same class not wrapping, and neither sets
    /// `usesSingleLineMode`.
    static func takesLines(_ editor: NSTextView) -> Bool {
        guard editor.isFieldEditor else { return true }
        guard let field = editor.delegate as? NSTextField, let cell = field.cell else { return false }
        return cell.wraps && !cell.usesSingleLineMode && field.maximumNumberOfLines != 1
    }
}

#endif
