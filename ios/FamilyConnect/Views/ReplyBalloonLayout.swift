//
//  ReplyBalloonLayout.swift
//  FamilyConnect
//
//  The layout for an OWN reply's balloon content — the 2026-08 alignment
//  rule (body on the trailing edge, quote on the leading one) WITHOUT the
//  greedy width that rule was first shipped with — and, since the owner's
//  follow-up, the LINE-COUNT rule that decides whether the body rides
//  trailing at all (`ReplyBodyAlignment`, `OwnReplyBodyAlignment` below).
//
//  Why a Layout and not a frame: SwiftUI can pin a child to an edge only
//  inside a frame that already has a width, and the only width a frame
//  modifier can take beyond the child's own is the PROPOSAL — maxWidth:
//  .infinity — which is the documented "every reply balloon came out a
//  full-width slab" regression, reintroduced for the own-reply subset when
//  this rule first landed. Android never had the problem: Compose's
//  `Modifier.align(End)` positions a child inside the column's MEASURED
//  width, which its other children decide. This Layout is that semantics
//  in SwiftUI: the container hugs its widest child, and each child then
//  places against the edge it is tagged with. Android is the reference —
//  only the body ever tags trailing; quote, attachments, cards and polls
//  keep the leading edge.
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

/// Which edge a balloon child hugs inside `ReplyContentLayout`. Ignored
/// entirely when the content is laid out by a plain VStack (every message
/// that is not an own reply).
nonisolated enum BalloonEdge {
    case leading
    case trailing
}

nonisolated struct BalloonEdgeKey: LayoutValueKey {
    static let defaultValue: BalloonEdge = .leading
}

extension View {
    /// Tag a balloon child with the edge it hugs in an own reply.
    func balloonEdge(_ edge: BalloonEdge) -> some View {
        layoutValue(key: BalloonEdgeKey.self, value: edge)
    }
}

/// A vertical stack that sizes to its WIDEST child (never the proposal)
/// and places each child against its tagged edge. `sizeThatFits` measures
/// every child at the proposed width so wrapping text answers with the
/// width it actually uses — which is what keeps a one-word reply's balloon
/// hugging its quote instead of taking the row.
nonisolated struct ReplyContentLayout: Layout {
    var spacing: CGFloat = 2

    func sizeThatFits(
        proposal: ProposedViewSize, subviews: Subviews, cache: inout ()
    ) -> CGSize {
        let child = ProposedViewSize(width: proposal.width, height: nil)
        var width: CGFloat = 0
        var height: CGFloat = 0
        for (index, subview) in subviews.enumerated() {
            let size = subview.sizeThatFits(child)
            width = max(width, size.width)
            height += size.height + (index > 0 ? spacing : 0)
        }
        return CGSize(width: width, height: height)
    }

    func placeSubviews(
        in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()
    ) {
        let child = ProposedViewSize(width: proposal.width, height: nil)
        var y = bounds.minY
        for subview in subviews {
            let size = subview.sizeThatFits(child)
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

/// The three alignment-dependent modifiers an own reply's body text
/// carries — its line ragging (`multilineTextAlignment`), the edge of
/// its width-filling frame, and the `balloonEdge` tag
/// `ReplyContentLayout` places it by — applied TOGETHER from one measured
/// line count, so they can never disagree with each other.
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
struct OwnReplyBodyAlignment: ViewModifier {
    let enabled: Bool
    /// The body's font — nil where the platform leaves the default in
    /// place (the Mac's non-emoji bodies), which the reference Text then
    /// inherits the same way.
    let font: Font?
    /// A sibling block has already decided the balloon's width, so the
    /// body fills it and wraps against the same edge (the caller's
    /// `fillsBalloonWidth`).
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
                // The Text's own height, read before the frame below can
                // change it. Ragging does not change how a body wraps, so
                // the count this feeds cannot flip the alignment back and
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
                .frame(
                    maxWidth: fillsWidth ? .infinity : nil,
                    alignment: alignsTrailing ? .trailing : .leading)
                .balloonEdge(alignsTrailing ? .trailing : .leading)
        } else {
            content
                .multilineTextAlignment(.leading)
                .frame(maxWidth: fillsWidth ? .infinity : nil, alignment: .leading)
                .balloonEdge(.leading)
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
