//
//  ChatListView.swift
//  FamilyConnect
//
//  The active phase's home: family chat pinned on top, direct chats by
//  recency below. Sorting is a two-key sort on (pinRank, lastMessageDate
//  desc) — pinRank exists on the entity precisely so this view needs no
//  special case for the family chat. The date leg is sorted in-memory on
//  the @Query result because SortDescriptor over an optional Date is
//  shakier across OS versions than a three-line comparator; chat lists
//  are tens of rows, not thousands.
//
//  Navigation is path-based (NavigationStack(path:)) so the New Chat
//  sheet can push the freshly created conversation programmatically —
//  and so a tapped push notification can do the same: this view is the
//  consumer of AppSession.pendingPushRoute, which makes cold-start taps
//  work for free (the route waits on the session until bootstrap
//  reaches .active and this view exists to act on it).
//

// iOS only — the Mac has its own views (MacViews/).
#if os(iOS)

import SwiftData
import SwiftUI

struct ChatListView: View {
    @Environment(AppSession.self) private var session
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Query private var chats: [ChatEntity]
    /// The roster, purely to resolve a direct chat's peer to their
    /// profile-picture version — one query for the list rather than one
    /// per row.
    @Query private var members: [MemberEntity]
    @Query private var notes: [NoteEntity]

    @State private var path: [Int64] = []
    @State private var showsNewChat = false
    @State private var showsSettings = false
    @State private var showsJoinRequests = false
    @State private var showsBoard = false

    /// pinRank asc (family first), then recency desc, then stable id.
    private var sortedChats: [ChatEntity] {
        chats.sorted { a, b in
            if a.pinRank != b.pinRank { return a.pinRank < b.pinRank }
            let dateA = a.lastMessageDate ?? .distantPast
            let dateB = b.lastMessageDate ?? .distantPast
            if dateA != dateB { return dateA > dateB }
            return a.chatID < b.chatID
        }
    }

    private var avatarVersions: [Int64: Int64] {
        Dictionary(members.map { ($0.userID, $0.avatarVersion) }, uniquingKeysWith: { first, _ in first })
    }

    var body: some View {
        NavigationStack(path: $path) {
            Group {
                if chats.isEmpty {
                    ContentUnavailableView(
                        "No chats yet",
                        systemImage: "bubble.left.and.bubble.right",
                        description: Text("Pull down to sync with the family server."))
                } else {
                    List(sortedChats) { chat in
                        NavigationLink(value: chat.chatID) {
                            ChatRowView(
                                chat: chat,
                                peerAvatarVersion: chat.peerUserID
                                    .flatMap { avatarVersions[$0] } ?? 0)
                        }
                    }
                    .listStyle(.plain)
                }
            }
            .navigationTitle(session.family?.name ?? "Chats")
            .navigationDestination(for: Int64.self) { chatID in
                ConversationView(chatID: chatID)
            }
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        showsNewChat = true
                    } label: {
                        Label("New Chat", systemImage: "square.and.pencil")
                    }
                }
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        showsSettings = true
                    } label: {
                        Label("Settings", systemImage: "gearshape")
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        showsBoard = true
                    } label: {
                        Label("Board", systemImage: "square.grid.2x2")
                    }
                    // A count of notes added since this device last showed
                    // the board — see AppSettings.boardSeenNoteID for why
                    // it is not the sync cursor.
                    .badge(newNoteCount)
                }
            }
            .safeAreaInset(edge: .top) {
                ConnectionBanner()
            }
            .refreshable {
                await coordinator.resync()
            }
            .sheet(isPresented: $showsNewChat) {
                NewChatView { chatID in
                    path.append(chatID)
                }
            }
            .sheet(isPresented: $showsSettings) {
                SettingsView()
            }
            .sheet(isPresented: $showsJoinRequests) {
                JoinRequestsSheet()
            }
            .sheet(isPresented: $showsBoard) {
                BoardView()
            }
            // Fires on both transitions, which is exactly what is wanted:
            // opening clears what is already pinned, closing catches
            // anything that arrived while the board was up.
            .onChange(of: showsBoard) { _, _ in markBoardSeen() }
        }
        .task {
            consumePendingRoute() // parked before this view existed (cold start)
        }
        .onChange(of: session.pendingPushRoute) { _, _ in
            consumePendingRoute() // arrived while the list is up (warm tap)
        }
        // A chat can now genuinely vanish under a reader: a direct chat
        // whose peer deleted their account goes, both halves (protocol.md,
        // "Deleting an account"). Standing in the pushed thread afterwards
        // means an empty view where every request answers 404, so it is
        // popped — the Mac already does this with its selection.
        //
        // Only what DISAPPEARED. A pushed id that was never in the list is
        // a push-routed chat this device has not cached yet, and popping
        // that is the bug consumePendingRoute deliberately avoids.
        .onChange(of: chats.map(\.chatID)) { old, new in
            let vanished = Set(old).subtracting(new)
            guard !vanished.isEmpty else { return }
            path.removeAll { vanished.contains($0) }
        }
    }

    /// Notes pinned since this device last had the board on screen.
    private var newNoteCount: Int {
        let seen = AppSettings.boardSeenNoteID
        return notes.filter { $0.noteID > seen }.count
    }

    /// The board is on screen, so everything on it has been shown.
    private func markBoardSeen() {
        let highest = notes.map(\.noteID).max() ?? 0
        if highest > AppSettings.boardSeenNoteID {
            AppSettings.boardSeenNoteID = highest
        }
    }

    /// Act on a parked notification tap, then clear it so it fires once.
    private func consumePendingRoute() {
        guard let route = session.pendingPushRoute else { return }
        session.pendingPushRoute = nil
        switch route {
        case .chat(let chatID):
            // Straight to the conversation, over whatever was on screen.
            // No local-existence check: after a reinstall the chat may
            // not be cached yet, and ConversationView + resync handle a
            // not-yet-known id gracefully.
            showsNewChat = false
            showsSettings = false
            showsJoinRequests = false
            path = [chatID]
        case .board:
            showsNewChat = false
            showsSettings = false
            showsJoinRequests = false
            path = []
            showsBoard = true
        case .joinRequests:
            // Owner-only screen; a member who somehow gets this push
            // falls back to the list itself.
            guard session.isOwner else { return }
            showsNewChat = false
            showsSettings = false
            path = []
            showsJoinRequests = true
        case .chatList:
            // "joined" and unknown kinds: this list is the destination.
            break
        }
    }
}

/// Push-route target for kind "join_request": the owner's manage screen,
/// wrapped in its own stack and model so it can be presented directly
/// without walking through Settings. FamilyManageView's own .task loads
/// the fresh model (family, members, pending requests).
private struct JoinRequestsSheet: View {
    @Environment(\.dismiss) private var dismiss
    @State private var model = SettingsModel()

    var body: some View {
        NavigationStack {
            FamilyManageView(settingsModel: model)
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Done") { dismiss() }
                    }
                }
        }
    }
}

/// One chat row: initials avatar, title, preview, relative time, unread
/// badge. Stock components + semantic colors throughout.
struct ChatRowView: View {
    let chat: ChatEntity
    /// Profile-picture version of the direct chat's peer; 0 for the
    /// family chat and for anyone without a picture.
    var peerAvatarVersion: Int64 = 0

    var body: some View {
        HStack(spacing: 12) {
            InitialsAvatar(
                title: chat.title,
                isFamily: chat.kind == "family",
                userID: chat.peerUserID,
                avatarVersion: peerAvatarVersion)

            VStack(alignment: .leading, spacing: 2) {
                Text(chat.title)
                    .font(.body.weight(chat.unreadCount > 0 ? .semibold : .regular))
                    .lineLimit(1)
                Text(chat.lastMessagePreview ?? "No messages yet")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }

            Spacer(minLength: 4)

            VStack(alignment: .trailing, spacing: 4) {
                if let date = chat.lastMessageDate {
                    Text(date, format: .relative(presentation: .named))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
                if chat.unreadCount > 0 {
                    Text("\(chat.unreadCount)")
                        .font(.caption2.bold())
                        .foregroundStyle(.white)
                        .padding(.horizontal, 7)
                        .padding(.vertical, 2)
                        .background(.tint, in: Capsule())
                }
            }
        }
        .padding(.vertical, 4)
    }
}


#endif
