//
//  MemberMentionTests.swift
//  FamilyConnectTests
//
//  The member-mention grammar (docs/protocol.md, "Mentioning a member"),
//  pinned by value against the server's `names_member` and Android's
//  MemberMention: the same bodies, the same names, the same answers.
//

import Foundation
import Testing
@testable import FamilyConnect

struct MemberMentionTests {

    @Test("a member is named only by the whole name at a boundary")
    func grammar() {
        #expect(MemberMentions.names("@Anna are you in?", "Anna"))
        #expect(MemberMentions.names("hey @Anna, dinner?", "Anna"))
        #expect(MemberMentions.names("(@Anna)", "Anna"))
        #expect(MemberMentions.names("@Uncle Bob is here", "Uncle Bob"))
        #expect(MemberMentions.names("@Анна привет", "Анна"))
        #expect(MemberMentions.names("@Анна", "Анна"))
        // Not the whole name, or not at a boundary.
        #expect(!MemberMentions.names("@Annabel", "Anna"))
        #expect(!MemberMentions.names("@Ann", "Anna"))
        #expect(!MemberMentions.names("@anna", "Anna"), "case is the sender's")
        #expect(!MemberMentions.names("mail@Anna", "Anna"))
        #expect(!MemberMentions.names("Anna", "Anna"), "no @")
        #expect(!MemberMentions.names("@Anna", ""))
        // The second @ is the one: the first is a longer word.
        #expect(MemberMentions.names("@Annabel and @Anna", "Anna"))
        let body = "@Anna and @Anna"
        #expect(MemberMentions.ranges(of: "Anna", in: body).map { String(body[$0]) } == ["@Anna", "@Anna"])
    }

    @Test("resolution is longest name first, each member once, in order of appearance")
    func resolve() {
        let roster = [
            MentionDTO(userID: 1, name: "Bob"),
            MentionDTO(userID: 2, name: "Uncle Bob"),
            MentionDTO(userID: 3, name: "Anna"),
        ]
        #expect(MemberMentions.resolve(body: "@Anna and @Uncle Bob: 7?", roster: roster) == [roster[2], roster[1]])
        #expect(MemberMentions.resolve(body: "@Uncle Bob and @Anna", roster: roster) == [roster[1], roster[2]])
        #expect(MemberMentions.resolve(body: "@Bob @Bob", roster: roster) == [roster[0]])
        #expect(MemberMentions.resolve(body: "no one here", roster: roster).isEmpty)
        #expect(MemberMentions.resolve(body: "@Bobby", roster: roster).isEmpty)
    }

    /// The grammar is the SERVER's, byte for byte — including where a
    /// combining mark, a ZWJ sequence or a variation selector follows the
    /// token. Those bytes are boundaries to the server and to Android, and
    /// the position after the token then sits inside a grapheme cluster:
    /// refusing the match there made one message a mention everywhere but
    /// on Apple, where the bubble drew plain text while the named member's
    /// phone said "mentioned you".
    @Test("a mention the server accepts is a mention here, mid-cluster or not")
    func graphemeBoundariesDoNotChangeTheGrammar() {
        let body = "@Anna\u{0301} are you in?"
        #expect(MemberMentions.names(body, "Anna"))
        let range = try! #require(MemberMentions.ranges(of: "Anna", in: body).first)
        #expect(
            String(body[range]) == "@Anna\u{0301}",
            "the highlight takes the whole cluster: the mark belongs to the token's last letter")
        #expect(MemberMentions.names("@Anna\u{200D}\u{1F469} hi", "Anna"))
        #expect(MemberMentions.names("@Anna\u{FE0F}", "Anna"))
        // And the ordinary vectors are untouched.
        #expect(MemberMentions.names("@Anna, dinner?", "Anna"))
        #expect(!MemberMentions.names("@Annabel", "Anna"))
        // The boundary class is ASCII alphanumerics and `_`, and NOTHING
        // else — a Unicode-aware "is this a letter" would call Ж a letter
        // and end the token differently from the server, which reads bytes.
        #expect(MemberMentions.names("@AnnaЖ", "Anna"))
        #expect(MemberMentions.names("@Anna文", "Anna"))
        #expect(!MemberMentions.names("@Annab", "Anna"))
        #expect(!MemberMentions.names("@Anna9", "Anna"))
        #expect(!MemberMentions.names("@Anna_", "Anna"))
    }

    /// Two members called Anna and one `@Anna`: the token can only name
    /// one, and it must be the SAME one on every platform. Roster order is
    /// not shared between the ports; the id is.
    @Test("equal names are broken by the lower id, not by roster order")
    func duplicateNamesResolveByID() {
        let annaHigh = MentionDTO(userID: 12, name: "Anna")
        let annaLow = MentionDTO(userID: 3, name: "Anna")
        #expect(MemberMentions.resolve(body: "@Anna?", roster: [annaHigh, annaLow]) == [annaLow])
        #expect(MemberMentions.resolve(body: "@Anna?", roster: [annaLow, annaHigh]) == [annaLow])
        let text = "@Anna?"
        let owners = MemberMentions.tokens(in: text, mentions: [annaHigh, annaLow]).map(\.member.userID)
        #expect(owners == [3], "and the bubble marks it for the same member")
    }

    /// A name that is the start of another's: `@Anna Lee` ends `@Anna` at a
    /// space, which is a boundary — only the claim keeps Anna out of it.
    @Test("a token claimed by the longer name is not the shorter one's too")
    func prefixNames() {
        let anna = MentionDTO(userID: 3, name: "Anna")
        let annaLee = MentionDTO(userID: 4, name: "Anna Lee")
        let roster = [anna, annaLee]
        #expect(MemberMentions.resolve(body: "@Anna Lee is here", roster: roster) == [annaLee])
        #expect(MemberMentions.resolve(body: "@Anna, is @Anna Lee coming?", roster: roster) == [anna, annaLee])
        #expect(MemberMentions.resolve(body: "@Anna Lee and @Anna", roster: roster) == [annaLee, anna])
        // And the bubble marks the tokens the same way, whatever order the
        // sender named them in.
        let text = "@Anna, is @Anna Lee coming?"
        let marked = MemberMentions.tokens(in: text, mentions: [anna, annaLee]).map { (String(text[$0.range]), $0.member.userID) }
        #expect(marked.map(\.0) == ["@Anna", "@Anna Lee"])
        #expect(marked.map(\.1) == [3, 4])
        // Through the renderer: the link on each token is its owner's.
        let body = MessageLinks.attributedBody(text, isMine: false, mentions: [anna, annaLee])
        let links = body.runs.compactMap { run -> (String, String)? in
            guard let link = run.link else { return nil }
            return (String(body[run.range].characters), link.absoluteString)
        }
        #expect(links.map(\.0) == ["@Anna", "@Anna Lee"], "got \(links)")
        #expect(links.map(\.1) == [MemberMentions.url(for: 3)!.absoluteString, MemberMentions.url(for: 4)!.absoluteString])
    }

    @Test("the query is the trailing @ token, and accepting rewrites it")
    func queryAndAccept() {
        #expect(MemberMentions.query(in: "hey @An") == "An")
        #expect(MemberMentions.query(in: "@") == "")
        #expect(MemberMentions.query(in: "hey @Uncle ") == "Uncle ")
        #expect(MemberMentions.query(in: "mail@x") == nil, "an address, not a mention")
        #expect(MemberMentions.query(in: "@Anna\nnext") == nil, "a line break ends the token")
        #expect(MemberMentions.query(in: "no at") == nil)
        #expect(MemberMentions.accept(draft: "hey @An", name: "Anna") == "hey @Anna ")
        #expect(MemberMentions.accept(draft: "@", name: "Uncle Bob") == "@Uncle Bob ")
    }

    @Test("candidates leave out the reader, the blocked and non-matches")
    func candidates() {
        let roster = [
            MentionDTO(userID: 7, name: "Me"),
            MentionDTO(userID: 8, name: "Anna"),
            MentionDTO(userID: 9, name: "Andy"),
            MentionDTO(userID: 10, name: "Bob"),
        ]
        #expect(MemberMentions.candidates(in: roster, matching: "an", excluding: [7, 9]) == [roster[1]])
        #expect(MemberMentions.candidates(in: roster, matching: "", excluding: [7]).count == 3)
        #expect(MemberMentions.candidates(in: roster, matching: "zz", excluding: []).isEmpty)
    }

    @Test("the private scheme round-trips a member id and nothing else")
    func scheme() throws {
        let url = try #require(MemberMentions.url(for: 42))
        #expect(MemberMentions.userID(from: url) == 42)
        #expect(MemberMentions.userID(from: URL(string: "https://example.com/42")!) == nil)
    }
}
