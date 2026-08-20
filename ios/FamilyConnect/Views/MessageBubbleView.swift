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
//  between the bubble and the timestamp; tapping a chip toggles that
//  emoji, long-pressing the bubble opens the picker sheet the parent
//  presents via `onLongPress`.
//

import SwiftUI

struct MessageBubbleView: View {
    let message: MessageSnapshot
    let isMine: Bool
    let showsSenderName: Bool
    let senderName: String?
    let isRead: Bool
    var reactionChips: [ReactionChip] = []
    var onRetry: () -> Void = {}
    var onDelete: () -> Void = {}
    var onToggleReaction: (String) -> Void = { _ in }
    var onLongPress: () -> Void = {}

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
        .onLongPressGesture {
            onLongPress()
        }
    }

    /// The aggregated reaction chips under the bubble. A chip the current
    /// user is part of gets the tinted treatment; tapping any chip
    /// toggles that emoji for the current user.
    private var reactionRow: some View {
        HStack(spacing: 4) {
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
                .accessibilityLabel("\(chip.emoji) \(chip.count)")
            }
        }
        .padding(.horizontal, 4)
        .padding(.top, 1)
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
