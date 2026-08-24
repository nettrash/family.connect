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
    /// `AssistantMentionTest.kt` carry these same strings.
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
}
