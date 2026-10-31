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
    @Environment(CallManager.self) private var calls
    @Query(sort: [SortDescriptor(\ChatEntity.pinRank), SortDescriptor(\ChatEntity.lastMessageDate, order: .reverse)])
    private var chats: [ChatEntity]
    @Query private var members: [MemberEntity]
    @Query private var notes: [NoteEntity]

    @Environment(\.openWindow) private var openWindow
    /// Raises the Settings scene (FamilyConnectApp) — the same window ⌘,
    /// and the App menu open, never a second copy of it. The gear used to
    /// present MacSettingsView as a sheet; it now rings the one doorbell
    /// this app has, so the toolbar path people already use survives the
    /// move to a window without becoming a rival panel.
    @Environment(\.openSettings) private var openSettings
    /// The window's video recorder (#79, S8.3): the toolbar cannot be
    /// covered by anything the content draws, so while it is open every
    /// item here is disabled and no sheet opens over it.
    @Environment(VideoMessagePresenter.self) private var videoRecorder: VideoMessagePresenter?
    @State private var selectedChatID: Int64?
    @State private var showingFamily = false
    /// Files were shared into the app and are waiting for a chat: the
    /// picker sheet is up. See ShareImport.
    @State private var showsShareTarget = false
    /// Several families (docs/protocol.md, "Several families"): the two
    /// ways in, opened from the sidebar's family menu, and what an
    /// invitation answered from that menu had to say.
    @State private var showsNewFamily = false
    @State private var showsJoinAnother = false
    @State private var switcherFailure: String?

    /// The chats the CURRENT family lists — every chat, with one family.
    private var listedChats: [ChatEntity] {
        chats.filter {
            FamilyRules.isListed(
                chatFamilyIDs: $0.familyIDs, in: session.currentFamilyID, home: session.homeFamilyID)
        }
    }

    /// The current family's notes, for its board's badge.
    private var familyNotes: [NoteEntity] {
        let familyKey = coordinator.currentFamilyKey
        return notes.filter { $0.familyID == familyKey }
    }

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
                    ForEach(listedChats) { chat in
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
            // The chat list's header: the family menu — the Mac's switcher —
            // and any invitation waiting (docs/protocol.md, "Several
            // families"). Both draw nothing for one family and nothing
            // pending, so the sidebar is unchanged then.
            .safeAreaInset(edge: .top) {
                VStack(spacing: 0) {
                    if session.showsFamilySwitcher {
                        familyMenu
                            .padding(.horizontal, 12)
                            .padding(.top, 6)
                    }
                    InvitationBanner()
                    // A paid family's deadline (docs/protocol.md, "Paid
                    // families"); nothing for any other family.
                    FamilyPlanBanner()
                }
            }
            .safeAreaInset(edge: .bottom) {
                ConnectionBanner()
                    .padding(.bottom, 8)
            }
        } detail: {
            if let selectedChatID, listedChats.contains(where: { $0.chatID == selectedChatID }) {
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
        .navigationTitle(session.family?.name ?? String(localized: "Family"))
        .toolbar {
            // The way back into a call whose window was closed. Closing
            // the Call window does not hang up (FamilyConnectApp), and
            // the thread's own call buttons are disabled while a call is
            // in progress — so without this a closed window was a call
            // with no screen at all until the far side ended it. Only
            // while there is a call to return to. Reopening re-attaches
            // the video surfaces (CallVideoSurface).
            if !calls.isIdle {
                ToolbarItem {
                    Button {
                        openWindow(id: MacWindow.call)
                    } label: {
                        Label("Return to Call", systemImage: "phone.badge.waveform")
                    }
                    .help("Return to Call")
                    .disabled(recorderOpen)
                }
            }
            ToolbarItem {
                // What is new to READ on the board since this Mac last
                // showed it (BoardBadge): notes pinned, and notes whose
                // text changed. Tidying the wall — dragging, resizing,
                // recolouring — is not news and never counts. NOT the sync
                // cursor either; see AppSettings.boardSeenContentSeq.
                //
                // `.badge` on a toolbar item draws only on macOS 26 (the
                // phone found the same on iOS: a silent no-op below); an
                // earlier system gets the chat row's capsule as an overlay.
                if #available(macOS 26, *) {
                    boardButton
                        .badge(newNoteCount)
                        .disabled(recorderOpen)
                } else {
                    boardButton
                        .overlay(alignment: .topTrailing) {
                            if newNoteCount > 0 {
                                Text("\(newNoteCount)")
                                    .font(.caption2.bold())
                                    .foregroundStyle(.white)
                                    .padding(.horizontal, 5)
                                    .padding(.vertical, 1)
                                    .background(Color.accentColor, in: Capsule())
                                    .offset(x: 8, y: -6)
                                    .allowsHitTesting(false)
                                    .accessibilityHidden(true)
                            }
                        }
                        .disabled(recorderOpen)
                }
            }
            ToolbarItem {
                // A new group (docs/protocol.md, "Groups", 2026-10-22): the
                // Mac's door to what the phone offers in its New Chat sheet.
                // Its own view with its own sheet; the group it creates is
                // selected, as a chat opened from the Family sheet is.
                NewGroupToolbarButton { selectedChatID = $0 }
                    .disabled(recorderOpen)
            }
            ToolbarItem {
                Button {
                    showingFamily = true
                } label: {
                    Label("Family", systemImage: "person.2")
                }
                .help("Members, invites and direct chats")
                // Its sheet would change `selectedChatID` under the recorder.
                .disabled(recorderOpen)
            }
            ToolbarItem {
                Button {
                    openSettings()
                } label: {
                    Label("Settings", systemImage: "gearshape")
                }
                .help("Settings")
                .disabled(recorderOpen)
            }
        }
        .sheet(isPresented: $showingFamily) {
            MacFamilyView(onOpenChat: { selectedChatID = $0 })
        }
        .familyDoorSheets(showsNewFamily: $showsNewFamily, showsJoinAnother: $showsJoinAnother)
        .alert(
            "Invitations",
            isPresented: Binding(get: { switcherFailure != nil }, set: { if !$0 { switcherFailure = nil } })
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(switcherFailure ?? "")
        }
        // The phone's alert: a send refused for the family's used storage
        // (`storage_allowance_used`) says why, once.
        .alert(
            coordinator.sendRefusalNotice ?? "",
            isPresented: Binding(
                get: { coordinator.sendRefusalNotice != nil },
                set: { if !$0 { coordinator.sendRefusalNotice = nil } })
        ) {
            Button("OK", role: .cancel) {}
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
            if selectedChatID == nil { selectedChatID = listedChats.first?.chatID }
            consumePendingRoute() // clicked before this window existed
            if session.pendingShareImport != nil { showsShareTarget = true }
            #if DEBUG
            // The Mac App Store capture opens ONE screen per launch, named
            // by `-v1.showScreen` (MacScreenshotRoute). Without that launch
            // argument this does nothing, and a Release build cannot be
            // given it. The chat-level screens — the thread chain and the
            // open polls — belong to MacConversationView and are set there.
            switch MacScreenshotRoute.requested {
            case .family: showingFamily = true
            case .board: openWindow(id: MacWindow.board)
            case .settings: openSettings()
            case .thread, .polls, .none: break
            }
            #endif
        }
        .onChange(of: session.pendingPushRoute) { _, _ in
            consumePendingRoute() // clicked while the window is up
        }
        // What arrived while the recorder covered the window waited for it
        // (#79, S4).
        .onChange(of: recorderOpen) { _, open in
            guard !open else { return }
            consumePendingRoute()
            if session.pendingShareImport != nil {
                showingFamily = false
                showsShareTarget = true
            }
        }
        .onChange(of: session.pendingShareImport) { _, pending in
            // A share arrived while the app is up: the Family sheet steps
            // aside so the picker is what the person sees. Settings is no
            // longer among them — it is a window now, and a window is not
            // something this view may close on somebody's behalf. It also
            // does not need to be: it is not covering this window, and the
            // share URL brings this one to the front.
            guard pending != nil, !recorderOpen else { return }
            showingFamily = false
            showsShareTarget = true
        }
        // A switch to another family lands here too: the open chat is not in
        // the new family's list, so its first chat — its family chat — is.
        .onChange(of: listedChats.map(\.chatID)) { _, ids in
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
        // Not under the video recorder: consumed when it closes.
        guard !recorderOpen else { return }
        guard let route = session.pendingPushRoute else { return }
        session.pendingPushRoute = nil
        // Settings is not cleared here for the reason the share handler
        // above gives: it stopped being a sheet on this window.
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
        case .reports:
            // The Mac's report inbox is a section of the same Family
            // sheet, so this is the same destination.
            guard session.isOwner else { return }
            showingFamily = true
        case .invitations, .familyPlan:
            // Answered from the sidebar's header, which is already on screen
            // — as is a paid family's deadline banner, in the family the
            // push opened.
            break
        case .chatList:
            break // Already here.
        }
    }

    private var recorderOpen: Bool { videoRecorder?.isOpen ?? false }

    private var newNoteCount: Int {
        BoardBadge.unreadCount(
            notes: familyNotes, marks: AppSettings.boardMarks(family: coordinator.currentFamilyKey))
    }

    /// The Mac's switcher: a menu in the chat list's header, titled with the
    /// family on screen (docs/protocol.md, "Several families").
    private var familyMenu: some View {
        Menu {
            FamilySwitcherMenuContent(
                onNewFamily: { showsNewFamily = true },
                onJoinAnother: { showsJoinAnother = true },
                onFailure: { switcherFailure = $0 })
        } label: {
            Label(session.family?.name ?? String(localized: "Family"), systemImage: "person.3")
                .lineLimit(1)
        }
        .menuStyle(.borderlessButton)
        .frame(maxWidth: .infinity, alignment: .leading)
        .help(Text("Switch Family"))
        .accessibilityLabel(Text("Switch Family"))
        .accessibilityValue(Text(session.family?.name ?? String(localized: "Family")))
        .disabled(recorderOpen)
    }


    /// The way into the board; the badge beside it is `newNoteCount`.

    private var boardButton: some View {

        Button {

            openWindow(id: MacWindow.board)

            markBoardSeen()

        } label: {

            Label("Board", systemImage: "square.grid.2x2")

        }

        .help("The family board")

    }

    private func markBoardSeen() {
        let familyKey = coordinator.currentFamilyKey
        AppSettings.setBoardMarks(
            BoardBadge.marksAfterShowing(notes: familyNotes, marks: AppSettings.boardMarks(family: familyKey)),
            family: familyKey)
    }

    /// Open one conversation on its own. Keyed by chat id, so asking
    /// twice raises the window that is already there rather than opening
    /// a second copy of the same thread.
    private func openInWindow(_ chatID: Int64) {
        openWindow(id: MacWindow.conversation, value: chatID)
    }
}

/// The toolbar's New Group button and the sheet it opens.
private struct NewGroupToolbarButton: View {
    let onCreate: (Int64) -> Void

    @State private var showsNewGroup = false

    var body: some View {
        Button {
            showsNewGroup = true
        } label: {
            Label("New Group", systemImage: "person.3")
        }
        .help(Text("New Group"))
        .sheet(isPresented: $showsNewGroup) {
            NewGroupView(onCreate: onCreate)
        }
    }
}

/// One sidebar row: who, the last thing said, when, and what is unread.
private struct MacChatRow: View {
    @Environment(ChatSyncCoordinator.self) private var coordinator

    /// The chat-list preview, with a blocked sender's last message replaced
    /// by the placeholder.
    ///
    /// The COUNT is deliberately untouched: in the family chat a blocked
    /// member's message still moves `unreadCount` and may still be the last
    /// message, because the count is the other half of the read marker and
    /// projecting one without the other desynchronises them.
    private func previewText(for chat: ChatEntity) -> String {
        if let sender = chat.lastMessageSenderID,
            sender != coordinator.currentUserID,
            coordinator.blockedUserIDs.contains(sender)
        {
            return String(localized: "Hidden — blocked member")
        }
        // `??` makes this a String, and Text(String) is verbatim — the
        // fallback has to be localized by hand.
        return chat.lastMessagePreview ?? String(localized: "No messages yet")
    }
    let chat: ChatEntity
    let peerAvatarVersion: Int64

    var body: some View {
        HStack(spacing: 10) {
            InitialsAvatar(
                title: chat.title,
                isFamily: chat.kind == "family",
                isGroup: chat.kind == ChatRules.group,
                userID: chat.peerUserID,
                avatarVersion: peerAvatarVersion,
                size: 34)
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(chat.title)
                        .font(.body.weight(chat.unreadCount > 0 ? .semibold : .regular))
                        // The family chat's title is the family's NAME,
                        // and a group's is the group's — the titles here
                        // that are not a person's — and a sidebar column
                        // cuts a long one; they get a second line (the
                        // iPad's rule).
                        .lineLimit(ChatRules.isFamilyOrGroup(chat.kind) ? 2 : 1)
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
                    // `??` makes this a String, and Text(String) is
                    // verbatim — the fallback has to be localized by hand.
                    // A preview from a blocked sender draws the placeholder
                    // and no sender name — protocol.md settles this in the
                    // `GET /chats` row, and it is the app's most-visited
                    // screen, so leaving it unmasked defeats the block outright.
                    // NOT revealable from the list: the peek belongs to the
                    // thread, and no gesture is wanted here.
                    Text(previewText(for: chat))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    Spacer(minLength: 0)
                    if chat.hasUnreadMention {
                        Text(verbatim: "@")
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.white)
                            .padding(.horizontal, 5)
                            .padding(.vertical, 1)
                            .background(Color.accentColor, in: Capsule())
                            .accessibilityLabel(Text("Mentions you"))
                    }
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
