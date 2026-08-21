//
//  LinkPreview.swift
//  FamilyConnect
//
//  The card under a message's first web link: what it holds, and how it
//  is read out of a page's HTML.
//
//  Parsing is a deliberately small, tolerant scanner rather than a real
//  HTML parser — it only needs the handful of <meta> tags every site
//  publishes for exactly this purpose (Open Graph, with Twitter cards
//  and plain <title>/<meta description> as fallbacks), and it must
//  behave identically to the Kotlin transcription in
//  android/…/ui/chat/LinkPreview.kt. Mirrored tests on both platforms
//  are the spec; change them together.
//
//  Everything works on a Character ARRAY with integer indices, and case
//  folding is ASCII-only. That is not fussiness: matching against a
//  `lowercased()` COPY and slicing the original with the indices it
//  yields is a crash. Unicode lowercasing is not length-preserving
//  (İ → i̇, ẞ → ss, KELVIN K → k), so the two strings drift apart and the
//  original gets sliced at an index that is out of bounds or mid-
//  character — a hard trap in Swift, silent title corruption in Kotlin.
//  Tag and attribute names are ASCII by definition, so ASCII-only
//  folding is both sufficient and length-preserving by construction.
//
//  It reads the <head> only. Bodies are megabytes of markup that cannot
//  contain the tags we want, and scanning them would cost far more than
//  the fetch.
//
//  PRIVACY: building one of these means THIS device contacts the linked
//  site (see LinkPreviewLoader). That is the trade the feature makes,
//  and why Settings can switch it off.
//

import Foundation

nonisolated struct LinkPreview: Equatable, Sendable {
    /// The link this describes — the canonical cache key.
    let url: URL
    let title: String
    /// Publisher name (og:site_name), falling back to the host.
    let siteName: String
    let description: String?
    /// Absolute URL of the card image, when the page offers one.
    let imageURL: URL?
}

nonisolated enum LinkPreviewParser {

    /// Longest prefix of a page worth scanning: everything we read lives
    /// in <head>, and the loader caps the download anyway.
    static let scanLimit = 200_000

    /// Both limits count UNICODE SCALARS, on both platforms — clamping
    /// by Swift's grapheme count and Kotlin's UTF-16 length would cut
    /// the same page at two different places (and split a surrogate
    /// pair on the Kotlin side, leaving a tofu box in the card).
    static let maxTitleLength = 140
    static let maxDescriptionLength = 300

    /// Longest entity this decodes, `&` and `;` excluded — bounds the
    /// lookahead so a page full of stray ampersands stays linear.
    private static let maxEntityLength = 10

    /// Read a preview out of `html` for `pageURL`. Returns nil when the
    /// page offers no usable title — a card with no title is just a
    /// second copy of the URL.
    static func parse(html: String, pageURL: URL) -> LinkPreview? {
        let head = Array(html.prefix(scanLimit))
        let metas = metaTags(in: head)

        let title = firstNonEmpty(
            metas["og:title"],
            metas["twitter:title"],
            titleTag(in: head))
        guard let title else { return nil }

        let description = firstNonEmpty(
            metas["og:description"],
            metas["twitter:description"],
            metas["description"])

        let siteName = firstNonEmpty(metas["og:site_name"]) ?? displayHost(of: pageURL)

        let image = firstNonEmpty(metas["og:image"], metas["og:image:url"], metas["twitter:image"])
            .flatMap { absoluteURL($0, relativeTo: pageURL) }

        return LinkPreview(
            url: pageURL,
            title: clamp(title, to: maxTitleLength),
            siteName: siteName,
            description: description.map { clamp($0, to: maxDescriptionLength) },
            imageURL: image)
    }

    /// Convenience for callers holding a String.
    static func parse(html: String, pageURL: String) -> LinkPreview? {
        guard let url = URL(string: pageURL) else { return nil }
        return parse(html: html, pageURL: url)
    }

    /// The host as a card would show it, without a leading "www.".
    static func displayHost(of url: URL) -> String {
        let host = url.host() ?? url.absoluteString
        return host.hasPrefix("www.") ? String(host.dropFirst(4)) : host
    }

    // MARK: - Scanning

    /// Every <meta> tag's key → content, keys lowercased. A tag's key is
    /// its `property` (Open Graph's spelling) or its `name` (everything
    /// else); the FIRST occurrence of a key wins, matching how browsers
    /// treat duplicated tags.
    static func metaTags(in html: [Character]) -> [String: String] {
        var result: [String: String] = [:]
        for tag in tags(named: "meta", in: html) {
            let attributes = attributes(inTag: tag)
            guard let key = attributes["property"] ?? attributes["name"],
                  let content = attributes["content"] else { continue }
            let normalizedKey = asciiLowercased(key)
            if result[normalizedKey] == nil {
                result[normalizedKey] = collapseWhitespace(decodeEntities(content))
            }
        }
        return result
    }

    /// The text of the first <title> element.
    static func titleTag(in html: [Character]) -> String? {
        guard let open = index(of: "<title", in: html, from: 0),
              isTagNameBoundary(html, at: open + 6) else {
            // A tag whose name merely starts with "title" is not it;
            // keep looking is overkill — pages have one <title>.
            return nil
        }
        guard let contentStart = endOfTag(html, from: open + 6) else { return nil }
        guard let close = index(of: "</title", in: html, from: contentStart + 1) else { return nil }
        let raw = String(html[(contentStart + 1)..<close])
        let text = collapseWhitespace(decodeEntities(raw))
        return text.isEmpty ? nil : text
    }

    /// Bodies of every `<name …>` tag (the part after the name, before
    /// the closing `>`), as raw strings.
    private static func tags(named name: String, in html: [Character]) -> [String] {
        var tags: [String] = []
        let opening = Array("<" + name)
        var index = 0
        while index < html.count {
            guard let open = self.index(of: String(opening), in: html, from: index) else { break }
            let afterName = open + opening.count
            guard isTagNameBoundary(html, at: afterName) else {
                index = afterName
                continue
            }
            guard let end = endOfTag(html, from: afterName) else { break }
            tags.append(String(html[afterName..<end]))
            index = end + 1
        }
        return tags
    }

    /// True when the tag name really ends here — "<metadata" must not
    /// match "<meta".
    private static func isTagNameBoundary(_ html: [Character], at index: Int) -> Bool {
        guard index < html.count else { return true }
        let character = html[index]
        return !(character.isLetter || character.isNumber)
    }

    /// Offset of the `>` that closes a tag whose body starts at `start`,
    /// skipping any `>` inside a quoted attribute value (real pages put
    /// them in descriptions).
    private static func endOfTag(_ html: [Character], from start: Int) -> Int? {
        var index = start
        var quote: Character?
        while index < html.count {
            let character = html[index]
            if let open = quote {
                if character == open { quote = nil }
            } else if character == "\"" || character == "'" {
                quote = character
            } else if character == ">" {
                return index
            }
            index += 1
        }
        return nil
    }

    /// First offset at or after `from` where `needle` matches, compared
    /// ASCII-case-insensitively. `needle` must already be lowercase.
    private static func index(of needle: String, in html: [Character], from: Int) -> Int? {
        let pattern = Array(needle)
        guard !pattern.isEmpty, html.count >= pattern.count else { return nil }
        var start = max(0, from)
        while start <= html.count - pattern.count {
            var offset = 0
            while offset < pattern.count,
                  asciiLowercased(html[start + offset]) == pattern[offset] {
                offset += 1
            }
            if offset == pattern.count { return start }
            start += 1
        }
        return nil
    }

    /// Attributes of one tag body, names lowercased. Tolerates single
    /// quotes, double quotes and unquoted values, in any order.
    static func attributes(inTag tag: String) -> [String: String] {
        var result: [String: String] = [:]
        let characters = Array(tag)
        var index = 0

        func skipWhitespace() {
            while index < characters.count, characters[index].isWhitespace { index += 1 }
        }

        while index < characters.count {
            skipWhitespace()
            var name = ""
            while index < characters.count,
                  !characters[index].isWhitespace,
                  characters[index] != "=",
                  characters[index] != "/" {
                name.append(characters[index])
                index += 1
            }
            skipWhitespace()
            guard index < characters.count, characters[index] == "=" else {
                // Valueless attribute — skip a stray "/" and continue.
                if index < characters.count, characters[index] == "/" { index += 1 }
                if name.isEmpty, index < characters.count { index += 1 }
                continue
            }
            index += 1 // '='
            skipWhitespace()
            var value = ""
            if index < characters.count, characters[index] == "\"" || characters[index] == "'" {
                let quote = characters[index]
                index += 1
                while index < characters.count, characters[index] != quote {
                    value.append(characters[index])
                    index += 1
                }
                if index < characters.count { index += 1 } // closing quote
            } else {
                while index < characters.count, !characters[index].isWhitespace {
                    value.append(characters[index])
                    index += 1
                }
            }
            if !name.isEmpty {
                let key = asciiLowercased(name)
                if result[key] == nil { result[key] = value }
            }
        }
        return result
    }

    // MARK: - Text helpers

    /// ASCII-only lowercasing: length-preserving by construction, which
    /// is what keeps parallel indices valid (see the header).
    static func asciiLowercased(_ text: String) -> String {
        String(text.map(asciiLowercased))
    }

    private static func asciiLowercased(_ character: Character) -> Character {
        guard let ascii = character.asciiValue, ascii >= 65, ascii <= 90 else { return character }
        return Character(UnicodeScalar(ascii + 32))
    }

    /// The named and numeric HTML entities that actually show up in
    /// titles and descriptions. The lookahead for the terminating `;`
    /// is bounded, so a page of stray ampersands cannot make this
    /// quadratic.
    static func decodeEntities(_ text: String) -> String {
        guard text.contains("&") else { return text }
        let characters = Array(text)
        var out = ""
        out.reserveCapacity(characters.count)
        var index = 0
        while index < characters.count {
            guard characters[index] == "&" else {
                out.append(characters[index])
                index += 1
                continue
            }
            var semicolon: Int?
            var lookahead = index + 1
            let limit = min(characters.count, index + maxEntityLength + 2)
            while lookahead < limit {
                if characters[lookahead] == ";" {
                    semicolon = lookahead
                    break
                }
                lookahead += 1
            }
            guard let semicolon,
                  let replacement = replacement(forEntity: String(characters[(index + 1)..<semicolon])) else {
                out.append(characters[index])
                index += 1
                continue
            }
            out.append(replacement)
            index = semicolon + 1
        }
        return out
    }

    private static func replacement(forEntity entity: String) -> String? {
        switch asciiLowercased(entity) {
        case "amp": return "&"
        case "lt": return "<"
        case "gt": return ">"
        case "quot": return "\""
        case "apos", "#39": return "'"
        case "nbsp": return "\u{00A0}"
        case "hellip": return "…"
        case "mdash": return "—"
        case "ndash": return "–"
        case "rsquo", "#8217": return "’"
        case "lsquo": return "‘"
        case "ldquo": return "“"
        case "rdquo": return "”"
        default:
            break
        }
        guard entity.hasPrefix("#") else { return nil }
        let digits = entity.dropFirst()
        let value: UInt32?
        if digits.hasPrefix("x") || digits.hasPrefix("X") {
            value = UInt32(digits.dropFirst(), radix: 16)
        } else {
            value = UInt32(digits, radix: 10)
        }
        guard let value, let scalar = Unicode.Scalar(value) else { return nil }
        return String(Character(scalar))
    }

    /// Runs of whitespace (including the newlines inside a wrapped meta
    /// tag) collapsed to single spaces, then trimmed.
    static func collapseWhitespace(_ text: String) -> String {
        text.split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }

    private static func firstNonEmpty(_ candidates: String?...) -> String? {
        for candidate in candidates {
            if let candidate {
                let trimmed = candidate.trimmingCharacters(in: .whitespacesAndNewlines)
                if !trimmed.isEmpty { return trimmed }
            }
        }
        return nil
    }

    /// Clamped to `limit` UNICODE SCALARS — the one unit both platforms
    /// can count identically (see maxTitleLength).
    private static func clamp(_ text: String, to limit: Int) -> String {
        guard text.unicodeScalars.count > limit else { return text }
        let kept = String(String.UnicodeScalarView(text.unicodeScalars.prefix(limit)))
        return kept.trimmingCharacters(in: .whitespaces) + "…"
    }

    /// Absolute form of a possibly-relative URL found in the page,
    /// restricted to http(s) — a card must never point at file: or a
    /// custom scheme just because a page said so.
    static func absoluteURL(_ raw: String, relativeTo base: URL) -> URL? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        let resolved: URL?
        if trimmed.hasPrefix("//") {
            resolved = URL(string: (base.scheme ?? "https") + ":" + trimmed)
        } else {
            resolved = URL(string: trimmed, relativeTo: base)?.absoluteURL
        }
        guard let resolved, let scheme = resolved.scheme?.lowercased(),
              scheme == "http" || scheme == "https" else { return nil }
        return resolved
    }
}
