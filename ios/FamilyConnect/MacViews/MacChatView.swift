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
    @Query private var members: [MemberEntity]
    @Query private var notes: [NoteEntity]

    @Environment(\.openWindow) private var openWindow
    @State private var selectedChatID: Int64?
    @State private var showingSettings = false
    @State private var showingFamily = false
    /// Files were shared into the app and are waiting for a chat: the
    /// picker sheet is up. See ShareImport.
    @State private var showsShareTarget = false

    /// userID → profile-picture version, worked out once for the whole
    /// sidebar rather than per row.
    private var avatarVersions: [Int64: Int64] {
        Dictionary(members.map { ($0.userID, $0.avatarVersion) }, uniquingKeysWith: { first, _ in first })
    }

    var body: some View {
        NavigationSplitView {
            List(selection: $selectedChatID) {
                Section("Chats") {
                    // NOTHING may sit between this row and the List's own
                    // click handling — no tap gesture, no contentShape.
                    //
                    // A `.onTapGesture(count: 2)` here (to open the chat in
                    // its own window) took the click wherever the gesture
                    // was drawn: clicking the name did nothing, clicking the
                    // space beside it selected. `.simultaneousGesture` did
                    // NOT fix it, and adding `.contentShape(Rectangle())`
                    // made it total by extending the gesture over the space
                    // that still worked. Opening in a window is the context
                    // menu's job; selection is the List's, uncontested.
                    ForEach(chats) { chat in
                        MacChatRow(
                            chat: chat,
                            peerAvatarVersion: chat.peerUserID.flatMap { avatarVersions[$0] } ?? 0)
                            .contextMenu {
                                Button("Open in New Window") { openInWindow(chat.chatID) }
                            }
                            .tag(chat.chatID)
                    }
                }
            }
            .listStyle(.sidebar)
            .navigationSplitViewColumnWidth(min: 240, ideal: 280, max: 380)
            .safeAreaInset(edge: .bottom) {
                ConnectionBanner()
                    .padding(.bottom, 8)
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
                    openWindow(id: MacWindow.board)
                    markBoardSeen()
                } label: {
                    Label("Board", systemImage: "square.grid.2x2")
                }
                .help("The family board")
                // Notes pinned since this Mac last showed the board. NOT
                // the sync cursor — see AppSettings.boardSeenNoteID.
                .badge(newNoteCount)
            }
            ToolbarItem {
                Button {
                    showingFamily = true
                } label: {
                    Label("Family", systemImage: "person.2")
                }
                .help("Members, invites and direct chats")
            }
            ToolbarItem {
                Button {
                    showingSettings = true
                } label: {
                    Label("Settings", systemImage: "gearshape")
                }
                .help("Settings")
            }
        }
        .sheet(isPresented: $showingSettings) {
            MacSettingsView()
        }
        .sheet(isPresented: $showingFamily) {
            MacFamilyView(onOpenChat: { selectedChatID = $0 })
        }
        // Where a share INTO the app lands: pick a chat, and the files
        // stage in its composer — nothing is sent until Send there.
        // Dismissing without choosing discards the files (a no-op after a
        // choice, so onDismiss can say it unconditionally).
        .sheet(isPresented: $showsShareTarget, onDismiss: {
            session.discardPendingShareImport()
        }) {
            ShareTargetPicker { chatID in
                session.chooseShareTarget(chatID: chatID)
                showsShareTarget = false
                selectedChatID = chatID
            }
        }
        // ⌘R from the menu bar.
        .onReceive(NotificationCenter.default.publisher(for: .macRequestResync)) { _ in
            Task { await coordinator.activate() }
        }
        .onAppear {
            // Open something rather than an empty pane: the family chat is
            // the one every member has. Selecting it does NOT read it — the
            // thread has to be in front of somebody first (ChatPresence),
            // which is what stops a Mac launched at login behind every other
            // window from quietly clearing the family chat's badge.
            if selectedChatID == nil { selectedChatID = chats.first?.chatID }
            consumePendingRoute() // clicked before this window existed
            if session.pendingShareImport != nil { showsShareTarget = true }
        }
        .onChange(of: session.pendingPushRoute) { _, _ in
            consumePendingRoute() // clicked while the window is up
        }
        .onChange(of: session.pendingShareImport) { _, pending in
            // A share arrived while the app is up: the sheets step aside
            // so the picker is what the person sees.
            guard pending != nil else { return }
            showingSettings = false
            showingFamily = false
            showsShareTarget = true
        }
        .onChange(of: chats.map(\.chatID)) { _, ids in
            if let selectedChatID, ids.contains(selectedChatID) { return }
            selectedChatID = ids.first
        }
    }

    /// Act on a clicked notification, then clear it so it fires once.
    ///
    /// The Mac had no consumer at all: MacAppDelegate parked the route and
    /// nothing ever picked it up, so clicking a banner brought the app
    /// forward showing whatever happened to be open. Same parser and the
    /// same routes as the phone (PushRoute), because the Mac now raises its
    /// own notifications too and two routing schemes would drift.
    private func consumePendingRoute() {
        guard let route = session.pendingPushRoute else { return }
        session.pendingPushRoute = nil
        showingSettings = false
        showingFamily = false
        switch route {
        case .chat(let chatID):
            // No local-existence check, the phone's reasoning: after a
            // reinstall the chat may not be cached yet, and the detail pane
            // says so rather than showing an empty thread.
            selectedChatID = chatID
        case .board:
            openWindow(id: MacWindow.board)
            markBoardSeen()
        case .joinRequests:
            // Pending requests live inside the Family sheet on the Mac.
            // Owner-only; a member who somehow gets this push stays put.
            guard session.isOwner else { return }
            showingFamily = true
        case .chatList:
            break // Already here.
        }
    }

    private var newNoteCount: Int {
        let seen = AppSettings.boardSeenNoteID
        return notes.filter { $0.noteID > seen }.count
    }

    private func markBoardSeen() {
        let highest = notes.map(\.noteID).max() ?? 0
        if highest > AppSettings.boardSeenNoteID {
            AppSettings.boardSeenNoteID = highest
        }
    }

    /// Open one conversation on its own. Keyed by chat id, so asking
    /// twice raises the window that is already there rather than opening
    /// a second copy of the same thread.
    private func openInWindow(_ chatID: Int64) {
        openWindow(id: MacWindow.conversation, value: chatID)
    }
}

/// One sidebar row: who, the last thing said, when, and what is unread.
private struct MacChatRow: View {
    let chat: ChatEntity
    let peerAvatarVersion: Int64

    var body: some View {
        HStack(spacing: 10) {
            InitialsAvatar(
                title: chat.title,
                isFamily: chat.kind == "family",
                userID: chat.peerUserID,
                avatarVersion: peerAvatarVersion,
                size: 34)
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(chat.title)
                        .font(.body.weight(chat.unreadCount > 0 ? .semibold : .regular))
                        .lineLimit(1)
                    Spacer(minLength: 0)
                    if let date = chat.lastMessageDate {
                        // Relative, and short: a sidebar column is narrow
                        // and "yesterday" is what the eye is looking for,
                        // not a timestamp.
                        Text(date, format: .relative(presentation: .numeric, unitsStyle: .narrow))
                            .font(.caption2)
                            .foregroundStyle(.tertiary)
                            .lineLimit(1)
                    }
                }
                HStack(spacing: 6) {
                    Text(chat.lastMessagePreview ?? "No messages yet")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    Spacer(minLength: 0)
                    if chat.unreadCount > 0 {
                        Text("\(chat.unreadCount)")
                            .font(.caption2.weight(.semibold).monospacedDigit())
                            .foregroundStyle(.white)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 1)
                            .background(Color.accentColor, in: Capsule())
                    }
                }
            }
        }
        .padding(.vertical, 3)
    }
}

#endif
