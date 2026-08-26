//
//  ComposerText.swift
//  FamilyConnect
//
//  The draft, and the one limit the wire puts on it.
//
//  A message body is 4000 characters (docs/protocol.md, "Limits") and until
//  this existed no client counted them. Pasting a wall of text filled the
//  field, looked as though it had worked, and then came back from
//  `POST /chats/{id}/messages` as `message_too_long` — at Send, minutes
//  later, with no hint of which part was too much. A ceiling has to be
//  enforced where the text ARRIVES, and said out loud when it bites.
//
//  Counted in UNICODE SCALARS, which is what the server counts
//  (`body.chars().count()` over a trimmed body) and NOT what Swift's
//  `String.count` counts: a waving-hand-with-skin-tone is one Character and
//  several scalars, so counting Characters would wave through bodies the
//  server then refuses — the exact failure this file exists to stop.
//  Untrimmed here, which is at most a few characters STRICTER than the
//  server; erring the other way is how a "you are within the limit" turns
//  into a rejected send.
//
//  Cutting, though, happens at CHARACTER boundaries. A cut at a scalar
//  boundary lands inside a grapheme cluster and leaves an emoji's debris in
//  the field — a skin tone with no hand, a flag with half its letters.
//
//  Appending rather than inserting at the caret is a decision, not a
//  shortcut: SwiftUI's TextField publishes no selection before iOS 18 /
//  macOS 15, the deployment targets are 17 / 14 and are not being raised,
//  and appending is already what every other door that writes into the
//  draft does (`insertAssistantMention` on both platforms).
//
//  Android counterpart: the composer's paste handling.
//

import Foundation

nonisolated enum ComposerText {

    /// docs/protocol.md, "Limits": message body, 4000 characters.
    ///
    /// A self-hosted server may be configured lower and does not advertise
    /// its limit — exactly like `MediaPrep.sizeLimit`. This is what the
    /// client holds itself to; a stricter server still answers
    /// `message_too_long` and the send fails with a message rather than
    /// silently.
    static let bodyLimit = 4000

    /// What happened when words were pasted into a draft.
    ///
    /// Three outcomes rather than a Bool because they are three different
    /// things to say: nothing, "the end was left out", and "there was no
    /// room at all". A paste that quietly drops half of what was on the
    /// clipboard is the failure mode this replaces.
    enum Paste: Equatable {
        /// All of it fit. The new draft.
        case appended(String)
        /// Some of it fit. The new draft, with the tail dropped.
        case truncated(String)
        /// None of it fit — the draft is already at the ceiling.
        case full
    }

    /// The length the server will measure.
    static func length(_ text: String) -> Int {
        text.unicodeScalars.count
    }

    /// Append pasted words to a draft, within the ceiling.
    static func appending(_ addition: String, to draft: String) -> Paste {
        let room = bodyLimit - length(draft)
        guard room > 0 else { return .full }
        if length(addition) <= room { return .appended(draft + addition) }
        let head = prefix(of: addition, scalars: room)
        // A single grapheme cluster wider than the room left has no
        // character at all that fits, and half of one is not a character.
        guard !head.isEmpty else { return .full }
        return .truncated(draft + head)
    }

    /// Cut a draft down to the ceiling; nil when it is already inside it.
    ///
    /// The backstop for the door that cannot be intercepted. Both systems'
    /// OWN paste — iOS's edit-menu Paste, the Mac field editor's ⌘V, a drag
    /// into the field — is handled inside the text field, and reaching it
    /// would mean replacing the composer with a UIViewRepresentable /
    /// NSViewRepresentable. What can always be seen is the draft it left
    /// behind, so that is where the ceiling is applied for those.
    static func clamping(_ draft: String) -> String? {
        guard length(draft) > bodyLimit else { return nil }
        return prefix(of: draft, scalars: bodyLimit)
    }

    /// The longest whole-Character prefix whose scalars fit in `room`.
    private static func prefix(of text: String, scalars room: Int) -> String {
        var kept = ""
        var used = 0
        for character in text {
            let width = character.unicodeScalars.count
            if used + width > room { break }
            kept.append(character)
            used += width
        }
        return kept
    }
}
