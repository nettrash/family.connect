//
//  Snapshots.swift
//  FamilyConnect
//
//  Plain value-type mirrors of the SwiftData entities plus the pure
//  presentation rules of the conversation screen (day sectioning, when a
//  sender name shows, when a bubble counts as read).
//
//  Why value types at all: @Model classes are MainActor-bound live objects;
//  the grouping/labelling rules below are pure functions that the unit
//  tests want to hammer with synthetic data and no ModelContainer. Views
//  convert entities → snapshots at render time (cheap: a handful of value
//  copies per visible message) and feed these functions.
//

import Foundation

// MARK: - Value mirrors

nonisolated struct ChatSnapshot: Equatable, Sendable, Identifiable {
    var id: Int64 { chatID }
    let chatID: Int64
    let kind: String
    let title: String
    let peerUserID: Int64?
    let unreadCount: Int
    let othersReadUpTo: Int64

    var isFamilyChat: Bool { kind == "family" }
}

/// One user's reaction to one message. Codable in the wire shape
/// (`{"user_id":9,"emoji":"❤️"}`) because MessageEntity persists its full
/// reaction state as exactly this JSON — one spelling, no translation.
nonisolated struct ReactionSnapshot: Equatable, Sendable, Codable {
    let userID: Int64
    let emoji: String

    enum CodingKeys: String, CodingKey {
        case userID = "user_id"
        case emoji
    }

    init(userID: Int64, emoji: String) {
        self.userID = userID
        self.emoji = emoji
    }
}

/// One option of a poll, as the store holds it and the bubble draws it.
///
/// Codable in the WIRE shape (`{"id":5,"text":"Pizza","votes":[7,9]}`)
/// because MessageEntity persists a poll as exactly the JSON object the
/// server sent — one spelling, no translation, and a stored poll that can
/// be diffed against the protocol document.
nonisolated struct PollOptionSnapshot: Equatable, Sendable, Codable, Identifiable {
    let id: Int64
    let text: String
    /// The full current list of user ids that chose this option, in the
    /// order the server holds them (which is the order they voted).
    var votes: [Int64] = []

    enum CodingKeys: String, CodingKey {
        case id
        case text
        case votes
    }

    init(id: Int64, text: String, votes: [Int64] = []) {
        self.id = id
        self.text = text
        self.votes = votes
    }
}

/// A poll's full current state, in the wire shape. See PollOptionSnapshot.
nonisolated struct PollSnapshot: Equatable, Sendable, Codable {
    /// The sequence the state was stamped with — 0 on a poll this device
    /// has only sent optimistically and the server has never confirmed.
    var pollSeq: Int64
    var closed: Bool
    var options: [PollOptionSnapshot]

    enum CodingKeys: String, CodingKey {
        case pollSeq = "poll_seq"
        case closed
        case options
    }

    init(pollSeq: Int64, closed: Bool, options: [PollOptionSnapshot]) {
        self.pollSeq = pollSeq
        self.closed = closed
        self.options = options
    }
}

nonisolated struct MessageSnapshot: Equatable, Sendable, Identifiable {
    var id: String { localID }
    let localID: String
    let serverID: Int64?
    let chatID: Int64
    let senderID: Int64
    let body: String
    let createdAt: Date
    let state: MessageStatus
    /// Full current reaction state; [] both for "never reacted" and
    /// "cleared" — the view only cares whether there is anything to draw.
    var reactions: [ReactionSnapshot] = []
    /// The quoted message when this one is a reply. nil otherwise.
    var replyTo: ReplyToSnapshot?
    /// True once the body has been edited — the bubble says so.
    var isEdited: Bool = false
    /// The FIRST attachment — kept because a dozen call sites (and the
    /// share path, which shares one file) want "the" attachment. Always
    /// `attachments.first` when the bridge built this snapshot.
    var attachment: AttachmentDTO?
    /// EVERY attachment this message carries, in the sender's order —
    /// 1 to 10 of them, [] for a message that carries none.
    var attachments: [AttachmentDTO] = []
    /// The poll this message IS, when it is one — the body being its
    /// question (docs/protocol.md, "Polls"). nil for every other message.
    var poll: PollSnapshot?
    /// The voice call this message records, when it is one. The bubble
    /// then draws the outcome and never the body (docs/protocol.md,
    /// "Voice calls").
    var call: CallDTO?
}

/// The quote a reply draws above its own text.
///
/// `excerpt` is normally the SERVER's — recomputed on every read — but a
/// client cuts its own while a send is still pending, and must cut it the
/// same way or the quote visibly changes length when the ack lands. A flat snapshot, not a
/// reference: the quoted message may be far outside the cached window, or
/// never have been fetched at all.
nonisolated struct ReplyToSnapshot: Equatable, Sendable {
    let messageID: Int64
    let senderID: Int64
    let excerpt: String
    /// One more level, and no more (docs/protocol.md, "Replies").
    var parent: QuotedParentSnapshot?

    /// Longest excerpt, in Unicode scalar values — matching
    /// `ReplyTo::MAX_EXCERPT_CHARS` on the server and `MAX_EXCERPT_CHARS`
    /// on Android.
    static let maxExcerptScalars = 120

    /// Cut like the server does: by SCALARS, not by Swift Characters.
    /// `String.prefix` counts extended grapheme clusters, so a body of
    /// family emoji (7 scalars each) would keep ~7x more text than the
    /// server returns and the quote would visibly shrink on ack.
    static func excerpt(of body: String) -> String {
        String(String.UnicodeScalarView(body.unicodeScalars.prefix(maxExcerptScalars)))
    }
}

/// The second and LAST level of a quote. A distinct type from
/// `ReplyToSnapshot` on purpose: with no `parent` of its own, there is
/// nothing for a third level to live in.
nonisolated struct QuotedParentSnapshot: Equatable, Sendable {
    let messageID: Int64
    let senderID: Int64
    let excerpt: String
}

nonisolated struct MemberSnapshot: Equatable, Sendable, Identifiable {
    var id: Int64 { userID }
    let userID: Int64
    let username: String
    let displayName: String
    let role: String
    let isCurrentUser: Bool
    let hasLeft: Bool
    var avatarVersion: Int64 = 0
    /// True for a deleted account's tombstone — see MemberEntity.
    var accountDeleted: Bool = false

    /// The name to draw: the translated placeholder for a deleted
    /// account, the stored name for everybody else (MemberEntity's
    /// `resolvedDisplayName`, in value form).
    var resolvedDisplayName: String {
        accountDeleted ? MemberDisplay.deletedAccountName : displayName
    }
}

// MARK: - Entity → snapshot bridges (MainActor: entities live there)

extension MessageSnapshot {
    init(_ entity: MessageEntity) {
        self.init(
            localID: entity.localID,
            serverID: entity.serverID,
            chatID: entity.chatID,
            senderID: entity.senderID,
            body: entity.body,
            createdAt: entity.createdAt,
            state: entity.state,
            reactions: entity.reactionList,
            replyTo: entity.replySnapshot,
            isEdited: entity.editSeq > 0,
            attachment: entity.attachmentSnapshot,
            attachments: entity.attachmentList,
            poll: entity.poll,
            call: entity.callSnapshot
        )
    }
}

extension ChatSnapshot {
    init(_ entity: ChatEntity) {
        self.init(
            chatID: entity.chatID,
            kind: entity.kind,
            title: entity.title,
            peerUserID: entity.peerUserID,
            unreadCount: entity.unreadCount,
            othersReadUpTo: entity.othersReadUpTo
        )
    }
}

extension MemberSnapshot {
    init(_ entity: MemberEntity) {
        self.init(
            userID: entity.userID,
            username: entity.username,
            displayName: entity.displayName,
            role: entity.role,
            isCurrentUser: entity.isCurrentUser,
            hasLeft: entity.hasLeft,
            avatarVersion: entity.avatarVersion,
            accountDeleted: entity.accountDeleted
        )
    }
}

// MARK: - Conversation presentation rules

/// One calendar day's worth of messages, in order. `day` is the start of
/// day in the grouping calendar, used both as the section id and to format
/// the "Today / Yesterday / date" pill.
nonisolated struct DaySection: Equatable, Sendable, Identifiable {
    var id: Date { day }
    let day: Date
    let messages: [MessageSnapshot]
    /// `localID` of the row in THIS section that the "N new messages"
    /// divider is drawn above, or nil when the boundary is not in this
    /// section (or not on screen at all).
    ///
    /// Carried by the section rather than worked out again in each thread
    /// for the reason day sectioning itself is: iOS and macOS draw the
    /// same conversation from this one builder, and a boundary computed
    /// twice is a boundary that eventually lands in two different places.
    /// At most one section can carry it — the builder stops looking once
    /// it has been placed.
    var unreadDividerAbove: String? = nil
}

/// One aggregated chip in the reaction row under a bubble: an emoji, how
/// many members chose it, and whether the current user is among them
/// (tints the chip and flips the tap from set to remove).
nonisolated struct ReactionChip: Equatable, Sendable, Identifiable {
    var id: String { emoji }
    let emoji: String
    let count: Int
    let includesMe: Bool
}

/// One emoji's row in the "who reacted" popover: the emoji plus the
/// display names of everyone who chose it, current user rendered as
/// "You" and listed first.
nonisolated struct ReactionDetail: Equatable, Sendable, Identifiable {
    var id: String { emoji }
    let emoji: String
    let names: [String]
    /// The first reactor listed, so the row can lead with their face.
    /// Nil only when the row somehow has no reactors. Android carries the
    /// same thing as its `reactorIds` map.
    let leadUserID: Int64?

    /// Explicit so the ~dozen construction sites that predate the avatar
    /// (tests included) stay valid.
    init(emoji: String, names: [String], leadUserID: Int64? = nil) {
        self.emoji = emoji
        self.names = names
        self.leadUserID = leadUserID
    }
}

/// The pure rules a poll bubble and the poll composer both need, kept out
/// of the views so they can be tested as a table (docs/protocol.md,
/// "Polls" and "Limits").
nonisolated enum PollPresentation {

    /// The protocol's own bounds, so the composer refuses locally what the
    /// server would refuse with `invalid_poll`.
    static let minOptions = 2
    static let maxOptions = 10
    static let maxOptionCharacters = 100

    /// The option this user currently holds, if any. One choice per
    /// member — there is no multiple choice — so the first hit is it.
    static func myOptionID(in poll: PollSnapshot, currentUserID: Int64) -> Int64? {
        poll.options.first { $0.votes.contains(currentUserID) }?.id
    }

    /// Everyone who has voted, counted once. A member can only hold one
    /// option, so this is normally just the sum — but a Set is what makes
    /// the footer honest against a state that somehow said otherwise.
    static func voterIDs(in poll: PollSnapshot) -> Set<Int64> {
        Set(poll.options.flatMap(\.votes))
    }

    /// How full one option's bar is drawn: its share of the votes CAST,
    /// not of the family. 0 while nobody has voted — a poll opens with
    /// every bar empty rather than every bar full.
    static func fraction(of option: PollOptionSnapshot, in poll: PollSnapshot) -> Double {
        let total = poll.options.reduce(0) { $0 + $1.votes.count }
        guard total > 0 else { return 0 }
        return Double(option.votes.count) / Double(total)
    }

    /// The voters on one option whose FACES and NAMES may be drawn — the
    /// full list minus anyone this reader has blocked.
    ///
    /// **Identity only. The arithmetic above is untouched**, and that is the
    /// whole rule: `fraction`, `voterIDs` and the "N of M voted" line all go
    /// on counting a blocked member's vote, because integers are not
    /// presence and a tally that moved when you blocked somebody would tell
    /// you they had voted (protocol.md, "Blocking a member").
    ///
    /// Filtering the votes into the snapshot instead — one filtered list
    /// that both the faces and the sums read — is the tempting shape and
    /// the wrong one: it makes the bars stop summing to the total and turns
    /// every count on the poll into a signal.
    static func drawableVoters(
        of option: PollOptionSnapshot,
        blockedUserIDs: Set<Int64>
    ) -> [Int64] {
        guard !blockedUserIDs.isEmpty else { return option.votes }
        return option.votes.filter { !blockedUserIDs.contains($0) }
    }

    /// What a tap on `optionID` means for this reader: the option they
    /// already hold clears their vote, anything else casts it. The
    /// protocol leaves this to each client deliberately — its own vote is
    /// an idempotent state-set, not a toggle.
    static func tapRetracts(optionID: Int64, in poll: PollSnapshot, currentUserID: Int64) -> Bool {
        myOptionID(in: poll, currentUserID: currentUserID) == optionID
    }

    /// The optimistic local rewrite of a vote: one member holds one
    /// option, so their id comes off every option first and goes back on
    /// exactly one unless they are retracting.
    ///
    /// APPENDED, not inserted: the server orders votes by when they were
    /// cast, and this one has just been.
    ///
    /// The caller must NOT bump `pollSeq` when storing this — only the
    /// server mints sequences, and a bumped one would make the
    /// authoritative reply fail the guard and be dropped.
    static func applyingVote(
        _ poll: PollSnapshot,
        optionID: Int64,
        userID: Int64,
        retracting: Bool
    ) -> PollSnapshot {
        var updated = poll
        updated.options = poll.options.map { option in
            var option = option
            option.votes.removeAll { $0 == userID }
            if !retracting && option.id == optionID { option.votes.append(userID) }
            return option
        }
        return updated
    }

    /// The options as the wire wants them, or nil when they are not a
    /// legal poll: 2–10 of them, each trimmed and non-empty and at most
    /// 100 characters, no two the same ignoring case.
    ///
    /// Returning the CLEANED list rather than a Bool is what stops the
    /// composer validating one string and sending another.
    static func sanitizedOptions(_ options: [String]) -> [String]? {
        let trimmed = options
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        guard (minOptions...maxOptions).contains(trimmed.count) else { return nil }
        guard trimmed.allSatisfy({ $0.count <= maxOptionCharacters }) else { return nil }
        // Case-insensitively distinct, the server's rule — and folded the
        // way the SERVER folds, not the way the reader's language would.
        // `lowercased()` is Unicode's locale-independent mapping on
        // purpose: `lowercased(with: .current)` would make "I" and "ı"
        // collide for a Turkish reader and not for anybody else, so the
        // same two options would be legal on one phone and refused on the
        // next, and only the server's answer would ever be the real one.
        let folded = Set(trimmed.map { $0.lowercased() })
        guard folded.count == trimmed.count else { return nil }
        return trimmed
    }
}

nonisolated enum MessagePresentation {

    /// The quick-set offered by the long-press picker. Client UI only —
    /// the server accepts any emoji (≤ 32 bytes UTF-8), so chips render
    /// whatever arrives; this is just what WE offer to send.
    static let quickReactions = ["❤️", "👍", "👎", "😂", "😮", "😢"]

    /// The reaction a bubble double-tap toggles (the Tapback-heart
    /// idiom). Kept inside the quick set so the capsule shows it
    /// selected. Same value on Android.
    static let doubleTapReaction = "❤️"

    /// Group an already-sorted message list into calendar-day sections.
    /// The input order is preserved inside each section, and sections come
    /// out in the order their first message appears — so a correctly
    /// sorted input yields chronologically sorted sections without a
    /// second sort.
    ///
    /// `firstUnreadID` is the SERVER id of the oldest message the reader
    /// has not seen, decided once at open (UnreadAnchor) and handed in
    /// here so that the section holding it can say which of its rows the
    /// "N new messages" divider belongs above. nil — the ordinary case,
    /// and every open of a chat with nothing unread — puts a divider
    /// nowhere. An id that is not in `messages` (it fell out of the render
    /// window, or was never in it) likewise places nothing, which is the
    /// honest outcome: the divider marks a row, and the row is not here.
    static func daySections(
        _ messages: [MessageSnapshot],
        firstUnreadID: Int64? = nil,
        calendar: Calendar = .current
    ) -> [DaySection] {
        // Built as (day, rows) pairs and only turned into DaySections at
        // the end. The obvious shape — `sections[last] = DaySection(…,
        // messages: sections[last].messages + [message])` — READS the array
        // it is assigning to, which defeats copy-on-write and makes this
        // quadratic in the number of messages in one day. It went unnoticed
        // while the phone only ever passed it a 60-row window; the Mac
        // passed it the whole thread, on every body evaluation.
        var days: [(day: Date, messages: [MessageSnapshot], dividerAbove: String?)] = []
        // At most one, structurally. The divider carries a CONSTANT scroll
        // id (it is the anchored open's target), so a second one drawn from
        // a duplicate row would make `scrollTo` ambiguous — and an ambiguous
        // scroll target is a silent no-op, which is the one failure mode
        // this whole rule is written to avoid.
        var placed = false
        for message in messages {
            let day = calendar.startOfDay(for: message.createdAt)
            if let last = days.indices.last, days[last].day == day {
                days[last].messages.append(message)
            } else {
                days.append((day: day, messages: [message], dividerAbove: nil))
            }
            // The boundary row, if this is it. Written into whichever
            // section the row landed in — which is why it is placed here
            // rather than searched for afterwards.
            if !placed, let firstUnreadID, message.serverID == firstUnreadID,
                let last = days.indices.last {
                days[last].dividerAbove = message.localID
                placed = true
            }
        }
        return days.map {
            DaySection(day: $0.day, messages: $0.messages, unreadDividerAbove: $0.dividerAbove)
        }
    }

    /// Whether this bubble draws as the "Hidden — blocked member" row.
    ///
    /// One rule in one place, because the alternative is the same `contains`
    /// at forty draw sites and a leak wherever somebody forgets it. A
    /// hidden row draws the placeholder and the timestamp and NOTHING else:
    /// no display name, no avatar, no attachment thumbnail, no reaction
    /// chips — all of which come back with the reveal (protocol.md,
    /// "Blocking a member").
    ///
    /// Own messages are never hidden: blocking yourself is refused, so the
    /// guard is belt-and-braces against a corrupt store rather than a real
    /// case.
    static func isHiddenByBlock(
        _ message: MessageSnapshot,
        blockedUserIDs: Set<Int64>,
        currentUserID: Int64
    ) -> Bool {
        guard message.senderID != currentUserID else { return false }
        return blockedUserIDs.contains(message.senderID)
    }

    /// Whether the sender's name caption shows above a bubble.
    ///
    /// Rules: only in the family chat (a direct chat has exactly one other
    /// person — the name would be noise), never on own bubbles, and only
    /// when the sender *changes* relative to the previous bubble in the
    /// same day section (the first bubble of a section has no previous).
    /// A hidden row draws no name at all, and the RUN GROUPING is
    /// unchanged: a hidden bubble still counts as its sender's run, so the
    /// next visible message from somebody else still gets its caption.
    /// Treating a hidden run head as absent instead would silently merge it
    /// into the run above and drop a caption a reader needs.
    static func showsSenderName(
        at index: Int,
        in section: [MessageSnapshot],
        isFamilyChat: Bool,
        currentUserID: Int64,
        blockedUserIDs: Set<Int64> = []
    ) -> Bool {
        guard isFamilyChat, section.indices.contains(index) else { return false }
        let message = section[index]
        guard message.senderID != currentUserID else { return false }
        guard !isHiddenByBlock(message, blockedUserIDs: blockedUserIDs, currentUserID: currentUserID)
        else { return false }
        guard index > 0 else { return true }
        return section[index - 1].senderID != message.senderID
    }

    /// Whether an own message counts as read by someone else: it must be
    /// confirmed (has a server id) and covered by the highest read marker
    /// any other member reported. Pending/failed messages are never read.
    static func isRead(_ message: MessageSnapshot, othersReadUpTo: Int64) -> Bool {
        guard let serverID = message.serverID else { return false }
        return serverID <= othersReadUpTo
    }

    /// True when a message is nothing but photos and/or videos — no
    /// caption, no quote, no poll, no call, nothing still arriving — and
    /// the bubble therefore draws it BARE: no balloon, the way an
    /// emoji-only body draws.
    ///
    /// The balloon exists to give TEXT a surface. A photo brings its own,
    /// and a photo inside a tinted balloon is a frame around a picture —
    /// which is why every mainstream messenger draws a lone photo bare.
    /// Every one of them also keeps the balloon for voice notes, documents
    /// and places, and so does this rule: a file, audio or location row is
    /// words and controls, which need the surface (and its wash and
    /// hairline are cut FOR a balloon — on the chat background they
    /// vanish). A caption, a quote or a row beside the photo keeps the
    /// balloon for the same reason: the words in it need the surface, and
    /// a photo hanging half out of a balloon is the look everyone moved
    /// away from. Same rule on Android (`isMediaOnly` in ChatItems.kt),
    /// pinned by mirrored vectors.
    static func isMediaOnly(_ message: MessageSnapshot, isStreaming: Bool = false) -> Bool {
        guard !isStreaming, message.body.isEmpty, message.replyTo == nil,
              message.poll == nil, message.call == nil,
              !message.attachments.isEmpty else { return false }
        return AttachmentAlbum.rows(of: message.attachments).isEmpty
    }

    /// Whether a lone photo/video tile draws its hairline.
    ///
    /// ONE sentence covers all three surfaces: a media tile draws a
    /// hairline only where its own pixels are not already the edge —
    /// over a balloon, or before the picture has landed.
    ///
    /// The hairline was cut for a balloon ("so a pale photo does not melt
    /// into a pale balloon"), and on a BARE message there is no balloon to
    /// melt into: what is left is a frame around a picture, which is the
    /// thing the bare treatment exists to remove. But a tile with no
    /// picture yet is not a picture — it is a reserved rectangle holding a
    /// 14%→6% wash, and on the chat background (pure white or pure black
    /// on Apple) its lower corners simply stop existing. So the stroke
    /// stays exactly as long as there is nothing else drawing an edge.
    ///
    /// NOT the album's rule. A pile's cards keep their stroke always,
    /// because there it separates photo from PHOTO: at the 6pt overlap the
    /// front card's top edge is the one line between two shots of the same
    /// scene, and before bytes the two washes stack to within 0.002 alpha
    /// of each other. Same rule on Android (`drawsHairline` in
    /// ChatItems.kt), pinned by mirrored vectors.
    static func drawsHairline(onBalloon: Bool, hasImage: Bool) -> Bool {
        onBalloon || !hasImage
    }

    /// Aggregate a message's raw reaction list into the chips the bubble
    /// draws: one per distinct emoji, in the order each emoji first
    /// appears in the list (the server preserves reaction order, so this
    /// is stable across re-renders — no popularity re-sorting jumps).
    static func reactionChips(
        _ reactions: [ReactionSnapshot],
        currentUserID: Int64
    ) -> [ReactionChip] {
        var order: [String] = []
        var counts: [String: Int] = [:]
        var mine: Set<String> = []
        for reaction in reactions {
            if counts[reaction.emoji] == nil { order.append(reaction.emoji) }
            counts[reaction.emoji, default: 0] += 1
            if reaction.userID == currentUserID { mine.insert(reaction.emoji) }
        }
        return order.map { emoji in
            ReactionChip(emoji: emoji, count: counts[emoji] ?? 0, includesMe: mine.contains(emoji))
        }
    }

    /// Expand a message's raw reaction list into the "who reacted"
    /// popover's rows: one per distinct emoji, in the same first-seen
    /// order as `reactionChips` (the two views must agree). Within an
    /// emoji, the current user shows as "You" and comes first; everyone
    /// else keeps reaction-list order under their display name from
    /// `names` ("Someone" when a reactor is no longer a known member).
    /// A blocked reactor is dropped from these ROWS and from nowhere else:
    /// `reactionChips` above keeps its COUNT, so the chip may read 3 while
    /// this popover names two. That gap is deliberate and is visible only
    /// to the blocker, who already knows they blocked somebody — whereas a
    /// count that moved would tell the BLOCKED person they had been
    /// (protocol.md, "Blocking a member").
    static func reactionDetails(
        _ reactions: [ReactionSnapshot],
        names: [Int64: String],
        currentUserID: Int64,
        blockedUserIDs: Set<Int64> = []
    ) -> [ReactionDetail] {
        var order: [String] = []
        var others: [String: [String]] = [:]
        var otherIDs: [String: [Int64]] = [:]
        var mine: Set<String> = []
        for reaction in reactions {
            if others[reaction.emoji] == nil {
                order.append(reaction.emoji)
                others[reaction.emoji] = []
            }
            if reaction.userID == currentUserID {
                mine.insert(reaction.emoji)
            } else if !blockedUserIDs.contains(reaction.userID) {
                others[reaction.emoji, default: []].append(
                    names[reaction.userID] ?? String(localized: "Someone"))
                otherIDs[reaction.emoji, default: []].append(reaction.userID)
            }
        }
        return order.map { emoji in
            let rest = others[emoji] ?? []
            let isMine = mine.contains(emoji)
            let all = isMine ? [String(localized: "You")] + rest : rest
            // Same rule as the names: "You" leads its emoji, so my own id
            // leads it too.
            let lead = isMine ? currentUserID : otherIDs[emoji]?.first
            return ReactionDetail(emoji: emoji, names: all, leadUserID: lead)
        }
    }
}
