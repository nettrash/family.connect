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

    /// `text`, with the person's typing in `field` going through the rule.
    ///
    /// The field's own write is stored AS IT IS, and the swap follows on
    /// the next turn of the main actor. Measured (EmojiTypingTests, a real
    /// UIKit field): a value stored in place of the one the field is
    /// writing never reaches the field — it is mid-edit, keeps its own
    /// text, and writes that straight back over the emoji. A write a turn
    /// later is an ordinary change from outside, which the field shows.
    func binding(_ text: Binding<String>, field: AnyHashable = 0) -> Binding<String> {
        #if os(iOS)
        Binding(
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
                    text.wrappedValue = decided
                }
            })
        #else
        text
        #endif
    }

    /// What the field should hold when it writes `new` over `old`.
    func typed(_ new: String, over old: String, in field: AnyHashable = 0) -> String {
        // A field writing back what it already holds is not a change (a
        // UIKit field writes every change twice).
        if new.utf16.elementsEqual(old.utf16) { return new }
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
