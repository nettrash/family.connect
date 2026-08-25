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
import SwiftUI
import Testing

@testable import FamilyConnect

@Suite("Markdown in a bubble")
struct MessageMarkdownTests {

    /// The rendered characters, which is what everything downstream indexes.
    private func plain(_ body: String) -> String {
        String(MessageMarkdown.render(body).characters)
    }

    /// The text of every block, in order — the shape a bubble draws.
    private func blockTexts(_ body: String) -> [String] {
        MessageMarkdown.blocks(body).compactMap { block in
            guard case .text(let rendered) = block else { return nil }
            return String(rendered.characters)
        }
    }

    /// The first table in a body, or nil when it has none.
    private func table(in body: String) -> MessageMarkdown.Table? {
        for block in MessageMarkdown.blocks(body) {
            if case .table(let table) = block { return table }
        }
        return nil
    }

    private func cells(_ row: [AttributedString]) -> [String] {
        row.map { String($0.characters) }
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

    /// The preview card must describe a link the reader can actually reach
    /// — which is not the same as one they can READ.
    @Test("the preview card describes the link the bubble draws")
    func previewUsesRenderedText() {
        // Bold markers around a URL: the raw body's detector match includes
        // the asterisks, which are not on screen.
        #expect(
            MessageLinks.firstWebLinkAsDrawn(in: "**https://example.com**")?.absoluteString
                == "https://example.com")
        // A markdown link has no bare URL in the rendered CHARACTERS at all
        // — the destination is an attribute, not text — so scanning glyphs
        // dropped the card from a message that is nothing but a link, and
        // dropped it on Apple only: Android's card comes from the same
        // spans its bubble is drawn from.
        #expect(
            MessageLinks.firstWebLinkAsDrawn(in: "see [the menu](https://example.com/menu)")?
                .absoluteString == "https://example.com/menu")
        // A scheme-less destination is normalised on the way, exactly as it
        // is for the tap — or the card would describe a URL that opens
        // nothing.
        #expect(
            MessageLinks.firstWebLinkAsDrawn(in: "[here](example.com/x)")?.absoluteString
                == "https://example.com/x")
        // The phishing shape: label and destination disagree, and the card
        // describes where the tap GOES, not what the label claims.
        #expect(
            MessageLinks.firstWebLinkAsDrawn(in: "[https://www.paypal.com](https://evil.example)")?
                .absoluteString == "https://evil.example")
        // Plain http still previews nowhere: ATS blocks the fetch, so a
        // card would appear on Android and not here.
        #expect(MessageLinks.firstWebLinkAsDrawn(in: "see http://example.com") == nil)
        #expect(MessageLinks.firstWebLinkAsDrawn(in: "no links here at all") == nil)
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

    /// The bug both parsers had: the opening fence `continue`d without
    /// pushing a separator, so the code welded onto the end of the line
    /// above it — "try this:let x = 1". Asserted as a WHOLE value, because
    /// `contains` is exactly what let it through on both platforms.
    @Test("a fenced block starts on its own line")
    func fenceKeepsThePrecedingNewline() {
        #expect(plain("try this:\n```\nlet x = 1\n```\ndone") == "try this:\nlet x = 1\ndone")
        // ...and a body that IS a fence gains no blank lines around it.
        #expect(plain("```\nlet x = 1\n```") == "let x = 1")
    }

    // MARK: - Headings

    @Test("one to three hashes are the three heading steps")
    func headings() {
        #expect(plain("# Big") == "Big")
        #expect(plain("## Medium") == "Medium")
        #expect(plain("### Small") == "Small")
        // No closing-sequence stripping, and the content is inline-parsed.
        #expect(plain("# Done #") == "Done #")
        #expect(plain("## say **hello**") == "say hello")
        // A heading is a font over the whole line, and one run of it.
        let heading = MessageMarkdown.render("# Big")
        #expect(heading.runs.count == 1)
        #expect(heading.runs.first?.font != nil)
        // Three DISTINCT steps. The exact fonts are not the contract (the
        // spec pins the ladder, not the points) but a ladder whose rungs
        // are equal is not a ladder.
        let steps = ["# x", "## x", "### x"].compactMap {
            MessageMarkdown.render($0).runs.first?.font
        }
        #expect(Set(steps).count == 3, "got \(steps)")
    }

    /// Every near miss is left EXACTLY as it was typed — a heading has to
    /// be something somebody meant, or a message starting with a hash tag
    /// silently grows a font.
    @Test("near misses are not headings")
    func headingNearMisses() {
        let untouched = [
            "#Heading",        // no space
            "#### X",          // deeper than the ladder goes
            "##### X",
            "# ",              // nothing after it
            "#",
            "###",
            "a # b",           // not at the start of a line
            "  # indented",    // ...and the start means the start
            "#1 fan",
        ]
        for body in untouched {
            #expect(plain(body) == body, "rewritten: \(body) → \(plain(body))")
        }
        // Mid-body, a heading line is still a heading line.
        #expect(plain("look:\n## Menu\nfries") == "look:\nMenu\nfries")
    }

    // MARK: - Lists

    @Test("all three bullet markers become one bullet, indent and all")
    func bullets() {
        #expect(plain("- milk") == "• milk")
        #expect(plain("* milk") == "• milk")
        #expect(plain("+ milk") == "• milk")
        #expect(plain("- a\n* b\n+ c") == "• a\n• b\n• c")
        // The indent is copied verbatim, which is what gives a nested list
        // without a parser that can mis-nest one.
        #expect(plain("- a\n  - b\n    - c") == "• a\n  • b\n    • c")
        #expect(plain("\t- tabbed") == "\t• tabbed")
        // Content is inline-parsed, and the marker never reaches the
        // emphasis parser.
        #expect(plain("- **milk** and eggs") == "• milk and eggs")
        #expect(plain("* italic *not* here") == "• italic not here")
    }

    @Test("near misses are not list items")
    func bulletNearMisses() {
        let untouched = [
            "- ",              // nothing after the marker
            "-",
            "---",             // a delimiter candidate, not an item
            "***",
            "-no space",
            "2 * 3 * 4 = 24",  // the `*` is not at the start of a line
            "a - b",
            "5-6",
        ]
        for body in untouched {
            #expect(plain(body) == body, "rewritten: \(body) → \(plain(body))")
        }
    }

    /// Ordered items are RECOGNISED and rendered as typed: they already
    /// read as a list, and re-numbering somebody's message is worse than
    /// leaving it.
    @Test("ordered lists are left exactly as they were typed")
    func orderedListsAreUntouched() {
        for body in ["1. milk", "1) milk", "1. a\n2. b\n3. c", "10. ten", "1.no space"] {
            #expect(plain(body) == body, "rewritten: \(body) → \(plain(body))")
        }
    }

    // MARK: - Tables

    @Test("a pipe table becomes a grid with the delimiter row's alignments")
    func tableParsing() throws {
        let body = """
            | day | who  | cost |
            | :-- | :--: | ---: |
            | Mon | Ann  | 5    |
            | Tue | Bob  |
            | Wed | Cat  | 7 | extra |
            """
        let table = try #require(self.table(in: body))
        #expect(cells(table.header) == ["day", "who", "cost"])
        #expect(table.alignments == [.leading, .center, .trailing])
        #expect(table.columnCount == 3)
        // A ragged row is padded and an over-long one is trimmed: dropping
        // the whole table over one missing pipe would be the worst answer
        // available.
        #expect(
            table.rows.map(cells) == [
                ["Mon", "Ann", "5"],
                ["Tue", "Bob", ""],
                ["Wed", "Cat", "7"],
            ])
    }

    @Test("the edge pipes are optional and a default column is left-aligned")
    func tableWithoutEdgePipes() throws {
        let table = try #require(self.table(in: "day | who\n--- | ---\nMon | Ann"))
        #expect(cells(table.header) == ["day", "who"])
        #expect(table.alignments == [.leading, .leading])
        #expect(table.rows.map(cells) == [["Mon", "Ann"]])
    }

    @Test("an escaped pipe is a character in a cell, not a cell boundary")
    func tableEscapedPipe() throws {
        let table = try #require(self.table(in: "| a | b |\n| - | - |\n| x \\| y | z |"))
        #expect(table.rows.map(cells) == [["x | y", "z"]])
    }

    /// The rule that makes a table cost nothing structurally: no per-cell
    /// hit test, no per-cell offset space, and no second place where a
    /// label and a destination can disagree.
    @Test("a link in a cell stays exactly as it was typed")
    func tableCellsHaveNoLinks() throws {
        let body = """
            | what | where |
            | --- | --- |
            | menu | [the menu](https://example.com/menu) |
            | site | https://example.com |
            """
        let table = try #require(self.table(in: body))
        #expect(
            table.rows.map(cells) == [
                ["menu", "[the menu](https://example.com/menu)"],
                ["site", "https://example.com"],
            ])
        for row in table.rows {
            for cell in row {
                #expect(cell.runs.allSatisfy { $0.link == nil }, "a cell carried a link")
            }
        }
        // ...but everything else inline still renders.
        let emphasised = try #require(self.table(in: "| a |\n| - |\n| **b** ~~c~~ `d` |"))
        #expect(emphasised.rows.map(cells) == [["b c d"]])
    }

    @Test("text around a table is its own block, with no stray blank lines")
    func tableSplitsTheBody() {
        let body = "before\n| a | b |\n| - | - |\n| 1 | 2 |\nafter"
        let blocks = MessageMarkdown.blocks(body)
        #expect(blocks.count == 3)
        #expect(blocks[1].isTable)
        #expect(blockTexts(body) == ["before", "after"])
    }

    /// A table needs a delimiter row that MATCHES its header, or it is not
    /// a table and every line stays as it was typed. `some | thing`
    /// followed by a `---` rule is two ordinary lines.
    @Test("a table that does not parse leaves every line as typed")
    func tableNearMisses() {
        let untouched = [
            "some | thing\n---",                 // one delimiter cell, two header cells
            "| a | b |\n| --- |",                // ...the same, with edge pipes
            "| --- | --- |\n| a | b |",          // a delimiter row with no header above it
            "| a | b |\n| x | y |",              // no delimiter row at all
            "| a | b |",                         // a header with nothing under it
            "cost: $5 (a bargain)",
        ]
        for body in untouched {
            #expect(plain(body) == body, "rewritten: \(body) → \(plain(body))")
            #expect(table(in: body) == nil, "found a table in: \(body)")
        }
    }

    /// `render` is the FLAT one string — what the link detector and the
    /// preview card index — so it recognises no tables at all and a pipe
    /// table comes back as the rows that were typed.
    @Test("the flat render leaves table rows alone")
    func flatRenderKeepsTableRows() {
        let body = "| day | who |\n| --- | --- |\n| Mon | Ann |"
        #expect(plain(body) == body)
    }


    // MARK: - The contract with Android

    /// FIVE NEAR MISSES, DECIDED ONCE. Each of these is a line the two
    /// parsers used to read differently, and every one of them is a message
    /// somebody actually typed — a heading with a pipe in it, a signature
    /// rule under a line of text, a half-typed marker. The same inputs and
    /// the same expected outputs are pinned in Android's
    /// MessageMarkdownTest.kt; a difference here is one message reading two
    /// ways in the same family.
    ///
    /// 1. Precedence is heading → bullet → table. A line starting `# ` or
    ///    `- ` is a heading or a bullet even if it contains a pipe.
    @Test("contract 1: a heading or a bullet wins over a table, pipes and all")
    func contractHeadingAndBulletBeatTable() {
        // A heading whose text happens to contain a pipe, over what looks
        // like a delimiter row. It is a heading and two ordinary lines.
        let heading = "# Q | A\n--- | ---\n1 | 2"
        #expect(table(in: heading) == nil, "the heading was eaten by a table")
        #expect(plain(heading) == "Q | A\n--- | ---\n1 | 2")

        // The same for a bullet — a shopping list item with a pipe in it.
        let bullet = "- a | b\n--- | ---\n1 | 2"
        #expect(table(in: bullet) == nil, "the bullet was eaten by a table")
        #expect(plain(bullet) == "• a | b\n--- | ---\n1 | 2")
    }

    /// 2. A table ends at a heading or a bullet line, as well as at a line
    ///    with no pipes.
    @Test("contract 2: a table ends at a heading or a bullet")
    func contractTableEndsAtHeadingOrBullet() throws {
        let body = """
            | a | b |
            | --- | --- |
            | 1 | 2 |
            # Heading | x
            - item | y
            """
        let table = try #require(self.table(in: body))
        #expect(table.rows.map(cells) == [["1", "2"]], "the heading was swallowed as a row")
        // ...and both lines below it are what they say they are.
        #expect(blockTexts(body) == ["Heading | x\n• item | y"])
    }

    /// 3. A body row that parses to ZERO cells (a lone `|`) ends the table
    ///    and is left as typed. A phantom all-empty row is worse than
    ///    stopping.
    @Test("contract 3: a row with no cells ends the table and stays as typed")
    func contractLonePipeEndsTheTable() throws {
        let body = "| a | b |\n| --- | --- |\n| 1 | 2 |\n|\n| 3 | 4 |"
        let table = try #require(self.table(in: body))
        #expect(cells(table.header) == ["a", "b"])
        #expect(table.rows.map(cells) == [["1", "2"]], "a phantom empty row was padded in")
        // Everything after the stop is drawn exactly as it was written,
        // including the `|` that ended it.
        #expect(blockTexts(body) == ["|\n| 3 | 4 |"])
    }

    /// 4. A delimiter row must contain at least one pipe, so a one-column
    ///    table is written `| --- |`. A bare `---` is far more often a rule
    ///    or a signature separator than somebody's one-column table.
    @Test("contract 4: a bare --- is not a delimiter row")
    func contractDelimiterNeedsAPipe() throws {
        let rule = "| Total |\n---\n| 12 |"
        #expect(table(in: rule) == nil, "a signature rule became a one-column table")
        #expect(plain(rule) == rule, "rewritten: \(plain(rule))")

        // Written with its pipes, the one-column table is a table.
        let real = try #require(self.table(in: "| Total |\n| --- |\n| 12 |"))
        #expect(cells(real.header) == ["Total"])
        #expect(real.rows.map(cells) == [["12"]])
    }

    /// 5. Whitespace-only content is not content: `"#  "` and `"-  "` are
    ///    left exactly as typed, the same as `"# "` and `"- "` always were.
    @Test("contract 5: whitespace is not heading or bullet content")
    func contractWhitespaceIsNotContent() {
        let untouched = ["#  ", "##   ", "###  ", "-  ", "*  ", "+  ", "- \t"]
        for body in untouched {
            #expect(plain(body) == body, "rewritten: \(body.debugDescription) → \(plain(body).debugDescription)")
        }
        // Real content after extra spaces is still content.
        #expect(plain("#  Big").hasSuffix("Big"))
        #expect(plain("-  milk").hasSuffix("milk"))
    }

    /// From the same lens, and the same decision: a URL that appears ONLY
    /// inside a table cell still gets a preview card. The cell is not
    /// tappable by construction (`MessageMarkdown.cell` says why), so the
    /// card under the balloon is the only way in — better than pretending
    /// the reader never saw it. Android matches this; pinned here so the
    /// Apple half of that agreement cannot drift away from it.
    @Test("contract: a URL only inside a table cell still previews")
    func contractTableCellURLStillPreviews() throws {
        let body = "| what | where |\n| --- | --- |\n| site | https://example.com/a |"
        // It is a real table, and the cell carries no tappable link.
        let table = try #require(self.table(in: body))
        #expect(table.rows.map(cells) == [["site", "https://example.com/a"]])
        for row in table.rows where row.contains(where: { $0.runs.contains { $0.link != nil } }) {
            Issue.record("a cell carried a link")
        }
        // ...and the card describes it anyway.
        #expect(
            MessageLinks.firstWebLinkAsDrawn(in: body)?.absoluteString == "https://example.com/a")
    }

    // MARK: - Blocks

    /// THE invariant. A heading is a font and a bullet is two characters,
    /// so neither splits a body; only a table does. Everything the bubble
    /// depends on — the `\.openURL` arbitration, the link hit test, the
    /// offsets every downstream pass indexes — is built on a message
    /// without a table being ONE `Text`.
    @Test("a body with no table is exactly one text block")
    func oneTextBlock() {
        let bodies = [
            "",
            "Dinner at 7?",
            "first line\nsecond line\n\nafter a gap",
            "# Heading\n- one\n- two\n1. three",
            "**bold** and [a link](https://example.com) and @ai",
            "try this:\n```\nlet x = 1\n```\ndone",
            "🎉🎉🎉",
            "| not | a table",
        ]
        for body in bodies {
            let blocks = MessageMarkdown.blocks(body)
            #expect(blocks.count == 1, "\(body.debugDescription) → \(blocks.count) blocks")
            #expect(blocks.first?.isTable == false)
        }
    }

    /// Every offset pass runs PER TEXT BLOCK, because each block is its
    /// own laid-out string and so its own offset space. A link detected in
    /// the block after a table must land on that block's glyphs.
    @Test("links and mentions keep their offsets in the block after a table")
    func blocksKeepTheirOwnOffsets() throws {
        let body = """
            **ask** @ai
            | a | b |
            | - | - |
            | 1 | 2 |
            **then** see https://example.com now
            """
        let blocks = MessageLinks.blocks(body, isMine: false)
        #expect(blocks.count == 3)
        #expect(blocks[1].isTable)

        guard case .text(let first) = blocks[0], case .text(let last) = blocks[2] else {
            Issue.record("expected text, table, text — got \(blocks)")
            return
        }
        #expect(String(first.characters) == "ask @ai")
        let mentioned = first.runs.compactMap { run -> String? in
            guard run.inlinePresentationIntent?.contains(.stronglyEmphasized) == true else {
                return nil
            }
            return String(first[run.range].characters)
        }
        #expect(mentioned == ["ask", "@ai"], "got \(mentioned)")

        #expect(String(last.characters) == "then see https://example.com now")
        let linked = last.runs.compactMap { run -> String? in
            guard run.link != nil else { return nil }
            return String(last[run.range].characters)
        }
        #expect(linked == ["https://example.com"], "got \(linked)")
    }
}
