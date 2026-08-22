//
//  MacMessageRow.swift
//  FamilyConnect
//
//  One message on the Mac: a balloon, its sender, and whatever it carries.
//
//  Deliberately plainer than the iOS bubble. The phone's version earns its
//  complexity from touch — a floating tapback menu, double-tap hearts,
//  hand-rolled link hit-testing because a child gesture would eat the
//  parent's. A Mac has a cursor and a right-click, so the same actions are
//  a context menu, which the system draws.
//

#if os(macOS)

import SwiftUI

struct MacMessageRow: View {
    let message: MessageEntity
    let senderName: String?
    let isMine: Bool
    /// Prime the composer to answer this message.
    var onReply: () -> Void = {}
    /// Borrow the composer to rewrite it (author only).
    var onEdit: () -> Void = {}
    /// Open a photo or video at full size.
    var onOpenAttachment: (AttachmentDTO) -> Void = { _ in }

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(AttachmentStore.self) private var attachments

    var body: some View {
        HStack {
            if isMine { Spacer(minLength: 60) }
            VStack(alignment: isMine ? .trailing : .leading, spacing: 2) {
                if !isMine, let senderName {
                    Text(senderName)
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.secondary)
                }
                balloon
                Text(
                    message.createdAt,
                    format: .dateTime.hour(.twoDigits(amPM: .omitted)).minute(.twoDigits))
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
            if !isMine { Spacer(minLength: 60) }
        }
        .contextMenu {
            // What the phone's long-press menu offers, drawn by the system
            // — which is the Mac idiom, and free.
            let acked = message.serverID != nil
            if acked {
                // A reaction needs a server id, and so does a quote.
                Menu("React") {
                    ForEach(MessagePresentation.quickReactions, id: \.self) { emoji in
                        Button(emoji) {
                            Task {
                                await coordinator.toggleReaction(
                                    localID: message.localID, emoji: emoji)
                            }
                        }
                    }
                }
                Button("Reply", action: onReply)
                if isMine, !message.body.isEmpty {
                    Button("Edit", action: onEdit)
                }
                Divider()
            }
            if !message.body.isEmpty {
                Button("Copy") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(message.body, forType: .string)
                }
            }
            if message.state == .failed {
                Divider()
                Button("Try Again") { coordinator.retry(localID: message.localID) }
                Button("Delete", role: .destructive) {
                    coordinator.deleteLocalMessage(localID: message.localID)
                }
            }
        }
    }

    @ViewBuilder
    private var balloon: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let quote = message.replySnapshot {
                MacQuoteBlock(quote: quote, isMine: isMine)
            }
            if let attachment = message.attachmentSnapshot {
                MacAttachmentBlock(attachment: attachment)
                    .onTapGesture { onOpenAttachment(attachment) }
            }
            if !message.body.isEmpty {
                Text(message.body)
                    .textSelection(.enabled)
            }
            let chips = MessagePresentation.reactionChips(
                message.reactionList, currentUserID: coordinator.currentUserID)
            if !chips.isEmpty {
                MacReactionRow(chips: chips) { emoji in
                    Task {
                        await coordinator.toggleReaction(localID: message.localID, emoji: emoji)
                    }
                }
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(
            isMine ? AnyShapeStyle(.tint) : AnyShapeStyle(Color.appSecondaryFill),
            in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .foregroundStyle(isMine ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
        .frame(maxWidth: 460, alignment: isMine ? .trailing : .leading)
        // Pending rows are dimmed until the server has them — the same
        // signal the phone gives, without the status glyph.
        .opacity(message.state == .pending ? 0.6 : 1)
    }
}

/// The emoji already on a message. Clicking one joins it — never removes,
/// same rule as the phone: taking a reaction away is done deliberately,
/// through the context menu.
private struct MacReactionRow: View {
    let chips: [ReactionChip]
    let onToggle: (String) -> Void

    var body: some View {
        HStack(spacing: 4) {
            ForEach(chips, id: \.emoji) { chip in
                Button {
                    onToggle(chip.emoji)
                } label: {
                    HStack(spacing: 3) {
                        Text(chip.emoji)
                        if chip.count > 1 {
                            Text("\(chip.count)").font(.caption2)
                        }
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .overlay(
                        Capsule().strokeBorder(
                            chip.includesMe ? Color.accentColor : Color.secondary.opacity(0.4),
                            lineWidth: 1))
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.top, 2)
    }
}

/// The quoted message above a reply, inside the balloon — same placement
/// as the phone.
private struct MacQuoteBlock: View {
    let quote: ReplyToSnapshot
    let isMine: Bool

    var body: some View {
        HStack(spacing: 6) {
            RoundedRectangle(cornerRadius: 1.5)
                .fill(isMine ? AnyShapeStyle(.white.opacity(0.8)) : AnyShapeStyle(Color.accentColor))
                .frame(width: 3)
            Text(quote.excerpt)
                .font(.caption)
                .lineLimit(2)
                .opacity(0.85)
        }
        .frame(maxWidth: 420, alignment: .leading)
    }
}

/// A photo, video or file inside a Mac balloon.
private struct MacAttachmentBlock: View {
    let attachment: AttachmentDTO

    @Environment(AttachmentStore.self) private var store

    var body: some View {
        // Reading `generation` is what makes this redraw when the fetch
        // lands — the store's caches are ObservationIgnored. Same rule as
        // the iOS bubble, and the same bug if it is left out.
        let _ = store.generation
        if attachment.isFile {
            Label(attachment.displayName, systemImage: "doc")
                .lineLimit(1)
                .truncationMode(.middle)
        } else if let image = store.image(id: attachment.id, preview: true)
            ?? store.image(id: attachment.id, preview: false) {
            image
                .resizable()
                .aspectRatio(contentMode: .fit)
                .frame(maxWidth: 320, maxHeight: 320)
                .clipShape(RoundedRectangle(cornerRadius: 8))
        } else {
            RoundedRectangle(cornerRadius: 8)
                .fill(.black.opacity(0.1))
                .frame(width: 240, height: 180)
                .overlay {
                    if attachment.isVideo {
                        Image(systemName: "play.circle.fill").font(.largeTitle)
                    } else {
                        ProgressView()
                    }
                }
        }
    }
}

#endif
