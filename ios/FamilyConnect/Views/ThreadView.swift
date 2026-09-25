//
//  ThreadView.swift
//  FamilyConnect
//
//  A chain of replies on a surface of its own (docs/protocol.md, "Threads").
//
//  WHY A QUERY AND NOT A LIST OF DTOs. The thread read answers whole
//  messages and they go through the same upsert a page of history does, so
//  this surface reads the STORE — the root and every row that names it —
//  and is live for free: a reply arriving on the socket, a reaction, an
//  edit, the reader's own send from the composer below, all land in the
//  store and this view redraws. A list held from one fetch would be stale
//  the moment the family answered again.
//
//  WHY THE CHAT'S OWN ROW. Rows draw with MessageBubbleView on the phone
//  and MacMessageRow on the Mac — the same bubbles, the same quotes, the
//  same hidden-row rule for a blocked member, the same reactions — because
//  a second, simpler renderer here is a second place for those rules to
//  drift.
//
//  A SHEET EVERYWHERE, like the open polls: a list of messages reads the
//  same at any width.
//
//  Android counterpart: ui/thread/ThreadScreen.kt
//

import SwiftData
import SwiftUI

/// What the sheet opens on: the chat and the root of the chain.
struct ThreadTarget: Identifiable, Hashable {
    let chatID: Int64
    let rootID: Int64
    var id: Int64 { rootID }
}

struct ThreadView: View {
    let chatID: Int64
    let rootID: Int64

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(AppSession.self) private var session
    @Environment(\.dismiss) private var dismiss
    @Query private var rows: [MessageEntity]
    @Query private var chats: [ChatEntity]
    @Query private var members: [MemberEntity]

    @State private var isLoading = true
    @State private var loadFailed = false
    @State private var draft = ""
    /// Rows this reader has peeked at — per row, per surface, never stored
    /// (protocol.md, "Blocking a member").
    @State private var revealed: Set<String> = []
    /// Quote levels peeked at — "<localID>#reply" / "<localID>#parent",
    /// the chat's own key (protocol.md, "Blocking a member").
    @State private var revealedQuotes: Set<String> = []
    #if os(macOS)
    @Environment(\.openWindow) private var openWindow
    #else
    @State private var viewingAlbum: AttachmentAlbum?
    #endif
    @FocusState private var composerFocused: Bool

    init(chatID: Int64, rootID: Int64) {
        self.chatID = chatID
        self.rootID = rootID
        // The root and every row that names it — this reader's own pending
        // replies included, which carry the root from the quote they hold
        // — oldest first, which is how a chain reads.
        _rows = Query(
            filter: #Predicate<MessageEntity> {
                $0.chatID == chatID && ($0.serverID == rootID || $0.threadRootID == rootID)
            },
            sort: \.createdAt)
    }

    var body: some View {
        NavigationStack {
            content
                .navigationTitle("Thread")
                .inlineNavigationTitle()
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Done") { dismiss() }
                            .keyboardShortcut(.cancelAction)
                    }
                }
        }
        #if os(macOS)
        .frame(width: 520, height: 600)
        #endif
        .task {
            loadFailed = !(await coordinator.loadThread(chatID: chatID, rootID: rootID))
            isLoading = false
        }
        #if !os(macOS)
        .sheet(item: $viewingAlbum) { album in
            AttachmentViewer(album: album)
        }
        #endif
    }

    @ViewBuilder
    private var content: some View {
        VStack(spacing: 0) {
            if rows.isEmpty {
                if isLoading {
                    ProgressView()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ContentUnavailableView(
                        "Couldn't load the thread",
                        systemImage: "exclamationmark.triangle",
                        description: Text("Try again in a moment."))
                }
            } else {
                ScrollViewReader { proxy in
                    ScrollView {
                        // Not lazy: a chain is bounded by a family's patience,
                        // not by scrolling, and the lazy stack's height
                        // estimates are the trap the chat already paid for.
                        // Sectioned by day exactly as the chat is: a chain
                        // that runs over several days needs the pills as
                        // much as the chat does, because a bubble carries
                        // only the time.
                        VStack(spacing: 6) {
                            ForEach(sections) { section in
                                #if os(macOS)
                                MacDayPill(day: section.day)
                                #else
                                DayPill(day: section.day)
                                #endif
                                ForEach(section.messages) { message in
                                    row(message)
                                        .id(message.localID)
                                }
                            }
                        }
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                    }
                    .defaultScrollAnchor(.bottom)
                    .onChange(of: rows.count) { _, _ in
                        if let last = snapshots.last {
                            withAnimation { proxy.scrollTo(last.localID, anchor: .bottom) }
                        }
                    }
                }
                if loadFailed {
                    // What is cached is drawn; what could not be fetched is
                    // said, rather than an empty screen over a real chain.
                    Text("Couldn't load the whole thread.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .padding(.bottom, 4)
                }
            }
            Divider()
            composer
        }
    }

    // MARK: - Rows, drawn as the chat draws them

    private var snapshots: [MessageSnapshot] {
        rows.map { MessageSnapshot($0) }
    }

    private var sections: [DaySection] {
        MessagePresentation.daySections(snapshots)
    }

    private var root: MessageEntity? {
        rows.first { $0.serverID == rootID }
    }

    private var chat: ChatEntity? {
        chats.first { $0.chatID == chatID }
    }

    private var isFamilyChat: Bool { chat?.kind == "family" }

    private var currentUserID: Int64 { AppSettings.currentUserID ?? -1 }

    private var familyMemberCount: Int {
        members.filter { !$0.hasLeft && !$0.accountDeleted }.count
    }

    private var memberNames: [Int64: String] {
        Dictionary(members.map { ($0.userID, $0.resolvedDisplayName) }, uniquingKeysWith: { first, _ in first })
    }

    private var avatarVersions: [Int64: Int64] {
        Dictionary(members.map { ($0.userID, $0.avatarVersion) }, uniquingKeysWith: { first, _ in first })
    }

    private func isHiddenByBlock(_ message: MessageSnapshot) -> Bool {
        MessagePresentation.isHiddenByBlock(
            message, blockedUserIDs: coordinator.blockedUserIDs, currentUserID: currentUserID)
    }

    @ViewBuilder
    private func row(_ message: MessageSnapshot) -> some View {
        let isMine = message.senderID == currentUserID
        #if os(macOS)
        MacMessageRow(
            message: message,
            senderName: memberNames[message.senderID],
            nameFor: { memberNames[$0] ?? String(localized: "Someone") },
            avatarVersionFor: { avatarVersions[$0] ?? 0 },
            isMine: isMine,
            showsSenderName: !isMine && isFamilyChat,
            isRead: MessagePresentation.isRead(
                message, othersReadUpTo: chat?.othersReadUpTo ?? 0, isFamilyChat: isFamilyChat),
            // Reply from a row is the composer below: whatever the row, a
            // reply from this surface answers the root. Edit and Report are
            // the chat's, and are not offered here rather than offered inert.
            onReply: { composerFocused = true },
            canEdit: false,
            canReport: false,
            isHiddenByBlock: isHiddenByBlock(message),
            isRevealed: revealed.contains(message.localID),
            onReveal: { revealed.insert(message.localID) },
            isReplyQuoteRevealed: revealedQuotes.contains("\(message.localID)#reply"),
            isParentQuoteRevealed: revealedQuotes.contains("\(message.localID)#parent"),
            onRevealQuote: { isReply in
                revealedQuotes.insert(isReply ? "\(message.localID)#reply" : "\(message.localID)#parent")
            },
            onTapMention: { openMember($0) },
            onOpenAttachment: { open($0, of: message) },
            memberCount: familyMemberCount)
        #else
        MessageBubbleView(
            message: message,
            isMine: isMine,
            showsSenderName: !isMine && isFamilyChat,
            senderName: memberNames[message.senderID],
            senderID: message.senderID,
            senderAvatarVersion: avatarVersions[message.senderID] ?? 0,
            isRead: MessagePresentation.isRead(
                message, othersReadUpTo: chat?.othersReadUpTo ?? 0, isFamilyChat: isFamilyChat),
            reactionChips: MessagePresentation.reactionChips(
                message.reactions, currentUserID: currentUserID),
            reactionDetails: MessagePresentation.reactionDetails(
                message.reactions,
                names: memberNames,
                currentUserID: currentUserID,
                blockedUserIDs: coordinator.blockedUserIDs),
            avatarVersions: avatarVersions,
            memberNames: memberNames,
            currentUserID: currentUserID,
            onTapMention: { openMember($0) },
            onOpenAttachment: { open($0, of: message) },
            onVote: { optionID in
                Task { await coordinator.vote(localID: message.localID, optionID: optionID) }
            },
            onClosePoll: {
                Task { await coordinator.closePoll(localID: message.localID) }
            },
            memberCount: familyMemberCount,
            onToggleReaction: { emoji in
                Task { await coordinator.toggleReaction(localID: message.localID, emoji: emoji) }
            },
            blockedUserIDs: coordinator.blockedUserIDs,
            isHiddenByBlock: isHiddenByBlock(message),
            isRevealed: revealed.contains(message.localID),
            onReveal: { revealed.insert(message.localID) },
            isReplyQuoteRevealed: revealedQuotes.contains("\(message.localID)#reply"),
            isParentQuoteRevealed: revealedQuotes.contains("\(message.localID)#parent"),
            onRevealQuote: { level in
                revealedQuotes.insert(
                    level == .reply ? "\(message.localID)#reply" : "\(message.localID)#parent")
            })
        #endif
    }

    /// Photos and videos open in the viewer, paged through the message's
    /// media exactly as in the chat — the phone's sheet, the Mac's own
    /// resizable window. A file has nothing to open here.
    private func open(_ attachment: AttachmentDTO, of message: MessageSnapshot) {
        guard !attachment.isFile else { return }
        let media = AttachmentAlbum.media(of: message.attachments)
        guard !media.isEmpty else { return }
        let index = media.firstIndex { $0.id == attachment.id } ?? 0
        let album = AttachmentAlbum(items: media, index: index)
        #if os(macOS)
        openWindow(id: MacWindow.attachment, value: album)
        #else
        viewingAlbum = album
        #endif
    }


    // MARK: - Member mentions (docs/protocol.md, "Mentioning a member")

    /// Everybody a mention may name: the current roster, by the name the
    /// app calls them.
    private var mentionRoster: [MentionDTO] {
        members.filter { !$0.hasLeft && !$0.accountDeleted }
            .map { MentionDTO(userID: $0.userID, name: $0.resolvedDisplayName) }
    }

    /// What the strip offers for the prefix being typed: never the reader
    /// themself, never the blocked, and never the assistant — which is not
    /// in the roster and has its own button.
    private func mentionCandidates(matching query: String) -> [MentionDTO] {
        MemberMentions.candidates(
            in: mentionRoster, matching: query,
            excluding: coordinator.blockedUserIDs.union([currentUserID]))
    }

    /// The members the text names, resolved at send — family chat only,
    /// nil when it names nobody so the wire stays as it was.
    private func resolvedMentions(in body: String) -> [MentionDTO]? {
        guard isFamilyChat else { return nil }
        let found = MemberMentions.resolve(body: body, roster: mentionRoster)
        return found.isEmpty ? nil : found
    }

    /// A tap on a name opens the one-to-one chat with that member — the
    /// reader's own name, a member who has left and a deleted account are
    /// highlighted and not tappable.
    private func openMember(_ userID: Int64) {
        guard userID != currentUserID,
              let member = members.first(where: { $0.userID == userID }),
              !member.hasLeft, !member.accountDeleted
        else { return }
        Task {
            if let chatID = try? await coordinator.openDirectChat(with: userID) {
                session.pendingPushRoute = .chat(chatID)
            }
        }
    }

    // MARK: - The composer

    private var canSend: Bool {
        root != nil && !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var composer: some View {
        VStack(alignment: .leading, spacing: 0) {
            if isFamilyChat, let query = MemberMentions.query(in: draft) {
                let candidates = mentionCandidates(matching: query)
                if !candidates.isEmpty {
                    MentionSuggestions(candidates: candidates) { name in
                        draft = MemberMentions.accept(draft: draft, name: name)
                    }
                }
            }
            composerRow
        }
    }

    private var composerRow: some View {
        HStack(alignment: .bottom, spacing: 8) {
            TextField("Reply in thread", text: $draft, axis: .vertical)
                .lineLimit(1...5)
                .textFieldStyle(.plain)
                .focused($composerFocused)
                .onSubmit(send)
            Button(action: send) {
                Image(systemName: "arrow.up.circle.fill")
                    .font(.title2)
            }
            .buttonStyle(.plain)
            .disabled(!canSend)
            .keyboardShortcut(.return, modifiers: .command)
            .accessibilityLabel(Text("Send"))
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    /// A reply to the ROOT, whatever the reader was looking at — the
    /// iMessage rule, and the one that keeps the root's count honest
    /// (protocol.md, "Answering from the thread").
    private func send() {
        guard let root, canSend else { return }
        let body = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        let quote = ReplyToDTO(
            messageID: root.serverID ?? rootID,
            senderID: root.senderID,
            // Cut exactly as the server will, so the bubble and its ack agree.
            excerpt: ReplyToSnapshot.excerpt(of: root.body))
        guard coordinator.send(
            body: body, in: chatID, replyTo: quote, mentions: resolvedMentions(in: body)) != nil
        else { return }
        draft = ""
    }
}
