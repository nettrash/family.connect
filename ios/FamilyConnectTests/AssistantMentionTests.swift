//
//  AssistantMentionTests.swift
//  FamilyConnectTests
//
//  Pins the `@ai` grammar. It is a cross-platform contract — the SERVER
//  decides from the same rule whether a family-chat message reaches the
//  assistant at all (server/src/mentions.rs) and Android draws the same
//  highlight (AssistantMention.kt) — so the vectors here ARE the spec and
//  the same table appears in all three places. A disagreement between them
//  is not a cosmetic bug: it is either a highlighted question that is never
//  answered, or an ordinary family message that quietly leaves the server.
//

import Foundation
import Testing

@testable import FamilyConnect

@Suite("Mentioning the assistant")
struct AssistantMentionTests {

    /// The shared table. `server/src/mentions.rs` and Android's
    /// `AssistantMentionTest.kt` carry these same strings — and the
    /// server's `the_three_ports_carry_the_same_vectors` READS THIS FILE
    /// and fails when they stop matching, so a vector added to one port and
    /// not the others is a red build rather than something a reader has to
    /// notice. Editing these tables by hand stays fine; renaming or
    /// reformatting them means teaching that test the new shape.
    static let mentions = [
        "@ai",
        "@AI",
        "@Ai",
        "@ai what is the weather",
        "hey @ai what is the weather",
        "hey @ai, what is the weather",
        "ask @ai.",
        "(@ai)",
        "\n@ai\n",
        "@@ai",
        "какая погода @ai",
        // No space after the token, which is normal in Japanese — the
        // ASCII-only boundary is what makes this a mention.
        "@aiこんにちは",
        "-@ai",
        "@ai?",
        "1+@ai",
    ]

    static let notMentions = [
        "",
        "@",
        "@a",
        "@aiden",
        "@ai_bot",
        "@ai2",
        "@aI3",
        "anna@ai.example",
        "x@ai",
        "1@ai",
        "_@ai",
        "ai",
        "email me at bob@aim.com",
        "@artificial intelligence",
    ]

    @Test("every shared vector that is a mention")
    func mentionVectors() {
        for body in Self.mentions {
            #expect(AssistantMention.mentions(body), "should be a mention: \(body)")
        }
    }

    @Test("every shared vector that is not")
    func nonMentionVectors() {
        for body in Self.notMentions {
            #expect(!AssistantMention.mentions(body), "should NOT be a mention: \(body)")
        }
    }

    @Test("the range covers the token and nothing else")
    func rangeCoversTheToken() {
        let body = "hey @ai there"
        let ranges = AssistantMention.ranges(in: body)
        #expect(ranges.count == 1)
        #expect(body[ranges[0]] == "@ai")
    }

    /// A body with multi-byte characters before the token is where a
    /// byte-vs-character mix-up would show, by slicing in the wrong place
    /// or trapping outright.
    @Test("a mention after non-ASCII text is sliced cleanly")
    func multibytePrefix() {
        for body in ["Привет @ai", "こんにちは @ai です", "🇷🇸 @ai"] {
            let ranges = AssistantMention.ranges(in: body)
            #expect(ranges.count == 1, "one mention in: \(body)")
            #expect(body[ranges[0]] == "@ai", "sliced the token in: \(body)")
        }
    }

    /// Every one is highlighted: a bubble that marked the first and left a
    /// second plain would read as a typo.
    @Test("all of them, not just the first")
    func everyMention() {
        let body = "@ai and also @AI, but not @aiden"
        let ranges = AssistantMention.ranges(in: body)
        #expect(ranges.count == 2)
        #expect(ranges.map { String(body[$0]) } == ["@ai", "@AI"])
    }

    /// The near miss must not stop the scan; `@@ai` must still be found by
    /// the second `@`.
    @Test("a near miss does not hide a real one")
    func scanContinuesPastANearMiss() {
        #expect(AssistantMention.mentions("@aiden asked @ai"))
        let ranges = AssistantMention.ranges(in: "@@ai")
        #expect(ranges.count == 1)
        #expect("@@ai"[ranges[0]] == "@ai")
    }

    // MARK: - `/draw` (docs/protocol.md, "Pictures")

    /// The picture vectors, mirrored exactly the way the mention ones are:
    /// `server/src/mentions.rs` (`DRAWS`) and Android's
    /// `AssistantMentionTest.kt` carry this same table, prompt and all.
    ///
    /// The PROMPT is asserted rather than a bool, and that is the point:
    /// what comes back is the whole of what will leave the server on this
    /// request, so a test that only asked "is this a draw?" would not
    /// notice the token travelling along with the words.
    static let draws = [
        ("/draw a cat", "a cat"),
        ("/DRAW a cat", "a cat"),
        ("/Draw a cat", "a cat"),
        ("   /draw a cat  ", "a cat"),
        ("/draw\na cat", "a cat"),
        // The family chat: one leading mention, and the words after the
        // token are still the whole of what leaves.
        ("@ai /draw a cat", "a cat"),
        ("@AI    /draw a cat in a hat", "a cat in a hat"),
        ("@ai\n/draw a cat", "a cat"),
        // A prompt that itself contains the tokens is still just words.
        ("/draw @ai holding a sign", "@ai holding a sign"),
        ("/draw /draw", "/draw"),
        // The PROMPT in the scripts this app is translated into.
        ("/draw \u{43A}\u{43E}\u{442} \u{432} \u{448}\u{43B}\u{44F}\u{43F}\u{435}", "\u{43A}\u{43E}\u{442} \u{432} \u{448}\u{43B}\u{44F}\u{43F}\u{435}"),
        ("/draw \u{732B}", "\u{732B}"),
        ("@ai /draw \u{1F408} on a mat", "\u{1F408} on a mat"),
        // WHITESPACE IS UNICODE `White_Space`, and these two code points
        // are exactly where the three ports' own libraries disagree about
        // it — which is why `AssistantMention` hand-rolls the predicate
        // rather than asking Foundation.
        //
        // U+0085 NEXT LINE **is** whitespace here and on the server. (Java
        // says it is not, which is what Android had to hand-roll around.)
        ("/draw\u{85}a cat", "a cat"),
        // U+200B ZERO WIDTH SPACE is **not** whitespace, so it stays in
        // the prompt — and a body that is nothing but the token and one of
        // them IS a picture request. `CharacterSet.whitespacesAndNewlines`
        // contains U+200B, so trimming with Foundation's set used to take
        // a character out of the prompt the server keeps and read the
        // second of these as an empty prompt the server acts on.
        ("/draw \u{200B}cat", "\u{200B}cat"),
        ("/draw \u{200B}", "\u{200B}"),
    ]

    static let notDraws = [
        "",
        "/draw",
        "  /draw  ",
        "/drawer",
        "/draws a cat",
        "/draw,a cat",
        "draw a cat",
        // The token has to be FIRST. This is the rule that keeps a family
        // DISCUSSING the feature from generating pictures of it.
        "hey @ai /draw a cat",
        "what does /draw do?",
        "please /draw a cat",
        // A mention that is not at the start does not license the token
        // either, for the same reason.
        "look @ai /draw a cat",
        // Two mentions: the second is not a leading one.
        "@ai @ai /draw a cat",
        // NON-ASCII BODIES. On the server these are the shapes that used
        // to bring the request down: `str::split_at` traps when its index
        // is not a character boundary, and the fifth BYTE of a body that
        // opens in Cyrillic, Japanese or Chinese usually is not one. Swift
        // cannot trap here, but it can slice in the wrong place just as
        // easily, so the same table is pinned on all three ports.
        "\u{41F}\u{440}\u{438}\u{432}\u{435}\u{442}",
        "\u{41F}\u{440}\u{438}\u{432}\u{435}\u{442}, \u{43A}\u{430}\u{43A} \u{434}\u{435}\u{43B}\u{430}?",
        "@ai \u{41F}\u{440}\u{438}\u{432}\u{435}\u{442}",
        "\u{3053}\u{3093}\u{306B}\u{3061}\u{306F}",
        "\u{4F60}\u{597D}\u{4E16}\u{754C}",
        "\u{417}\u{434}\u{440}\u{430}\u{432}\u{43E}",
        // The same trap one byte further in: `/dra` then a three-byte
        // character.
        "/dra\u{20AC} a cat",
        // A BARE PREFIX OF THE TOKEN, in each script that trips the byte
        // index — three shorter than the token, then five bytes or more
        // with no character boundary at five.
        "/d",
        "/dr",
        "/dra",
        "/dr\u{438}",
        "/dr\u{3042}",
        "/dr\u{1F408}",
        "\u{41F}",
        "\u{41F}\u{440}",
        "\u{41F}\u{440}\u{438}",
        "\u{41F}\u{440}\u{438}\u{432}",
        "\u{3053}",
        "\u{3053}\u{3093}",
        "\u{1F408}",
        "\u{1F408}\u{1F408}",
        "\u{1F3A8} \u{43D}\u{430}\u{440}\u{438}\u{441}\u{443}\u{439} \u{43A}\u{43E}\u{442}\u{430}",
        "@ai \u{1F408}",
        // A HOMOGLYPH: Cyrillic `\u{430}` where the token wants ASCII `a`.
        "/dr\u{430}w a cat",
        // U+001C–U+001F are **not** whitespace, so these are the token
        // followed by a longer word. Java's `Character.isWhitespace` says
        // they are, which is what Android had to hand-roll around; Swift
        // agrees with the server here and this pins that it keeps doing so.
        "/draw\u{1C}a cat",
        "/draw\u{1D}a cat",
        "/draw\u{1E}a cat",
        "/draw\u{1F}a cat",
        // A COMBINING MARK on the token's last letter. The server steps one
        // CHARACTER past `/draw`, finds U+0301 and reads a longer word.
        // Swift counts in grapheme CLUSTERS, where `w` and its accent are
        // one `Character` — so five of them landed on the space and this
        // side used to call it a picture request. The scan works in
        // SCALARS now, which is what the server's `char` is.
        "/draw\u{301} a cat",
        "@ai /draw\u{301} a cat",
    ]

    @Test("every shared vector that asks for a picture, and the words it sends")
    func drawVectors() {
        for (body, prompt) in Self.draws {
            #expect(
                AssistantMention.drawPrompt(in: body) == prompt,
                "\(body) should ask for: \(prompt)")
        }
    }

    @Test("every shared vector that is an ordinary message")
    func nonDrawVectors() {
        for body in Self.notDraws {
            #expect(
                AssistantMention.drawPrompt(in: body) == nil,
                "should NOT ask for a picture: \(body)")
        }
    }

    /// The bool the composers ask — the twin of `mentions(_:)`, and it has
    /// to answer exactly what the prompt scan does or the `/draw` button
    /// would type a second token into a body that already asks.
    @Test("the bool and the prompt agree on every vector")
    func asksForPictureAgreesWithThePrompt() {
        for (body, _) in Self.draws {
            #expect(AssistantMention.asksForPicture(body), "should ask: \(body)")
        }
        for body in Self.notDraws {
            #expect(!AssistantMention.asksForPicture(body), "should not ask: \(body)")
        }
    }

    /// The two grammars are independent, and the family chat needs both at
    /// once: the server only looks for `/draw` on a message that already
    /// mentioned the assistant.
    @Test("a family-chat picture request is also a mention")
    func drawInTheFamilyChat() {
        #expect(AssistantMention.mentions("@ai /draw a cat"))
        #expect(AssistantMention.drawPrompt(in: "@ai /draw a cat") == "a cat")
        // And one in a PRIVATE thread mentions nobody, which is why the
        // private path never consults the mention grammar.
        #expect(!AssistantMention.mentions("/draw a cat"))
    }

    /// The highlight has to sit exactly on the five characters the server
    /// acts on — no leading whitespace, no mention, no prompt.
    @Test("the highlighted range is the token itself")
    func drawRangeCoversTheToken() {
        for body in ["/draw a cat", "   /draw a cat", "@ai /draw a cat", "/DRAW a cat"] {
            guard let range = AssistantMention.drawTokenRange(in: body) else {
                Issue.record("no range in: \(body)")
                continue
            }
            #expect(body[range].lowercased() == "/draw", "sliced the token in: \(body)")
        }
        // Nothing to highlight where the server would act on nothing.
        for body in Self.notDraws {
            #expect(AssistantMention.drawTokenRange(in: body) == nil, "no range in: \(body)")
        }
    }

    /// Multi-byte text either side of the token is where a byte-versus-
    /// character mix-up shows: on the server this grammar slices a `str` by
    /// byte offset, which traps outright if the offset is not a character
    /// boundary. Swift cannot trap here, but it can very easily slice in
    /// the wrong place, so the same shapes are pinned.
    @Test("non-ASCII bodies are sliced cleanly, or refused")
    func drawWithMultibyteText() {
        #expect(AssistantMention.drawPrompt(in: "/draw кот в шляпе") == "кот в шляпе")
        #expect(AssistantMention.drawPrompt(in: "/draw 猫") == "猫")
        #expect(AssistantMention.drawPrompt(in: "@ai /draw 🐈 on a mat") == "🐈 on a mat")
        // A body whose FIFTH byte lands inside a multi-byte character: the
        // token does not match, and nothing is sliced.
        #expect(AssistantMention.drawPrompt(in: "/dra€ a cat") == nil)
        #expect(AssistantMention.drawPrompt(in: "/dra") == nil)
        #expect(AssistantMention.drawPrompt(in: "🐈") == nil)
        // A leading `@aiden` is not a leading mention, so nothing licenses
        // the token behind it.
        #expect(AssistantMention.drawPrompt(in: "@aiden /draw a cat") == nil)
    }

    /// The whitespace table, code point by code point.
    ///
    /// Whitespace is where three hand-written ports of one grammar part
    /// company. The server is Rust's `char::is_whitespace`, which is the
    /// Unicode `White_Space` property. Kotlin's `Char.isWhitespace()` is
    /// Java's union of `isWhitespace` and `isSpaceChar`, which adds
    /// U+001C–U+001F and drops U+0085. Foundation's
    /// `CharacterSet.whitespacesAndNewlines` — what this file used to trim
    /// with — adds U+200B. Three tables, three answers, one grammar.
    ///
    /// `Unicode.Scalar.Properties.isWhitespace` IS the property, so this
    /// pins the whole of it rather than the six code points that differ:
    /// `server/src/mentions.rs` asserts the same table in
    /// `whitespace_is_the_unicode_property_and_nothing_else`, and Android's
    /// `AssistantMentionTest` the same again.
    @Test("whitespace is the Unicode property and nothing else")
    func whitespaceIsTheUnicodeProperty() {
        let whitespace: [Unicode.Scalar] = [
            "\u{9}", "\u{A}", "\u{B}", "\u{C}", "\u{D}", "\u{20}", "\u{85}", "\u{A0}",
            "\u{1680}", "\u{2000}", "\u{2001}", "\u{2002}", "\u{2003}", "\u{2004}",
            "\u{2005}", "\u{2006}", "\u{2007}", "\u{2008}", "\u{2009}", "\u{200A}",
            "\u{2028}", "\u{2029}", "\u{202F}", "\u{205F}", "\u{3000}",
        ]
        for scalar in whitespace {
            #expect(scalar.properties.isWhitespace, "U+\(String(scalar.value, radix: 16)) is whitespace")
        }
        // The five Java calls whitespace and Unicode does not, plus the one
        // Foundation calls whitespace and Unicode does not.
        for scalar: Unicode.Scalar in ["\u{1C}", "\u{1D}", "\u{1E}", "\u{1F}", "\u{200B}"] {
            #expect(
                !scalar.properties.isWhitespace,
                "U+\(String(scalar.value, radix: 16)) is NOT whitespace")
        }
        // And the one Foundation gets wrong, spelled out: this is the
        // reason the trim here is hand-rolled.
        #expect(CharacterSet.whitespacesAndNewlines.contains("\u{200B}"))
        #expect(!Unicode.Scalar("\u{200B}").properties.isWhitespace)
    }

    /// Every PREFIX of every vector, and the point is that each one is
    /// answered rather than mis-sliced.
    ///
    /// On the server the equivalent test catches an outright trap: a
    /// five-BYTE slice taken without asking whether byte five is a
    /// character boundary brings the request down on the word
    /// "\u{41F}\u{440}\u{438}\u{432}\u{435}\u{442}". Swift cannot trap
    /// on a `String.Index` walk, so what this pins instead is that the
    /// scan terminates and never claims a token in a body that has none —
    /// the same family of inputs, checked the way this language can.
    @Test("no prefix of any vector is mis-read")
    func noPrefixIsMisread() {
        let bodies =
            Self.draws.map(\.0) + Self.notDraws + Self.mentions + Self.notMentions
        for body in bodies {
            for index in body.indices {
                let head = String(body[..<index])
                if let range = AssistantMention.drawTokenRange(in: head) {
                    #expect(
                        head[range].lowercased() == AssistantMention.drawToken,
                        "sliced something other than the token out of: \(head)")
                }
                _ = AssistantMention.mentions(head)
            }
        }
    }
}
