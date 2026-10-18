//
//  RichBlockView.swift
//  FamilyConnect
//
//  Formulas, diagrams and graphs in a balloon (issue #97; the design doc's
//  "In the bubble"). Shared by the phone and the Mac, like the rest of
//  MessageBodyView.
//
//  A RICH BLOCK is its own block, like a table: until it is drawn — and for
//  good if it cannot be — its source in the monospaced face, exactly as a
//  code block shows today (a display formula with its delimiters); drawn,
//  the picture, scaled down to the balloon but never up, a diagram's on a
//  card with rounded corners. Tapping a diagram or a graph opens it full
//  size and zoomable.
//
//  AN INLINE FORMULA is a picture in the run of text, on the line's
//  baseline: `RichInlineText` swaps each formula's run for its drawing
//  inside the ONE `Text` the bubble is built around (see MessageMarkdown on
//  why one Text is a constraint), so links, the double-tap arbitration and
//  wrapping all keep working. Until drawn, the run's own characters show —
//  the source with its delimiters, in the code face.
//
//  ACCESSIBILITY: a drawing's label is its source. Nothing here adds a
//  string to translate.
//

import SwiftUI

/// A rich block in a balloon.
struct RichBlockView: View {
    let block: MessageMarkdown.RichBlock
    let look: RichLook
    /// The balloon's own double-tap (the quick heart) and long press — a
    /// diagram's single tap must not swallow either.
    var onDoubleTap: () -> Void = {}
    var onLongPress: (() -> Void)?

    @State private var showsFull = false

    private var job: RichJob { RichJob(block: block, look: look) }

    var body: some View {
        // Read in the body, so this one block re-renders when its drawing
        // lands (RichSlot is observed on its own).
        let slot = RichRenderer.shared.slot(for: job)
        Group {
            if case .drawn(let drawing)? = slot.outcome {
                drawn(drawing)
            } else {
                source
            }
        }
        .task(id: job) { await RichRenderer.shared.draw(job) }
    }

    /// The source, as a code block shows it today.
    private var source: some View {
        Text(verbatim: block.fallback)
            .font(.system(.body, design: .monospaced))
            // Wraps rather than truncates, for the reason the body Text
            // gives (the reply-truncation fix).
            .fixedSize(horizontal: false, vertical: true)
    }

    @ViewBuilder
    private func drawn(_ drawing: RichDrawing) -> some View {
        let picture = ScaledDownImage(drawing: drawing)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(Text(verbatim: block.source))
        if block.kind == .math {
            // A formula is ink on no background: nothing to tap open.
            picture
                .accessibilityAddTraits(.isImage)
        } else {
            picture
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                .contentShape(Rectangle())
                #if os(macOS)
                .hoverCursor(.pointingHand)
                #endif
                // Count 2 BEFORE count 1, and both as onTapGesture: that is
                // what makes them exclusive (the photo tile's rule) — a
                // double tap hearts, a tap opens.
                .onTapGesture(count: 2) { onDoubleTap() }
                .onTapGesture(count: 1) { showsFull = true }
                #if os(iOS)
                .simultaneousGesture(LongPressGesture().onEnded { _ in onLongPress?() })
                #endif
                .accessibilityAddTraits([.isImage, .isButton])
                // A bare gesture publishes no accessibility action
                // (ZZAXProbeTests); this is what makes the trait true.
                .accessibilityAction { showsFull = true }
                #if os(iOS)
                .fullScreenCover(isPresented: $showsFull) {
                    RichDrawingViewer(drawing: drawing, label: block.source)
                }
                #else
                .sheet(isPresented: $showsFull) {
                    RichDrawingViewer(drawing: drawing, label: block.source)
                }
                #endif
        }
    }
}

/// A drawing at its natural size, or narrower when the balloon is — never
/// wider. Its height follows its width, so it is NOT flexible in height:
/// a balloon distributing height (the reply-quote case) cannot squeeze it.
private struct ScaledDownImage: View {
    let drawing: RichDrawing

    var body: some View {
        FitWidthLayout(natural: CGSize(width: drawing.width, height: drawing.height)) {
            Image(decorative: drawing.image, scale: drawing.scale)
                .resizable()
                .interpolation(.high)
        }
    }
}

/// One child, offered `min(proposed width, natural width)` and the height
/// that keeps the natural aspect ratio.
private struct FitWidthLayout: Layout {
    let natural: CGSize

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = min(proposal.width ?? natural.width, natural.width)
        let height = natural.width > 0 ? natural.height * width / natural.width : natural.height
        return CGSize(width: max(width, 0), height: max(height, 0))
    }

    func placeSubviews(
        in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()
    ) {
        for subview in subviews {
            subview.place(at: bounds.origin, proposal: ProposedViewSize(bounds.size))
        }
    }
}

/// A text block with inline formulas: each formula's run is its drawing
/// once there is one, inside the one `Text`.
enum RichInlineText {

    /// The jobs a text block's formulas need, in order — empty for the
    /// common block with none.
    static func jobs(in text: AttributedString, look: RichLook) -> [RichJob] {
        text.runs.compactMap { run in
            run[MessageMarkdown.InlineMathAttribute.self].map { RichJob(inlineFormula: $0.source, look: look) }
        }
    }

    /// The block as one `Text`: drawn formulas as pictures on the baseline,
    /// everything else — and every formula not yet drawn — as the
    /// attributed runs it already was.
    ///
    /// Built by INTERPOLATION into a `Text`, which is how SwiftUI takes a
    /// picture into a run of text (`Text + Text` is deprecated in the iOS
    /// 26 SDK). The interpolation is assembled in code, not written as a
    /// literal, so no "%@%@" key is ever extracted into the string catalog;
    /// at run time the key matches nothing and is used as it is.
    static func text(_ attributed: AttributedString, look: RichLook) -> Text {
        let formulas = attributed.runs.filter { $0[MessageMarkdown.InlineMathAttribute.self] != nil }
        guard !formulas.isEmpty else { return Text(attributed) }
        var pieces: [Text] = []
        var pending = AttributedString()
        for run in attributed.runs {
            guard let formula = run[MessageMarkdown.InlineMathAttribute.self],
                  case .drawn(let drawing)? = RichRenderer.shared.slot(
                    for: RichJob(inlineFormula: formula.source, look: look)).outcome
            else {
                pending.append(attributed[run.range])
                continue
            }
            if !pending.characters.isEmpty {
                pieces.append(Text(pending))
                pending = AttributedString()
            }
            // The picture's bottom sits on the baseline; lowering it by
            // what lies below the formula's own baseline lines the two up.
            pieces.append(
                Text(Image(drawing.image, scale: drawing.scale, label: Text(verbatim: formula.source)))
                    .baselineOffset(-(drawing.height - drawing.baseline)))
        }
        if !pending.characters.isEmpty { pieces.append(Text(pending)) }
        guard pieces.count > 1 else { return pieces.first ?? Text(attributed) }
        var interpolation = LocalizedStringKey.StringInterpolation(literalCapacity: 0, interpolationCount: pieces.count)
        for piece in pieces { interpolation.appendInterpolation(piece) }
        return Text(LocalizedStringKey(stringInterpolation: interpolation))
    }
}

/// A diagram or a graph, full size and zoomable: pinch (or a trackpad's
/// pinch), double-tap to zoom in and out, scroll to pan.
///
/// Not the photo viewer: that one pages an album, streams video and shares
/// an attachment the server holds, none of which a drawing has. What it
/// shares with it is the gesture vocabulary.
struct RichDrawingViewer: View {
    let drawing: RichDrawing
    /// The source — the picture's accessibility label.
    let label: String

    @Environment(\.dismiss) private var dismiss
    @State private var zoom: CGFloat = 1
    @State private var pinchBase: CGFloat = 1

    private static let maxZoom: CGFloat = 4

    var body: some View {
        NavigationStack {
            GeometryReader { geometry in
                // Full size — or the whole of it, when full size is bigger
                // than the screen; zoom goes on from there.
                let fit = min(
                    1, geometry.size.width / max(drawing.width, 1),
                    geometry.size.height / max(drawing.height, 1))
                ScrollView([.horizontal, .vertical]) {
                    Image(decorative: drawing.image, scale: drawing.scale)
                        .resizable()
                        .interpolation(.high)
                        .frame(width: drawing.width * fit * zoom, height: drawing.height * fit * zoom)
                        .accessibilityLabel(Text(verbatim: label))
                        .frame(minWidth: geometry.size.width, minHeight: geometry.size.height)
                }
                .scrollIndicators(.hidden)
                .gesture(
                    MagnifyGesture()
                        .onChanged { value in
                            zoom = min(max(pinchBase * value.magnification, 1), Self.maxZoom)
                        }
                        .onEnded { _ in pinchBase = zoom })
                .onTapGesture(count: 2) {
                    withAnimation(.snappy(duration: 0.2)) { zoom = zoom > 1 ? 1 : 2.5 }
                    pinchBase = zoom
                }
            }
            .background(Color.appGroupedBackground)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                        .keyboardShortcut(.cancelAction)
                }
            }
        }
        #if os(macOS)
        // A sheet cannot be resized on a Mac, so it opens at the drawing's
        // size, within reason.
        .frame(
            width: min(max(drawing.width + 40, 420), 1000),
            height: min(max(drawing.height + 100, 320), 760))
        #endif
    }
}
