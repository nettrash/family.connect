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

    /// The message body with every detected link, email and phone
    /// number turned into a tappable, underlined .link run.
    static func attributedBody(_ text: String, isMine: Bool) -> AttributedString {
        let key = ((isMine ? "m|" : "t|") + text) as NSString
        if let boxed = cache.object(forKey: key) {
            return boxed.value
        }
        let built = build(text, isMine: isMine)
        cache.setObject(Box(built), forKey: key)
        return built
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
        guard let detector else { return AttributedString(text) }
        let mutable = NSMutableAttributedString(string: text)
        let fullRange = NSRange(location: 0, length: mutable.length)
        for match in detector.matches(in: text, range: fullRange) {
            guard let url = url(for: match) else { continue }
            mutable.addAttribute(.link, value: url, range: match.range)
        }
        var attributed = AttributedString(mutable)
        for run in attributed.runs where run.link != nil {
            attributed[run.range].underlineStyle = .single
            if isMine {
                attributed[run.range].foregroundColor = .white
            }
        }
        return attributed
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
