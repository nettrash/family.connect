//
//  ReplyBalloonLayout.swift
//  FamilyConnect
//
//  The layout for a balloon's content — every balloon, since 2026-09-03;
//  before that only an OWN reply's — and the two rules it carries:
//
//  1. THE 2026-08 ALIGNMENT RULE for own replies (body on the trailing
//     edge, quote on the leading one) WITHOUT the greedy width that rule
//     was first shipped with — and, since the owner's follow-up, the
//     LINE-COUNT rule that decides whether the body rides trailing at all
//     (`ReplyBodyAlignment`, `OwnReplyBodyAlignment` below).
//
//  2. THE FILL RULE. A photo, a pile, a poll, a link card or a table has
//     already decided how wide a balloon is, and the caption under it
//     should wrap against that same edge instead of floating narrow above
//     it. That used to be done with `.frame(maxWidth: .infinity)` on the
//     caption — which is the documented full-width-slab regression, and it
//     WAS shipping: every album with a caption and every poll came out as
//     wide as the thread column, 488pt of balloon around a 195pt pile on an
//     iPad and the full row on a phone. A frame can only borrow the
//     PROPOSAL; the width wanted here is a SIBLING's, and only a Layout
//     can see a sibling. So the caption is now TAGGED (`balloonFillsWidth`)
//     and this layout measures in two passes: first everything that is not
//     a caption, whose widest member is the balloon's anchor; then the
//     captions, proposed that anchor width (never narrower than
//     `fillFloor`, so a caption under a tall portrait photo does not wrap
//     at 160pt). The container then hugs its widest child as before — a
//     short caption under a wide photo does not widen anything, and a long
//     one wraps where the photo ends.
//
//  Why a Layout and not a frame, in general: SwiftUI can pin a child to an
//  edge only inside a frame that already has a width, and the only width a
//  frame modifier can take beyond the child's own is the PROPOSAL —
//  maxWidth: .infinity — which is the slab regression above. Android never
//  had the problem: Compose's `Modifier.align(End)` positions a child
//  inside the column's MEASURED width, which its other children decide.
//  This Layout is that semantics in SwiftUI: the container hugs its widest
//  child, and each child then places against the edge it is tagged with.
//  Android is the reference — only the body ever tags trailing; quote,
//  attachments, cards and polls keep the leading edge.
//
//  THE LINE-COUNT RULE. A short answer under a quote reads as a reply
//  when it sits on the right; a paragraph does not — three or more
//  right-ragged lines read as a typesetting accident, and the eye has to
//  hunt for each line's start. So the body rides trailing only while it
//  fits in `maxTrailingLines`, and goes back to the leading edge (left-
//  ragged, leading-tagged) the moment it wraps past that. The count is
//  MEASURED, never guessed from characters: the body's rendered height
//  over one line's height in the same font, which survives Dynamic Type,
//  a new system font and emoji ladders alike.
//

import SwiftUI

/// Which edge a balloon child hugs inside `BalloonContentLayout`.
nonisolated enum BalloonEdge {
    case leading
    case trailing
}

nonisolated struct BalloonEdgeKey: LayoutValueKey {
    static let defaultValue: BalloonEdge = .leading
}

/// Whether a balloon child is a CAPTION — text that takes its width from
/// the widest of its siblings rather than from the proposal (the fill
/// rule in the header). Everything else — photos, piles, polls, cards,
/// quotes, reaction chips — is an anchor: it is measured first, and the
/// widest anchor is the width the captions are offered.
nonisolated struct BalloonFillKey: LayoutValueKey {
    static let defaultValue = false
}

extension View {
    /// Tag a balloon child with the edge it hugs in an own reply.
    func balloonEdge(_ edge: BalloonEdge) -> some View {
        layoutValue(key: BalloonEdgeKey.self, value: edge)
    }

    /// Tag a balloon child as a caption that wraps against its widest
    /// sibling (`BalloonContentLayout`'s fill rule). Ignored by any other
    /// container, which is deliberate: a plain VStack draws the text at its
    /// own width, and nothing gets wider.
    func balloonFillsWidth(_ fills: Bool) -> some View {
        layoutValue(key: BalloonFillKey.self, value: fills)
    }
}

/// A vertical stack that sizes to its WIDEST child (never the proposal)
/// and places each child against its tagged edge. `sizeThatFits` measures
/// every child at the proposed width so wrapping text answers with the
/// width it actually uses — which is what keeps a one-word reply's balloon
/// hugging its quote instead of taking the row.
///
/// Children tagged `balloonFillsWidth(true)` are measured LAST, at the
/// width of the widest untagged child (or `fillFloor` when that is wider),
/// so a caption wraps where the photo or the poll ends; with no untagged
/// sibling at all they get the proposal, exactly like an untagged child.
nonisolated struct BalloonContentLayout: Layout {
    var spacing: CGFloat = 2
    /// The narrowest measure a caption is offered when it has an anchor:
    /// a caption under a 160pt portrait photo wraps at this width, not at
    /// 160. Zero means "exactly the anchor".
    var fillFloor: CGFloat = 0

    func sizeThatFits(
        proposal: ProposedViewSize, subviews: Subviews, cache: inout ()
    ) -> CGSize {
        let sizes = measure(subviews, proposal: proposal)
        let width = sizes.map(\.width).max() ?? 0
        let height = sizes.map(\.height).reduce(0, +)
            + spacing * CGFloat(max(0, sizes.count - 1))
        return CGSize(width: width, height: height)
    }

    func placeSubviews(
        in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()
    ) {
        let sizes = measure(subviews, proposal: proposal)
        var y = bounds.minY
        for (subview, size) in zip(subviews, sizes) {
            let x = subview[BalloonEdgeKey.self] == .trailing
                ? bounds.maxX - size.width
                : bounds.minX
            subview.place(
                at: CGPoint(x: x, y: y),
                anchor: .topLeading,
                proposal: ProposedViewSize(width: size.width, height: size.height))
            y += size.height + spacing
        }
    }

    /// Every child's size, anchors first and captions second — the two
    /// passes the header describes. Pure over its inputs, so `sizeThatFits`
    /// and `placeSubviews` cannot disagree.
    private func measure(_ subviews: Subviews, proposal: ProposedViewSize) -> [CGSize] {
        let column = ProposedViewSize(width: proposal.width, height: nil)
        var sizes = [CGSize](repeating: .zero, count: subviews.count)
        var anchor: CGFloat = 0
        for (index, subview) in subviews.enumerated() where !subview[BalloonFillKey.self] {
            sizes[index] = subview.sizeThatFits(column)
            anchor = max(anchor, sizes[index].width)
        }
        // Never wider than the column itself: a floor above the proposal
        // would hand a caption more room than the balloon can have.
        let measure = anchor > 0
            ? min(max(anchor, fillFloor), proposal.width ?? .infinity)
            : (proposal.width ?? .infinity)
        let fill = ProposedViewSize(width: measure, height: nil)
        for (index, subview) in subviews.enumerated() where subview[BalloonFillKey.self] {
            sizes[index] = subview.sizeThatFits(fill)
        }
        return sizes
    }
}

/// The rule that decides which edge an own reply's BODY hugs, pure so it
/// can be pinned by a unit test without a renderer.
nonisolated enum ReplyBodyAlignment {
    /// The most lines a body may wrap to and still sit trailing. Three
    /// and more go back to the leading edge.
    static let maxTrailingLines = 2

    static func alignsTrailing(lineCount: Int) -> Bool {
        lineCount <= maxTrailingLines
    }

    /// How many lines a body of `bodyHeight` is, given one line's height
    /// in the same font. A ROUNDED ratio: SwiftUI reports a wrapped Text
    /// as close to N × the single-line height, but the two measurements
    /// arrive in separate layout passes and can differ by a point of
    /// leading, which a floor would turn into an off-by-one. Never fewer
    /// than one, and one — the safe answer, the old rule — for any
    /// measurement that has not happened yet or cannot (a zero or
    /// negative line height, a non-finite body).
    static func lineCount(bodyHeight: CGFloat, lineHeight: CGFloat) -> Int {
        guard lineHeight > 0, bodyHeight.isFinite, bodyHeight > 0 else { return 1 }
        return max(1, Int((bodyHeight / lineHeight).rounded()))
    }
}

/// The alignment-dependent modifiers an own reply's body text carries —
/// its line ragging (`multilineTextAlignment`) and the `balloonEdge` tag
/// `BalloonContentLayout` places it by — applied TOGETHER from one
/// measured line count, so they can never disagree with each other. It
/// also carries the caption tag (`balloonFillsWidth`) for every body, own
/// reply or not, because the fill rule and the edge rule are read by the
/// same layout and belong on the same view.
///
/// Sits AFTER the body's `.fixedSize(horizontal: false, vertical: true)`
/// and its streaming-cursor overlay, and BEFORE any frame: the height it
/// reads is the Text's own, which is the only height that counts lines.
/// The line height comes from a hidden reference `Text("Ag")` in the same
/// font, measured the same way, so no font metric is hard-coded anywhere.
///
/// `enabled == false` — every message that is not an own reply — measures
/// nothing at all and lays out plain leading, exactly what those bodies
/// always had. A body split around tables (MessageBodyBlocks) gets one of
/// these PER TEXT BLOCK, and each block decides for itself: a two-line
/// block before a table may sit trailing while the four-line block after
/// it sits leading. That is deliberate — the rule is about how a run of
/// lines reads, and each block is its own run.
///
/// No `.frame` any more, on purpose. The old `.frame(maxWidth: .infinity)`
/// that `fillsWidth` used to switch on is the slab regression the file
/// header describes; the width a caption wraps at is now the layout's
/// decision, and the edge it sits on is the layout's too.
struct OwnReplyBodyAlignment: ViewModifier {
    let enabled: Bool
    /// The body's font — nil where the platform leaves the default in
    /// place (the Mac's non-emoji bodies), which the reference Text then
    /// inherits the same way.
    let font: Font?
    /// A sibling block has already decided the balloon's width, so the
    /// body wraps against the same edge (the caller's `fillsBalloonWidth`).
    let fillsWidth: Bool

    @State private var bodyHeight: CGFloat = 0
    @State private var lineHeight: CGFloat = 0

    private var alignsTrailing: Bool {
        guard enabled else { return false }
        return ReplyBodyAlignment.alignsTrailing(
            lineCount: ReplyBodyAlignment.lineCount(bodyHeight: bodyHeight, lineHeight: lineHeight))
    }

    func body(content: Content) -> some View {
        if enabled {
            content
                .multilineTextAlignment(alignsTrailing ? .trailing : .leading)
                // The Text's own height, read before anything can change
                // it. Ragging does not change how a body wraps, so the
                // count this feeds cannot flip the alignment back and
                // forth.
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { bodyHeight = $0 }
                .background {
                    // One line of this font, however tall this SDK and
                    // this Dynamic Type size make it. `fixedSize` so it
                    // answers with its ideal single-line size even under a
                    // body narrower than the two glyphs.
                    Text(verbatim: "Ag")
                        .font(font)
                        .fixedSize()
                        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { lineHeight = $0 }
                        .hidden()
                }
                .balloonEdge(alignsTrailing ? .trailing : .leading)
                .balloonFillsWidth(fillsWidth)
        } else {
            content
                .multilineTextAlignment(.leading)
                .balloonEdge(.leading)
                .balloonFillsWidth(fillsWidth)
        }
    }
}

extension View {
    /// The own-reply body rule, in one place for both platforms: see
    /// `OwnReplyBodyAlignment`.
    func ownReplyBodyAlignment(enabled: Bool, font: Font?, fillsWidth: Bool) -> some View {
        modifier(OwnReplyBodyAlignment(enabled: enabled, font: font, fillsWidth: fillsWidth))
    }
}
