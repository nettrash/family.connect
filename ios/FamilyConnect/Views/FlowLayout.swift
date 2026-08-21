//
//  FlowLayout.swift
//  FamilyConnect
//
//  A left-to-right flow: subviews fill a row at their ideal size and wrap
//  to the next when the proposed width runs out — what the reaction-chip
//  row needs once a message collects more chips than fit beside a bubble
//  (an HStack would overflow the screen instead).
//
//  Sizing mirrors an HStack when everything fits on one line: the layout
//  reports its content size (widest row, summed row heights), not the
//  full proposal, so the bubble column keeps hugging its content.
//  `rowAlignment` places each wrapped row against the leading or trailing
//  edge — trailing under own bubbles, leading under others', matching the
//  column alignment.
//

import SwiftUI

nonisolated struct FlowLayout: Layout {
    var rowAlignment: HorizontalAlignment = .leading
    var spacing: CGFloat = 4

    /// Ideal subview sizes, measured once per subview set and rounded UP
    /// to whole points. Inside a lazy container a layout whose reported
    /// size jitters by fractions of a point between passes makes the lazy
    /// placement cache invalidate — and with an animation anywhere on the
    /// row, re-layout every frame, forever (a 100%-CPU spin observed as
    /// "main thread busy"). Integral, memoized measurements are stable by
    /// construction.
    struct Cache {
        var sizes: [CGSize]
    }

    func makeCache(subviews: Subviews) -> Cache {
        Cache(sizes: Self.measure(subviews))
    }

    func updateCache(_ cache: inout Cache, subviews: Subviews) {
        cache.sizes = Self.measure(subviews)
    }

    private static func measure(_ subviews: Subviews) -> [CGSize] {
        subviews.map { subview in
            let raw = subview.sizeThatFits(.unspecified)
            return CGSize(width: raw.width.rounded(.up), height: raw.height.rounded(.up))
        }
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout Cache) -> CGSize {
        let rows = rows(fitting: proposal.width ?? .infinity, sizes: cache.sizes)
        let width = rows.map(\.width).max() ?? 0
        let height = rows.reduce(0) { $0 + $1.height } + spacing * CGFloat(max(0, rows.count - 1))
        return CGSize(width: width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout Cache) {
        var y = bounds.minY
        for row in rows(fitting: bounds.width, sizes: cache.sizes) {
            var x = rowAlignment == .trailing ? bounds.maxX - row.width : bounds.minX
            for element in row.elements {
                let subview = subviews[element.index]
                subview.place(
                    at: CGPoint(x: x, y: y + (row.height - element.size.height) / 2),
                    proposal: ProposedViewSize(element.size))
                x += element.size.width + spacing
            }
            y += row.height + spacing
        }
    }

    // MARK: - Row assembly

    private struct Row {
        var elements: [(index: Int, size: CGSize)] = []
        var width: CGFloat = 0
        var height: CGFloat = 0
    }

    /// Greedy fill: each subview at its (cached, integral) ideal size,
    /// wrapping before a subview that would overflow `maxWidth` (a subview
    /// wider than the whole row still gets a row to itself rather than
    /// vanishing).
    private func rows(fitting maxWidth: CGFloat, sizes: [CGSize]) -> [Row] {
        var rows: [Row] = []
        var current = Row()
        for (index, size) in sizes.enumerated() {
            let widthIfAdded = current.elements.isEmpty ? size.width : current.width + spacing + size.width
            if !current.elements.isEmpty && widthIfAdded > maxWidth {
                rows.append(current)
                current = Row()
            }
            current.width = current.elements.isEmpty ? size.width : current.width + spacing + size.width
            current.height = max(current.height, size.height)
            current.elements.append((index, size))
        }
        if !current.elements.isEmpty {
            rows.append(current)
        }
        return rows
    }
}
