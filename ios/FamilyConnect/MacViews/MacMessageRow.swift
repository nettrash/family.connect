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

import AppKit
import SwiftUI

struct MacMessageRow: View {
    let message: MessageSnapshot
    let senderName: String?
    /// Resolves any sender id to a name — the quote block needs names for
    /// people other than this row's sender.
    var nameFor: (Int64) -> String = { _ in String(localized: "Someone") }
    /// Profile-picture version per user, so the who-reacted rows lead with
    /// a face like the phone's do.
    var avatarVersionFor: (Int64) -> Int64 = { _ in 0 }
    let isMine: Bool
    /// Family chat, run head, not mine — the phone's rule, shared.
    var showsSenderName: Bool = false
    /// The last of a run carries the time; the rest do not.
    var showsTimestamp: Bool = true
    var isRunStart: Bool = true
    var isRunEnd: Bool = true
    var onReply: () -> Void = {}
    var onEdit: () -> Void = {}
    var onOpenAttachment: (AttachmentDTO) -> Void = { _ in }

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @State private var hovering = false
    /// The full 771-entry catalogue, behind "More reactions…".
    @State private var showsEmojiPicker = false
    /// Who left what — the Mac's answer to long-pressing a chip.
    @State private var showsReactors = false

    /// Corners tighten where a balloon meets its run mates on the sender's
    /// side — the same shape language the phone uses, which is what makes
    /// a burst read as one turn in the conversation rather than four.
    private var shape: UnevenRoundedRectangle {
        let big: CGFloat = 14
        let tight: CGFloat = 4
        return UnevenRoundedRectangle(
            topLeadingRadius: isMine ? big : (isRunStart ? big : tight),
            bottomLeadingRadius: isMine ? big : (isRunEnd ? big : tight),
            bottomTrailingRadius: isMine ? (isRunEnd ? big : tight) : big,
            topTrailingRadius: isMine ? (isRunStart ? big : tight) : big,
            style: .continuous)
    }

    var body: some View {
        HStack(alignment: .bottom, spacing: 6) {
            if isMine { Spacer(minLength: 80) }
            VStack(alignment: isMine ? .trailing : .leading, spacing: 2) {
                if showsSenderName, let senderName {
                    Text(senderName)
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.secondary)
                        .padding(.leading, 2)
                }
                balloon
                if message.state == .failed {
                    // A send that failed is the one thing here the user has
                    // to act on, so it says so in place rather than only
                    // under a right-click.
                    Button {
                        coordinator.retry(localID: message.localID)
                    } label: {
                        Label("Try Again", systemImage: "exclamationmark.circle.fill")
                            .font(.caption2)
                            .foregroundStyle(.red)
                    }
                    .buttonStyle(.plain)
                    .padding(.horizontal, 2)
                } else if showsTimestamp {
                    HStack(spacing: 4) {
                        if message.isEdited {
                            Text("edited")
                        }
                        Text(
                            message.createdAt,
                            format: .dateTime.hour(.twoDigits(amPM: .omitted)).minute(.twoDigits))
                    }
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
                    .padding(.horizontal, 2)
                }
            }
            if !isMine { Spacer(minLength: 80) }
        }
        // Runs breathe less than turns do: 1pt inside a run, 8 between.
        .padding(.top, isRunStart ? 8 : 1)
        .onHover { hovering = $0 }
        .contextMenu { rowMenu }
        .sheet(isPresented: $showsEmojiPicker) {
            VStack(spacing: 0) {
                EmojiPickerView { emoji in
                    showsEmojiPicker = false
                    Task {
                        await coordinator.toggleReaction(
                            localID: message.localID, emoji: emoji)
                    }
                }
                Divider()
                HStack {
                    Spacer()
                    Button("Cancel") { showsEmojiPicker = false }
                        .keyboardShortcut(.cancelAction)
                }
                .padding(12)
            }
            .frame(width: 420, height: 460)
        }
        .popover(isPresented: $showsReactors, arrowEdge: .bottom) {
            reactorList
        }
    }

    /// Who left what, one row per emoji in chip order.
    ///
    /// My own row is the remove control — the only way a reaction comes
    /// off, so it can never happen by accident. Same rule the phone
    /// settled on, and the same wording.
    private var reactorList: some View {
        let details = MessagePresentation.reactionDetails(
            message.reactions,
            names: Dictionary(
                uniqueKeysWithValues: message.reactions.map { ($0.userID, nameFor($0.userID)) }),
            currentUserID: coordinator.currentUserID)
        let chips = MessagePresentation.reactionChips(
            message.reactions, currentUserID: coordinator.currentUserID)
        return VStack(alignment: .leading, spacing: 8) {
            ForEach(details) { detail in
                let isMineReaction = chips.first { $0.emoji == detail.emoji }?.includesMe == true
                HStack(spacing: 8) {
                    if let leadUserID = detail.leadUserID {
                        InitialsAvatar(
                            title: detail.names.first ?? "?",
                            userID: leadUserID,
                            avatarVersion: avatarVersionFor(leadUserID),
                            size: 24)
                    }
                    Text(detail.emoji)
                    Text(detail.names.formatted(.list(type: .and)))
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                    if isMineReaction {
                        Text("Click to remove")
                            .font(.caption2)
                            .foregroundStyle(.tint)
                    }
                }
                .contentShape(Rectangle())
                .onTapGesture {
                    guard isMineReaction else { return }
                    showsReactors = false
                    Task {
                        await coordinator.toggleReaction(
                            localID: message.localID, emoji: detail.emoji)
                    }
                }
                .hoverCursor(isMineReaction ? .pointingHand : .arrow)
            }
        }
        .padding(12)
        .frame(minWidth: 220, alignment: .leading)
    }

    /// Reply / React / Edit / Copy.
    ///
    /// Attached BOTH to the row and to the body text, because
    /// `.textSelection(.enabled)` turns the text into an AppKit text view
    /// that installs the system's own menu (Look Up, Translate, Services…)
    /// and wins over the row's — so right-clicking the words themselves
    /// showed macOS's menu and none of ours.
    @ViewBuilder
    private var rowMenu: some View {
        // React needs a SERVER id — the endpoint is
        // `…/messages/{id}/reaction` — so there is nothing to react to
        // until the message is acked. Hiding it is correct, not a gap.
        let acked = message.serverID != nil
        if acked {
            Menu("React") {
                ForEach(MessagePresentation.quickReactions, id: \.self) { emoji in
                    Button(emoji) {
                        Task {
                            await coordinator.toggleReaction(
                                localID: message.localID, emoji: emoji)
                        }
                    }
                }
                Divider()
                // The server takes any emoji ≤ 32 bytes and
                // EmojiPickerView is already platform-free, so the Mac was
                // missing only the way in.
                Button("More reactions…") { showsEmojiPicker = true }
            }
            if !message.reactions.isEmpty {
                Button("See who reacted") { showsReactors = true }
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

    @ViewBuilder
    private var balloon: some View {
        VStack(alignment: .leading, spacing: 5) {
            if let quote = message.replyTo {
                MacQuoteBlock(quote: quote, isMine: isMine, nameFor: nameFor)
            }
            if let attachment = message.attachment {
                MacAttachmentBlock(attachment: attachment, isMine: isMine)
                    .onTapGesture { onOpenAttachment(attachment) }
            }
            if !message.body.isEmpty {
                Text(message.body)
                    .textSelection(.enabled)
                    .contextMenu { rowMenu }
                    .fixedSize(horizontal: false, vertical: true)
            }
            let chips = MessagePresentation.reactionChips(
                message.reactions, currentUserID: coordinator.currentUserID)
            if !chips.isEmpty {
                MacReactionRow(chips: chips) { chip in
                    // A click never takes a reaction away. On a chip I am
                    // not part of, join it; on one I AM part of, show who
                    // reacted — where my own row is the explicit remove.
                    // Undoing something you never meant to do is a worse
                    // failure than one extra click.
                    if chip.includesMe {
                        showsReactors = true
                    } else {
                        Task {
                            await coordinator.toggleReaction(
                                localID: message.localID, emoji: chip.emoji)
                        }
                    }
                }
            }
        }
        .padding(.horizontal, 11)
        .padding(.vertical, 7)
        .background(
            isMine ? AnyShapeStyle(.tint) : AnyShapeStyle(Color.appSecondaryFill),
            in: shape)
        .foregroundStyle(isMine ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
        .frame(maxWidth: 460, alignment: isMine ? .trailing : .leading)
        // A pending row is dimmed until the server has it — the same
        // signal the phone gives, without a status glyph.
        .opacity(message.state == .pending ? 0.55 : 1)
        // A hint of lift under the cursor: this row has a menu on it, and
        // nothing else says so on a Mac.
        .shadow(color: .black.opacity(hovering ? 0.12 : 0), radius: 3, y: 1)
        .animation(.easeOut(duration: 0.12), value: hovering)
    }
}

/// The emoji already on a message.
///
/// Clicking one JOINS it and never removes — the phone's rule, and it was
/// only ever a comment here: the click went straight to `toggleReaction`,
/// which takes a reaction off when it is already yours. Removing is
/// deliberate, through "See who reacted".
private struct MacReactionRow: View {
    let chips: [ReactionChip]
    /// The whole chip, not just its emoji: the caller has to know whether
    /// this is mine to decide between joining and showing who reacted.
    let onPick: (ReactionChip) -> Void

    var body: some View {
        HStack(spacing: 4) {
            ForEach(chips, id: \.emoji) { chip in
                Button {
                    onPick(chip)
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
    /// Who a sender id belongs to. Needed now there are two levels: an
    /// excerpt stacked on another excerpt with no names attached is
    /// unreadable — you cannot tell who said which half.
    let nameFor: (Int64) -> String

    var body: some View {
        HStack(spacing: 6) {
            RoundedRectangle(cornerRadius: 1.5)
                .fill(isMine ? AnyShapeStyle(.white.opacity(0.8)) : AnyShapeStyle(Color.accentColor))
                .frame(width: 3)
            VStack(alignment: .leading, spacing: 1) {
                // Context for the quote under it, so it is quieter and
                // capped at one line.
                if let parent = quote.parent {
                    Text("\(nameFor(parent.senderID)): \(parent.excerpt)")
                        .font(.caption2)
                        .lineLimit(1)
                        .opacity(0.55)
                }
                Text("\(nameFor(quote.senderID)): \(quote.excerpt)")
                    .font(.caption)
                    .lineLimit(2)
                    .opacity(0.85)
            }
        }
        .frame(maxWidth: 420, alignment: .leading)
    }
}

/// A photo, video or file inside a Mac balloon.
private struct MacAttachmentBlock: View {
    let attachment: AttachmentDTO
    /// For CONTRAST: an own balloon is filled with the tint, so anything
    /// drawn in the accent colour there would be invisible.
    let isMine: Bool

    @Environment(AttachmentStore.self) private var store

    var body: some View {
        // Reading `generation` is what makes this redraw when the fetch
        // lands — the store's caches are ObservationIgnored. Same rule as
        // the iOS bubble, and the same bug if it is left out.
        let _ = store.generation
        if attachment.isFile {
            HStack(spacing: 9) {
                Image(systemName: "doc")
                    .font(.system(size: 20))
                    .foregroundStyle(isMine ? AnyShapeStyle(.white) : AnyShapeStyle(.tint))
                    .frame(width: 30, height: 30)
                    .background(
                        (isMine ? Color.white.opacity(0.18) : Color.accentColor.opacity(0.12)),
                        in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                VStack(alignment: .leading, spacing: 1) {
                    Text(attachment.displayName)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Text(attachment.displaySize)
                        .font(.caption)
                        .opacity(0.7)
                }
            }
            .padding(.vertical, 6)
            .padding(.horizontal, 8)
            .background(
                (isMine ? Color.white.opacity(0.14) : Color.black.opacity(0.05)),
                in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .strokeBorder(isMine ? Color.white.opacity(0.16) : Color.black.opacity(0.06)))
            .hoverCursor(.pointingHand)
        } else if let image = store.image(id: attachment.id, preview: true)
            ?? store.image(id: attachment.id, preview: false) {
            image
                .resizable()
                .aspectRatio(contentMode: .fit)
                .frame(maxWidth: 320, maxHeight: 320)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .overlay {
                    if attachment.isVideo {
                        Image(systemName: "play.circle.fill")
                            .font(.system(size: 40))
                            .foregroundStyle(.white, .black.opacity(0.35))
                            .shadow(radius: 3)
                    }
                }
                // A hairline so a pale photo does not melt into a pale balloon.
                .overlay(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .strokeBorder(isMine ? Color.white.opacity(0.18) : Color.black.opacity(0.08)))
                // Clickable, and on a Mac only the cursor says so.
                .hoverCursor(.pointingHand)
        } else {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(LinearGradient(
                    colors: isMine
                        ? [.white.opacity(0.22), .white.opacity(0.10)]
                        : [.black.opacity(0.10), .black.opacity(0.04)],
                    startPoint: .top, endPoint: .bottom))
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
