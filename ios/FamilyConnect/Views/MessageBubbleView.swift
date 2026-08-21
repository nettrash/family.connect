//
//  MessageBubbleView.swift
//  FamilyConnect
//
//  One bubble. Mine: trailing, tint background, white text. Theirs:
//  leading, secondarySystemFill — semantic colors so both appearances
//  work without a palette. Below the bubble: HH:mm plus, on own
//  messages, the delivery glyph ladder —
//
//    clock            pending (optimistic row, not yet on the server)
//    checkmark        on the server (ack/echo/REST confirmed)
//    double checkmark someone else read it (serverID ≤ othersReadUpTo),
//                     tinted; composed from two offset SF checkmarks
//                     because the system font has no double-check glyph
//    red exclamation  failed — tap to retry; long-press for the sheet
//                     with Retry/Delete
//
//  The sender-name caption (family chat, sender change — rules in
//  MessagePresentation) renders above the bubble in the accent color.
//
//  Reactions: aggregated chips (emoji + count, tinted when the current
//  user is included — aggregation rules in MessagePresentation) render
//  between the bubble and the timestamp in a wrapping FlowLayout, so
//  many chips fold to new lines instead of overflowing. Tapping a chip
//  toggles that emoji; long-pressing a chip pops the "who reacted" list
//  (rows from MessagePresentation.reactionDetails, passed in by the
//  parent). Chips spring in and out keyed by their emoji, counts roll
//  with numericText, and a soft impact fires when the current user's own
//  reaction state changes — i.e. when their toggle lands.
//
//  Long-pressing the bubble itself calls `onLongPress` — the parent
//  floats its reaction picker over the bubble, finding it through the
//  BubbleAnchorKey bounds this view publishes under its localID.
//

import SwiftUI

struct MessageBubbleView: View {
    let message: MessageSnapshot
    let isMine: Bool
    let showsSenderName: Bool
    let senderName: String?
    let isRead: Bool
    var reactionChips: [ReactionChip] = []
    var reactionDetails: [ReactionDetail] = []
    var onRetry: () -> Void = {}
    var onDelete: () -> Void = {}
    var onToggleReaction: (String) -> Void = { _ in }
    var onLongPress: () -> Void = {}
    /// True only while THIS bubble hosts the floating reaction picker.
    /// Anchors are position-dependent, so publishing them from every
    /// bubble makes the overlay's preference re-evaluate on every
    /// scrolled frame — with tall bubbles that is real jank. Only the
    /// pressed bubble's anchor is ever read, so only it publishes.
    var publishesAnchor: Bool = false

    /// Drives the "who reacted" popover a chip long-press opens.
    @State private var showsReactionDetails = false

    var body: some View {
        HStack(alignment: .bottom, spacing: 0) {
            if isMine { Spacer(minLength: 48) }

            VStack(alignment: isMine ? .trailing : .leading, spacing: 2) {
                if showsSenderName, let senderName {
                    Text(senderName)
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.tint)
                        .padding(.horizontal, 4)
                }

                bubble

                if !reactionChips.isEmpty {
                    reactionRow
                }

                HStack(spacing: 4) {
                    Text(message.createdAt, format: .dateTime.hour(.twoDigits(amPM: .omitted)).minute(.twoDigits))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    if isMine {
                        statusGlyph
                    }
                }
                .padding(.horizontal, 4)
            }

            if !isMine { Spacer(minLength: 48) }
        }
        // A toggle landing = the set of chips that include me changing —
        // whether from the optimistic write or the server's echo.
        .sensoryFeedback(.impact(flexibility: .soft), trigger: reactionChips.filter(\.includesMe).map(\.emoji))
    }

    /// The aggregated reaction chips under the bubble, wrapping to new
    /// lines when they outgrow the column. A chip the current user is
    /// part of gets the tinted treatment; tapping any chip toggles that
    /// emoji for the current user; long-pressing one pops who reacted.
    private var reactionRow: some View {
        FlowLayout(rowAlignment: isMine ? .trailing : .leading, spacing: 4) {
            ForEach(reactionChips) { chip in
                Button {
                    onToggleReaction(chip.emoji)
                } label: {
                    HStack(spacing: 3) {
                        Text(chip.emoji)
                            .font(.footnote)
                        if chip.count > 1 {
                            Text("\(chip.count)")
                                .font(.caption2.weight(.medium))
                                .foregroundStyle(chip.includesMe ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                                .contentTransition(.numericText())
                        }
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(
                        chip.includesMe ? AnyShapeStyle(.tint.opacity(0.15)) : AnyShapeStyle(Color(.secondarySystemFill)),
                        in: Capsule())
                    .overlay {
                        if chip.includesMe {
                            Capsule().strokeBorder(.tint, lineWidth: 1)
                        }
                    }
                }
                .buttonStyle(.plain)
                // Simultaneous so the Button's tap keeps working; being
                // deeper than the bubble's own long-press, this one wins
                // when the press starts on a chip.
                .simultaneousGesture(LongPressGesture().onEnded { _ in
                    showsReactionDetails = true
                })
                .transition(.scale.combined(with: .opacity))
                .accessibilityLabel("\(chip.emoji) \(chip.count)")
            }
        }
        .padding(.horizontal, 4)
        .padding(.top, 1)
        // Scoped to the chip row: an animation watching the whole bubble
        // row's frame kept a spring alive against layout re-passes.
        .animation(.spring(duration: 0.25, bounce: 0.3), value: reactionChips)
        .popover(isPresented: $showsReactionDetails) {
            reactionDetailsList
                .presentationCompactAdaptation(.popover)
        }
    }

    /// The "who reacted" popover body: each emoji in chip order with the
    /// names of its reactors ("You" first when the current user is one).
    private var reactionDetailsList: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(reactionDetails) { detail in
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(detail.emoji)
                    Text(detail.names.formatted(.list(type: .and)))
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .padding(12)
    }

    private var bubble: some View {
        Text(message.body)
            .font(.body)
            .foregroundStyle(isMine ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(
                isMine ? AnyShapeStyle(.tint) : AnyShapeStyle(Color(.secondarySystemFill)),
                in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            // The floating reaction picker grows out of this exact rect;
            // the parent resolves it from the preference by localID. Empty
            // unless this bubble is the picker's host (see publishesAnchor).
            .anchorPreference(key: BubbleAnchorKey.self, value: .bounds) {
                publishesAnchor ? [message.localID: $0] : [:]
            }
            .onLongPressGesture {
                onLongPress()
            }
    }

    @ViewBuilder
    private var statusGlyph: some View {
        switch message.state {
        case .pending:
            Image(systemName: "clock")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .accessibilityLabel("Sending")
        case .sent:
            if isRead {
                DoubleCheckmark()
                    .foregroundStyle(.tint)
                    .accessibilityLabel("Read")
            } else {
                Image(systemName: "checkmark")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Sent")
            }
        case .failed:
            Button {
                onRetry()
            } label: {
                HStack(spacing: 2) {
                    Image(systemName: "exclamationmark.circle.fill")
                    Text("Tap to retry")
                }
                .font(.caption2)
                .foregroundStyle(.red)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Failed. Tap to retry.")
        }
    }
}

/// Two overlapped SF checkmarks — the system has no double-check symbol.
private struct DoubleCheckmark: View {
    var body: some View {
        ZStack(alignment: .leading) {
            Image(systemName: "checkmark")
            Image(systemName: "checkmark")
                .offset(x: 4)
        }
        .font(.caption2)
        .padding(.trailing, 4)
    }
}
