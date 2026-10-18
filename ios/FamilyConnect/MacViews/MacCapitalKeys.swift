//
//  MacCapitalKeys.swift
//  FamilyConnect
//
//  Capital letters at the start of a sentence, in whichever MULTI-LINE
//  field has the keyboard (issue #97, docs/capitals-and-rich-markdown-
//  2026-10-17.md, "Capital letters").
//
//  A phone's keyboard does this itself; a Mac's does not, in any app. So
//  the press is taken here, from a local key monitor installed beside
//  MacNewLineKeys's, and answered with the capital — through the text
//  view's own `insertText(_:replacementRange:)`, which is what typing does:
//  undo groups it, the field's delegate sees it, and SwiftUI's binding
//  updates exactly as if the letter had been typed upper-case. The rule
//  itself is `Capitals`, shared with Windows and the web.
//
//  ONLY while the system's "Capitalise words automatically" is on
//  (`NSSpellChecker.isAutomaticCapitalizationEnabled`): that is the owner's
//  setting for this behaviour on a Mac, and an app that capitalised with
//  it off would be overriding somebody's explicit choice.
//
//  Left alone, always:
//  - a single-line field (search, sign-in, a name) — `takesLines`, the
//    same set MacNewLineKeys uses, and the same set every desktop client
//    capitalises in;
//  - a press with ⌘ or ⌃ (a shortcut, not a letter);
//  - an input METHOD (Japanese, Chinese, Korean…): its letters are the
//    start of a composition, not text, and a capital pushed into the field
//    under it would break the composition before its marked text even
//    appeared. Only a plain keyboard LAYOUT is helped; marked text, which
//    a dead key also leaves, is a second guard.
//
//  TAKING IT BACK: a Backspace pressed straight after a capital this put
//  in — caret right behind it, nothing selected, the text unchanged since —
//  puts the lower-case letter back instead of deleting it. Any other key,
//  or a click, forgets the offer.
//

#if os(macOS)

import AppKit
import Carbon.HIToolbox

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

    private static var offer: Offer?

    static func install() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .leftMouseDown, .rightMouseDown]) { event in
            guard event.type == .keyDown else {
                // A click moves the caret, or means to: the offer is over.
                offer = nil
                return event
            }
            return take(event) ? nil : event
        }
    }

    /// Delete (Backspace) — the key above Return.
    private static let backspaceKey: UInt16 = 51

    /// Whether the press was taken: a capital put in, or one taken back.
    ///
    /// `enabled` and `plainLayout` are the two facts that come from the
    /// system rather than the event; a test passes them, because neither
    /// the owner's setting nor his current input source is the test's to
    /// change.
    @discardableResult
    static func take(
        _ event: NSEvent,
        enabled: Bool = NSSpellChecker.isAutomaticCapitalizationEnabled,
        plainLayout: Bool = currentSourceIsKeyboardLayout()
    ) -> Bool {
        guard event.type == .keyDown else { return false }
        let pending = offer
        // Whatever this press is, it is the last one the offer can wait for.
        offer = nil
        guard enabled else { return false }
        let held = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        guard !held.contains(.command), !held.contains(.control) else { return false }
        guard let editor = (event.window ?? NSApp.keyWindow)?.firstResponder as? NSTextView,
              editor.isEditable, !editor.hasMarkedText(), MacNewLineKeys.takesLines(editor)
        else { return false }

        if event.keyCode == backspaceKey {
            guard !held.contains(.option), let pending else { return false }
            return takeBack(pending, in: editor)
        }

        guard plainLayout,
              let characters = event.characters, characters.count == 1,
              let typed = characters.first,
              let upper = Capitals.upperCase(of: typed)
        else { return false }
        let selection = editor.selectedRange()
        let text = editor.string as NSString
        guard selection.location != NSNotFound, selection.location <= text.length else { return false }
        let before = text.substring(to: selection.location)
        guard Capitals.capitalizes(before: before, typed: typed) else { return false }

        editor.insertText(upper, replacementRange: selection)
        offer = Offer(
            editor: editor, text: editor.string, caret: editor.selectedRange().location,
            lower: String(typed), upper: upper)
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

#endif
