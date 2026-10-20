//
//  EmojiTypingField.swift
//  FamilyConnect
//
//  Emoji as you type on iPhone and iPad (issue #101, docs/emoji-as-you-
//  type-2026-10-18.md, "Where it runs"): the prose fields' text bindings,
//  wrapped.
//
//  A phone's keyboard sends no key events, so this judges by what the
//  field writes back. A change that is EXACTLY ONE SCALAR ADDED AT THE END
//  is a character typed, and goes through the rule — `EmojiTyping`, shared
//  with every client — with what the field held before as `before`. A
//  paste, a dictation, a suggestion or an autocorrection changes more than
//  that and is left alone; so is a character typed anywhere but the end,
//  because SwiftUI's field does not say where its caret is.
//
//  TAKING IT BACK: while the swap is live, a change that removes the last
//  character of what the field held — a Backspace right behind the emoji —
//  puts back what was typed instead. Any other change, or the field's text
//  changed from anywhere else (Send, an edit loaded), drops the swap.
//
//  ONE per view, holding one live swap: only one field has the keyboard,
//  and a change in another field drops it. `field` tells a view's fields
//  apart — a poll's question and each of its options.
//
//  The Mac's fields are AppKit's, with real keys and a real caret, and
//  MacCapitalKeys does this there; on the Mac `binding(_:field:)` hands the
//  binding back untouched, so the two never both act.
//

import SwiftUI

@MainActor
final class EmojiTypingField {

    private struct Live {
        let field: AnyHashable
        /// What the field held right after the swap.
        let text: String
        let swap: EmojiTyping.Swap
    }

    private var live: Live?

    /// The swap being written into the field, so the field's echo of it is
    /// taken as what it is rather than judged as a change of its own.
    private var writing: String?

    /// `text`, with the person's typing in `field` going through the rule.
    ///
    /// The field's own write is stored AS IT IS, and the swap follows on
    /// the next turn of the main actor, AS AN EDIT OF THE FIELD ITSELF —
    /// the focused UIKit text view's own `replace(_:withText:)`, which the
    /// field then writes back through this binding like any other change.
    ///
    /// Never by storing the emoji in the binding. Measured with the
    /// simulator's on-screen keyboard (2026-10-20): the field ignores a
    /// value stored from outside while the keyboard is typing into it — it
    /// kept showing ":)" over a draft of "Hi 😊", and the next key wrote
    /// "Hi :) " straight back. A unit test that calls `insertText` on the
    /// field never sees this; only real keys do. The binding is written only
    /// when no focused text view holds what the field wrote.
    func binding(_ text: Binding<String>, field: AnyHashable = 0) -> Binding<String> {
        #if os(iOS)
        return Binding(
            get: { text.wrappedValue },
            set: { (new: String, transaction: Transaction) in
                let decided = self.typed(new, over: text.wrappedValue, in: field)
                text.transaction(transaction).wrappedValue = new
                guard !decided.utf16.elementsEqual(new.utf16) else { return }
                Task { @MainActor in
                    // Only over what the field wrote: anything since — the
                    // next key, a Send — and the swap never happened.
                    guard text.wrappedValue.utf16.elementsEqual(new.utf16) else {
                        if self.live?.text.utf16.elementsEqual(decided.utf16) == true { self.live = nil }
                        return
                    }
                    self.writing = decided
                    _ = FocusedTextInput.edit(holding: new, into: decided)
                    // The field echoes its edit through the setter at once;
                    // without one — no text view, or one holding something
                    // else — the binding is the only way left.
                    if self.writing != nil {
                        self.writing = nil
                        text.wrappedValue = decided
                    }
                }
            })
        #else
        return text
        #endif
    }

    /// What the field should hold when it writes `new` over `old`.
    func typed(_ new: String, over old: String, in field: AnyHashable = 0) -> String {
        // A field writing back what it already holds is not a change (a
        // UIKit field writes every change twice).
        if new.utf16.elementsEqual(old.utf16) { return new }
        if let writing, new.utf16.elementsEqual(writing.utf16) {
            self.writing = nil
            return new
        }
        let pending = live
        live = nil
        let held = pending.flatMap { pending in
            pending.field == field && pending.text.utf16.elementsEqual(old.utf16) ? pending.swap : nil
        }

        // The last character removed, right behind the swap: take it back.
        if let held, !old.isEmpty, new.utf16.elementsEqual(old.dropLast().utf16),
           let undone = held.backspace(text: old, caret: old.utf16.count) {
            return undone.text
        }

        // One scalar typed at the end.
        let before = old.unicodeScalars
        let after = new.unicodeScalars
        guard let typed = after.last, after.count == before.count + 1, after.starts(with: before),
              let swap = EmojiTyping.replace(before: old, typed: typed, live: held)
        else { return new }
        let text = swap.apply(before: old, after: "").text
        live = Live(field: field, text: text, swap: swap)
        return text
    }
}

#if os(iOS)
/// The text view the keyboard is typing into, edited the way the keyboard
/// edits it.
@MainActor
private enum FocusedTextInput {

    private static weak var found: UIResponder?

    /// Turns `text` into `wanted` by replacing only what differs, if the
    /// first responder is a text input holding exactly `text`. False — and
    /// nothing touched — otherwise.
    static func edit(holding text: String, into wanted: String) -> Bool {
        found = nil
        UIApplication.shared.sendAction(#selector(UIResponder.emojiTypingFindFirstResponder(_:)), to: nil, from: nil, for: nil)
        guard let input = found as? (UIResponder & UITextInput) else { return false }
        found = nil
        let start = input.beginningOfDocument
        guard let whole = input.textRange(from: start, to: input.endOfDocument),
              input.text(in: whole)?.utf16.elementsEqual(text.utf16) == true
        else { return false }

        // UITextInput counts in UTF-16, and so does this — kept off the
        // middle of a surrogate pair, which 😊 and 😩 would otherwise share.
        let have = Array(text.utf16)
        let want = Array(wanted.utf16)
        var prefix = 0
        while prefix < min(have.count, want.count), have[prefix] == want[prefix] { prefix += 1 }
        if prefix > 0, prefix < have.count, UTF16.isTrailSurrogate(have[prefix]) { prefix -= 1 }
        var suffix = 0
        while suffix < min(have.count, want.count) - prefix,
              have[have.count - 1 - suffix] == want[want.count - 1 - suffix] { suffix += 1 }
        if suffix > 0, UTF16.isTrailSurrogate(have[have.count - suffix]) { suffix -= 1 }

        guard let from = input.position(from: start, offset: prefix),
              let to = input.position(from: start, offset: have.count - suffix),
              let range = input.textRange(from: from, to: to)
        else { return false }
        input.replace(range, withText: String(decoding: want[prefix..<(want.count - suffix)], as: UTF16.self))
        return true
    }

    fileprivate static func answer(_ responder: UIResponder) { found = responder }
}

extension UIResponder {
    /// Sent down the responder chain with no target, so the first responder
    /// answers it — UIKit's one public way to ask who that is.
    @objc fileprivate func emojiTypingFindFirstResponder(_ sender: Any?) {
        FocusedTextInput.answer(self)
    }
}
#endif
