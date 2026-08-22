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

    @State private var path: [Int64] = []
    @State private var showsNewChat = false
    @State private var showsSettings = false
    @State private var showsJoinRequests = false

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
        }
        .task {
            consumePendingRoute() // parked before this view existed (cold start)
        }
        .onChange(of: session.pendingPushRoute) { _, _ in
            consumePendingRoute() // arrived while the list is up (warm tap)
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

/// Circle with the chat title's initials — the classic no-photo avatar.
struct InitialsAvatar: View {
    let title: String
    var isFamily = false
    /// Who this circle stands for. Given both, the profile picture
    /// replaces the initials once it has been fetched; without them (or
    /// before it lands) the initials are what shows, so a row never
    /// waits on the network to render.
    var userID: Int64?
    var avatarVersion: Int64 = 0
    var size: CGFloat = 44

    @Environment(AvatarStore.self) private var avatars

    private var initials: String {
        let words = title.split(separator: " ").prefix(2)
        let letters = words.compactMap { $0.first.map(String.init) }
        return letters.isEmpty ? "?" : letters.joined().uppercased()
    }

    private var picture: Image? {
        guard let userID, avatarVersion > 0 else { return nil }
        return avatars.image(userID: userID, version: avatarVersion)
    }

    var body: some View {
        ZStack {
            Circle()
                .fill(.tint.opacity(0.2))
            if let picture {
                picture
                    .resizable()
                    .aspectRatio(contentMode: .fill)
            } else if isFamily {
                Image(systemName: "house.fill")
                    .font(.system(size: size * 0.41))
                    .foregroundStyle(.tint)
            } else {
                Text(initials)
                    .font(.system(size: size * 0.36, weight: .semibold))
                    .foregroundStyle(.tint)
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
    }
}
