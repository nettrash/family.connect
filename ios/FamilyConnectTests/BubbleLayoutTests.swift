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
//  THE RAGGING TESTS at the bottom read the same bands sideways: each
//  band's left-most and right-most inked column say which edge the lines
//  rag against, which is the own-reply rule (trailing through two lines,
//  leading from three — OwnReplyBodyAlignment). That alignment is state
//  written by onGeometryChange during layout, so those bubbles are hosted
//  in a real UIWindow and laid out before they are read, where the other
//  tests can use ImageRenderer directly.
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

    // MARK: - The balloon hugs what it holds

    /// A thread column wide enough that "hugs its content" and "takes the
    /// column" are far apart: the widest tile is 240pt and a poll is
    /// capped at 280pt, so a balloon that took this column would be ~512pt
    /// and one that hugs, ~260-310pt. (At the phone-ish 320pt render width
    /// above the two cannot be told apart, which is how the slab shipped.)
    private static let wideColumn: CGFloat = 560

    /// The balloon's inset on each side of its content, plus the 48pt
    /// spacer an own message leaves on its far side: the room every
    /// balloon takes around what it draws.
    private static let balloonInsets: CGFloat = 12 * 2

    @Test("a photo with a caption makes a balloon as wide as the photo, not the column")
    func captionedPhotoBalloonHugsThePhoto() throws {
        // 800×600 → a 240×180 tile, the widest a tile gets.
        let width = try balloonWidth(
            body: "Sunday at the lake 🦆",
            attachments: [photo(width: 800, height: 600)])
        #expect(
            width <= AttachmentView.maxWidth + Self.balloonInsets + 4,
            """
            a captioned photo's balloon is \(Int(width))pt wide around a \
            \(Int(AttachmentView.maxWidth))pt tile — the caption is taking the \
            column's width instead of the photo's (the full-width-slab \
            regression: a .frame(maxWidth: .infinity) on the body Text). \
            BalloonContentLayout must offer the caption the photo's width.
            """)
        #expect(width >= AttachmentView.maxWidth, "the balloon cannot be narrower than its tile")
    }

    @Test("a long caption under a tall photo wraps at the widest tile, not at the tall photo's own width")
    func longCaptionUnderPortraitPhotoWrapsAtTheFloor() throws {
        // 600×1200 → height capped at 320, so a 160×320 tile.
        let width = try balloonWidth(
            body: Self.body,
            attachments: [photo(width: 600, height: 1200)])
        // Wider than the 160pt tile — a paragraph at 160pt is a ribbon —
        // and no wider than the 240pt floor the layout is given.
        #expect(width > 160 + Self.balloonInsets + 8, "the caption wrapped at the portrait tile's own width (\(Int(width))pt)")
        #expect(
            width <= AttachmentView.maxWidth + Self.balloonInsets + 4,
            "the caption took \(Int(width))pt — more than the \(Int(AttachmentView.maxWidth))pt floor a caption is offered")
    }

    @Test("a poll's balloon is as wide as the poll, not the column")
    func pollBalloonHugsThePoll() throws {
        let poll = PollSnapshot(
            pollSeq: 1, closed: false,
            options: [
                PollOptionSnapshot(id: 1, text: "Roast at ours", votes: [7]),
                PollOptionSnapshot(id: 2, text: "Everyone brings a dish", votes: []),
                PollOptionSnapshot(id: 3, text: "Café by the park", votes: []),
            ])
        // A question LONGER than the poll is wide: without the fill rule
        // it wraps at the column (~512pt) and the balloon follows it; with
        // the rule it wraps at the poll's 280pt. A short question fits
        // either way and would prove nothing.
        let width = try balloonWidth(
            body: "Sunday lunch — what are we doing this week, and who is bringing what?",
            poll: poll)
        // PollBubbleView caps itself at 280pt; the balloon adds its insets.
        #expect(
            width <= 280 + Self.balloonInsets + 4,
            """
            a poll's balloon is \(Int(width))pt wide around a poll capped at \
            280pt — the question is taking the column's width instead of the \
            poll's. BalloonContentLayout must offer it the poll's width.
            """)
        #expect(width >= 200, "a three-option poll cannot be this narrow (\(Int(width))pt)")
    }

    @Test("a plain text balloon still hugs its one line")
    func textBalloonStillHugsItsText() throws {
        let width = try balloonWidth(body: "Ellie.")
        #expect(width < 120, "a one-word balloon came out \(Int(width))pt wide")
    }

    /// The balloon's drawn width in a wide column: the horizontal extent
    /// of every opaque pixel. The canvas is transparent, an own balloon is
    /// filled with the tint, and its timestamp and tick sit under its
    /// trailing edge — so the extent IS the balloon.
    private func balloonWidth(
        body: String, attachments: [AttachmentDTO] = [], poll: PollSnapshot? = nil
    ) throws -> CGFloat {
        let renderer = ImageRenderer(
            content: bubble(body: body, replyTo: nil, attachments: attachments, poll: poll, width: Self.wideColumn))
        renderer.scale = 1
        let image = try #require(renderer.cgImage, "the bubble did not render")
        let extent = try opaqueExtent(in: image)
        return CGFloat(extent.right - extent.left)
    }

    /// The left-most and right-most column holding any opaque pixel.
    private func opaqueExtent(in image: CGImage) throws -> (left: Int, right: Int) {
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
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        var left = width
        var right = 0
        for y in 0..<height {
            for x in 0..<width where pixels[(y * width + x) * 4 + 3] > 0 {
                left = min(left, x)
                right = max(right, x + 1)
            }
        }
        #expect(right > left, "nothing was drawn")
        return (left, right)
    }

    // MARK: - Fixtures

    private var quote: ReplyToSnapshot {
        ReplyToSnapshot(messageID: 41, senderID: 7, excerpt: Self.excerpt)
    }

    private func snapshot(
        body: String, replyTo: ReplyToSnapshot?,
        attachments: [AttachmentDTO] = [], poll: PollSnapshot? = nil
    ) -> MessageSnapshot {
        MessageSnapshot(
            localID: "local-1",
            serverID: 1338,
            chatID: 42,
            senderID: 7,
            body: body,
            createdAt: Date(timeIntervalSince1970: 1_700_000_000),
            state: .sent,
            replyTo: replyTo,
            attachment: attachments.first,
            attachments: attachments,
            poll: poll)
    }

    /// A photo the tile can size from its metadata alone — nothing is
    /// fetched, the placeholder draws at the shape the numbers give.
    private func photo(width: Int, height: Int) -> AttachmentDTO {
        AttachmentDTO(
            id: 900, kind: "photo", mime: "image/jpeg", size: 1234,
            width: width, height: height, durationMS: nil, hasPreview: true,
            name: nil, latitude: nil, longitude: nil, accuracyM: nil)
    }

    /// An attachment store that answers nothing: the tile under test is
    /// sized by metadata, and the balloon around it is the subject.
    private func inertAttachmentStore() -> AttachmentStore {
        let api = APIClient(serverURL: URL(string: "https://attachments.invalid"))
        let directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("bubble-layout-\(UUID().uuidString)")
        return AttachmentStore(api: api, directory: directory)
    }

    // MARK: - Rendering

    /// The real bubble at a fixed width as an own message: white ink on the
    /// tint is what makes a text row detectable.
    private func bubble(
        body: String, replyTo: ReplyToSnapshot?,
        attachments: [AttachmentDTO] = [], poll: PollSnapshot? = nil,
        width: CGFloat = BubbleLayoutTests.renderWidth
    ) -> some View {
        MessageBubbleView(
            message: snapshot(body: body, replyTo: replyTo, attachments: attachments, poll: poll),
            isMine: true,
            showsSenderName: false,
            senderName: nil,
            isRead: true,
            memberNames: [7: "You"],
            currentUserID: 7)
            .frame(width: width)
            .tint(.blue)
            .environment(\.dynamicTypeSize, .large)
            .environment(LinkPreviewLoader())
            .environment(inertAttachmentStore())
            // A poll's voter faces read the avatar store, and the store is
            // a hard requirement of that view rather than the bubble's
            // optional one. Same inert client: nothing is fetched.
            .environment(AvatarStore(api: APIClient(serverURL: URL(string: "https://avatars.invalid"))))
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

    /// One run of inked rows, with the columns its ink spans.
    private struct Band {
        var start: Int
        var end: Int
        var left: Int
        var right: Int
        var height: Int { end - start }
        var width: Int { right - left }
    }

    /// Rows of pixels containing white ink, collapsed into contiguous bands.
    private func bands(in image: CGImage) throws -> [Band] {
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

        var result: [Band] = []
        var open: Band?
        for y in 0..<height {
            var inked = 0
            var left = Int.max
            var right = Int.min
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
                    left = min(left, x)
                    right = max(right, x + 1)
                }
            }
            // The accent bar is exactly 3pt wide and is what welds the
            // quote's lines together, so 3 inked pixels IS a row.
            if inked >= 3 {
                if var band = open {
                    band.left = min(band.left, left)
                    band.right = max(band.right, right)
                    open = band
                } else {
                    open = Band(start: y, end: y, left: left, right: right)
                }
            } else if var band = open {
                band.end = y
                result.append(band)
                open = nil
            }
        }
        if var band = open {
            band.end = height
            result.append(band)
        }
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
        var welded: [Band] = []
        for band in result {
            if let last = welded.last, band.start - last.end <= 2 {
                welded[welded.count - 1] = Band(
                    start: last.start, end: band.end,
                    left: min(last.left, band.left), right: max(last.right, band.right))
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
        let tallest = try #require(measured.max { $0.height < $1.height })
        return tallest.height
    }

    // MARK: - The own-reply ragging rule

    /// Two lines, the second much shorter than the first — so which edge
    /// the lines rag against is unmistakable. Wraps once at the render
    /// width, after "you" (the sanity checks below say so).
    private static let twoLineBody = "Yes I totally agree with you about that"

    @Test("an own reply of three or more lines rags its lines LEFT")
    func longOwnReplyRagsLeading() throws {
        // Self.body is the four-plus-line fixture the truncation test uses.
        let lines = try textBands(body: Self.body)
        #expect(lines.count >= 3, "fixture too short to cross the three-line threshold")
        let widest = try #require(lines.max { $0.width < $1.width })
        let shortest = try #require(lines.min { $0.width < $1.width })
        #expect(shortest.width < widest.width - 20, "fixture lines are too alike to tell an edge")

        #expect(
            abs(shortest.left - widest.left) <= 4,
            """
            a long own reply's lines do not share a left edge (shortest at             \(shortest.left), widest at \(widest.left)) — three or more             lines must go back to leading alignment
            """)
        #expect(
            shortest.right < widest.right - 20,
            """
            a long own reply's shortest line still reaches the right edge             (\(shortest.right) vs \(widest.right)) — it is right-ragged,             which is the rule for two lines at most
            """)
    }

    @Test("an own reply of one or two lines still rags its lines RIGHT")
    func shortOwnReplyRagsTrailing() throws {
        let lines = try textBands(body: Self.twoLineBody)
        #expect(lines.count == 2, "fixture wrapped to \(lines.count) line(s), not the two it needs")
        let widest = try #require(lines.max { $0.width < $1.width })
        let shortest = try #require(lines.min { $0.width < $1.width })
        #expect(shortest.width < widest.width - 20, "fixture lines are too alike to tell an edge")

        #expect(
            abs(shortest.right - widest.right) <= 4,
            """
            a short own reply's lines do not share a right edge (shortest at             \(shortest.right), widest at \(widest.right)) — through two             lines the body stays trailing-aligned
            """)
        #expect(
            shortest.left > widest.left + 20,
            """
            a short own reply's shortest line starts at the left edge             (\(shortest.left) vs \(widest.left)) — it is left-ragged, which             is the rule from three lines on
            """)
    }

    /// The body's line bands of an own reply, hosted for real: every band
    /// except the quote block, which is the tallest (its accent bar welds
    /// its lines) and always sits on the leading edge whatever the body
    /// does.
    private func textBands(body: String) throws -> [Band] {
        let measured = try bands(in: renderHosted(body: body, replyTo: quote))
        let tallest = try #require(measured.max { $0.height < $1.height })
        return measured.filter { $0.start != tallest.start }
    }

    /// The reply balloon in a UIWindow, laid out until the alignment
    /// state written by onGeometryChange has fed back into layout, then
    /// captured. ImageRenderer would draw the first pass only — the one
    /// before the body's height has been read — and always show the
    /// trailing (unmeasured) alignment.
    private func renderHosted(body: String, replyTo: ReplyToSnapshot?) throws -> CGImage {
        let canvas = CGRect(x: 0, y: 0, width: Self.renderWidth, height: 600)
        let host = UIHostingController(
            rootView: bubble(body: body, replyTo: replyTo)
                .frame(height: canvas.height, alignment: .top))
        // Transparent, like ImageRenderer's canvas: the band reader takes
        // any light pixel for ink, and a white view background would be
        // one band from top to bottom.
        host.view.backgroundColor = .clear
        // A window outside every scene is never composited on iOS 13+,
        // and an uncomposited window draws nothing — so it joins the test
        // host's scene.
        let scene = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: canvas)
        window.frame = canvas
        window.backgroundColor = .clear
        window.rootViewController = host
        window.isHidden = false
        defer { window.isHidden = true }

        // Layout, state write, layout again: two passes settle it, the
        // third is slack. Each spin lets SwiftUI apply the pending @State.
        for _ in 0..<3 {
            window.layoutIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        }

        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = false
        let image = UIGraphicsImageRenderer(bounds: canvas, format: format).image { context in
            // The layer tree, not drawHierarchy: the latter needs the
            // window to have been through the compositor, which a
            // just-shown test window may not have been.
            window.layer.render(in: context.cgContext)
        }
        return try #require(image.cgImage, "the hosted bubble did not render")
    }
}

#endif
