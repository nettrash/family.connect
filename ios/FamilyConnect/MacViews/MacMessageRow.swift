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
    /// The assistant is still writing this one — a cursor, nothing more.
    var isStreaming: Bool = false
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
    /// Shared preview cache — asking it for a link's state is what starts
    /// the (single, app-wide) fetch for that link.
    @Environment(LinkPreviewLoader.self) private var previewLoader
    /// The environment's own openURL, captured BEFORE this row overrides
    /// it: the override defers into this, never into itself.
    @Environment(\.openURL) private var systemOpenURL
    /// The in-flight deferred link open — non-nil exactly while a first
    /// click waits out the double-click window (see `handleLinkClick`).
    @State private var pendingLinkOpen: Task<Void, Never>?
    /// True once this row has actually been on screen. The Mac thread now
    /// renders a bounded WINDOW rather than the viewport, so binding the
    /// fetch to rendering would contact every host in that window — links
    /// the reader has not scrolled to, and may never see.
    @State private var hasBeenVisible = false
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
                    // Double-click is ❤️, as on the phone
                    // (MessagePresentation.doubleTapReaction). On the
                    // BALLOON, not the row: the row spans the full width
                    // including the 80pt spacer, and double-clicking empty
                    // space beside a message must not leave a reaction on
                    // it. Acked only — there is no id to react to before.
                    .onTapGesture(count: 2) { quickHeart() }
                    // Every link in this balloon — in the text and on the
                    // preview card — opens through the arbitration above,
                    // so a double-click still reaches the heart. Captured
                    // BEFORE the override (`systemOpenURL`), or the
                    // deferral would recurse into itself.
                    .environment(
                        \.openURL,
                        OpenURLAction { url in
                            handleLinkClick(url)
                            return .handled
                        })
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
        // A preview is fetched only once the row has actually been ON
        // SCREEN, not merely rendered. The thread renders a bounded WINDOW
        // of rows, so binding the fetch to rendering would contact every
        // linked host in that window — pages the reader has not scrolled to
        // and may never see. Geometry, not `onAppear`: in a non-lazy stack
        // every row "appears" at creation.
        .onGeometryChange(for: Bool.self) { geometry in
            guard previewableLink != nil, let viewport = geometry.bounds(of: .scrollView) else {
                return false
            }
            let frame = geometry.frame(in: .scrollView)
            return frame.maxY >= 0 && frame.minY <= viewport.height
        } action: { visible in
            if visible { hasBeenVisible = true }
        }
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

    private func quickHeart() {
        guard message.serverID != nil else { return }
        Task {
            await coordinator.toggleReaction(
                localID: message.localID,
                emoji: MessagePresentation.doubleTapReaction)
        }
    }

    /// Reply / React / Edit / Copy, over the WHOLE row.
    ///
    /// The body text deliberately does NOT use `.textSelection(.enabled)`.
    /// That turns the Text into an AppKit text view which installs the
    /// system's own menu (Look Up, Translate, Services…) — and that menu
    /// wins: right-clicking the words showed macOS's menu and none of ours,
    /// so React was unreachable over most of a balloon. Attaching this menu
    /// to the Text as well does NOT beat it (measured — it still lost), so
    /// the selection has to go.
    ///
    /// Copying a whole message is the "Copy" item below; what is given up
    /// is dragging to select PART of one. If that is wanted back, the way
    /// to have both is a hover-revealed button on the balloon (the Slack
    /// pattern), not text selection.
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
                if attachment.isLocation {
                    // Its own view, shared with the phone: a location has
                    // no bytes, so none of the download machinery applies,
                    // and it carries its own click-to-open-in-Maps.
                    LocationAttachmentView(attachment: attachment, isMine: isMine)
                } else if attachment.isAudio {
                    // The player IS the interaction; a click belongs to its
                    // own controls, so no open/heart pair here.
                    MacAttachmentBlock(attachment: attachment, isMine: isMine)
                } else {
                    MacAttachmentBlock(attachment: attachment, isMine: isMine)
                        // Count 2 BEFORE count 1, and both as onTapGesture:
                        // that is what makes them exclusive. A bare
                        // single-click handler on a CHILD masks the
                        // balloon's double-click outright, and
                        // double-clicking a photo would open it AND heart
                        // it — the same bug the phone had.
                        .onTapGesture(count: 2) { quickHeart() }
                        .onTapGesture(count: 1) { onOpenAttachment(attachment) }
                }
            }
            if isStreaming && message.body.isEmpty {
                // The row exists but nothing has arrived yet; an empty
                // balloon would look broken.
                Text(verbatim: "▍").opacity(0.6)
            }
            if !message.body.isEmpty {
                // One text block — everything without a table — comes back
                // as exactly this `Text` and nothing around it.
                MessageBodyBlocks(
                    blocks: bodyBlocks,
                    isStreaming: isStreaming,
                    isMine: isMine
                ) { text, carriesCursor in
                    Text(text)
                        .font(emojiFont)
                        .fixedSize(horizontal: false, vertical: true)
                        .overlay(alignment: .bottomTrailing) {
                            // The last text block is the only one still
                            // growing, so it is the one that ends in a
                            // cursor.
                            if isStreaming, carriesCursor {
                                Text(verbatim: "▍").opacity(0.6).offset(x: 8)
                            }
                        }
                        // A table has already decided how wide this balloon
                        // is. Text left to itself reports the width it
                        // WANTS, which leaves a narrow paragraph floating
                        // beside a wide grid; filling makes it wrap against
                        // the same edge. Gated on there BEING a table — the
                        // phone learned the hard way that doing this
                        // unconditionally makes every balloon full width.
                        .frame(maxWidth: hasTable ? .infinity : nil, alignment: .leading)
                }
            }
            if let preview = linkPreview {
                LinkPreviewCard(
                    preview: preview,
                    image: previewLoader.image(for: preview.url),
                    onOpen: { handleLinkClick($0) })
                    .padding(.top, 4)
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
        // A message that is nothing but emoji renders BARE, exactly as it
        // does on the phone and on Android: the padding and the shape stay
        // (so the hover target, the context menu and the run-aware corners
        // are untouched) and only the fill goes.
        .background(
            isEmojiOnly
                ? AnyShapeStyle(Color.clear)
                : isMine ? AnyShapeStyle(.tint) : AnyShapeStyle(Color.appSecondaryFill),
            in: shape)
        // ...and with the fill gone, white would vanish. Own messages flip
        // to .primary so a monochrome pictograph (☂, ™) is still visible
        // against the window. Same rule as MessageBubbleView.
        .foregroundStyle(
            isMine && !isEmojiOnly ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
        .frame(maxWidth: 460, alignment: isMine ? .trailing : .leading)
        // A pending row is dimmed until the server has it — the same
        // signal the phone gives, without a status glyph.
        .opacity(message.state == .pending ? 0.55 : 1)
        // A hint of lift under the cursor: this row has a menu on it, and
        // nothing else says so on a Mac. Suppressed on a bare balloon,
        // where there is no balloon to lift — the shadow would fall on the
        // glyphs themselves and read as a smudge.
        .shadow(
            color: .black.opacity(hovering && !isEmojiOnly ? 0.12 : 0), radius: 3, y: 1)
        .animation(.easeOut(duration: 0.12), value: hovering)
    }

    /// True when the body is nothing but a few emoji.
    private var isEmojiOnly: Bool { EmojiOnly.displayFontSize(for: message.body) != nil }

    /// The web link this bubble would preview, if any. Emoji-only bodies
    /// have no links, and tel:/mailto: are not previewable.
    ///
    /// Resolved over the RENDERED text, like the phone: markdown deletes
    /// characters, so detecting over the raw body previews links the reader
    /// cannot see and misses ones they can.
    private var previewableLink: URL? {
        guard !isEmojiOnly else { return nil }
        return MessageLinks.firstWebLinkAsDrawn(in: message.body)
    }

    /// The card to draw under this bubble, once its fetch has landed.
    private var linkPreview: LinkPreview? {
        guard hasBeenVisible, let url = previewableLink,
            case .loaded(let preview) = previewLoader.state(for: url)
        else { return nil }
        return preview
    }

    /// What the balloon draws: ONE text block unless the body carries a
    /// table (MessageMarkdown).
    ///
    /// Markdown and the `@ai` mention, through the SAME renderer the phone
    /// uses — a message written on one platform has to read the same on the
    /// other, and two renderers would be two subsets within a week.
    ///
    /// Emoji-only bodies branch AROUND it, exactly as they do on the phone
    /// and on Android: the ladder's whole subject is that the message is
    /// nothing but glyphs, and running a markup pass over it could only
    /// take something away.
    private var bodyBlocks: [MessageMarkdown.Block] {
        guard !isEmojiOnly else { return [.text(AttributedString(message.body))] }
        return MessageLinks.blocks(message.body, isMine: isMine)
    }

    /// A table sets the balloon's width, so the text around it wraps
    /// against the grid instead of floating narrow beside it — the Mac's
    /// half of what the phone calls `fillsBalloonWidth`.
    private var hasTable: Bool {
        bodyBlocks.contains { $0.isTable }
    }

    /// Open a clicked link — unless a second click arrives first, in which
    /// case the reader was double-clicking to ❤️.
    ///
    /// SwiftUI's `Text` handles a link's own click INTERNALLY and does it
    /// before the balloon's count-2 gesture can run, so without this a
    /// double-click over link glyphs opens the page twice and never leaves
    /// a heart. The phone hit exactly this and solved it exactly this way;
    /// the Mac needed it the moment links became live here.
    ///
    /// Deferring by the double-click window costs a clearly perceptible
    /// pause before a page opens, which is the price of the heart working
    /// everywhere rather than only over the padding.
    private func handleLinkClick(_ url: URL) {
        if pendingLinkOpen != nil {
            pendingLinkOpen?.cancel()
            pendingLinkOpen = nil
            // Unacked has no id to react to, so the click stays an open.
            if message.serverID != nil {
                quickHeart()
            } else {
                systemOpenURL(url)
            }
            return
        }
        pendingLinkOpen = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 350_000_000)
            guard !Task.isCancelled else { return }
            pendingLinkOpen = nil
            systemOpenURL(url)
        }
    }

    /// The body font: the shared emoji ladder, or the inherited default.
    ///
    /// The ladder itself is a pinned cross-platform contract (96/80/68/56,
    /// with mirrored vectors in EmojiOnly.swift and EmojiOnly.kt) and must
    /// NOT be edited to suit the Mac. What is adjusted here, at the call
    /// site, is the one thing that genuinely differs: those points were
    /// chosen against a phone's ~17pt body text, and a Mac's is ~13pt, so
    /// a literal 96pt glyph sits far larger relative to everything around
    /// it than the same message does on a phone. Scaling by the ratio of
    /// the two body sizes keeps the PROPORTION the ladder was designed for,
    /// which is what "the same as iOS" means on a different-sized surface.
    ///
    /// No `@ScaledMetric`: macOS has no Dynamic Type for it to track.
    private var emojiFont: Font? {
        guard let size = EmojiOnly.displayFontSize(for: message.body) else { return nil }
        return .system(size: size * Self.macBodyRatio)
    }

    /// macOS's default body size over iOS's. One constant to change if the
    /// Mac's emoji ever want to be bigger or smaller.
    private static let macBodyRatio: CGFloat = 13.0 / 17.0
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
        // No width of its own, on purpose. `frame(maxWidth:)` in SwiftUI is
        // GREEDY — it takes the whole proposal whenever one is offered — so
        // the 420 that used to be here reported ~420pt however short the
        // excerpt was, and every reply balloon on the Mac came out a
        // fixed-width slab regardless of what the reply said. The balloon
        // already caps itself at 460 after its background, which is the
        // bound that was actually wanted.
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
        if attachment.isAudio {
            // Nothing to look at — a player, not a tile or a row.
            AudioPlayerView(attachment: attachment, isMine: isMine)
        } else if attachment.isFile {
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
