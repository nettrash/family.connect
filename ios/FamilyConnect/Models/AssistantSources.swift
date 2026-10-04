//
//  AssistantSources.swift
//  FamilyConnect
//
//  The "Sources" footer the SERVER appends to an assistant answer that
//  looked something up (docs/protocol.md, "Looking things up" — "How
//  sources are shown"):
//
//      <answer>
//
//      Sources: [Title 1](url1) · [Title 2](url2) · [Title 3](url3)
//      [Weather data by Open-Meteo.com](https://open-meteo.com/) · Wikipedia, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) · Powered by Brave
//
//  It is plain markdown, so the bubble already draws it with tappable
//  links and nothing here changes how it LOOKS. What this decides is the
//  one thing that must differ: a reply carrying it gets NO link-preview
//  card (decision 7 of docs/information-streams-2026-10-03.md). Without
//  that, every Apple device showing the answer would contact the first
//  cited site to build a card — a fetch nobody asked for, made because the
//  assistant cited it.
//
//  The WHOLE reply, not only the footer's links, because the server leaves
//  nothing else linkable in such a reply: a markdown link the model wrote
//  to anything that was not a returned source becomes its plain label, and
//  a bare URL or domain is defused to `example[.]com`. Every link that
//  survives in it is a source or a provider's credit — exactly what is not
//  to be previewed — and that holds for a source cited inline that the
//  footer's three did not include.
//
//  Recognised by SHAPE, from the body alone, and deliberately not by who
//  sent it: the same answer quoted, copied or forwarded is still a list of
//  sources, and the rule cannot fail open on a client that has not yet
//  learned the assistant's account id. A member who types a footer of
//  their own only takes a card away from their own message.
//
//  The "Sources" word is the server's, in the nine languages it writes it
//  in (`FooterWords` in server/src/lookups.rs); a credit line alone — the
//  weather-only answer has no links to list — is recognised by the
//  providers' own fixed links.
//

import Foundation

nonisolated enum AssistantSources {

    /// "Sources" as the server writes it, in each language it writes it in.
    /// A footer in a language added to the server later is still caught
    /// whenever it carries a credit line.
    static let sourcesLabels: Set<String> = [
        "Sources", "Quellen", "Fuentes", "出典", "Источники", "Извори", "Izvori", "来源",
    ]

    /// Does this body end in the server's sources footer?
    static func hasFooter(_ body: String) -> Bool {
        footerLineCount(body) > 0
    }

    // MARK: - Internals

    private static func trimmedLines(_ body: String) -> [String] {
        let trimmed = body.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.split(separator: "\n", omittingEmptySubsequences: false)
            .map { String($0).trimmingCharacters(in: CharacterSet(charactersIn: "\r")) }
    }

    /// 0, 1 or 2: how many trailing lines are the footer. The footer is a
    /// sources line, a credit line, or a sources line followed by a credit
    /// line, and it is always separated from the answer by a blank line —
    /// the server appends "\n\n" — so a credit-shaped sentence in the middle
    /// of somebody's words is not one.
    private static func footerLineCount(_ body: String) -> Int {
        let lines = trimmedLines(body)
        guard lines.count >= 3 else { return 0 }
        let last = lines[lines.count - 1]
        let previous = lines[lines.count - 2]
        if isCreditLine(last) {
            if isSourcesLine(previous), lines.count >= 4, isBlank(lines[lines.count - 3]) {
                return 2
            }
            return isBlank(previous) ? 1 : 0
        }
        if isSourcesLine(last), isBlank(previous) {
            return 1
        }
        return 0
    }

    private static func isBlank(_ line: String) -> Bool {
        line.trimmingCharacters(in: .whitespaces).isEmpty
    }

    /// `[title](https://…)` — a footer link as the server writes it: a
    /// title without the characters that would end a markdown label, and
    /// an http(s) destination with its spaces, parentheses, angle brackets
    /// and quotes percent-encoded.
    private static let link = #"\[[^\[\]\\`\n]*\]\(https?://[^\s()<>"]+\)"#

    private static let sourcesLinks = try! NSRegularExpression(
        pattern: "^" + link + "(?: · " + link + ")*$")

    /// `Sources: [a](…) · [b](…)`, with the label in one of the server's
    /// languages.
    static func isSourcesLine(_ line: String) -> Bool {
        guard let colon = line.range(of: ": ") else { return false }
        let label = String(line[..<colon.lowerBound])
        guard sourcesLabels.contains(label) else { return false }
        let links = String(line[colon.upperBound...])
        return matches(sourcesLinks, links)
    }

    private static let creditItems: [NSRegularExpression] = [
        // "[Weather data by Open-Meteo.com](https://open-meteo.com/)", in
        // any of the server's languages — the destination is the fixed part.
        #"^\[[^\[\]\n]*Open-Meteo\.com\]\(https://open-meteo\.com/\)$"#,
        // "Wikipedia, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)".
        #"^[^\[\]\n,]+, \[CC BY-SA 4\.0\]\(https://creativecommons\.org/licenses/by-sa/4\.0/\)$"#,
        // Brave's own words, never translated.
        #"^Powered by Brave$"#,
    ].map { try! NSRegularExpression(pattern: $0) }

    /// A line made only of the providers' credits, joined by " · ".
    static func isCreditLine(_ line: String) -> Bool {
        let items = line.components(separatedBy: " · ")
        guard !items.isEmpty else { return false }
        return items.allSatisfy { item in creditItems.contains { matches($0, item) } }
    }

    private static func matches(_ expression: NSRegularExpression, _ text: String) -> Bool {
        let range = NSRange(text.startIndex..., in: text)
        return expression.firstMatch(in: text, range: range) != nil
    }
}
