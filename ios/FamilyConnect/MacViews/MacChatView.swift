//
//  MacChatView.swift
//  FamilyConnect
//
//  The Mac app's main window: a sidebar of chats beside the open thread.
//
//  This is a SEPARATE view rather than the iOS one with conditionals, and
//  the reason is structural, not cosmetic. The iOS thread is built around
//  things a Mac does not have — a navigation stack you push onto, a
//  keyboard that pushes an input bar up through a safe-area inset, a
//  tap-and-hold menu that floats over a dimmed screen. A Mac has a
//  persistent sidebar, a resizable window, a real cursor and a Return key
//  that sends. Sharing the CORE (sync, models, sockets, media) is what
//  matters; sharing the view code would mean neither platform getting the
//  shape it wants.
//
//  Selection lives here rather than in a navigation path: on the Mac a
//  chat stays open while you use the rest of the app, and the window
//  should still be showing it when you come back.
//

#if os(macOS)

import SwiftData
import SwiftUI

struct MacChatView: View {
    @Environment(AppSession.self) private var session
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Query(sort: [SortDescriptor(\ChatEntity.pinRank), SortDescriptor(\ChatEntity.lastMessageDate, order: .reverse)])
    private var chats: [ChatEntity]

    @State private var selectedChatID: Int64?
    @State private var showingSettings = false

    var body: some View {
        NavigationSplitView {
            List(selection: $selectedChatID) {
                ForEach(chats) { chat in
                    MacChatRow(chat: chat)
                        .tag(chat.chatID)
                }
            }
            .navigationSplitViewColumnWidth(min: 220, ideal: 260, max: 360)
            .safeAreaInset(edge: .bottom) {
                ConnectionBanner()
            }
        } detail: {
            if let selectedChatID, chats.contains(where: { $0.chatID == selectedChatID }) {
                MacConversationView(chatID: selectedChatID)
                    // Rebuild cleanly when the selection moves: the thread
                    // holds per-chat state (draft, scroll, reply) that must
                    // not leak from one conversation into the next.
                    .id(selectedChatID)
            } else {
                ContentUnavailableView(
                    "No conversation selected",
                    systemImage: "bubble.left.and.bubble.right",
                    description: Text("Pick a chat from the sidebar."))
            }
        }
        .navigationTitle(session.family?.name ?? "Family")
        .toolbar {
            ToolbarItem {
                Button {
                    showingSettings = true
                } label: {
                    Label("Settings", systemImage: "gearshape")
                }
            }
        }
        .sheet(isPresented: $showingSettings) {
            MacSettingsView()
        }
        // ⌘R from the menu bar.
        .onReceive(NotificationCenter.default.publisher(for: .macRequestResync)) { _ in
            Task { await coordinator.activate() }
        }
        .onAppear {
            // Open something rather than an empty pane: the family chat is
            // the one every member has.
            if selectedChatID == nil { selectedChatID = chats.first?.chatID }
        }
        .onChange(of: chats.map(\.chatID)) { _, ids in
            if let selectedChatID, ids.contains(selectedChatID) { return }
            selectedChatID = ids.first
        }
    }
}

/// One sidebar row: title, the last thing said, and the unread count.
private struct MacChatRow: View {
    let chat: ChatEntity

    var body: some View {
        HStack(spacing: 10) {
            InitialsAvatar(
                title: chat.title,
                userID: chat.peerUserID ?? 0,
                avatarVersion: 0,
                size: 32)
            VStack(alignment: .leading, spacing: 1) {
                Text(chat.title)
                    .font(.body)
                    .lineLimit(1)
                Text(chat.lastMessagePreview ?? "No messages yet")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 4)
            if chat.unreadCount > 0 {
                Text("\(chat.unreadCount)")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Color.accentColor, in: Capsule())
            }
        }
        .padding(.vertical, 2)
    }
}

#endif
