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
//    red exclamation  failed — tap to retry, context menu to delete
//
//  The sender-name caption (family chat, sender change — rules in
//  MessagePresentation) renders above the bubble in the accent color.
//

import SwiftUI

struct MessageBubbleView: View {
    let message: MessageSnapshot
    let isMine: Bool
    let showsSenderName: Bool
    let senderName: String?
    let isRead: Bool
    var onRetry: () -> Void = {}
    var onDelete: () -> Void = {}

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
        .contextMenu {
            if message.state == .failed {
                Button {
                    onRetry()
                } label: {
                    Label("Try Again", systemImage: "arrow.clockwise")
                }
                Button(role: .destructive) {
                    onDelete()
                } label: {
                    Label("Delete", systemImage: "trash")
                }
            }
        }
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
