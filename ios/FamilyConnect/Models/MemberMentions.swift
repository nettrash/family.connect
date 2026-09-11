//
//  MemberMentions.swift
//  FamilyConnect
//
//  Member mentions (docs/protocol.md, "Mentioning a member"): the wire
//  object, and the grammar every composer and every bubble share.
//
//  The GRAMMAR is the assistant token's rule applied to a name: `@` followed
//  by exactly the name, at a boundary on both sides — an ASCII letter, digit
//  or `_` after it means it is a longer word, so `@Ann` is not found inside
//  `@Anna` and `mail@Anna` is an address. The server checks the same thing
//  (server/src/mentions.rs `names_member`) and refuses a mention the body
//  does not carry, so the two must agree or a message is refused for a
//  name the sender can see in front of them.
//
//  The RESOLUTION is from the text, at send: every active member whose
//  `@Name` the body carries, each member once, in order of first
//  appearance. From the text rather than from what the picker inserted, so
//  a name typed by hand mentions too, a name deleted after picking does
//  not, and a draft parked across screens loses nothing. Names are tried
//  LONGEST FIRST and a token once claimed is not offered again, so
//  `@Anna Lee` names Anna Lee and not also Anna: the boundary after `@Anna`
//  is the space, and without the claim both would be named and both
//  notified.
//
//  Android counterpart: util/MemberMention.kt.
//

import Foundation
import SwiftUI

/// A member a message names: the id, and the display name AS TYPED after
/// the `@`, so a bubble can find the token to highlight without knowing
/// what the member is called today. One shape for the wire, the store and
/// the views.
nonisolated struct MentionDTO: Codable, Equatable, Hashable, Sendable {
    let userID: Int64
    let name: String

    enum CodingKeys: String, CodingKey {
        case userID = "user_id"
        case name
    }

    init(userID: Int64, name: String) {
        self.userID = userID
        self.name = name
    }
}

nonisolated enum MemberMentions {
    /// The private scheme a highlighted name carries as its `.link`, so a
    /// tap on it reaches the member through the bubble's own link
    /// arbitration rather than a browser. Never on the wire.
    static let scheme = "fcmember"

    static func url(for userID: Int64) -> URL? {
        URL(string: "\(scheme)://\(userID)")
    }

    static func userID(from url: URL) -> Int64? {
        guard url.scheme == scheme, let host = url.host else { return nil }
        return Int64(host)
    }

    /// Every `@name` in `body`, the `@` included, as ranges into that same
    /// string — the whole name, at a boundary on both sides.
    static func ranges(of name: String, in body: String) -> [Range<String.Index>] {
        guard !name.isEmpty else { return [] }
        let utf8 = body.utf8
        let bytes = Array(utf8)
        let token = Array(name.utf8)
        let length = token.count + 1
        guard bytes.count >= length else { return [] }
        var found: [Range<String.Index>] = []
        var index = 0
        while index + length <= bytes.count {
            if bytes[index] == UInt8(ascii: "@"),
               bytes[(index + 1)..<(index + length)].elementsEqual(token),
               index == 0 || AssistantMention.isBoundary(bytes[index - 1]),
               index + length == bytes.count || AssistantMention.isBoundary(bytes[index + length]),
               let lowerUTF8 = utf8.index(utf8.startIndex, offsetBy: index, limitedBy: utf8.endIndex),
               let upperUTF8 = utf8.index(utf8.startIndex, offsetBy: index + length, limitedBy: utf8.endIndex),
               // `@` is ASCII, and an ASCII byte can only begin a
               // Character, so the lower bound is always a real position.
               let lower = lowerUTF8.samePosition(in: body) {
                // The END may not be. A combining mark, a ZWJ sequence or a
                // variation selector right after the token attaches to the
                // token's last letter, so the byte after it sits INSIDE a
                // grapheme cluster. The server matched anyway — its rule is
                // bytes and nothing else — so rejecting the match here made
                // the same message a mention on the server and on Android
                // and plain text on Apple. The match stands; the highlight
                // takes the whole cluster, which is the only way it can be
                // drawn.
                found.append(lower..<clusterEnd(atOrAfter: upperUTF8, in: body))
                index += length
            } else {
                index += 1
            }
        }
        return found
    }

    /// The first Character boundary at or after `index` — the end of the
    /// grapheme cluster it falls inside, when it falls inside one.
    private static func clusterEnd(atOrAfter index: String.Index, in body: String) -> String.Index {
        if let exact = index.samePosition(in: body) { return exact }
        var probe = body.startIndex
        while probe < body.endIndex {
            let next = body.index(after: probe)
            if next > index { return next }
            probe = next
        }
        return body.endIndex
    }

    /// Does `body` name this member — say `@` followed by exactly `name`?
    static func names(_ body: String, _ name: String) -> Bool {
        !ranges(of: name, in: body).isEmpty
    }

    /// The members `body` names, resolved against the roster — see the
    /// header. Empty when it names nobody.
    static func resolve(body: String, roster: [MentionDTO]) -> [MentionDTO] {
        guard body.contains("@") else { return [] }
        var seen = Set<Int64>()
        var claimed: [Range<String.Index>] = []
        var found: [(offset: Int, member: MentionDTO)] = []
        for member in roster.sorted(by: {
            // Longest name first, then the LOWER id — a family may hold two
            // members called Anna, and one `@Anna` can only name one of
            // them. Roster order would name a different Anna on each
            // platform; the id is the one tie-break all three ports share
            // (docs/protocol.md, "Mentioning a member").
            $0.name.utf8.count != $1.name.utf8.count
                ? $0.name.utf8.count > $1.name.utf8.count
                : $0.userID < $1.userID
        }) {
            guard !seen.contains(member.userID),
                  let first = ranges(of: member.name, in: body)
                      .first(where: { range in !claimed.contains { $0.overlaps(range) } })
            else { continue }
            seen.insert(member.userID)
            claimed.append(first)
            found.append((body.utf8.distance(from: body.utf8.startIndex, to: first.lowerBound), member))
        }
        return found.sorted { $0.offset < $1.offset }.map(\.member)
    }

    /// Every `@Name` token the message draws, one owner per token: the
    /// members' tokens, longest name first, a token claimed once — the same
    /// rule `resolve` named them by, so the bubble marks `@Anna Lee` as Anna
    /// Lee even when the message names Anna as well.
    static func tokens(in text: String, mentions: [MentionDTO]) -> [(range: Range<String.Index>, member: MentionDTO)] {
        guard !mentions.isEmpty else { return [] }
        var claimed: [Range<String.Index>] = []
        var found: [(range: Range<String.Index>, member: MentionDTO)] = []
        for mention in mentions.sorted(by: {
            $0.name.utf8.count != $1.name.utf8.count
                ? $0.name.utf8.count > $1.name.utf8.count
                : $0.userID < $1.userID
        }) {
            for range in ranges(of: mention.name, in: text) where !claimed.contains(where: { $0.overlaps(range) }) {
                claimed.append(range)
                found.append((range, mention))
            }
        }
        return found.sorted { $0.range.lowerBound < $1.range.lowerBound }
    }

    /// The prefix being typed after a trailing `@`, or nil when the composer
    /// is not mid-mention: no `@` at a boundary, or a line break after it.
    /// Empty when the `@` was just typed — every candidate is offered then.
    static func query(in draft: String) -> String? {
        guard let at = draft.lastIndex(of: "@") else { return nil }
        if at > draft.startIndex {
            let previous = draft.utf8[draft.utf8.index(before: at)]
            guard AssistantMention.isBoundary(previous) else { return nil }
        }
        let tail = draft[draft.index(after: at)...]
        guard !tail.contains("\n") else { return nil }
        return String(tail)
    }

    /// The roster narrowed to what `query` could be the start of — the
    /// reader themself, the blocked and anyone `excluding` names left out —
    /// in roster order.
    static func candidates(
        in roster: [MentionDTO], matching query: String, excluding: Set<Int64>
    ) -> [MentionDTO] {
        let needle = query.lowercased()
        return roster.filter { member in
            !excluding.contains(member.userID)
                && (needle.isEmpty || member.name.lowercased().hasPrefix(needle))
        }
    }

    /// The names in a note that CANNOT be doors: every member it says who
    /// is not somebody this reader could open a chat with — their own name,
    /// a member they blocked, a member who has left or deleted their
    /// account (docs/protocol.md, "Board").
    ///
    /// `openTo` is whoever a composer would offer for a bare `@`, which is
    /// the same question asked from the other side — so the two can never
    /// disagree about which names are live.
    static func closedNames(in mentions: [MentionDTO], openTo: [MentionDTO]) -> Set<Int64> {
        let open = Set(openTo.map(\.userID))
        return Set(mentions.map(\.userID)).subtracting(open)
    }

    /// The draft with the trailing `@prefix` replaced by `@Name `.
    static func accept(draft: String, name: String) -> String {
        guard let at = draft.lastIndex(of: "@") else { return draft + "@" + name + " " }
        return String(draft[..<at]) + "@" + name + " "
    }
}
extension MemberMentions {

    /// A board note's text with the names it says drawn as names
    /// (docs/protocol.md, "Board").
    ///
    /// BOLD, in the note's own ink, and never a colour of its own — a
    /// sticker's pastel is a ground like any other, and a tint on it is the
    /// mention that cannot be read. `linking` decides whether each name is
    /// also a door: it is in the note somebody has OPENED, and it is not on
    /// the sticker, whose whole face is a drag handle.
    static func noteText(
        _ text: String,
        mentions: [MentionDTO],
        linking: Bool,
        excluding: Set<Int64> = []
    ) -> AttributedString {
        var attributed = AttributedString(text)
        guard !mentions.isEmpty else { return attributed }
        for (range, member) in tokens(in: text, mentions: mentions) {
            guard let target = Range(range, in: attributed) else { continue }
            attributed[target].inlinePresentationIntent = .stronglyEmphasized
            if linking, !excluding.contains(member.userID),
               let url = url(for: member.userID) {
                attributed[target].link = url
                // A link run is drawn in the accent colour unless somebody
                // says otherwise, and the accent is a pastel's enemy: the
                // ink is the note's own (docs/protocol.md, "Mentioning a
                // member").
                attributed[target].foregroundColor = .black.opacity(0.85)
                attributed[target].underlineStyle = nil
            }
        }
        return attributed
    }
}
