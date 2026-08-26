//
//  ReplyBalloonLayout.swift
//  FamilyConnect
//
//  The layout for an OWN reply's balloon content — the 2026-08 alignment
//  rule (body on the trailing edge, quote on the leading one) WITHOUT the
//  greedy width that rule was first shipped with.
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
//  only the body tags trailing; quote, attachments, cards and polls keep
//  the leading edge.
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
