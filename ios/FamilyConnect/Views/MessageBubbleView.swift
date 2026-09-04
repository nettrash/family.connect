//
//  MessageBubbleView.swift
//  FamilyConnect
//
//  One bubble. Mine: trailing, tint background, white text. Theirs:
//  leading, secondarySystemFill — semantic colors so both appearances
//  work without a palette. Emoji-only messages render bare: transparent
//  balloon, glyphs on the EmojiOnly size ladder (identical on Android).
//  So does a message that is nothing but photos or videos: no balloon and
//  no inset, the tile IS the message (MessagePresentation.isMediaOnly).
//  Text bodies go through MessageLinks: URLs, emails and phone numbers
//  render as tappable links (browser / Mail / call).
//  Below the bubble: HH:mm plus, on own
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
//  Double-tapping an acked bubble toggles the quick heart
//  (MessagePresentation.doubleTapReaction), same as picking ❤️ from
//  the capsule.
//

// iOS only — the Mac has its own views (MacViews/).
#if os(iOS)

import SwiftUI

struct MessageBubbleView: View {
    let message: MessageSnapshot
    let isMine: Bool
    /// The assistant is still writing this one, so the bubble shows a
    /// cursor. Purely cosmetic: the row's body is already whatever has
    /// arrived, and the authoritative text lands as an edit either way.
    ///
    /// True for an EMPTY assistant row from the moment it is fanned out,
    /// not from the first delta — a picture answer has no deltas at all,
    /// and the empty row is its whole "still working" state (protocol.md,
    /// "How a picture comes back").
    var isStreaming: Bool = false
    /// An `ai_error` named this row: the answer stopped.
    ///
    /// Only ever drawn where the row would otherwise be BLANK. A text
    /// answer that failed midway keeps whatever arrived and says nothing
    /// extra — a partial answer speaks for itself, and the member can ask
    /// again. A picture answer has nothing to speak with: its body stays
    /// empty by design and the attachment never came, so without this the
    /// row that "has to have somewhere to fail" fails into an empty
    /// balloon.
    var assistantFailed: Bool = false
    let showsSenderName: Bool
    let senderName: String?
    /// Who sent it, for the run-head avatar. Only consulted when
    /// showsSenderName is true, so direct chats never carry one.
    var senderID: Int64 = 0
    var senderAvatarVersion: Int64 = 0
    /// Where this bubble sits in a RUN — consecutive messages from one
    /// sender in one day section (the Mac's and Android's rule, ported).
    /// The corners between two bubbles of a run are tight, so a burst
    /// reads as one voice; the time is stamped once, under the last of
    /// the run, rather than under every bubble. Both default to true, so
    /// a bubble drawn alone (tests, previews) is a whole run.
    var isRunStart: Bool = true
    var isRunEnd: Bool = true
    let isRead: Bool
    var reactionChips: [ReactionChip] = []
    var reactionDetails: [ReactionDetail] = []
    /// userID → profile-picture version, for the faces in the
    /// who-reacted list. Empty is fine; it just means initials.
    var avatarVersions: [Int64: Int64] = [:]
    /// userID → display name, so a quote can name its author.
    var memberNames: [Int64: String] = [:]
    /// Who is reading, so a quote of my own message says "You".
    var currentUserID: Int64 = 0
    /// Tapping a quote asks to jump to the quoted message.
    var onTapQuote: (Int64) -> Void = { _ in }
    /// Tapping a photo or video asks to open it full-screen.
    var onOpenAttachment: (AttachmentDTO) -> Void = { _ in }
    /// Tapping a poll option: cast this vote, or clear it when it is the
    /// one already held (the coordinator decides which).
    var onVote: (Int64) -> Void = { _ in }
    /// The author's one-way close.
    var onClosePoll: () -> Void = {}
    /// How many people could vote, for the poll's footer.
    var memberCount: Int = 0
    /// Tapping a call record rings the peer again. nil when there is
    /// nobody to ring (calls off on this server).
    var onCallBack: (() -> Void)? = nil
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
    /// Everyone this reader has blocked, passed down for the surfaces
    /// inside a bubble that filter by identity — the poll's faces today.
    /// The bubble's OWN hidden-row decision is made by the caller.
    var blockedUserIDs: Set<Int64> = []
    /// This bubble's sender is blocked. Decided by the caller through
    /// `MessagePresentation.isHiddenByBlock`, which already refuses to
    /// hide the reader's own messages — so the `isMine` arms below need no
    /// change.
    var isHiddenByBlock: Bool = false
    /// Peeked at. A peek, not a setting: per row, per device, never on the
    /// wire, and gone on a chat switch.
    var isRevealed: Bool = false
    var onReveal: () -> Void = {}
    /// Which level of a quote a tap revealed. The two are independent
    /// peeks: revealing a quote never reveals the row it points at.
    enum QuoteLevel { case reply, parent }
    var isReplyQuoteRevealed: Bool = false
    var isParentQuoteRevealed: Bool = false
    var onRevealQuote: (QuoteLevel) -> Void = { _ in }

    /// Drives the "who reacted" popover a chip long-press opens.
    @State private var showsReactionDetails = false

    /// The in-flight deferred link open (see handleLinkTap). Non-nil
    /// exactly while a first link-tap waits out the double-tap window.
    @State private var pendingLinkOpen: Task<Void, Never>?

    /// The environment's own openURL, captured BEFORE the bubble
    /// overrides it — the override defers into this, never into itself.
    @Environment(\.openURL) private var systemOpenURL

    /// Dynamic Type factor for the emoji ladder: base 1, scaled with the
    /// body style, so the wrapped value IS the current body-relative
    /// scale. A fixed .system(size:) would invert the feature at
    /// accessibility sizes (emoji-only bubbles smaller than scaled body
    /// text); Android gets the same scaling for free from sp units.
    @ScaledMetric(relativeTo: .body) private var emojiFontScale: CGFloat = 1

    /// Shared preview cache — asking it for a link's state is what
    /// starts the (single, app-wide) fetch for that link.
    @Environment(LinkPreviewLoader.self) private var previewLoader
    /// Held so a HIDDEN row can arm the same fetches a visible one does —
    /// see `armRemoteFetches`.
    ///
    /// OPTIONAL, unlike `previewLoader` above: `@Environment(T.self)` in
    /// its non-optional form TRAPS when the object is absent, and these two
    /// need an `APIClient` to build, which is far more than a layout test
    /// measuring text truncation should have to assemble. The link
    /// preview — the one fetch that reaches a THIRD party, and so the one
    /// that is actually an oracle — stays non-optional and stays armed
    /// unconditionally.
    @Environment(AttachmentStore.self) private var attachmentStore: AttachmentStore?
    @Environment(AvatarStore.self) private var avatars: AvatarStore?

    /// True once this bubble has actually been on screen. The thread
    /// renders a ~60-row window, not just the viewport, so binding the
    /// fetch to rendering would contact every host in the window —
    /// links the reader has not scrolled to, and may never see.
    @State private var hasBeenVisible = false

    /// The web link this bubble would preview, if any. Emoji-only
    /// bodies have no links, and tel:/mailto: are not previewable.
    private var previewableLink: URL? {
        guard !isEmojiOnly else { return nil }
        // The RENDERED text, matching what the balloon draws: markdown
        // deletes characters, so detecting over the raw body previews links
        // the reader cannot see and misses ones they can.
        return MessageLinks.firstWebLinkAsDrawn(in: message.body)
    }

    /// The card to draw under this bubble, once its fetch has landed.
    private var linkPreview: LinkPreview? {
        guard hasBeenVisible, let url = previewableLink,
              case .loaded(let preview) = previewLoader.state(for: url) else { return nil }
        return preview
    }

    /// The blocks the body lays out in — ONE text block unless it carries
    /// a table (MessageMarkdown).
    ///
    /// Emoji-only bodies branch around the renderer here, exactly as they
    /// do on the Mac and on Android: the ladder's whole subject is that the
    /// message is nothing but glyphs, and a markup pass could only take
    /// something away.
    private var bodyBlocks: [MessageMarkdown.Block] {
        if isEmojiOnly { return [.text(AttributedString(message.body))] }
        return MessageLinks.blocks(message.body, isMine: isMine)
    }

    /// True when some other block in the balloon — a link card, a photo, a
    /// table — has already set how wide it is, and the text should wrap
    /// against that width rather than its own.
    private var fillsBalloonWidth: Bool {
        linkPreview != nil
            || message.attachment != nil
            // A poll sets the balloon's width, so the question wraps
            // against the bars rather than floating narrow above them.
            || message.poll != nil
            || bodyBlocks.contains { $0.isTable }
    }

    /// An own message that IS a reply lays its balloon out with
    /// BalloonContentLayout, where the quote keeps the left edge and the
    /// body chooses its own by line count: trailing while it fits in two
    /// lines, back to leading from three (`OwnReplyBodyAlignment` — the
    /// product rule as of 2026-08, refined at the owner's ask; see the
    /// content stack's comment in `bubble`). Everyone else's messages,
    /// and own messages without a quote, keep one left edge.
    /// Drawn as the collapsed placeholder right now.
    private var isHidden: Bool { isHiddenByBlock && !isRevealed }

    /// The collapsed stand-in for a blocked member's message: the
    /// placeholder and the timestamp, and nothing else. No display name, no
    /// avatar, no attachment thumbnail, no reaction chips — all of them
    /// come back with the reveal.
    ///
    /// NO `.frame(height:)`, deliberately. One line of scaled caption
    /// inside the balloon's own paddings IS the thin row, and it stays thin
    /// at every Dynamic Type size; a hard-coded height clips its own
    /// caption at AX5, silently, because this stack is non-lazy and
    /// real-height. It is the same lesson `emojiFontScale` above records
    /// about fixed `.system(size:)`.
    private var hiddenRow: some View {
        HStack(spacing: 6) {
            Image(systemName: "hand.raised.slash")
                .font(.caption)
            Text("Hidden — blocked member")
                .font(.caption)
        }
        .foregroundStyle(.secondary)
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(Color(.secondarySystemFill)))
        .contentShape(Rectangle())
        // A TAP reveals. Not a long press: that opens the menu, which a
        // hidden row does not get.
        .onTapGesture { onReveal() }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text("Hidden message from a blocked member. Double tap to show it."))
        .accessibilityAddTraits(.isButton)
        // Not decoration: a bare `.onTapGesture` publishes no accessibility
        // action, so the trait alone announces a button that does nothing
        // (measured — ZZAXProbeTests). Same lesson as the quote block and
        // the reaction chips.
        .accessibilityAction { onReveal() }
        // A hidden row STILL FETCHES, and draws none of it.
        .task(id: hasBeenVisible) { armRemoteFetches() }
    }

    /// Ask for everything a VISIBLE row would have asked for, and draw none
    /// of it (protocol.md, "Blocking a member").
    ///
    /// This is the requirement with no compile-time signal and no visual
    /// one: `LinkPreviewLoader.state(for:)`, `AttachmentStore.image(id:)`
    /// and `AvatarStore.image(userID:version:)` all START the fetch on the
    /// first ask, and on the hidden path every one of their callers
    /// disappears — so the natural implementation simply stops asking,
    /// reads as obviously correct, passes every test, and rebuilds the
    /// oracle the protocol spends a paragraph refusing: the poster of a
    /// link owns that host's log and can count distinct fetchers before and
    /// after being blocked.
    ///
    /// `preview: true`, because a hidden row "fetches nothing a visible row
    /// would not have fetched yet" — the poster frame, never the 90 MB
    /// video.
    ///
    /// MAP TILES ARE ARMED TOO, and they are the one fetch here that needs
    /// a road of its own. The other three have a loader whose first ask
    /// starts the request, so calling it is enough; a map has no loader —
    /// the `Map` view IS the request, and the `if isHidden` branch below
    /// is exclusive, so a hidden location row never builds one. Hence
    /// `MapTileWarmer`, which makes the same request through
    /// `MKMapSnapshotter` and discards the image. protocol.md names map
    /// tiles in the same breath as the link preview, and the doc decides
    /// this rather than a judgement about whose server log is readable.
    private func armRemoteFetches() {
        // Unconditional, and the load-bearing one: this is the request that
        // leaves the family's own server.
        _ = linkPreview
        // Best-effort. These reach the family's own box, whose operator can
        // see everything anyway, so a context without the stores loses
        // defence in depth rather than the oracle itself.
        if let attachmentStore {
            _ = attachmentStore.generation
            for attachment in message.attachments where !attachment.isFile {
                _ = AttachmentView.image(for: attachment, in: attachmentStore)
            }
        }
        if senderAvatarVersion > 0, let avatars {
            _ = avatars.image(userID: senderID, version: senderAvatarVersion)
        }
        // HIDDEN ONLY, unlike the three above. Those go through loaders
        // that dedupe, so asking twice costs nothing; a snapshotter does
        // not, and a visible row has already asked by drawing its map. A
        // row that fetched twice would be as distinguishable as one that
        // did not fetch at all.
        if isHidden {
            MapTileWarmer.warm(localID: message.localID, attachments: message.attachments)
        }
    }

    private var isOwnReply: Bool {
        isMine && message.replyTo != nil
    }

    /// The balloon's outline: 18pt corners, except the two on the
    /// speaker's side that face another bubble of the same run, which
    /// are tight — the Mac's geometry (MacMessageRow.shape), so three
    /// messages in a row read as one voice rather than three stickers.
    private var balloonShape: UnevenRoundedRectangle {
        let big: CGFloat = 18
        let tight: CGFloat = 5
        return UnevenRoundedRectangle(
            topLeadingRadius: isMine ? big : (isRunStart ? big : tight),
            bottomLeadingRadius: isMine ? big : (isRunEnd ? big : tight),
            bottomTrailingRadius: isMine ? (isRunEnd ? big : tight) : big,
            topTrailingRadius: isMine ? (isRunStart ? big : tight) : big,
            style: .continuous)
    }

    /// The narrowest measure a caption is offered under a photo, a pile
    /// or a poll (BalloonContentLayout's `fillFloor`): the phone's widest
    /// tile, on every idiom. NOT the iPad's 320pt tile — a floor above a
    /// 280pt poll would let its question run past the bars it wraps
    /// against, which is the mismatch the fill rule exists to prevent.
    static let captionFloor: CGFloat = 240

    var body: some View {
        HStack(alignment: .bottom, spacing: 0) {
            if isMine { Spacer(minLength: 48) }

            VStack(alignment: isMine ? .trailing : .leading, spacing: 2) {
                // The branch is INSIDE the VStack, not at the call site.
                // `.id(message.localID)`, the unread-divider id and the
                // bottom sentinel all attach outside this view or to
                // sibling rows, so swapping the children leaves every
                // scroll target, the anchor preference key and the jump
                // highlight untouched — whereas branching at the call site
                // would put a different view type under `.id()` and break
                // all three at once.
                //
                // The timestamp footer below stays: a hidden row draws the
                // placeholder AND the timestamp, and nothing else.
                if isHidden {
                    hiddenRow
                } else {
                if showsSenderName, let senderName {
                    // The sender's face rides the name line at the head of
                    // a run rather than in a gutter beside every bubble:
                    // the thread's layout (and its scroll-anchoring
                    // arithmetic) stays exactly as it was.
                    HStack(spacing: 5) {
                        InitialsAvatar(
                            title: senderName,
                            userID: senderID,
                            avatarVersion: senderAvatarVersion,
                            size: 20)
                        Text(senderName)
                            .font(.caption.weight(.medium))
                            .foregroundStyle(.tint)
                    }
                    .padding(.horizontal, 4)
                    .accessibilityElement(children: .combine)
                }

                bubble
                }

                // The footer rides the LAST bubble of a run, plus any
                // bubble that has something of its own to say there: an
                // edit, or a send still pending or failed — that footer is
                // the retry button, and it cannot wait for the run to end.
                if isRunEnd || message.isEdited || message.state != .sent {
                HStack(spacing: 4) {
                    Text(message.createdAt, format: .dateTime.hour(.twoDigits(amPM: .omitted)).minute(.twoDigits))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    if message.isEdited {
                        // Beside the timestamp, not inside the balloon: it
                        // is metadata about the message, like the time and
                        // the delivery tick, not part of what was said.
                        Text("edited")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                    if isMine {
                        statusGlyph
                    }
                }
                .padding(.horizontal, 4)
                }
            }

            if !isMine { Spacer(minLength: 48) }
        }
        // A toggle landing = the set of chips that include me changing —
        // whether from the optimistic write or the server's echo.
        .sensoryFeedback(.impact(flexibility: .soft), trigger: reactionChips.filter(\.includesMe).map(\.emoji))
        // Arms the preview only once the bubble is really in the
        // viewport — geometry, not onAppear, because in this non-lazy
        // window every row "appears" at creation.
        .onGeometryChange(for: Bool.self) { geometry in
            guard previewableLink != nil, let viewport = geometry.bounds(of: .scrollView) else {
                return false
            }
            let frame = geometry.frame(in: .scrollView)
            return frame.maxY >= 0 && frame.minY <= viewport.height
        } action: { visible in
            if visible { hasBeenVisible = true }
        }
    }

    /// The aggregated reaction chips, inside the balloon and wrapping to
    /// new lines when they outgrow it.
    ///
    /// A tap never takes a reaction away: on a chip the current user is
    /// not part of it joins that reaction (moving theirs if they had a
    /// different one — the protocol allows one per user), and on a chip
    /// they ARE part of it opens the who-reacted list, where their own
    /// row is the explicit "tap to remove". Removing used to ride on the
    /// same press that shows who reacted — a Button action plus a
    /// simultaneous long press fires BOTH, so looking at who reacted
    /// usually took your own reaction off — and undoing something you
    /// never meant to do is a worse failure than one extra tap. No
    /// Button here for the same reason: its action and the long press
    /// are not mutually exclusive.
    private var reactionRow: some View {
        FlowLayout(rowAlignment: isMine ? .trailing : .leading, spacing: 4) {
            ForEach(reactionChips) { chip in
                HStack(spacing: 4) {
                    Text(chip.emoji)
                        // Explicit, and the same number Android uses: the
                        // two were drifting apart (.footnote is 13pt,
                        // bodyLarge is 16sp) for the same UI element.
                        .font(.system(size: 18))
                    if chip.count > 1 {
                        Text("\(chip.count)")
                            .font(.caption.weight(.medium))
                            // The BALLOON's content colour, because with
                            // no fill the count sits directly on the
                            // balloon: .primary is black in light mode,
                            // which is unreadable over the tint. This is
                            // the colour the message text already uses,
                            // so contrast is whatever the bubble already
                            // guarantees.
                            .foregroundStyle(bubbleContentColor)
                            .contentTransition(.numericText())
                    }
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                // No fill. A filled pill has to pick a surface colour,
                // and every choice is wrong somewhere: the app background
                // reads as a black blob on a dark theme, and a wash of
                // the balloon's own colour loses contrast on the tinted
                // side. Transparent sidesteps both — the padding is still
                // the tap target (contentShape below).
                .overlay {
                    if chip.includesMe {
                        // Full strength, never a wash: this hairline is
                        // the ONLY thing separating my reaction from
                        // someone else's (a count of 1 draws no number),
                        // and the tap rule diverges on exactly that. At
                        // half alpha it computes to ~2:1 over the tinted
                        // balloon in dark mode — invisible. Not .tint
                        // either: on my balloon that IS the background.
                        Capsule().strokeBorder(bubbleContentColor, lineWidth: 1)
                    }
                }
                .contentShape(Capsule())
                .onTapGesture { chipPrimaryAction(chip) }
                .onLongPressGesture {
                    showsReactionDetails = true
                }
                .transition(.scale.combined(with: .opacity))
                // Gestures carry no accessibility action of their own and
                // the Button that used to provide one is gone (it fired
                // together with the long press — the bug this replaced),
                // so the chip publishes its own element, trait and both
                // actions explicitly. Android gets these from
                // combinedClickable.
                .accessibilityElement(children: .combine)
                .accessibilityAddTraits(.isButton)
                .accessibilityLabel("\(chip.emoji) \(chip.count)")
                .accessibilityHint(chip.includesMe ? "Shows who reacted" : "Reacts with \(chip.emoji)")
                .accessibilityAction { chipPrimaryAction(chip) }
                .accessibilityAction(named: "See who reacted") {
                    showsReactionDetails = true
                }
            }
        }
        // Breathing room between the message and its reactions — they
        // are about the message, not part of it. Plus the balloon
        // VStack's own 2pt spacing.
        .padding(.top, 6)
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
    /// The current user's own row is the remove control — the only way a
    /// reaction comes off, so it can never happen by accident. Mine-ness
    /// comes from the chips, not from matching the localized "You".
    private var reactionDetailsList: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(reactionDetails) { detail in
                let isMineReaction = reactionChips.first { $0.emoji == detail.emoji }?.includesMe == true
                HStack(spacing: 8) {
                    // The row aggregates one emoji's reactors, so it
                    // leads with the first one's face — matching Android.
                    if let leadUserID = detail.leadUserID {
                        InitialsAvatar(
                            title: detail.names.first ?? "?",
                            userID: leadUserID,
                            avatarVersion: avatarVersions[leadUserID] ?? 0,
                            size: 24)
                    }
                    Text(detail.emoji)
                    Text(detail.names.formatted(.list(type: .and)))
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                    if isMineReaction {
                        Text("Tap to remove")
                            .font(.caption2)
                            .foregroundStyle(.tint)
                    }
                }
                .contentShape(Rectangle())
                .onTapGesture {
                    guard isMineReaction else { return }
                    showsReactionDetails = false
                    onToggleReaction(detail.emoji)
                }
                // My own row is the one and only way to take a reaction
                // off, and a bare gesture is unreachable by VoiceOver —
                // so it gets a real action, named for what it does.
                .accessibilityElement(children: .combine)
                .accessibilityAddTraits(isMineReaction ? .isButton : [])
                .accessibilityAction(named: Text("Remove reaction")) {
                    guard isMineReaction else { return }
                    showsReactionDetails = false
                    onToggleReaction(detail.emoji)
                }
            }
        }
        .padding(12)
    }

    /// The placeholder a masked quote level draws, and the same words a
    /// hidden row draws.
    private static let hiddenCaption = String(
        localized: "Hidden — blocked member",
        comment: "Stands in for a blocked member's message or quoted text")

    /// Is this quote LEVEL masked? Two independent answers, because both
    /// levels carry their own `senderID` and are separately blockable: a
    /// reply BY an unblocked member TO a blocked one, whose own parent is a
    /// third person, is an ordinary shape. One flag for both gets it wrong
    /// in both directions.
    private func quoteLevelHidden(senderID: Int64, revealed: Bool) -> Bool {
        guard !revealed, senderID != currentUserID else { return false }
        return blockedUserIDs.contains(senderID)
    }

    /// The quoted message, drawn inside the balloon above the reply's own
    /// text: a tinted bar, the author, and the server's excerpt.
    ///
    /// It is deliberately NOT a link to a live row — the quoted message may
    /// be pages back or not cached at all, so this draws the snapshot the
    /// server sent and leaves jumping to the tap handler, which degrades to
    /// nothing when the target is not loaded.
    @ViewBuilder
    private func quoteBlock(_ quote: ReplyToSnapshot) -> some View {
        let replyHidden = quoteLevelHidden(
            senderID: quote.senderID, revealed: isReplyQuoteRevealed)
        let parentHidden = quote.parent.map {
            quoteLevelHidden(senderID: $0.senderID, revealed: isParentQuoteRevealed)
        } ?? false
        return HStack(spacing: 6) {
            // The accent bar reads as "quoted" at a glance; on my own
            // balloon it takes the content colour, since .tint on
            // primaryContainer is nearly invisible.
            RoundedRectangle(cornerRadius: 1.5)
                .fill(isMine ? bubbleContentColor : Color.accentColor)
                .frame(width: 3)
            VStack(alignment: .leading, spacing: 1) {
                // The second level first, and quieter: it is context for
                // the quote below it, not the thing being answered. One
                // line only — two levels at two lines each would be a wall
                // of grey above every reply in a busy thread.
                if let parent = quote.parent {
                    Text(
                        parentHidden
                            ? Self.hiddenCaption
                            : "\(quoteAuthorName(senderID: parent.senderID)): \(parent.excerpt)"
                    )
                        .font(.caption2)
                        .foregroundStyle(bubbleContentColor.opacity(0.5))
                        .lineLimit(1)
                }
                // No name AND no excerpt when masked: the quote "draws as
                // the same Hidden — blocked member treatment as a row
                // does, on every surface that draws a quote".
                if !replyHidden {
                    Text(quoteAuthorName(quote))
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(isMine ? bubbleContentColor : Color.accentColor)
                }
                Text(replyHidden ? Self.hiddenCaption : quote.excerpt)
                    .font(.caption)
                    .foregroundStyle(bubbleContentColor.opacity(0.75))
                    .lineLimit(2)
            }
        }
        .padding(.vertical, 4)
        .padding(.trailing, 4)
        // The accent bar above has a width-only frame, which leaves this
        // whole block infinitely flexible in height — and one greedy child
        // makes the balloon's stack distribute height instead of granting
        // it, which is what truncated the reply's own body (see the body
        // Text). Pinning it here fixes the cause rather than the symptom,
        // and it also stops the bar itself running on past the excerpt it
        // is pointing at. Vertical only: the bar's width must stay the 3pt
        // it asks for.
        .fixedSize(horizontal: false, vertical: true)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(Color.primary.opacity(0.06)))
        .contentShape(Rectangle())
        // count: 1 with the balloon's own double-tap still reachable: a
        // bare .onTapGesture on a CHILD wins outright over the parent's
        // count-2 gesture, so double-tapping the quote used to fire two
        // jumps and never the heart. simultaneousGesture lets both see
        // the touch; the 2-tap recognizer claims the double.
        .simultaneousGesture(TapGesture(count: 2).onEnded { toggleQuickHeart() })
        // A masked level reveals instead of jumping. Jumping would page
        // history in and flash a highlight on a row the app is hiding.
        .onTapGesture {
            if replyHidden || parentHidden {
                onRevealQuote(replyHidden ? .reply : .parent)
            } else {
                onTapQuote(quote.messageID)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(
            quoteAccessibilityLabel(quote, replyHidden: replyHidden, parentHidden: parentHidden))
        .accessibilityAddTraits(.isButton)
        // The trait alone is a LIE: a bare .onTapGesture publishes no
        // accessibility action, so VoiceOver announces "button" and
        // double-tapping does nothing (measured — ZZAXProbeTests:
        // tap-only gives accessibilityActivate()=false, tap + this
        // modifier gives true). Same lesson as the reaction chips above
        // and the Android link spans.
        // Moved as a PAIR with the gesture above, for the reason the
        // comment gives: miss this and VoiceOver still fires the jump on a
        // masked quote.
        .accessibilityAction {
            if replyHidden || parentHidden {
                onRevealQuote(replyHidden ? .reply : .parent)
            } else {
                onTapQuote(quote.messageID)
            }
        }
    }

    private func quoteAuthorName(_ quote: ReplyToSnapshot) -> String {
        quoteAuthorName(senderID: quote.senderID)
    }

    /// VoiceOver hears the same two levels the eye sees, innermost last.
    ///
    /// A COMPLETELY SEPARATE code path from the drawn Text above, and the
    /// one everybody forgets: the server sends `excerpt` unchanged by
    /// design, so 120 characters of a blocked member's words sit right here
    /// and get read aloud after the visual mask has landed. Each masked
    /// half contributes the placeholder and never the excerpt or the name.
    private func quoteAccessibilityLabel(
        _ quote: ReplyToSnapshot, replyHidden: Bool, parentHidden: Bool
    ) -> String {
        let head =
            replyHidden
            ? String(localized: "Replying to a hidden message",
                       comment: "VoiceOver: a quote whose author is blocked")
            : String(localized: "Replying to \(quoteAuthorName(quote)): \(quote.excerpt)")
        guard let parent = quote.parent else { return head }
        let tail =
            parentHidden
            ? String(localized: "which replied to a hidden message",
                       comment: "VoiceOver: the second quote level, when ITS author is blocked")
            : String(
                localized:
                    "which replied to \(quoteAuthorName(senderID: parent.senderID)): \(parent.excerpt)")
        return "\(head), \(tail)"
    }

    private func quoteAuthorName(senderID: Int64) -> String {
        if senderID == currentUserID { return String(localized: "You") }
        return memberNames[senderID] ?? String(localized: "Someone")
    }

    private var bubble: some View {
        // Emoji-only messages render bare: no balloon, just the glyphs
        // (the padding stays so the long-press target and the picker
        // anchor keep their size). So does a message that is nothing but
        // photos or videos — the tile is the message, and a balloon
        // around it was a frame around a picture
        // (MessagePresentation.isMediaOnly says what keeps the balloon,
        // and why). Foreground goes primary on both
        // sides — the rare monochrome pictograph (☂, ™) would vanish
        // as white-on-transparent on own messages. Text messages go
        // through MessageLinks so URLs, emails and phone numbers are
        // tappable; emoji-only ones skip the detector.
        //
        // The chips render INSIDE this balloon, under the text: they
        // belong to the message, and outside they made the bubble's
        // footprint ragged.
        VStack(alignment: isMine ? .trailing : .leading, spacing: 2) {
            // Alignment rule, updated 2026-08 at the product owner's ask.
            // Content used to share ONE left edge unconditionally ("text
            // reads from the left; only the balloon has a side"), and that
            // rule is now deliberately reversed for exactly one case: in
            // MY replies a SHORT body — one or two lines — sits against
            // the trailing edge, while the QUOTE keeps the left one. A
            // body that wraps to three lines or more goes back to the
            // leading edge (the owner's follow-up: a right-ragged
            // paragraph reads as an accident). The line count is measured
            // by OwnReplyBodyAlignment, which owns all three
            // alignment-dependent modifiers on the body Text. Everyone
            // else's messages, and own messages without a quote, still
            // share one left edge.
            //
            // How the quote stays left while the body sits right — and
            // the balloon still HUGS its content: BalloonContentLayout. Its
            // header records why a frame cannot do this (the only width a
            // frame can borrow is the proposal, which is the documented
            // full-width-slab regression); the container takes its width
            // from the widest child, Android-style, and each child places
            // against its tagged edge. The load-bearing
            // `.fixedSize(horizontal: false, vertical: true)` on the quote
            // block and the body Text is untouched: vertical pinning is
            // what keeps a reply from truncating its own body
            // (BubbleLayoutTests pins that, and pins the hugging too).
            // EVERY balloon, since 2026-09-03 — not only an own reply's.
            // The plain VStack it replaced could not see a sibling's
            // width, so the caption under a photo or a poll used to be
            // given `.frame(maxWidth: .infinity)` to wrap against it, and
            // that frame made every album-with-caption and every poll a
            // full-width slab (488pt of balloon around a 195pt pile on an
            // iPad). The layout's fill rule measures the photo, pile,
            // poll or card first and offers the caption THAT width, never
            // narrower than the widest tile (`fillFloor`), so the balloon
            // hugs what it holds on every screen size. For everything
            // else this is the VStack it replaced: hugs the widest child,
            // everything on the leading edge.
            BalloonContentLayout(spacing: 2, fillFloor: Self.captionFloor) {
            if let quote = message.replyTo {
                quoteBlock(quote)
            }
            if !message.attachments.isEmpty {
                attachmentBlock(message.attachments)
                    .padding(.bottom, message.body.isEmpty ? 0 : 4)
            }
            // An assistant reply that has not started arriving yet: the row
            // exists (so the bubble is in place and the thread has already
            // scrolled) but there is nothing in it. A cursor says "working"
            // where an empty balloon would look broken.
            if isStreaming && message.body.isEmpty {
                Text(verbatim: "▍")
                    .font(bubbleFont)
                    .foregroundStyle(bubbleContentColor.opacity(0.6))
                    .symbolEffect(.pulse)
            }
            // …and where it stopped instead of starting. The bubble is
            // already in place and already scrolled to; this is the only
            // thing that can go in it.
            if assistantFailed && message.body.isEmpty && message.attachments.isEmpty {
                Label("Couldn't answer that. Ask again.", systemImage: "exclamationmark.circle")
                    .font(bubbleFont)
                    .foregroundStyle(bubbleContentColor.opacity(0.7))
                    .fixedSize(horizontal: false, vertical: true)
            }
            // A call record draws its outcome INSTEAD of the body, which
            // is an English placeholder for clients that predate calls
            // (docs/protocol.md, "Voice calls").
            if let call = message.call {
                CallRecordView(
                    call: call,
                    isMine: isMine,
                    onCallBack: onCallBack,
                    onDoubleTap: { toggleQuickHeart() },
                    onLongPress: { onLongPress() })
                    .foregroundStyle(bubbleContentColor)
            }
            // A photo needs no caption, and an empty Text would still take
            // a line's height inside the balloon.
            else if !message.body.isEmpty {
                // One text block — everything without a table — comes back
                // as exactly this `Text` and nothing around it. A table
                // stacks blocks instead, and only then.
                MessageBodyBlocks(
                    blocks: bodyBlocks,
                    isStreaming: isStreaming,
                    isMine: isMine
                ) { text, carriesCursor in
                    Text(text)
                        .font(bubbleFont)
                        .foregroundStyle(bubbleContentColor)
                        // WHY A REPLY USED TO LOSE ITS LAST LINES.
                        //
                        // The quote's accent bar is a Shape with a width-only
                        // frame, so it is infinitely flexible in HEIGHT. One
                        // such child flips this VStack out of "everybody gets
                        // their ideal height" into height DISTRIBUTION: SwiftUI
                        // sizes the least flexible child first and offers each
                        // an equal share of what is left. A Text is finitely
                        // flexible (it can always shed lines), so it was sized
                        // FIRST, handed roughly half the balloon, and a Text
                        // given less height than it needs does not wrap — it
                        // TRUNCATES with an ellipsis. Hence a reply ending in
                        // "…" with blank space under it, and only ever a reply:
                        // the quote is the only height-greedy block a balloon
                        // can hold (attachments, link cards and the map all pin
                        // their height).
                        //
                        // This is the modifier macOS has always had
                        // (MacMessageRow's body Text) and is why the Mac never
                        // showed the bug. `horizontal: false` is load-bearing:
                        // it cannot touch width, so it cannot re-trigger the
                        // full-width-balloon regression the comments above and
                        // below warn about. The quote block below is pinned too,
                        // which is the cause rather than the symptom.
                        .fixedSize(horizontal: false, vertical: true)
                        // While the assistant is writing, the text ends in a
                        // cursor rather than just stopping mid-word — on the
                        // LAST text block, which is the only one still growing.
                        .overlay(alignment: .bottomTrailing) {
                            if isStreaming, carriesCursor {
                                Text(verbatim: "▍")
                                    .font(bubbleFont)
                                    .foregroundStyle(bubbleContentColor.opacity(0.6))
                                    .offset(x: 8)
                            }
                        }
                        // The reply rule, both of its modifiers at once: the
                        // line ragging and the BalloonContentLayout edge tag,
                        // from ONE measured line count (trailing through two
                        // lines, leading from three). Placed AFTER the cursor
                        // overlay, which does not change the Text's size, so
                        // the height it counts lines from is the Text's own.
                        // Off (plain leading, nothing measured) for every
                        // message that is not an own reply.
                        //
                        // `fillsWidth`: a sibling block — a link card, a
                        // photo, a poll — has already decided how wide this
                        // balloon is, and the text should wrap against the
                        // same edge. It used to do that with a width-filling
                        // FRAME, which took the whole column and made every
                        // captioned photo and every poll a full-width slab;
                        // it is now a TAG the layout reads, and the layout
                        // offers the caption the sibling's width (see
                        // BalloonContentLayout's fill rule). Gated on there
                        // BEING such a block.
                        .ownReplyBodyAlignment(
                            enabled: isOwnReply,
                            font: bubbleFont,
                            fillsWidth: fillsBalloonWidth)
                }
                // The colour again, one level up: a table's cells are Texts
                // of their own and have no colour of their own, and on my
                // balloon .primary is black on the tint.
                .foregroundStyle(bubbleContentColor)
            }

            if let preview = linkPreview {
                LinkPreviewCard(
                    preview: preview,
                    image: previewLoader.image(for: preview.url),
                    onOpen: { systemOpenURL($0) },
                    onLongPress: { onLongPress() },
                    onDoubleTap: { toggleQuickHeart() })
                    .padding(.top, 4)
            }

            // Under the question, which IS the body above (docs/
            // protocol.md, "Polls"). Shared with the Mac.
            if let poll = message.poll {
                PollBubbleView(
                    poll: poll,
                    currentUserID: currentUserID,
                    isAuthor: isMine,
                    // A vote needs a server message id to PUT against —
                    // the same gate a reaction has.
                    isVotable: message.serverID != nil,
                    memberCount: memberCount,
                    memberNames: memberNames,
                    avatarVersions: avatarVersions,
                    isMine: isMine && !isBare,
                    blockedUserIDs: blockedUserIDs,
                    onVote: onVote,
                    onClose: onClosePoll,
                    onDoubleTap: { toggleQuickHeart() },
                    onLongPress: { onLongPress() })
            }

            }

            if !reactionChips.isEmpty {
                reactionRow
            }
        }
            // A media-only message drops the balloon's inset along with
            // its fill: the tile's edge lands where the balloon's edge
            // would have, so a bare photo is not a photo floating 12pt
            // inside an invisible frame. Emoji keep the inset — a glyph
            // has no edge of its own, and the padding is its long-press
            // target. The tile carries the gestures itself.
            .padding(.horizontal, isMediaOnly ? 0 : 12)
            .padding(.vertical, isMediaOnly ? 0 : 8)
            .background(
                isBare
                    ? AnyShapeStyle(Color.clear)
                    : isMine ? AnyShapeStyle(.tint) : AnyShapeStyle(Color(.secondarySystemFill)),
                in: balloonShape)
            // The floating reaction picker grows out of this exact rect;
            // the parent resolves it from the preference by localID. Empty
            // unless this bubble is the picker's host (see publishesAnchor).
            .anchorPreference(key: BubbleAnchorKey.self, value: .bounds) {
                publishesAnchor ? [message.localID: $0] : [:]
            }
            .onTapGesture(count: 2) {
                // Double-tap = the quick heart (Tapback idiom), through
                // the same toggle path as the capsule, so a second
                // double-tap removes it. Only reachable over NON-link
                // glyphs — Text's internal link tap preempts this
                // gesture, so over links the heart comes from
                // handleLinkTap's own double-tap arbitration.
                toggleQuickHeart()
            }
            .onLongPressGesture {
                onLongPress()
            }
            // Text fires link taps through the environment's openURL —
            // and, measured on device: it fires them for EACH tap of a
            // double-tap while the count-2 gesture above never runs over
            // link glyphs. So the override below is the arbitration
            // layer: defer every open past the double-tap window, and
            // treat a second fire inside the window as the heart.
            .environment(\.openURL, OpenURLAction { url in
                handleLinkTap(url)
                return .handled
            })
    }

    /// What a bubble draws for its attachment set.
    ///
    /// ONE attachment is exactly the block this bubble has always drawn —
    /// the thumbnail at the attachment's own shape, or the file/audio/
    /// location row. SEVERAL: the photos and videos (AttachmentAlbum's
    /// cut, in SENT order) become one pile when there are two or more and
    /// stay the ordinary free-shape thumbnail when there is one, and any
    /// files, audio or location stack under it as the rows they have
    /// always been. The pile opens its first item; the viewer pages from
    /// there. No new image pipeline — the pile draws the same previews.
    @ViewBuilder
    private func attachmentBlock(_ attachments: [AttachmentDTO]) -> some View {
        if attachments.count == 1, let attachment = attachments.first {
            AttachmentView(
                attachment: attachment,
                onOpen: { onOpenAttachment(attachment) },
                onLongPress: { onLongPress() },
                onDoubleTap: { toggleQuickHeart() },
                isMine: attachmentsOnTint,
                onBalloon: !isMediaOnly)
        } else {
            let media = AttachmentAlbum.media(of: attachments)
            let rows = AttachmentAlbum.rows(of: attachments)
            VStack(alignment: isMine ? .trailing : .leading, spacing: 4) {
                if media.count >= 2 {
                    AlbumStackView(
                        album: AttachmentAlbum(items: media, index: 0),
                        onOpen: { onOpenAttachment(media[0]) },
                        onLongPress: { onLongPress() },
                        onDoubleTap: { toggleQuickHeart() },
                        isMine: attachmentsOnTint)
                } else if let single = media.first {
                    AttachmentView(
                        attachment: single,
                        onOpen: { onOpenAttachment(single) },
                        onLongPress: { onLongPress() },
                        onDoubleTap: { toggleQuickHeart() },
                        isMine: attachmentsOnTint,
                        onBalloon: !isMediaOnly)
                }
                ForEach(rows) { attachment in
                    AttachmentView(
                        attachment: attachment,
                        onOpen: { onOpenAttachment(attachment) },
                        onLongPress: { onLongPress() },
                        onDoubleTap: { toggleQuickHeart() },
                        isMine: attachmentsOnTint,
                        // A row always keeps its balloon (isMediaOnly is
                        // false whenever one is present), so this is
                        // always true — one rule, passed everywhere.
                        onBalloon: !isMediaOnly)
                }
            }
        }
    }

    /// One tap on a link run. First fire: schedule the open after the
    /// double-tap window. Second fire inside the window: the user is
    /// double-tapping link glyphs — cancel the open and heart instead
    /// (falling back to opening when the bubble cannot take a reaction
    /// yet, so a pending bubble's links never go dead).
    private func handleLinkTap(_ url: URL) {
        if pendingLinkOpen != nil {
            pendingLinkOpen?.cancel()
            pendingLinkOpen = nil
            if message.serverID != nil {
                toggleQuickHeart()
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

    /// What tapping a chip does — the one place the rule lives, shared
    /// by the touch gesture and the accessibility action.
    private func chipPrimaryAction(_ chip: ReactionChip) {
        if chip.includesMe {
            showsReactionDetails = true
        } else {
            onToggleReaction(chip.emoji)
        }
    }

    /// The double-tap reaction, gated exactly like the capsule: a
    /// reaction needs a server message id.
    private func toggleQuickHeart() {
        guard message.serverID != nil else { return }
        onToggleReaction(MessagePresentation.doubleTapReaction)
    }

    /// True when the body is nothing but emoji — the bare, big-glyph
    /// treatment. Cheap for ordinary text: the scan stops at the first
    /// non-emoji scalar.
    private var isEmojiOnly: Bool {
        EmojiOnly.displayFontSize(for: message.body) != nil
    }

    /// True when the message is nothing but photos and/or videos — the
    /// other bare treatment. The rule, and why files, audio and places
    /// are not in it, lives in MessagePresentation.isMediaOnly.
    private var isMediaOnly: Bool {
        MessagePresentation.isMediaOnly(message, isStreaming: isStreaming)
    }

    /// True when the balloon draws with no fill — an emoji-only body or a
    /// media-only message. Everything that adapts to "nothing behind me"
    /// (the content colour, the chips' outline, the poll's tone) keys off
    /// this, never off one of its halves.
    private var isBare: Bool { isEmojiOnly || isMediaOnly }

    /// The tone the attachment blocks draw against. Their `isMine` is a
    /// CONTRAST switch (white hairline and placeholder over the tint),
    /// so a bare tile — sitting on the chat background, not the tint —
    /// takes the neutral side whichever side of the thread it is on.
    private var attachmentsOnTint: Bool { isMine && !isMediaOnly }

    /// What everything drawn ON the balloon derives from: white on my
    /// tinted balloon, primary otherwise — including bare bubbles
    /// (emoji-only, media-only), where white would vanish against the
    /// chat background. One rule keeps the text, the chips and their
    /// outlines legible on both balloon tones.
    private var bubbleContentColor: Color {
        isMine && !isBare ? .white : .primary
    }

    /// Emoji-only messages render on the EmojiOnly size ladder (one
    /// emoji biggest through four smallest), scaled with Dynamic Type;
    /// everything else is body text. Same ladder as Android.
    private var bubbleFont: Font {
        if let size = EmojiOnly.displayFontSize(for: message.body) {
            return .system(size: size * emojiFontScale)
        }
        return .body
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

#endif
