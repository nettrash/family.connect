//
//  MessageMarkdownTests.swift
//  FamilyConnectTests
//
//  What a chat bubble renders, and — more importantly — what it leaves
//  completely alone.
//
//  Most messages are not markdown. A renderer that quietly rewrites an
//  ordinary sentence is worse than no renderer at all, so half of these
//  tests assert that nothing happened.
//
//  The other half pins the property the whole design rests on: markdown
//  DELETES characters, and every offset-based pass downstream — the link
//  detector, the `@ai` highlight, and on Android the tap hit test — must
//  index the RENDERED text rather than the raw body. A test that only
//  checked "it came out bold" would miss a link pointing at the wrong
//  glyphs.
//

import Foundation
import Testing

@testable import FamilyConnect

@Suite("Markdown in a bubble")
struct MessageMarkdownTests {

    /// The rendered characters, which is what everything downstream indexes.
    private func plain(_ body: String) -> String {
        String(MessageMarkdown.render(body).characters)
    }

    @Test("ordinary messages are left exactly as they were typed")
    func ordinaryTextIsUntouched() {
        let untouched = [
            "Dinner at 7?",
            "see you at the shop",
            "2 * 3 * 4 = 24",
            "call me on 555-1234",
            "https://example.com/a_b_c",
            "he said \"what?\" and left",
            "snake_case_name stays whole",
            "a * b",
            "5 < 6 > 4",
            "cost: $5 (a bargain)",
            "",
            "🎉🎉🎉",
        ]
        for body in untouched {
            #expect(plain(body) == body, "rewritten: \(body) → \(plain(body))")
        }
    }

    /// CommonMark's flanking rules, which Foundation applies here and
    /// Android hand-implements to match. This table is the contract between
    /// the two: the same vectors appear in MessageMarkdownTest.kt, and they
    /// were checked against Foundation's actual output rather than guessed.
    @Test("CommonMark flanking rules, mirrored on Android")
    func flankingRules() {
        // Underscores inside a word are not emphasis.
        #expect(plain("call user_name_field now") == "call user_name_field now")
        #expect(plain("a_b_c") == "a_b_c")
        // A delimiter run followed (or preceded) by a space opens nothing,
        // which is what keeps arithmetic readable.
        #expect(plain("2 * 3 * 4 = 24") == "2 * 3 * 4 = 24")
        #expect(plain("a * b") == "a * b")
        // ...but a real one still works, including intraword asterisks,
        // which CommonMark does allow.
        #expect(plain("*real italic*") == "real italic")
        #expect(plain("5*6*7") == "567")
        #expect(plain("_x_") == "x")
        // `__init__` at a word boundary IS emphasis in CommonMark, however
        // surprising — pinned so Android is held to the same answer.
        #expect(plain("__init__ is special") == "init is special")
    }

    @Test("the subset renders and its markers are consumed")
    func theSubsetRenders() {
        #expect(plain("**bold**") == "bold")
        #expect(plain("*italic*") == "italic")
        #expect(plain("~~gone~~") == "gone")
        #expect(plain("`code`") == "code")
        #expect(plain("say **hello** there") == "say hello there")
    }

    /// The reason `[label](url)` is worth having and the reason it is worth
    /// watching: the URL disappears from the visible text entirely.
    /// Nested emphasis. Pinned on both platforms because Foundation gets
    /// CommonMark's delimiter-run rule right for free and Android had to
    /// implement it — a divergence here means one message reading two ways.
    @Test("nested emphasis keeps both levels")
    func nestedEmphasis() {
        #expect(plain("*this is **very** important*") == "this is very important")
        #expect(plain("*a **b** c*") == "a b c")
        #expect(plain("_a __b__ c_") == "a b c")
        // NOT pinned as shared: `**outer *inner***` needs CommonMark's full
        // delimiter STACK, which Foundation has and Android's hand-written
        // parser deliberately does not. Android leaves those markers in the
        // text — a safe failure, and a stated limit rather than a surprise
        // (see `three delimiters closing at once are left as typed` there).
    }

    @Test("a markdown link shows its label and carries its destination")
    func markdownLink() {
        let rendered = MessageMarkdown.render("see [the menu](https://example.com/menu)")
        #expect(String(rendered.characters) == "see the menu")
        let linked = rendered.runs.compactMap { run in
            run.link.map { (String(rendered[run.range].characters), $0.absoluteString) }
        }
        #expect(linked.count == 1)
        #expect(linked.first?.0 == "the menu")
        #expect(linked.first?.1 == "https://example.com/menu")
    }

    @Test("a fenced block keeps its contents verbatim, markup and all")
    func fencedBlock() {
        let body = "try this:\n```\nlet x = **not bold**\n```\ndone"
        let rendered = plain(body)
        #expect(rendered.contains("let x = **not bold**"), "fence contents are literal: \(rendered)")
        #expect(rendered.contains("try this:"))
        #expect(rendered.contains("done"))
        #expect(!rendered.contains("```"), "the fence markers themselves are consumed")
    }

    /// Somebody typing the third backtick of an opening fence must not
    /// watch the rest of their message become a code block.
    @Test("an unclosed fence is not a fence")
    func unclosedFence() {
        let body = "```\nhalf written"
        #expect(plain(body) == body)
    }

    @Test("a language tag on the fence is dropped rather than drawn")
    func fenceLanguageTag() {
        let rendered = plain("```swift\nlet x = 1\n```")
        #expect(rendered.contains("let x = 1"))
        #expect(!rendered.contains("swift"), "the tag is metadata, not text: \(rendered)")
    }

    /// THE property. Markdown removes characters, so a link detected over
    /// the raw body would land on the wrong glyphs — this asserts the
    /// finished attributed string agrees with itself.
    @Test("a detected link lands on the right glyphs after markdown shortens the text")
    func detectedLinkOffsetsSurviveMarkdown() {
        let body = "**important** go to https://example.com now"
        let attributed = MessageLinks.attributedBody(body, isMine: false)
        let rendered = String(attributed.characters)
        #expect(rendered == "important go to https://example.com now")
        let linked = attributed.runs.compactMap { run -> String? in
            guard run.link != nil else { return nil }
            return String(attributed[run.range].characters)
        }
        #expect(linked == ["https://example.com"], "got \(linked) in \(rendered)")
    }

    /// The phishing shape, and the reason the overlap rule exists: a
    /// markdown link whose LABEL is itself a URL renders as that URL's
    /// text, which the detector would then match as a link to the place it
    /// names — two destinations over the same glyphs. The author's own
    /// destination is what the message declares, so it must be the only one.
    @Test("a link label that looks like a URL keeps the author's destination")
    func labelThatLooksLikeAURL() {
        let body = "[https://www.paypal.com](https://evil.example)"
        let attributed = MessageLinks.attributedBody(body, isMine: false)
        let links = attributed.runs.compactMap { $0.link?.absoluteString }
        #expect(
            links == ["https://evil.example"],
            "exactly one destination, and it is the one that was written: \(links)")
    }

    /// Foundation hands back the destination exactly as typed, so a
    /// scheme-less one opens nothing. Android normalises the same way —
    /// without this the identical message is a live link on one platform
    /// and a dead one on the other.
    @Test("a scheme-less link destination gains one")
    func schemelessDestination() {
        let attributed = MessageLinks.attributedBody("[here](example.com/x)", isMine: false)
        let links = attributed.runs.compactMap { $0.link?.absoluteString }
        #expect(links == ["https://example.com/x"], "got \(links)")
    }

    /// The preview card must describe a link the reader can actually SEE.
    @Test("the preview card looks at the rendered text")
    func previewUsesRenderedText() {
        // Bold markers around a URL: the raw body's detector match includes
        // the asterisks, which are not on screen.
        #expect(
            MessageLinks.firstWebLinkAsDrawn(in: "**https://example.com**")?.absoluteString
                == "https://example.com")
        // And a markdown link has no bare URL in the raw body at all.
        #expect(
            MessageLinks.firstWebLinkAsDrawn(in: "see [the menu](https://example.com/menu)") == nil,
            "the label is not a URL, so there is nothing to preview")
    }

    @Test("@ai is marked, and only where the server would act on it")
    func mentionHighlight() {
        let attributed = MessageLinks.attributedBody("hey @ai and @aiden", isMine: false)
        let rendered = String(attributed.characters)
        let marked = attributed.runs.compactMap { run -> String? in
            guard run.inlinePresentationIntent?.contains(.stronglyEmphasized) == true else {
                return nil
            }
            return String(attributed[run.range].characters)
        }
        #expect(marked == ["@ai"], "got \(marked) in \(rendered)")
    }

    /// An emoji-only body never reaches this path (the bubble branches
    /// around it into a plain string), but the renderer must still not
    /// mangle one if it ever does.
    @Test("emoji survive the renderer untouched")
    func emojiSurvive() {
        for body in ["😀", "👨‍👩‍👧‍👦", "🇷🇸", "❤️"] {
            #expect(plain(body) == body)
        }
    }

    /// Newlines are what `.inlineOnlyPreservingWhitespace` exists to
    /// protect: without it a multi-line message collapses into one line.
    @Test("line breaks are preserved")
    func lineBreaks() {
        let body = "first line\nsecond line\n\nafter a gap"
        #expect(plain(body) == body)
    }
}
