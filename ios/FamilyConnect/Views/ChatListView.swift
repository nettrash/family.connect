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
//  sheet can push the freshly created conversation programmatically.
//

import SwiftData
import SwiftUI

struct ChatListView: View {
    @Environment(AppSession.self) private var session
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Query private var chats: [ChatEntity]

    @State private var path: [Int64] = []
    @State private var showsNewChat = false
    @State private var showsSettings = false

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
                            ChatRowView(chat: chat)
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
        }
    }
}

/// One chat row: initials avatar, title, preview, relative time, unread
/// badge. Stock components + semantic colors throughout.
struct ChatRowView: View {
    let chat: ChatEntity

    var body: some View {
        HStack(spacing: 12) {
            InitialsAvatar(title: chat.title, isFamily: chat.kind == "family")

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

    private var initials: String {
        let words = title.split(separator: " ").prefix(2)
        let letters = words.compactMap { $0.first.map(String.init) }
        return letters.isEmpty ? "?" : letters.joined().uppercased()
    }

    var body: some View {
        ZStack {
            Circle()
                .fill(.tint.opacity(0.2))
            if isFamily {
                Image(systemName: "house.fill")
                    .font(.system(size: 18))
                    .foregroundStyle(.tint)
            } else {
                Text(initials)
                    .font(.headline)
                    .foregroundStyle(.tint)
            }
        }
        .frame(width: 44, height: 44)
    }
}
