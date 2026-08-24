//
//  MessageLinks.swift
//  FamilyConnect
//
//  Tappable data in message bodies: web links open the browser, email
//  addresses open Mail, phone numbers offer a call. Detection is
//  NSDataDetector — the platform's own detector, so chat bubbles agree
//  with what the rest of the OS considers a link or a phone number —
//  and the result is an AttributedString whose .link runs SwiftUI Text
//  makes tappable natively. Taps route through the environment's
//  openURL, which MessageBubbleView overrides to arbitrate link taps
//  against the double-tap heart (Text's internal link tap preempts
//  view-level gestures over link glyphs).
//
//  Unlike EmojiOnly this is deliberately NOT a byte-identical
//  cross-platform scanner: URL and especially phone grammars are
//  platform-tuned (locales, carriers), so each app leans on its own
//  platform detector and the two agree on the CATEGORIES (web, email,
//  phone), not on every edge case. Android counterpart:
//  android/…/ui/chat/MessageLinks.kt (Linkify).
//
//  Styling: link runs are underlined; on own bubbles (white text on the
//  tint background) they are forced white — the default accent-colored
//  link would drown in the tint. Other bubbles keep the accent color.
//
//  Bubble bodies re-render on every ConversationView body evaluation
//  and detection is regex-grade work, so results are memoized in a
//  bounded NSCache keyed by (side, body).
//

import Foundation
import SwiftUI

nonisolated enum MessageLinks {

    /// A message body, ready to draw: markdown rendered, every detected
    /// link, email and phone number turned into a tappable underlined
    /// `.link` run, and `@ai` marked as a mention.
    ///
    /// All three in ONE attributed string, and one `Text` at the call site
    /// — see MessageMarkdown for why that is a constraint rather than a
    /// convenience.
    static func attributedBody(_ text: String, isMine: Bool) -> AttributedString {
        let key = ((isMine ? "m|" : "t|") + text) as NSString
        if let boxed = cache.object(forKey: key) {
            return boxed.value
        }
        let built = build(text, isMine: isMine)
        cache.setObject(Box(built), forKey: key)
        return built
    }

    /// The first https link in a body AS IT IS DRAWN — what the preview
    /// card describes.
    ///
    /// Rendered first, for the reason `build` gives: markdown deletes
    /// characters, so detecting over the raw body finds links in text the
    /// bubble never shows. `**https://a.example**` would have previewed a
    /// URL whose asterisks are not on screen, and `[label](url)` would
    /// preview nothing at all even though a link is plainly there.
    static func firstWebLinkAsDrawn(in text: String) -> URL? {
        firstWebLink(in: String(MessageMarkdown.render(text).characters))
    }

    /// The first https link in a body — what the preview card
    /// describes. Phone numbers and email addresses are detected too but
    /// have nothing to preview, previewing every link in a message would
    /// bury the message itself, and plain http is excluded on purpose
    /// (ATS blocks it, so a card would appear on Android and not here).
    ///
    /// Memoized for the same reason `attributedBody` is: this runs from
    /// a bubble's body, which re-evaluates for the whole window.
    static func firstWebLink(in text: String) -> URL? {
        let key = ("first|" + text) as NSString
        if let boxed = firstLinkCache.object(forKey: key) {
            return boxed.value
        }
        let found = detectFirstWebLink(in: text)
        firstLinkCache.setObject(URLBox(found), forKey: key)
        return found
    }

    private static func detectFirstWebLink(in text: String) -> URL? {
        guard let detector, text.contains(".") else { return nil }
        let range = NSRange(location: 0, length: (text as NSString).length)
        for match in detector.matches(in: text, range: range) where match.resultType == .link {
            guard let url = match.url else { continue }
            if url.scheme?.lowercased() == "https" { return url }
        }
        return nil
    }

    private final class URLBox {
        let value: URL?
        init(_ value: URL?) { self.value = value }
    }

    private static let firstLinkCache: NSCache<NSString, URLBox> = {
        let cache = NSCache<NSString, URLBox>()
        cache.countLimit = 512
        return cache
    }()

    // MARK: - Internals

    /// One detector for the process — building one compiles its
    /// patterns, which is far more expensive than matching.
    private static let detector = try? NSDataDetector(
        types: NSTextCheckingResult.CheckingType.link.rawValue
            | NSTextCheckingResult.CheckingType.phoneNumber.rawValue)

    private final class Box {
        let value: AttributedString
        init(_ value: AttributedString) { self.value = value }
    }

    private static let cache: NSCache<NSString, Box> = {
        let cache = NSCache<NSString, Box>()
        cache.countLimit = 512
        return cache
    }()

    private static func build(_ text: String, isMine: Bool) -> AttributedString {
        // MARKDOWN FIRST, and the order is load-bearing.
        //
        // The detector works in offsets, and markdown DELETES characters —
        // the `**`, the backticks, the `](url)`. Detecting over the raw
        // body and then rendering would leave every link after the first
        // markup token pointing at the wrong glyphs. Rendering first and
        // detecting over what is actually drawn makes the offsets correct
        // by construction, which is the same rule Android's hit test needs.
        var attributed = MessageMarkdown.render(text)
        let rendered = String(attributed.characters)

        // Markdown's own destinations, made openable.
        //
        // Foundation hands back exactly what was typed, so `[here](example.com)`
        // produces a scheme-less URL that opens nothing. Android normalises
        // the same way — without this the identical message is a live link
        // on one platform and a dead one on the other.
        for run in attributed.runs where run.link != nil {
            if let normalized = Self.normalized(run.link) {
                attributed[run.range].link = normalized
            }
        }

        if let detector {
            let fullRange = NSRange(location: 0, length: (rendered as NSString).length)
            for match in detector.matches(in: rendered, range: fullRange) {
                guard let url = url(for: match),
                    let range = attributedRange(match.range, in: attributed, of: rendered)
                else { continue }
                // NEVER over a link the markup already declared.
                //
                // `[https://www.paypal.com](https://evil.example)` renders
                // as that first URL's text, which the detector then matches
                // as a link to the place it NAMES — overwriting the
                // author's destination, or not, depending on order. Either
                // way a tap could open somewhere the reader had every
                // reason to think was what they were looking at. The
                // author's own destination is what the message declares, so
                // it stands and the detector's duplicate is dropped.
                guard attributed[range].runs.allSatisfy({ $0.link == nil }) else { continue }
                // Applied ONTO the rendered attributed string rather than
                // onto a fresh copy: a copy carries no emphasis, so taking
                // it wholesale would throw the markdown away.
                attributed[range].link = url
            }
        }

        for run in attributed.runs where run.link != nil {
            attributed[run.range].underlineStyle = .single
            if isMine {
                attributed[run.range].foregroundColor = .white
            }
        }
        highlightMentions(in: &attributed, isMine: isMine)
        return attributed
    }

    /// Mark `@ai` so it reads as addressed to somebody.
    ///
    /// Same grammar the SERVER decides by (AssistantMention, mirrored in
    /// three places): a highlight the server would not act on, or an
    /// unhighlighted token it would, is a family watching a question go
    /// unanswered with no way to tell why.
    ///
    /// Applied over the RENDERED text, after markdown, for the same reason
    /// the detector is — and it is only a style, so it never fights the
    /// link runs above for a tap.
    private static func highlightMentions(in attributed: inout AttributedString, isMine: Bool) {
        let rendered = String(attributed.characters)
        for range in AssistantMention.ranges(in: rendered) {
            let nsRange = NSRange(range, in: rendered)
            guard let target = attributedRange(nsRange, in: attributed, of: rendered) else {
                continue
            }
            attributed[target].inlinePresentationIntent = .stronglyEmphasized
            if !isMine {
                attributed[target].foregroundColor = .accentColor
            }
        }
    }

    /// A markdown destination that can actually be opened, or nil when it
    /// already could be.
    ///
    /// Only a scheme is added, and only when there is none: anything the
    /// author wrote in full is left exactly as written. `https` rather than
    /// `http`, matching Android's `normalize`.
    private static func normalized(_ url: URL?) -> URL? {
        guard let url else { return nil }
        guard url.scheme == nil else { return nil }
        return URL(string: "https://\(url.absoluteString)")
    }

    /// Map a range of the rendered PLAIN text onto the attributed string it
    /// came from.
    ///
    /// `AttributedString` has no initialiser taking a `String.Index` range,
    /// and its own indices are not interchangeable with a `String`'s — so
    /// the bridge is character OFFSETS, which both count the same way
    /// (grapheme clusters). Everything here indexes the rendered text, so
    /// there is no raw-versus-rendered mismatch left to get wrong.
    private static func attributedRange(
        _ nsRange: NSRange, in attributed: AttributedString, of rendered: String
    ) -> Range<AttributedString.Index>? {
        guard let stringRange = Range(nsRange, in: rendered) else { return nil }
        let start = rendered.distance(from: rendered.startIndex, to: stringRange.lowerBound)
        let length = rendered.distance(from: stringRange.lowerBound, to: stringRange.upperBound)
        let characters = attributed.characters
        guard start >= 0, length >= 0, start + length <= characters.count else { return nil }
        let lower = characters.index(characters.startIndex, offsetBy: start)
        let upper = characters.index(lower, offsetBy: length)
        return lower..<upper
    }

    /// The URL a match should open. Web links and emails come back from
    /// the detector already carrying their scheme (http/https, mailto);
    /// phone numbers go through telURL so the dial string is actually
    /// dialable — tapping one offers the call.
    private static func url(for match: NSTextCheckingResult) -> URL? {
        switch match.resultType {
        case .link:
            return match.url
        case .phoneNumber:
            guard let phone = match.phoneNumber else { return nil }
            return telURL(for: phone)
        default:
            return nil
        }
    }

    /// tel: URL for a detector phone match. Two traps a naive
    /// digits-only filter falls into: the detector keeps extensions,
    /// normalized behind a ';' ("555-123-4567 x89" → "555-123-4567;89"),
    /// and concatenating those digits dials a wrong, longer number — so
    /// the extension rides separately in RFC 3966 ;ext= form. And
    /// vanity letters (1-800-GOT-JUNK) are part of the number — they
    /// map to their keypad digits instead of being stripped.
    private static func telURL(for phone: String) -> URL? {
        let parts = phone.split(separator: ";", maxSplits: 1, omittingEmptySubsequences: true)
        guard let main = parts.first else { return nil }
        let number = dialableDigits(String(main))
        guard !number.isEmpty else { return nil }
        var tel = "tel:" + number
        if parts.count > 1 {
            let ext = parts[1].filter(\.isNumber)
            if !ext.isEmpty {
                tel += ";ext=" + ext
            }
        }
        return URL(string: tel)
    }

    /// Keypad form of one side of a phone match: digits, + and * pass
    /// through, vanity letters become their keypad digit, and '#' is
    /// percent-encoded — bare, URL(string:) would read it as a fragment
    /// delimiter and silently truncate the dial string.
    private static func dialableDigits(_ text: String) -> String {
        var out = ""
        for character in text.uppercased() {
            switch character {
            case "0"..."9", "+", "*": out.append(character)
            case "#": out.append("%23")
            case "A", "B", "C": out.append("2")
            case "D", "E", "F": out.append("3")
            case "G", "H", "I": out.append("4")
            case "J", "K", "L": out.append("5")
            case "M", "N", "O": out.append("6")
            case "P", "Q", "R", "S": out.append("7")
            case "T", "U", "V": out.append("8")
            case "W", "X", "Y", "Z": out.append("9")
            default: break
            }
        }
        return out
    }
}
