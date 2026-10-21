//
//  ComposerRowLayout.swift
//  FamilyConnect
//
//  The phone's and the iPad's message row as a Layout (#106,
//  docs/composer-and-wide-layout-2026-10-20.md, 2.1): the paperclip, the
//  field and the slot on one line — or, when the controls would leave the
//  field less than `ComposerRow.shortestTextRun` (Dynamic Type AX3 and
//  above), the field on a line of its own with the paperclip and the slot
//  below it at the two ends. Plus the voice row, laid over the paperclip and
//  the field while a voice message records (S2.3), wherever they are.
//
//  WHY A LAYOUT AND NOT `ViewThatFits`. ViewThatFits picks between two
//  SUBTREES, so the message field would exist twice — two TextFields on one
//  focus binding, and a field that is a different view, with the keyboard
//  dropped, every time the choice flips (a rotation at a large text size
//  does it). A Layout moves ONE field: its identity, its focus and its
//  caret survive the change of arrangement because nothing about the view
//  changes but where it is put. It also answers from the real widths the
//  controls have at the current text size, rather than from a guess about
//  which sizes are "large".
//
//  Children say which part they are with `.composerRowRole(_:)`; a child
//  with no role is the cover (the voice row). Any child may be absent.
//

import SwiftUI

struct ComposerRowLayout: Layout {

    /// The gap between the parts, both ways.
    var spacing: CGFloat
    /// What the field spends on anything that is not text — its leading
    /// padding plus the room its glyphs take (`ComposerRow.trailingInset`).
    /// Fed the WIDEST the field's glyphs get in this chat, not the current
    /// count: the video button leaves at the first character typed, and a
    /// row that re-arranged itself at that moment would move under the
    /// thumb, which is the one thing #106 set out to stop.
    var fieldChrome: CGFloat
    /// The shortest text run the field may keep beside the controls —
    /// `ComposerRow.shortestTextRun`, scaled with the text size.
    var textRun: CGFloat = ComposerRow.shortestTextRun

    enum Role: Sendable {
        case paperclip, field, slot, cover
    }

    fileprivate struct RoleKey: LayoutValueKey {
        static let defaultValue: Role = .cover
    }

    private struct Parts {
        var paperclip: LayoutSubview?
        var field: LayoutSubview?
        var slot: LayoutSubview?
        var cover: LayoutSubview?

        init(_ subviews: LayoutSubviews) {
            for subview in subviews {
                switch subview[RoleKey.self] {
                case .paperclip: paperclip = subview
                case .field: field = subview
                case .slot: slot = subview
                case .cover: cover = subview
                }
            }
        }
    }

    private struct Frames {
        var size: CGSize = .zero
        var paperclip: CGRect = .zero
        var field: CGRect = .zero
        var slot: CGRect = .zero
        var cover: CGRect = .zero
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        frames(width: proposal.width, parts: Parts(subviews)).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let parts = Parts(subviews)
        let frames = frames(width: bounds.width, parts: parts)
        func place(_ subview: LayoutSubview?, _ frame: CGRect) {
            subview?.place(
                at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY),
                anchor: .topLeading,
                proposal: ProposedViewSize(frame.size))
        }
        place(parts.paperclip, frames.paperclip)
        place(parts.field, frames.field)
        place(parts.slot, frames.slot)
        place(parts.cover, frames.cover)
    }

    private func frames(width proposed: CGFloat?, parts: Parts) -> Frames {
        // The paperclip and the slot are fixed boxes (scaled with the text
        // size); their ideal IS their size.
        let paperclip = parts.paperclip?.sizeThatFits(.unspecified) ?? .zero
        let slot = parts.slot?.sizeThatFits(.unspecified) ?? .zero
        let leadingGap = parts.paperclip == nil ? 0 : spacing
        let trailingGap = parts.slot == nil ? 0 : spacing

        // Asked for an ideal width (a fixedSize parent): one line, with the
        // field at its own ideal.
        let width = proposed ?? {
            let field = parts.field?.sizeThatFits(.unspecified).width ?? 0
            return paperclip.width + leadingGap + field + trailingGap + slot.width
        }()

        var out = Frames()
        switch ComposerRow.arrangement(
            width: width, paperclip: paperclip.width, slot: slot.width,
            spacing: spacing, fieldChrome: fieldChrome, textRun: textRun) {
        case .inline:
            let fieldWidth = max(0, width - paperclip.width - leadingGap - slot.width - trailingGap)
            let field = parts.field?.sizeThatFits(ProposedViewSize(width: fieldWidth, height: nil))
                ?? .zero
            let height = max(paperclip.height, field.height, slot.height)
            // Bottom-aligned, as the HStack was: the paperclip and the slot
            // stay level with the field's LAST line as a draft grows.
            out.paperclip = CGRect(x: 0, y: height - paperclip.height,
                                   width: paperclip.width, height: paperclip.height)
            out.field = CGRect(x: paperclip.width + leadingGap, y: height - field.height,
                               width: fieldWidth, height: field.height)
            out.slot = CGRect(x: width - slot.width, y: height - slot.height,
                              width: slot.width, height: slot.height)
            out.cover = coverFrame(over: paperclip.width + leadingGap + fieldWidth, bottom: height,
                                   cover: parts.cover)
            out.size = CGSize(width: width, height: height)
        case .stacked:
            let field = parts.field?.sizeThatFits(ProposedViewSize(width: width, height: nil)) ?? .zero
            let line = max(paperclip.height, slot.height)
            let height = field.height + (line > 0 ? spacing + line : 0)
            out.field = CGRect(x: 0, y: 0, width: width, height: field.height)
            out.paperclip = CGRect(x: 0, y: height - paperclip.height,
                                   width: paperclip.width, height: paperclip.height)
            out.slot = CGRect(x: width - slot.width, y: height - slot.height,
                              width: slot.width, height: slot.height)
            // The voice row takes the controls' line — beside the slot, as
            // it always sat — and the field's line above it stays where it
            // is, emptied, so nothing jumps when the recording ends.
            out.cover = coverFrame(over: max(0, width - slot.width - trailingGap), bottom: height,
                                   cover: parts.cover)
            out.size = CGSize(width: width, height: height)
        }
        return out
    }

    /// The voice row's frame: as wide as what it covers, at its own height,
    /// on the bottom edge — the ZStack(alignment: .bottom) it replaces.
    private func coverFrame(over width: CGFloat, bottom: CGFloat, cover: LayoutSubview?) -> CGRect {
        guard let cover else { return .zero }
        let height = min(bottom, cover.sizeThatFits(ProposedViewSize(width: width, height: nil)).height)
        return CGRect(x: 0, y: bottom - height, width: width, height: height)
    }
}

extension View {
    /// Which part of `ComposerRowLayout` this child is.
    func composerRowRole(_ role: ComposerRowLayout.Role) -> some View {
        layoutValue(key: ComposerRowLayout.RoleKey.self, value: role)
    }
}
