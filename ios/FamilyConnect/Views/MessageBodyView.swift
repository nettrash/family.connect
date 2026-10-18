//
//  MessageBodyView.swift
//  FamilyConnect
//
//  What a balloon draws where its text goes.
//
//  PLATFORM-FREE, and it has to be: a message written on one client must
//  read the same on the other, and a second copy of the table layout would
//  be a second subset within a week. Only the two things that genuinely
//  differ — the font and colour of a line of body text, and where the
//  streaming cursor is drawn — come in from the call site, as the closure
//  each platform already had inline.
//
//  ONE TEXT BLOCK MUST STAY ONE `Text`. A body with no table is a single
//  block, and this view then emits the caller's `Text` and nothing else:
//  no stack, no container, no extra layout pass. The bubble's `\.openURL`
//  arbitration, its link hit test, the reply-truncation fix pinned by
//  BubbleLayoutTests and the non-lazy scroll window are all built on that
//  one view being the whole body — see MessageMarkdown for the long
//  version.
//
//  A text block with INLINE FORMULAS (#97) is still that one `Text`: the
//  formulas' drawings go inside it (RichInlineText), so the caller gets a
//  `Text` either way and decorates it exactly as before. A RICH block —
//  a display formula, a diagram, a graph — is its own block, like a table
//  (RichBlockView).
//

import SwiftUI

/// The blocks of one message body, in the order they were typed.
struct MessageBodyBlocks<TextBlock: View>: View {
    let blocks: [MessageMarkdown.Block]
    /// The assistant is still writing this one, so the body ends in a
    /// cursor.
    var isStreaming: Bool = false
    /// Own balloons are filled with the tint, which the table's hairline
    /// has to survive — and a formula is drawn in the balloon's own text
    /// colour, which differs between mine and theirs.
    var isMine: Bool = false
    /// The balloon's double-tap (the quick heart) and long press, for a
    /// diagram's own tap to leave reachable (RichBlockView).
    var onDoubleTap: () -> Void = {}
    var onLongPress: (() -> Void)?
    /// Draws one text block, handed over as the `Text` to decorate. The
    /// flag is true for the block the streaming cursor rides, so a caller
    /// that draws one puts it in exactly one place.
    @ViewBuilder let textBlock: (Text, Bool) -> TextBlock

    @Environment(\.colorScheme) private var colorScheme
    /// The body text size a formula is set at: Dynamic Type's on iOS, a
    /// Mac's fixed 13 points.
    #if os(iOS)
    @ScaledMetric(relativeTo: .body) private var bodySize: CGFloat = 17
    #else
    private let bodySize: CGFloat = 13
    #endif

    private var look: RichLook {
        RichLook(isMine: isMine, dark: colorScheme == .dark, fontSize: Double(bodySize))
    }

    var body: some View {
        if blocks.count == 1, case .text(let only) = blocks[0] {
            // The whole body, as the one `Text` the bubble is built
            // around. Not a stack of one: a container here would change
            // how the caller's `.frame` and `.fixedSize` resolve, which is
            // the reply-truncation bug all over again.
            text(only, carriesCursor: true)
        } else {
            VStack(alignment: .leading, spacing: 6) {
                ForEach(Array(blocks.enumerated()), id: \.offset) { index, block in
                    switch block {
                    case .text(let rendered):
                        text(rendered, carriesCursor: index == cursorBlock)
                    case .table(let table):
                        MarkdownTableView(table: table, isMine: isMine)
                    case .rich(let rich):
                        RichBlockView(
                            block: rich, look: look, onDoubleTap: onDoubleTap, onLongPress: onLongPress)
                    }
                }
                if isStreaming, cursorBlock == nil {
                    // The body ends in a table or a drawing, and a cursor
                    // blinking inside a cell would read as part of the
                    // data. It gets a line of its own instead.
                    textBlock(Text(verbatim: ""), true)
                }
            }
        }
    }

    /// One text block, its formulas drawn in once they are — and asked for
    /// while they are not. A block with no formula is `Text(attributed)`
    /// and no more, the shape every bubble had before #97.
    @ViewBuilder
    private func text(_ attributed: AttributedString, carriesCursor: Bool) -> some View {
        let jobs = RichInlineText.jobs(in: attributed, look: look)
        if jobs.isEmpty {
            textBlock(Text(attributed), carriesCursor)
        } else {
            // `.task` lays nothing out, so the caller's Text is still the
            // whole of the view as far as layout is concerned.
            textBlock(RichInlineText.text(attributed, look: look), carriesCursor)
                .task(id: jobs) {
                    for job in jobs { await RichRenderer.shared.draw(job) }
                }
        }
    }

    /// The block the cursor rides: the last one, when it is text.
    private var cursorBlock: Int? {
        guard let last = blocks.indices.last, case .text = blocks[last] else { return nil }
        return last
    }
}

/// A GFM pipe table inside a balloon.
///
/// NO SCROLL VIEW, ever. The table fits the balloon and its cells wrap: a
/// nested scrollable inside the clients' bounded non-lazy row window is
/// the exact shape that produced two captured macOS hang reports, and a
/// table that wraps costs a few more real rows and nothing else.
///
/// EVERY CELL IS WIDTH-GREEDY, so the columns share the balloon evenly.
/// That is not a look, it is what makes the table MEASURABLE: left to size
/// its columns from their content, `Grid` divides the width one way while
/// measuring and another while drawing, and a cell that wraps to four
/// lines is reported as two (measured at 288pt: 96pt reported against
/// 172pt drawn). The balloon then draws itself short and the message
/// overflows the bubble it is in. Even columns are also exactly what
/// Android's weighted `Row`s give, so the two clients agree on the shape.
struct MarkdownTableView: View {
    let table: MessageMarkdown.Table
    var isMine: Bool = false

    var body: some View {
        Grid(alignment: .topLeading, horizontalSpacing: 10, verticalSpacing: 4) {
            GridRow {
                ForEach(Array(table.header.enumerated()), id: \.offset) { column, text in
                    cell(text, column: column, isHeader: true)
                }
            }
            // A view outside a GridRow spans every column, which is what
            // makes this one hairline rather than one per cell. Unsized
            // horizontally so a rule can never be what decides how wide
            // the table is.
            Rectangle()
                .fill(ruleColor)
                .frame(height: 1)
                .gridCellUnsizedAxes(.horizontal)
            ForEach(Array(table.rows.enumerated()), id: \.offset) { _, row in
                GridRow {
                    ForEach(Array(row.enumerated()), id: \.offset) { column, text in
                        cell(text, column: column, isHeader: false)
                    }
                }
            }
        }
    }

    private func cell(_ text: AttributedString, column: Int, isHeader: Bool) -> some View {
        Text(text)
            .bold(isHeader)
            .multilineTextAlignment(textAlignment(column))
            // Cells WRAP. Without this a Grid row that has to give up
            // height sheds lines from its text instead of growing, which
            // is the same squeeze that used to truncate a reply's body —
            // measured here as a cell that draws 81pt of a 140pt answer.
            .fixedSize(horizontal: false, vertical: true)
            // Width-greedy, for the reason in the note on this view: it is
            // what makes the columns measure the way they draw. The
            // alignment rides here too, which is why no cell declares a
            // `gridColumnAlignment` — filling its column, each cell places
            // its own text.
            .frame(maxWidth: .infinity, alignment: alignment(column))
    }

    /// The hairline under the header. `separator` is a dark grey, which is
    /// all but invisible on a tinted own balloon, so there it follows the
    /// content colour — the same call the quote bar makes on both
    /// platforms.
    private var ruleColor: Color {
        isMine ? Color.white.opacity(0.5) : Color.appSeparator
    }

    private func alignment(_ column: Int) -> Alignment {
        switch table.alignment(column) {
        case .leading: return .leading
        case .center: return .center
        case .trailing: return .trailing
        }
    }

    /// A wrapped cell has lines of its own to align, which is a separate
    /// question from where the column sits.
    private func textAlignment(_ column: Int) -> TextAlignment {
        switch table.alignment(column) {
        case .leading: return .leading
        case .center: return .center
        case .trailing: return .trailing
        }
    }
}
