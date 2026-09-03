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
    /// True for an EMPTY assistant row from the moment it is fanned out:
    /// a picture answer streams nothing, so the empty row is the whole of
    /// its "still working" state (protocol.md, "How a picture comes back").
    var isStreaming: Bool = false
    /// An `ai_error` named this row, and it is BLANK — no partial text and
    /// no picture. The phone's rule, and its wording.
    var assistantFailed: Bool = false
    let isMine: Bool
    /// Family chat, run head, not mine — the phone's rule, shared.
    var showsSenderName: Bool = false
    /// The last of a run carries the time; the rest do not.
    var showsTimestamp: Bool = true
    var isRunStart: Bool = true
    var isRunEnd: Bool = true
    /// Whether some other member has read this message — feeds the
    /// delivery ladder beside the timestamp, exactly as on the phone.
    var isRead: Bool = false
    var onReply: () -> Void = {}
    var onEdit: () -> Void = {}
    /// Ask the conversation to open the report sheet for this message's
    /// sender. The row does not own the sheet: on the Mac it belongs to the
    /// window, not to a row that scrolls away under it.
    var onReport: () -> Void = {}
    /// This row's sender is blocked. Decided by the conversation through
    /// `MessagePresentation.isHiddenByBlock`.
    var isHiddenByBlock: Bool = false
    var isRevealed: Bool = false
    var onReveal: () -> Void = {}
    /// Quote peeks, owned by the conversation: the Mac thread renders a
    /// SLIDING SUFFIX that drops old rows while the reader sits at the
    /// bottom, so per-row `@State` would be destroyed in exactly the case
    /// the protocol names — a revealed row staying revealed when newer
    /// messages arrive after it.
    var isReplyQuoteRevealed: Bool = false
    var isParentQuoteRevealed: Bool = false
    var onRevealQuote: (Bool) -> Void = { _ in }
    /// Clicking a quote asks to jump to the quoted message — the phone's
    /// `onTapQuote`, ported with the same best-effort contract: the
    /// receiver may do nothing when the target is not cached.
    var onTapQuote: (Int64) -> Void = { _ in }
    var onOpenAttachment: (AttachmentDTO) -> Void = { _ in }
    /// A drag out to the Finder found nothing to hand over — offline, a
    /// 404, or retention swept the attachment. The WINDOW says so, not this
    /// row: a row scrolls out from under its own message, and the composer
    /// strip is where every other media failure on this window already
    /// appears. `DraggedAttachment` has why a silent failure is not an
    /// option here.
    var onAttachmentDragFailed: () -> Void = {}
    /// How many people could vote, for a poll's footer.
    var memberCount: Int = 0
    /// Clicking a call record rings the peer again; nil when calls are
    /// off on this server.
    var onCallBack: (() -> Void)? = nil

    @Environment(ChatSyncCoordinator.self) private var coordinator
    /// Shared preview cache — asking it for a link's state is what starts
    /// the (single, app-wide) fetch for that link.
    @Environment(LinkPreviewLoader.self) private var previewLoader
    /// Held only so a HIDDEN row can arm the attachment fetch a visible one
    /// does — the row itself draws no attachments, `MacAttachmentBlock` and
    /// `MacAlbumStack` do. Optional for the reason the phone's is: the
    /// non-optional form traps when absent, and this store needs an
    /// `APIClient` to build.
    @Environment(AttachmentStore.self) private var attachmentStore: AttachmentStore?
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

    /// Drawn as the collapsed placeholder right now.
    private var isHidden: Bool { isHiddenByBlock && !isRevealed }

    /// The Mac's collapsed stand-in. A SINGLE click reveals.
    ///
    /// No count-2 gesture here, deliberately: reacting to a hidden message
    /// is exactly what a hidden row must not offer, and a child tap
    /// gesture beside a parent double-click re-creates the child-wins bug
    /// this file documents twice. (The thread is a ScrollView over a plain
    /// VStack rather than a List, so the "List rows lose clicks" trap does
    /// not apply here.)
    private var hiddenRow: some View {
        HStack(spacing: 6) {
            Image(systemName: "hand.raised.slash")
            Text("Hidden — blocked member")
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(Color(nsColor: .quaternarySystemFill)))
        .contentShape(Rectangle())
        .help("Click to show")
        .onTapGesture { onReveal() }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text("Hidden message from a blocked member. Click to show it."))
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { onReveal() }
        .task(id: hasBeenVisible) { armRemoteFetches() }
    }

    /// Ask for what a visible row would ask for, and draw none of it — the
    /// requirement with no compile-time and no visual signal (protocol.md,
    /// "Blocking a member").
    ///
    /// No avatar arm here, unlike the phone: a Mac message row draws no
    /// sender avatar at all, so there is no call site to preserve and no
    /// differential to leak. The MAP is armed, because this row does draw
    /// one — see MapTileWarmer.
    private func armRemoteFetches() {
        _ = linkPreview
        if let attachmentStore {
            _ = attachmentStore.generation
            for attachment in message.attachments where !attachment.isFile {
                // The store directly: `AttachmentView` is iOS-only.
                // `preview: true` — a hidden row fetches nothing a visible
                // row would not have fetched yet, so the poster frame and
                // never the 90 MB video.
                _ = attachmentStore.image(id: attachment.id, preview: true)
            }
        }
        // The map has no loader to ask — the `Map` view IS the request,
        // and a hidden row never builds one. See MapTileWarmer. Hidden
        // only: a visible row has already asked by drawing its map, and a
        // row that fetched twice is as distinguishable as one that did not
        // fetch at all.
        if isHidden {
            MapTileWarmer.warm(localID: message.localID, attachments: message.attachments)
        }
    }

    var body: some View {
        HStack(alignment: .bottom, spacing: 6) {
            if isMine { Spacer(minLength: 80) }
            VStack(alignment: isMine ? .trailing : .leading, spacing: 2) {
                // The run-corner geometry below and the `isRunStart` top
                // padding are deliberately NOT fed `isHidden`: run GROUPING
                // is unchanged by hiding, so a hidden bubble still counts
                // as its sender's run and the next visible message from
                // somebody else still gets its caption.
                if isHidden {
                    hiddenRow
                } else {
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
                }
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
                } else if showsTimestamp || isHidden {
                    // UNCONDITIONAL on a hidden row, unlike a visible one.
                    // `showsTimestamp` is fed by `isRunEnd`, so only the
                    // last row of a run carries a time — and three
                    // consecutive messages from a blocked member would draw
                    // three placeholders, two of them literally nothing.
                    // "A hidden row draws the placeholder and the timestamp
                    // and nothing else.
                    HStack(spacing: 4) {
                        if message.isEdited {
                            Text("edited")
                        }
                        Text(
                            message.createdAt,
                            format: .dateTime.hour(.twoDigits(amPM: .omitted)).minute(.twoDigits))
                        if isMine {
                            statusGlyph
                        }
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
            currentUserID: coordinator.currentUserID,
            blockedUserIDs: coordinator.blockedUserIDs)
        // The chips below are deliberately not given the blocked set.
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
        // The function, not just the balloon branch: `handleLinkClick`
        // reaches here too, so guarding only the gesture would leave a
        // reachable path to reacting to a hidden message.
        guard !isHidden, message.serverID != nil else { return }
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
        let acked = message.serverID != nil && !isHidden
        // A hidden row offers Report and Unblock and nothing else. Copy in
        // particular would put the blocked body on the pasteboard, which is
        // the same leak the phone's long-press gate closes.
        if !isHidden {
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
        }
        if message.state == .failed && !isHidden {
            Divider()
            Button("Try Again") { coordinator.retry(localID: message.localID) }
            Button("Delete", role: .destructive) {
                coordinator.deleteLocalMessage(localID: message.localID)
            }
        }
        // AFTER the failed block, not inside it: inside, Block would be
        // unreachable on an unsent-but-visible message.
        //
        // A native contextMenu, so items append freely — there is no size
        // arithmetic here, which is the one way the Mac menu is simpler
        // than the iOS one. It is also NOT the same menu: the Mac has
        // React / See who reacted / Try Again / Delete and no Share, and
        // porting the iOS row set wholesale would smuggle a Share item
        // onto the Mac that nobody asked for.
        if isOtherMember {
            Divider()
            // Grouped under Safety, matching the phone and the roster. A
            // native submenu here, unlike iOS's paged custom panel: an
            // AppKit context menu nests on its own.
            Menu("Safety") {
                Button("Report…") { onReport() }
                if coordinator.blockedUserIDs.contains(message.senderID) {
                    Button("Unblock") {
                        let userID = message.senderID
                        Task { await coordinator.unblock(userID: userID) }
                    }
                } else {
                    Button("Block", role: .destructive) {
                        let userID = message.senderID
                        Task { await coordinator.block(userID: userID) }
                    }
                }
            }
        }
    }

    /// Somebody else's message, from a real member — never the assistant,
    /// whose reserved account is absent from the roster on purpose, so the
    /// server would refuse a block naming it.
    private var isOtherMember: Bool {
        message.senderID != coordinator.currentUserID
            && message.senderID != AppSettings.assistantUserID
    }

    @ViewBuilder
    private var balloon: some View {
        // Alignment rule, updated 2026-08 at the product owner's ask:
        // content used to share one left edge unconditionally, but in MY
        // replies a SHORT body — one or two lines — now sits against the
        // trailing edge while the QUOTE keeps the left one; from three
        // lines the body goes back to the leading edge (the owner's
        // follow-up — a right-ragged paragraph reads as an accident).
        // OwnReplyBodyAlignment, shared with the phone, measures the
        // count and owns every alignment-dependent modifier on the body.
        // Everything else — others' messages, own messages without a
        // quote — is unchanged. ReplyContentLayout (shared with the
        // phone) is what lets the body sit trailing while the quote keeps
        // the leading edge AND the balloon still hugs its widest child —
        // the frame-based version of this rule took the row's width, the
        // documented slab regression.
        let contentStack = isOwnReply
            ? AnyLayout(ReplyContentLayout(spacing: 5))
            : AnyLayout(VStackLayout(alignment: .leading, spacing: 5))
        contentStack {
            if let quote = message.replyTo {
                MacQuoteBlock(
                    quote: quote,
                    isMine: isMine,
                    nameFor: nameFor,
                    isBlockedSender: { coordinator.blockedUserIDs.contains($0) },
                    isReplyRevealed: isReplyQuoteRevealed,
                    isParentRevealed: isParentQuoteRevealed,
                    onRevealQuote: onRevealQuote,
                    onTapQuote: onTapQuote,
                    onDoubleTap: { quickHeart() })
            }
            if !message.attachments.isEmpty {
                attachmentStack(message.attachments)
            }
            if isStreaming && message.body.isEmpty {
                // The row exists but nothing has arrived yet; an empty
                // balloon would look broken.
                Text(verbatim: "▍").opacity(0.6)
            }
            if assistantFailed && message.body.isEmpty && message.attachments.isEmpty {
                Label("Couldn't answer that. Ask again.", systemImage: "exclamationmark.circle")
                    .opacity(0.75)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let call = message.call {
                // The outcome, never the placeholder body — the same view
                // the phone draws (docs/protocol.md, "Voice calls").
                CallRecordView(
                    call: call,
                    isMine: isMine,
                    onCallBack: onCallBack,
                    onDoubleTap: { quickHeart() })
                // The shared record is clickable (call back) and on a Mac
                // only the cursor says so — the quote block's rule. An
                // arrow, not nothing, when there is no call-back, so the
                // hand is a promise the click keeps.
                .hoverCursor(onCallBack != nil ? .pointingHand : .arrow)
            } else if !message.body.isEmpty {
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
                        // The reply rule, all three of its modifiers at
                        // once — line ragging, the frame's edge and the
                        // ReplyContentLayout tag — from one measured line
                        // count: trailing through two lines, leading from
                        // three. After the cursor overlay (which does not
                        // change the Text's size) and before any frame, so
                        // it counts lines from the Text's own height.
                        //
                        // `fillsWidth`: a table or a poll has already
                        // decided how wide this balloon is. Text left to
                        // itself reports the width it WANTS, which leaves
                        // a narrow paragraph floating beside a wide grid;
                        // filling makes it wrap against the same edge.
                        // Gated on there BEING such a block — the phone
                        // learned the hard way that doing this
                        // unconditionally makes every balloon full width.
                        .ownReplyBodyAlignment(
                            enabled: isOwnReply,
                            font: emojiFont,
                            fillsWidth: fillsBalloonWidth)
                }
            }
            if let preview = linkPreview {
                LinkPreviewCard(
                    preview: preview,
                    image: previewLoader.image(for: preview.url),
                    onOpen: { handleLinkClick($0) })
                    .padding(.top, 4)
            }
            // The question is the body above; this is the poll under it.
            // The same view the phone draws — nothing about a poll is
            // platform-specific, and two of them would be two subsets
            // within a week.
            if let poll = message.poll {
                PollBubbleView(
                    poll: poll,
                    currentUserID: coordinator.currentUserID,
                    isAuthor: isMine,
                    // A vote needs a server message id to PUT against.
                    isVotable: message.serverID != nil,
                    memberCount: memberCount,
                    // uniquingKeysWith, never uniqueKeysWithValues: one
                    // member holds one option, but a state that somehow
                    // said otherwise would TRAP the app rather than draw a
                    // slightly wrong tally.
                    memberNames: Dictionary(
                        poll.options.flatMap(\.votes).map { ($0, nameFor($0)) },
                        uniquingKeysWith: { first, _ in first }),
                    avatarVersions: Dictionary(
                        poll.options.flatMap(\.votes).map { ($0, avatarVersionFor($0)) },
                        uniquingKeysWith: { first, _ in first }),
                    isMine: isMine && !isBare,
                    // The Mac half. This poll is posted by an UNBLOCKED
                    // member and voted in by a blocked one, so it has
                    // nothing to do with the hidden-row branch and is the
                    // easiest thing in the step to miss.
                    blockedUserIDs: coordinator.blockedUserIDs,
                    onVote: { optionID in
                        Task { await coordinator.vote(localID: message.localID, optionID: optionID) }
                    },
                    onClose: {
                        Task { await coordinator.closePoll(localID: message.localID) }
                    },
                    // The balloon's double-click is the quick heart, and a
                    // child must pass it up rather than swallow it.
                    onDoubleTap: { quickHeart() })
            }
            let chips = MessagePresentation.reactionChips(
                message.reactions, currentUserID: coordinator.currentUserID)
            if !chips.isEmpty {
                MacReactionRow(chips: chips, onTintedBalloon: isMine && !isBare) { chip in
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
        // A message that is nothing but photos or videos drops the inset
        // along with the fill (below): the tile's edge lands where the
        // balloon's would have. Emoji keep it — a glyph has no edge of its
        // own, and the padding is its hover target.
        .padding(.horizontal, isMediaOnly ? 0 : 11)
        .padding(.vertical, isMediaOnly ? 0 : 7)
        // A message that is nothing but emoji renders BARE, exactly as it
        // does on the phone and on Android: the padding and the shape stay
        // (so the hover target, the context menu and the run-aware corners
        // are untouched) and only the fill goes. So does a media-only
        // message — the tile is the message, and a balloon around it was
        // a frame around a picture (MessagePresentation.isMediaOnly says
        // what keeps the balloon, and why).
        .background(
            isBare
                ? AnyShapeStyle(Color.clear)
                : isMine ? AnyShapeStyle(.tint) : AnyShapeStyle(Color.appSecondaryFill),
            in: shape)
        // ...and with the fill gone, white would vanish. Own messages flip
        // to .primary so a monochrome pictograph (☂, ™) is still visible
        // against the window. Same rule as MessageBubbleView.
        .foregroundStyle(
            isMine && !isBare ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
        .frame(maxWidth: 460, alignment: isMine ? .trailing : .leading)
        // A pending row is dimmed until the server has it — the same
        // signal the phone gives, without a status glyph.
        .opacity(message.state == .pending ? 0.55 : 1)
        // A hint of lift under the cursor: this row has a menu on it, and
        // nothing else says so on a Mac. Suppressed on a bare balloon,
        // where there is no balloon to lift — the shadow would fall on the
        // glyphs themselves and read as a smudge.
        .shadow(
            color: .black.opacity(hovering && !isBare ? 0.12 : 0), radius: 3, y: 1)
        .animation(.easeOut(duration: 0.12), value: hovering)
    }

    /// What the balloon draws for its attachment set — one attachment is
    /// exactly the block it has always drawn; several become a pile of
    /// the photos and videos (AttachmentAlbum's cut, SENT order) when
    /// there are two or more, the ordinary thumbnail when there is one,
    /// with files, audio and a location stacked as rows under it. Same
    /// composition as the phone's, from the same blocks.
    @ViewBuilder
    private func attachmentStack(_ attachments: [AttachmentDTO]) -> some View {
        if attachments.count == 1, let attachment = attachments.first {
            singleAttachment(attachment)
        } else {
            let media = AttachmentAlbum.media(of: attachments)
            let rows = AttachmentAlbum.rows(of: attachments)
            VStack(alignment: .leading, spacing: 4) {
                if media.count >= 2 {
                    MacAlbumStack(
                        album: AttachmentAlbum(items: media, index: 0),
                        isMine: attachmentsOnTint,
                        onOpen: { onOpenAttachment(media[0]) })
                        // Count 2 before count 1 — the exclusivity
                        // rule the single block explains.
                        .onTapGesture(count: 2) { quickHeart() }
                        .onTapGesture(count: 1) { onOpenAttachment(media[0]) }
                } else if let single = media.first {
                    singleAttachment(single)
                }
                ForEach(rows) { attachment in
                    singleAttachment(attachment)
                }
            }
        }
    }

    @ViewBuilder
    private func singleAttachment(_ attachment: AttachmentDTO) -> some View {
        if attachment.isLocation {
            // Its own view, shared with the phone: a location has
            // no bytes, so none of the download machinery applies,
            // and it carries its own click-to-open-in-Maps.
            LocationAttachmentView(attachment: attachment, isMine: attachmentsOnTint)
        } else if attachment.isAudio {
            // The player IS the interaction; a click belongs to its
            // own controls, so no open/heart pair here.
            MacAttachmentBlock(attachment: attachment, isMine: attachmentsOnTint)
        } else {
            MacAttachmentBlock(
                attachment: attachment, isMine: attachmentsOnTint, onBalloon: !isMediaOnly)
                // Drag out to the Finder, which on a Mac is what people try
                // on a picture before they look for a menu. A PROMISE, not
                // a URL — `DraggedAttachment` has the whole reason, and it
                // is not a small one: the bytes this tile is DRAWING are
                // usually the 600px preview, and handing those over would
                // put the wrong picture on somebody's Desktop under the
                // right name.
                //
                // Innermost of the three modifiers on purpose, so the tap
                // pair below keeps the exact order and precedence it had
                // (count 2 before count 1, both above this view). A drag
                // needs pointer movement and a click does not, so they do
                // not compete — but the ordering here is the part a human
                // has to confirm at a trackpad, because a drag cannot be
                // synthesised in a test.
                //
                // Only this branch, which is exactly photo / video / file:
                // a location has no bytes and a voice note's tile is a
                // scrubber. `DraggedAttachment.isDraggable` is that rule
                // written down, and refuses both again on the way through.
                .draggable(dragPromise(for: attachment))
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

    /// The file promise one tile offers the rest of the Mac.
    ///
    /// The coordinator and the callback are copied into local lets before
    /// the closures are written, so neither closure captures `self`. A
    /// promise outlives the row that made it — the drag is still going
    /// while the thread scrolls, and the export runs after the drop — and
    /// the two things it genuinely needs (a `@MainActor` coordinator, and
    /// somewhere to report to) are the two things worth carrying. The rest
    /// of the row, environment values and all, is not.
    private func dragPromise(for attachment: AttachmentDTO) -> DraggedAttachment {
        let coordinator = coordinator
        let reportFailure = onAttachmentDragFailed
        return DraggedAttachment(
            attachment: attachment,
            // THE only source of original bytes. See DraggedAttachment.
            download: { await coordinator.localFileURL(for: attachment) },
            onFailure: { reportFailure() })
    }

    /// True when the body is nothing but a few emoji.
    private var isEmojiOnly: Bool { EmojiOnly.displayFontSize(for: message.body) != nil }

    /// True when the message is nothing but photos and/or videos — the
    /// other bare treatment (MessagePresentation.isMediaOnly has the rule
    /// and why files, audio and places stay in a balloon).
    private var isMediaOnly: Bool {
        MessagePresentation.isMediaOnly(message, isStreaming: isStreaming)
    }

    /// No fill behind the content — emoji-only or media-only. Everything
    /// that adapts to "nothing behind me" keys off this, never off one
    /// half of it.
    private var isBare: Bool { isEmojiOnly || isMediaOnly }

    /// The tone the attachment blocks draw against: their `isMine` is a
    /// CONTRAST switch (white hairline and placeholder over the tint), so
    /// a bare tile on the window's own background takes the neutral side
    /// whichever side of the thread it is on.
    private var attachmentsOnTint: Bool { isMine && !isMediaOnly }

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

    /// A table or a POLL has already decided how wide the balloon is, so
    /// the text wraps against it instead of floating narrow beside it.
    private var fillsBalloonWidth: Bool {
        hasTable || message.poll != nil
    }

    /// An own message that IS a reply lays out with ReplyContentLayout:
    /// the quote keeps the left edge and the body picks its own by line
    /// count — trailing through two lines, leading from three — the
    /// product rule, shared with the phone (see the balloon stack's
    /// comment and OwnReplyBodyAlignment). Everyone else's messages, and
    /// own messages without a quote, keep one left edge.
    private var isOwnReply: Bool {
        isMine && message.replyTo != nil
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
    fileprivate static let macBodyRatio: CGFloat = 13.0 / 17.0

    /// The phone's delivery ladder (MessageBubbleView.statusGlyph), ported:
    /// clock while pending, checkmark when the server has it, a tinted
    /// double checkmark once another member has read it.
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
            // Never reached: a failed row shows the inline "Try Again"
            // button instead of a timestamp (see the row body).
            EmptyView()
        }
    }
}

/// Two overlapped SF checkmarks — the system has no double-check symbol.
/// The phone's DoubleCheckmark, verbatim.
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

/// The emoji already on a message.
///
/// Clicking one JOINS it and never removes — the phone's rule, and it was
/// only ever a comment here: the click went straight to `toggleReaction`,
/// which takes a reaction off when it is already yours. Removing is
/// deliberate, through "See who reacted".
private struct MacReactionRow: View {
    let chips: [ReactionChip]
    /// True on my own tint-filled balloon — where an accent-colored ring
    /// would sit on an accent-colored ground. The phone's lesson
    /// (MessageBubbleView): stroke with the balloon's CONTENT color.
    let onTintedBalloon: Bool
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
                            // The pinned cross-platform chip-emoji size
                            // (18pt on the phone, 16sp on Android), scaled
                            // by the Mac's body ratio like the emoji ladder.
                            .font(.system(size: 18 * MacMessageRow.macBodyRatio))
                        if chip.count > 1 {
                            Text("\(chip.count)").font(.caption2)
                        }
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .overlay(
                        Capsule().strokeBorder(
                            chip.includesMe
                                ? (onTintedBalloon ? Color.white : Color.primary)
                                : Color.secondary.opacity(0.4),
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
    /// Whether a sender is blocked by this reader. A closure rather than
    /// the coordinator: this is a private struct of plain `let` props with
    /// no environment of its own.
    let isBlockedSender: (Int64) -> Bool
    /// Peeks, per level. Independent: revealing a quote never reveals the
    /// row it points at.
    let isReplyRevealed: Bool
    let isParentRevealed: Bool
    let onRevealQuote: (Bool) -> Void
    /// A click asks to jump to the quoted message.
    let onTapQuote: (Int64) -> Void
    /// A double-click is still the balloon's quick heart. The balloon's
    /// own count-2 gesture cannot see through a child's bare
    /// `.onTapGesture`, so this block re-arms it itself — the phone's
    /// child-wins lesson (MessageBubbleView's quote), same fix.
    let onDoubleTap: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            RoundedRectangle(cornerRadius: 1.5)
                .fill(isMine ? AnyShapeStyle(.white.opacity(0.8)) : AnyShapeStyle(Color.accentColor))
                .frame(width: 3)
            VStack(alignment: .leading, spacing: 1) {
                // Context for the quote under it, so it is quieter and
                // capped at one line.
                if let parent = quote.parent {
                    Text(
                        parentHidden
                            ? MacQuoteBlock.hiddenCaption
                            : "\(nameFor(parent.senderID)): \(parent.excerpt)")
                        .font(.caption2)
                        .lineLimit(1)
                        .opacity(0.55)
                }
                Text(
                    replyHidden
                        ? MacQuoteBlock.hiddenCaption
                        : "\(nameFor(quote.senderID)): \(quote.excerpt)")
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
        // bound that was actually wanted. (In an OWN reply the balloon
        // uses ReplyContentLayout, which hugs its widest child and places
        // this block on the leading edge — BalloonEdgeKey's default; it is
        // never given a width at the call site either. Only the BODY
        // carries a trailing tag, via OwnReplyBodyAlignment.)
        // The phone's load-bearing pin, ported with the rest: the accent
        // bar is a Shape with a width-only frame — infinitely flexible in
        // height — and an unpinned block beside a body Text flips the
        // balloon into height DISTRIBUTION, the reply-truncation bug
        // (MessageBubbleView's quoteBlock comment; BubbleLayoutTests).
        .fixedSize(horizontal: false, vertical: true)
        .contentShape(Rectangle())
        // Clickable, and on a Mac only the cursor says so — the same
        // affordance the attachment tiles give.
        .hoverCursor(.pointingHand)
        // count: 1 with the balloon's double-click still reachable: a bare
        // `.onTapGesture` on a CHILD wins outright over the parent's
        // count-2 gesture, so double-clicking the quote would fire two
        // jumps and never the heart. simultaneousGesture lets both see the
        // click; the 2-click recognizer claims the double. The phone's
        // pattern (MessageBubbleView's quote), verbatim.
        .simultaneousGesture(TapGesture(count: 2).onEnded { onDoubleTap() })
        .onTapGesture {
            if replyHidden || parentHidden {
                onRevealQuote(replyHidden)
            } else {
                onTapQuote(quote.messageID)
            }
        }
        // The Mac quote had no accessibility at all; the phone's combined
        // element, spoken levels, trait AND explicit action come over as
        // one. The action is not decoration: a bare .onTapGesture
        // publishes no accessibility action, so the trait alone announces
        // "button" and activating it does nothing (measured on the phone —
        // ZZAXProbeTests).
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibilityText)
        .accessibilityAddTraits(.isButton)
        .accessibilityAction {
            if replyHidden || parentHidden {
                onRevealQuote(replyHidden)
            } else {
                onTapQuote(quote.messageID)
            }
        }
    }

    static let hiddenCaption = String(
        localized: "Hidden — blocked member",
        comment: "Stands in for a blocked member's message or quoted text")

    /// The two levels are separately blockable — see the phone's
    /// `quoteLevelHidden`.
    private var replyHidden: Bool {
        !isReplyRevealed && isBlockedSender(quote.senderID)
    }
    private var parentHidden: Bool {
        guard let parent = quote.parent, !isParentRevealed else { return false }
        return isBlockedSender(parent.senderID)
    }

    /// VoiceOver hears the same two levels the eye sees, innermost last —
    /// the phone's wording, through the same localized keys.
    ///
    /// The SAME separate-code-path leak the phone has, in its own function:
    /// the server sends `excerpt` unchanged by design, so a masked level
    /// still reads its 120 characters aloud unless it is masked here too.
    private var accessibilityText: String {
        let head =
            replyHidden
            ? String(localized: "Replying to a hidden message",
                       comment: "VoiceOver: a quote whose author is blocked")
            : String(localized: "Replying to \(nameFor(quote.senderID)): \(quote.excerpt)")
        guard let parent = quote.parent else { return head }
        let tail =
            parentHidden
            ? String(localized: "which replied to a hidden message",
                       comment: "VoiceOver: the second quote level, when ITS author is blocked")
            : String(localized: "which replied to \(nameFor(parent.senderID)): \(parent.excerpt)")
        return "\(head), \(tail)"
    }
}

/// A photo, video or file inside a Mac balloon.
private struct MacAttachmentBlock: View {
    let attachment: AttachmentDTO
    /// For CONTRAST: an own balloon is filled with the tint, so anything
    /// drawn in the accent colour there would be invisible.
    let isMine: Bool
    /// False when this tile IS the message (a media-only row, no balloon).
    /// Only the hairline reads it — MessagePresentation.drawsHairline.
    var onBalloon: Bool = true

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
                (isMine ? Color.white.opacity(0.14) : Color.primary.opacity(0.05)),
                in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .strokeBorder(isMine ? Color.white.opacity(0.16) : Color.primary.opacity(0.06)))
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
                // A hairline only where the tile's own pixels are not
                // already the edge (MessagePresentation.drawsHairline).
                .overlay {
                    if MessagePresentation.drawsHairline(onBalloon: onBalloon, hasImage: true) {
                        RoundedRectangle(cornerRadius: 12, style: .continuous)
                            .strokeBorder(isMine ? Color.white.opacity(0.18) : Color.primary.opacity(0.08))
                    }
                }
                // Clickable, and on a Mac only the cursor says so.
                .hoverCursor(.pointingHand)
        } else {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(LinearGradient(
                    colors: isMine
                        ? [.white.opacity(0.22), .white.opacity(0.10)]
                        : [.primary.opacity(0.10), .primary.opacity(0.04)],
                    startPoint: .top, endPoint: .bottom))
                .frame(width: 240, height: 180)
                .overlay {
                    if attachment.isVideo {
                        Image(systemName: "play.circle.fill").font(.largeTitle)
                    } else {
                        ProgressView()
                    }
                }
                // A tile with no picture yet is NOT a picture: it is a
                // reserved rectangle holding a wash, and its lower corners
                // die against the window. This branch drew no hairline at
                // all while the loaded one above drew one, so a Mac photo
                // visibly GAINED an edge when its bytes landed. Both now
                // answer to MessagePresentation.drawsHairline — which is
                // unconditionally true here, because there is no picture.
                .overlay(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .strokeBorder(isMine ? Color.white.opacity(0.18) : Color.primary.opacity(0.08)))
        }
    }
}

/// Several photos in one Mac balloon, as a pile: the first at its own
/// shape on a 300pt card, the second and third peeking out behind it,
/// the count in a corner — the phone's AlbumStackView, at Mac size.
///
/// A pile rather than a grid for the phone's reason: a grid crops every
/// photo to one square and leaves an odd one alone at an edge. The card
/// is sized from METADATA (AttachmentAlbum.cardSize) — this thread is
/// the same non-lazy real-heights window the phone's is, and a card that
/// grew when its preview landed would shove it.
///
/// No gestures of its own: the balloon attaches the double-click heart
/// and the single-click open, in that order, exactly as it does to a
/// lone photo. The accessibility action is the one thing that has to
/// live here, because a bare click gesture publishes none.
private struct MacAlbumStack: View {
    /// The message's media, in sent order; at least two.
    let album: AttachmentAlbum
    let isMine: Bool
    /// Opens the viewer at the first item — for VoiceOver, which never
    /// sees the balloon's click.
    let onOpen: () -> Void

    @Environment(AttachmentStore.self) private var store

    /// The balloon's media width, so the pile is as wide as a lone photo.
    private static let cardWidth: CGFloat = 300

    private var card: CGSize {
        AttachmentAlbum.cardSize(for: album.items[0], maxWidth: Self.cardWidth)
    }

    var body: some View {
        // Reading `generation` is what makes this redraw when a fetch
        // lands — the iOS bubble's rule, and the same bug if left out.
        let _ = store.generation
        ZStack(alignment: .bottom) {
            if album.items.count >= 3 {
                layer(album.items[2], .third)
            }
            layer(album.items[1], .second)
            topCard
        }
        // The peek room is reserved and the card sits at the bottom of
        // it, so the tilted corners above are drawn, never clipped —
        // AttachmentAlbum.Layer keeps every corner inside that room.
        .frame(width: card.width, height: card.height + AttachmentAlbum.peek, alignment: .bottom)
        .contentShape(Rectangle())
        // Clickable, and on a Mac only the cursor says so.
        .hoverCursor(.pointingHand)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityText)
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { onOpen() }
    }

    /// "Album, 1 of N" — the viewer's own position wording.
    private var accessibilityText: String {
        let position = String(
            localized: "\(1) of \(album.count)",
            comment: "Which photo of an album is being looked at: the first number is its position, the second the album's size.")
        return "\(String(localized: "Album", comment: "Several photos or videos sent as one message.")), \(position)"
    }

    private var topCard: some View {
        tile(album.items[0])
            .overlay {
                if album.items[0].isVideo {
                    Image(systemName: "play.circle.fill")
                        .font(.system(size: 40))
                        .foregroundStyle(.white, .black.opacity(0.35))
                        .shadow(radius: 3)
                }
            }
            .overlay(alignment: .topTrailing) {
                HStack(spacing: 3) {
                    Image(systemName: "photo.stack")
                    Text(album.count, format: .number)
                }
                .font(.caption2.weight(.medium))
                .foregroundStyle(.white)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(.black.opacity(0.55), in: Capsule())
                .padding(6)
            }
    }

    /// A card behind the top one: shrunk, tilted and lifted, so the pile
    /// reads as several things rather than one card with a thick edge.
    /// Shrunk about the TOP edge — the phone's rule, for the phone's
    /// reason: a centre shrink pulls a tall card's top edge down further
    /// than the lift raises it, and the pile drew as one card
    /// (AttachmentAlbum.Layer has the geometry).
    private func layer(_ item: AttachmentDTO, _ place: AttachmentAlbum.Layer) -> some View {
        tile(item)
            .scaleEffect(place.scale, anchor: .top)
            .rotationEffect(.degrees(place.tilt))
            .offset(y: -place.offset(for: card))
    }

    /// The item's preview filled and clipped into the card's frame, over
    /// the same soft placeholder a lone photo shows until its bytes land.
    private func tile(_ item: AttachmentDTO) -> some View {
        ZStack {
            LinearGradient(
                colors: isMine
                    ? [.white.opacity(0.22), .white.opacity(0.10)]
                    : [.primary.opacity(0.10), .primary.opacity(0.04)],
                startPoint: .top, endPoint: .bottom)
            if let image = store.image(id: item.id, preview: true)
                ?? store.image(id: item.id, preview: false) {
                image
                    .resizable()
                    .aspectRatio(contentMode: .fill)
            }
        }
        .frame(width: card.width, height: card.height)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        // A hairline so a pale photo does not melt into a pale balloon.
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(isMine ? Color.white.opacity(0.18) : Color.primary.opacity(0.08)))
    }
}

#endif
