//
//  MacCapitalKeys.swift
//  FamilyConnect
//
//  Two things typing does in a prose field on a Mac, from one key monitor:
//
//  - CAPITAL LETTERS at the start of a sentence, in whichever MULTI-LINE
//    field has the keyboard (issue #97, docs/capitals-and-rich-markdown-
//    2026-10-17.md, "Capital letters");
//  - EMOJI AS YOU TYPE — `:)` becomes 😊 and `:fire:` 🔥 the moment the
//    last character goes in — in EVERY prose field, single-line ones
//    included (issue #101, docs/emoji-as-you-type-2026-10-18.md).
//
//  A phone's keyboard capitalises itself; a Mac's does not, in any app. So
//  the press is taken here, from a local key monitor installed beside
//  MacNewLineKeys's, and answered through the text view's own
//  `insertText(_:replacementRange:)`, which is what typing does: undo
//  groups it, the field's delegate sees it, and SwiftUI's binding updates
//  exactly as if the letter had been typed upper-case. The rules
//  themselves are `Capitals` and `EmojiTyping`, shared with Windows and
//  the web. Capitals run first: the emoji rule sees the character the
//  capitals decided (`xD ` at the start of a sentence is `XD `).
//
//  Capitals ONLY while the system's "Capitalise words automatically" is on
//  (`NSSpellChecker.isAutomaticCapitalizationEnabled`): that is the owner's
//  setting for this behaviour on a Mac, and an app that capitalised with
//  it off would be overriding somebody's explicit choice. Emoji are always
//  on (the owner's choice for #101): no system switch speaks for them.
//
//  WHICH FIELDS. Every multi-line field is prose — the composer (also the
//  edit field), the thread reply, a board note, a poll's question, a
//  report's note — and is known by `takesLines`, the set MacNewLineKeys
//  uses. A single-line prose field — a list's line, a poll's option — is
//  marked by `.proseTextEntry()`, which on the Mac puts a MacProseFieldMark
//  behind it. Username, server address, codes, search, a sticker's label
//  and a place are never marked, and are left alone.
//
//  Left alone, always:
//  - a press with ⌘ or ⌃ (a shortcut, not a letter);
//  - an input METHOD (Japanese, Chinese, Korean…): its letters are the
//    start of a composition, not text, and a character pushed into the
//    field under it would break the composition before its marked text
//    even appeared. Only a plain keyboard LAYOUT is helped; marked text,
//    which a dead key also leaves, is a second guard.
//
//  TAKING IT BACK: a Backspace pressed straight after a capital or an
//  emoji this put in — caret right behind it, nothing selected, the text
//  unchanged since — puts back what was typed (the lower-case letter, the
//  `:-)`, the `:fire:`) instead of deleting. Any other key, a click, a
//  paste or a change from anywhere else forgets the offer. ⌘Z after an
//  emoji takes it away too: the swap is its own undo step, after the
//  typing of its last character.
//

#if os(macOS)

import AppKit
import Carbon.HIToolbox
import SwiftUI

@MainActor
enum MacCapitalKeys {

    private static var monitor: Any?

    /// The capital just put in, while Backspace may still take it back.
    private struct Offer {
        weak var editor: NSTextView?
        /// The whole text right after the capital went in: any change since
        /// — a paste, a deletion, the field cleared by Send — voids it.
        let text: String
        /// Where the caret was, right behind the capital (UTF-16).
        let caret: Int
        let lower: String
        let upper: String
    }

    /// The emoji just put in: a Backspace may take it back, and the next
    /// key may grow it (`:(` then `(` is `:((`). One at a time — only one
    /// field has the keyboard.
    private struct LiveSwap {
        weak var editor: NSTextView?
        /// The whole text right after the swap: any change since voids it.
        let text: String
        let swap: EmojiTyping.Swap
    }

    private static var offer: Offer?
    private static var live: LiveSwap?

    static func install() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .leftMouseDown, .rightMouseDown]) { event in
            guard event.type == .keyDown else {
                // A click moves the caret, or means to: the offers are over.
                offer = nil
                live = nil
                return event
            }
            return take(event) ? nil : event
        }
    }

    /// Delete (Backspace) — the key above Return.
    private static let backspaceKey: UInt16 = 51

    /// Whether the press was taken: a capital or an emoji put in, or one
    /// taken back.
    ///
    /// `enabled` (the system's capitalisation switch — it does not gate the
    /// emoji) and `plainLayout` are the two facts that come from the system
    /// rather than the event; a test passes them, because neither the
    /// owner's setting nor his current input source is the test's to
    /// change.
    @discardableResult
    static func take(
        _ event: NSEvent,
        enabled: Bool = NSSpellChecker.isAutomaticCapitalizationEnabled,
        plainLayout: Bool = currentSourceIsKeyboardLayout()
    ) -> Bool {
        guard event.type == .keyDown else { return false }
        let pendingCapital = offer
        let pendingSwap = live
        // Whatever this press is, it is the last one the offers can wait for.
        offer = nil
        live = nil
        let held = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        guard !held.contains(.command), !held.contains(.control) else { return false }
        guard let editor = (event.window ?? NSApp.keyWindow)?.firstResponder as? NSTextView,
              editor.isEditable, !editor.hasMarkedText()
        else { return false }
        let multiLine = MacNewLineKeys.takesLines(editor)
        guard multiLine || MacProseFieldMark.marks(editor) else { return false }
        // Only one of the two can be pending: each press clears both, and a
        // press sets at most one.
        let swap = pendingSwap.flatMap { $0.editor === editor && editor.string == $0.text ? $0.swap : nil }

        if event.keyCode == backspaceKey {
            guard !held.contains(.option) else { return false }
            if let swap { return takeBack(swap, in: editor) }
            guard enabled, multiLine, let pendingCapital else { return false }
            return takeBack(pendingCapital, in: editor)
        }

        guard plainLayout,
              let characters = event.characters, characters.count == 1,
              let typed = characters.first
        else { return false }
        let selection = editor.selectedRange()
        let text = editor.string as NSString
        guard selection.location != NSNotFound, NSMaxRange(selection) <= text.length else { return false }
        let before = text.substring(to: selection.location)

        // Capitals first: the character the emoji rule sees is theirs.
        var put = String(typed)
        var capital: String?
        if enabled, multiLine, let upper = Capitals.upperCase(of: typed),
           Capitals.capitalizes(before: before, typed: typed) {
            capital = upper
            put = upper
        }

        let scalars = put.unicodeScalars
        if scalars.count == 1, let scalar = scalars.first,
           let made = EmojiTyping.replace(before: before, typed: scalar, live: swap) {
            putEmoji(made, typed: put, replacing: selection, in: editor)
            return true
        }

        guard let capital else { return false }
        editor.insertText(capital, replacementRange: selection)
        offer = Offer(
            editor: editor, text: editor.string, caret: editor.selectedRange().location,
            lower: String(typed), upper: capital)
        return true
    }

    /// The emoji in place of what was typed: the last character goes in as
    /// typing does, and then — its own undo step, so one ⌘Z takes the emoji
    /// away and leaves the typing — the swap's range becomes the emoji,
    /// through `shouldChangeText`/`didChangeText` so undo, the delegate and
    /// the binding all see an ordinary edit.
    private static func putEmoji(
        _ made: EmojiTyping.Swap, typed: String, replacing selection: NSRange, in editor: NSTextView
    ) {
        let expected = (editor.string as NSString).replacingCharacters(in: selection, with: typed)
        // Over the selection exactly as a key would put it — NSNotFound, as
        // AppKit's own typing passes — so it joins the typing before it.
        editor.insertText(typed, replacementRange: NSRange(location: NSNotFound, length: 0))
        // Typing may have done more than type — a text replacement, a
        // spelling correction — and then the swap's range is not what the
        // rule looked at: the character stays as typed.
        let caret = editor.selectedRange().location
        guard editor.string == expected, made.at <= caret else { return }
        editor.breakUndoCoalescing()
        let range = NSRange(location: made.at, length: caret - made.at)
        guard replace(range, with: made.put, in: editor) else { return }
        live = LiveSwap(editor: editor, text: editor.string, swap: made)
    }

    /// Put back what was typed over the emoji — `Swap.backspace`, the
    /// rule every port shares, decides whether the caret is right behind it.
    private static func takeBack(_ swap: EmojiTyping.Swap, in editor: NSTextView) -> Bool {
        let selection = editor.selectedRange()
        guard selection.length == 0,
              swap.backspace(text: editor.string, caret: selection.location) != nil
        else { return false }
        let range = NSRange(location: swap.at, length: (swap.put as NSString).length)
        return replace(range, with: swap.typed, in: editor)
    }

    /// One undoable edit: `range` becomes `string`, the caret right behind it.
    private static func replace(_ range: NSRange, with string: String, in editor: NSTextView) -> Bool {
        guard let storage = editor.textStorage, editor.shouldChangeText(in: range, replacementString: string)
        else { return false }
        storage.replaceCharacters(
            in: range, with: NSAttributedString(string: string, attributes: editor.typingAttributes))
        editor.didChangeText()
        editor.setSelectedRange(NSRange(location: range.location + (string as NSString).length, length: 0))
        return true
    }

    /// Put the lower-case letter back over the capital — only when nothing
    /// has happened since it went in.
    private static func takeBack(_ pending: Offer, in editor: NSTextView) -> Bool {
        guard pending.editor === editor, editor.string == pending.text else { return false }
        let selection = editor.selectedRange()
        let length = (pending.upper as NSString).length
        guard selection.length == 0, selection.location == pending.caret, pending.caret >= length
        else { return false }
        let range = NSRange(location: pending.caret - length, length: length)
        guard (editor.string as NSString).substring(with: range) == pending.upper else { return false }
        editor.insertText(pending.lower, replacementRange: range)
        return true
    }

    /// True when the keyboard is a plain LAYOUT (U.S., Russian, German…)
    /// rather than an input method, whose letters begin a composition.
    nonisolated static func currentSourceIsKeyboardLayout() -> Bool {
        guard let source = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue(),
              let raw = TISGetInputSourceProperty(source, kTISPropertyInputSourceType)
        else { return false }
        let type = Unmanaged<CFString>.fromOpaque(raw).takeUnretainedValue()
        return CFEqual(type, kTISTypeKeyboardLayout)
    }
}

/// What makes a SINGLE-LINE SwiftUI field a prose field on the Mac (#101):
/// `.proseTextEntry()` puts one of these behind the field, and a field
/// editor whose text field sits on a mark is a prose field's.
///
/// By GEOMETRY, on purpose. SwiftUI says nothing about which NSTextField
/// backs which TextField, and a single-line field edits through the
/// window's shared field editor; but a background is laid out exactly
/// where its view is, so the field editor's text field has its middle on
/// its own mark and on nobody else's. Nothing here walks SwiftUI's private
/// view hierarchy, and an unmarked field — a username, a search — can only
/// be taken for prose by sitting on top of a prose field.
struct MacProseFieldMark: NSViewRepresentable {

    /// Every mark in a window, held weakly: a field gone is a mark gone.
    private static let all = NSHashTable<NSView>.weakObjects()

    final class Mark: NSView {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if window != nil { MacProseFieldMark.all.add(self) }
        }

        /// Never in the way of a click.
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
    }

    func makeNSView(context: Context) -> Mark { Mark() }

    func updateNSView(_ nsView: Mark, context: Context) {}

    /// Whether `editor` — a field editor — is editing a marked field.
    static func marks(_ editor: NSTextView) -> Bool {
        guard editor.isFieldEditor, let field = editor.delegate as? NSTextField,
              let window = field.window
        else { return false }
        let frame = field.convert(field.bounds, to: nil)
        let middle = NSPoint(x: frame.midX, y: frame.midY)
        return all.allObjects.contains { mark in
            mark.window === window && !mark.isHiddenOrHasHiddenAncestor
                && mark.convert(mark.bounds, to: nil).contains(middle)
        }
    }
}

#endif
