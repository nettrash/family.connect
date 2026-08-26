//
//  BubbleLayoutTests.swift
//  FamilyConnectTests
//
//  The one bubble defect a logic test cannot see: a reply losing the last
//  lines of its own body.
//
//  WHY THIS IS A PIXEL TEST. The balloon's TOTAL HEIGHT is identical broken
//  and fixed — a squeezed `Text` truncates instead of wrapping and the
//  height it gives up is absorbed by the height-greedy quote bar beside it,
//  so the bubble measures the same either way. The accessibility label is
//  identical too: truncation is visual only, so XCUITest's `label` still
//  reports the whole body. Nothing but the drawn glyphs differs, so nothing
//  but the drawn glyphs can be asserted.
//
//  WHAT IS ASSERTED, and why it needs no pinned font metrics: the balloon
//  is rendered TWICE with the same body, once as a reply and once not, and
//  the two are compared against each other. Adding a quote must cost the
//  body no lines. That invariant survives a new SDK, a new system font and
//  a different Dynamic Type default, which a hard-coded line count would
//  not.
//
//  HOW LINES ARE COUNTED. On an own-message balloon the content colour is
//  white on the tint, so a row of pixels containing white ink is a row of
//  text. Contiguous inked rows are one band. The quote block always counts
//  as exactly ONE band however many lines it has, because its accent bar is
//  a solid rule running the full height of the block and welds those rows
//  together — which is precisely why the expected relationship is
//  `withQuote == withoutQuote + 1` rather than a sum of line counts. Band
//  ORDER is never relied on (a bitmap context's row order is a Core Graphics
//  detail); the quote is identified by being the tallest band, since it is
//  the only one taller than a line of text.
//

#if os(iOS)

import CoreGraphics
import Foundation
import SwiftUI
import Testing
import UIKit
@testable import FamilyConnect

@MainActor
struct BubbleLayoutTests {

    /// Wide enough for several lines at the default body size, narrow
    /// enough that the body is guaranteed to wrap past the threshold where
    /// the squeeze used to bite (measured: 4+ lines).
    private static let renderWidth: CGFloat = 320

    private static let body = """
        Yes I totally agree with you about that, let's meet tomorrow at the \
        usual place around six in the evening and bring the papers with you, \
        we will need them.
        """

    private static let excerpt = """
        Do you think we should move tomorrow's meeting to the evening \
        instead of the morning slot that we agreed on last week?
        """

    // MARK: - The regression

    @Test("A reply's body keeps every line the same body keeps without a quote")
    func replyDoesNotTruncateItsOwnBody() throws {
        let plain = try inkBands(replyTo: nil)
        let reply = try inkBands(replyTo: quote)

        // Sanity: the fixture has to actually wrap, or the test proves
        // nothing. Three lines is below the measured threshold.
        #expect(plain >= 4, "fixture too short to exercise the squeeze")

        // The whole quote block is one welded band; every body line the
        // plain bubble drew must still be drawn.
        #expect(
            reply == plain + 1,
            """
            a reply drew \(reply) bands where \(plain) + 1 were expected — \
            the body lost \(plain + 1 - reply) line(s) to the quote. This is \
            the height-greedy accent bar making the balloon's VStack \
            distribute height instead of granting it; the body Text needs \
            .fixedSize(horizontal: false, vertical: true).
            """)
    }

    @Test("The quote's accent bar is sized by its own excerpt, not by the spare height")
    func accentBarHugsItsQuote() throws {
        // The bar is the tallest thing in the quote block, so the welded
        // band's height IS the bar's height. Same body both times — so the
        // balloon is the same width and the same height — and only the
        // excerpt differs. A bar that measures itself must therefore come
        // out visibly shorter for a one-line excerpt; a GREEDY bar is handed
        // the balloon's spare height either way and comes out the same.
        let short = try quoteBandHeight(excerpt: "See you at six")
        let long = try quoteBandHeight(excerpt: Self.excerpt)

        #expect(
            long > short + 8,
            """
            the quote bar measured \(long)px around a two-line excerpt and \
            \(short)px around a one-line one — too close to be its own text. \
            The bar is a Shape with a width-only frame, so it is infinitely \
            flexible in height and absorbs whatever the balloon has spare; \
            the quote block needs .fixedSize(horizontal: false, vertical: true).
            """)
    }

    /// The same defect one layer along: a balloon that MEASURES shorter
    /// than it DRAWS.
    ///
    /// `Grid` divides the width one way while measuring and another while
    /// laying out — columns sized from their content come out narrower in
    /// the second pass, so a cell that wraps to four lines is reported as
    /// two. The bubble then hands the thread a height that is ~76pt short
    /// of what it puts on screen, and in a non-lazy stack of rows that is a
    /// message drawn over the one below it. MarkdownTableView makes every
    /// cell width-greedy to stop it; this is what says so.
    ///
    /// The ink stops at the last glyph, so what is compared is a FLOOR —
    /// the balloon's own padding and its timestamp sit below it. A short
    /// measurement still fails it, which is the case that matters.
    @Test("a wrapping table reports a height that fits what it draws")
    func tableBalloonMeasuresWhatItDraws() throws {
        let body = """
            Here is the plan:
            | day | who | cost |
            | :-- | :--: | ---: |
            | Mon | Ann | 5 |
            | Tue | Bob and a rather long name that has to wrap | 12 |
            after the table
            """
        let host = UIHostingController(rootView: bubble(body: body, replyTo: nil))
        let reported = host.sizeThatFits(
            in: CGSize(width: Self.renderWidth, height: CGFloat.greatestFiniteMagnitude)).height
        // Drawn with room to spare, so nothing is clipped by the very
        // measurement under test.
        let drawn = try #require(
            bands(in: try render(body: body, replyTo: nil, height: 900)).map(\.end).max())

        #expect(
            reported >= CGFloat(drawn),
            """
            the balloon measured \(Int(reported))pt and drew glyphs down to \
            \(drawn)pt, so the thread reserves less room than the message \
            needs and the row below it is overdrawn. A table's cells have to \
            be width-greedy — Grid's own column sizing does not survive the \
            trip from measuring to drawing.
            """)
    }

    // MARK: - Fixtures

    private var quote: ReplyToSnapshot {
        ReplyToSnapshot(messageID: 41, senderID: 7, excerpt: Self.excerpt)
    }

    private func snapshot(body: String, replyTo: ReplyToSnapshot?) -> MessageSnapshot {
        MessageSnapshot(
            localID: "local-1",
            serverID: 1338,
            chatID: 42,
            senderID: 7,
            body: body,
            createdAt: Date(timeIntervalSince1970: 1_700_000_000),
            state: .sent,
            replyTo: replyTo)
    }

    // MARK: - Rendering

    /// The real bubble at a fixed width as an own message: white ink on the
    /// tint is what makes a text row detectable.
    private func bubble(body: String, replyTo: ReplyToSnapshot?) -> some View {
        MessageBubbleView(
            message: snapshot(body: body, replyTo: replyTo),
            isMine: true,
            showsSenderName: false,
            senderName: nil,
            isRead: true,
            memberNames: [7: "You"],
            currentUserID: 7)
            .frame(width: Self.renderWidth)
            .tint(.blue)
            .environment(\.dynamicTypeSize, .large)
            .environment(LinkPreviewLoader())
    }

    /// That bubble, rendered. `height` pins the canvas instead of letting
    /// the bubble's own measurement decide it — which is the whole subject
    /// of one of the tests above.
    private func render(
        body: String, replyTo: ReplyToSnapshot?, height: CGFloat? = nil
    ) throws -> CGImage {
        let view = bubble(body: body, replyTo: replyTo)
            .frame(height: height, alignment: .top)

        let renderer = ImageRenderer(content: view)
        // Pixels, not points: the counting below is per pixel row, and a
        // renderer left at screen scale would multiply every band height.
        renderer.scale = 1
        let image = try #require(renderer.cgImage, "the bubble did not render")
        return image
    }

    /// Rows of pixels containing white ink, collapsed into contiguous bands.
    private func bands(in image: CGImage) throws -> [(start: Int, end: Int)] {
        let width = image.width
        let height = image.height
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        let context = try #require(
            CGContext(
                data: &pixels,
                width: width,
                height: height,
                bitsPerComponent: 8,
                bytesPerRow: width * 4,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue),
            "could not read the rendered pixels")
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))

        var result: [(start: Int, end: Int)] = []
        var bandStart: Int?
        for y in 0..<height {
            var inked = 0
            for x in 0..<width {
                let index = (y * width + x) * 4
                // Light ink on the tint. The floor is 170 rather than 200
                // because the quote excerpt is drawn at 0.75 opacity, which
                // lands around (191, 205, 255) over blue — at 200 the
                // excerpt rows would read as empty and the quote would stop
                // being one welded band. The balloon itself is ~(0, 122,
                // 255): high in blue, so all three channels are required.
                if pixels[index] > 170, pixels[index + 1] > 170, pixels[index + 2] > 170 {
                    inked += 1
                }
            }
            // The accent bar is exactly 3pt wide and is what welds the
            // quote's lines together, so 3 inked pixels IS a row.
            if inked >= 3, bandStart == nil {
                bandStart = y
            } else if inked < 3, let start = bandStart {
                result.append((start, y))
                bandStart = nil
            }
        }
        if let start = bandStart { result.append((start, height)) }
        // Weld micro-gaps. A glyph row can dip below the 3-pixel floor for
        // a single row INSIDE a line (a descender's waist), splitting one
        // line into two bands — measured on the left-aligned control body,
        // whose sixth line split at exactly one blank row, while the same
        // body trailing-aligned (an own reply, since the 2026-08 alignment
        // rule) did not. Counting must not depend on which edge the text
        // rags against, so bands separated by fewer than 3 blank rows are
        // one band. A REAL lost line removes a full line pitch (~22 rows at
        // .large), which this cannot absorb — the regression the suite
        // exists for still fails it.
        var welded: [(start: Int, end: Int)] = []
        for band in result {
            if let last = welded.last, band.start - last.end <= 2 {
                welded[welded.count - 1] = (last.start, band.end)
            } else {
                welded.append(band)
            }
        }
        return welded
    }

    private func inkBands(replyTo: ReplyToSnapshot?) throws -> Int {
        try bands(in: render(body: Self.body, replyTo: replyTo)).count
    }

    /// The quote block's height, found as the TALLEST band rather than by
    /// position: a bitmap context's row order is a Core Graphics detail, and
    /// the quote is the only band that is more than one line of text.
    private func quoteBandHeight(excerpt: String) throws -> Int {
        let replyTo = ReplyToSnapshot(messageID: 41, senderID: 7, excerpt: excerpt)
        let measured = try bands(in: render(body: Self.body, replyTo: replyTo))
        let tallest = try #require(measured.max { ($0.end - $0.start) < ($1.end - $1.start) })
        return tallest.end - tallest.start
    }
}

#endif
