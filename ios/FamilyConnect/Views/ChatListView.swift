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
    @Environment(CallManager.self) private var calls
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
    /// Files were shared into the app and are waiting for a chat: the
    /// picker sheet is up. See ShareImport.
    @State private var showsShareTarget = false
    /// A system call request naming a contact nobody is linked to yet:
    /// the "who is this?" sheet is up (CallRequestLinkSheet).
    @State private var linkingCall: LinkingCall?
    /// Why a system call request could not be placed.
    @State private var callRequestError: String?

    private struct LinkingCall: Identifiable {
        let id = UUID()
        /// nil when the person is only choosing for this call (a name two
        /// members share) and there is no contact to remember it for.
        let contactIdentifier: String?
        let contactName: String?
        let handle: CallRequest.Handle?
        let video: Bool
    }

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
                    // A ScrollView, not a bare view: the copy promises
                    // pull-to-refresh, and .refreshable only works on
                    // scrollable content. containerRelativeFrame keeps the
                    // placeholder centered while the bounce exists.
                    ScrollView {
                        ContentUnavailableView(
                            "No chats yet",
                            systemImage: "bubble.left.and.bubble.right",
                            description: Text("Pull down to sync with the family server."))
                        .containerRelativeFrame([.horizontal, .vertical])
                    }
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
            .navigationTitle(session.family?.name ?? String(localized: "Chats"))
            .navigationDestination(for: Int64.self) { chatID in
                ConversationView(chatID: chatID)
                    // Rebuild cleanly when the routed chat changes IN
                    // PLACE: a notification tap while another chat is open
                    // replaces the path's element (`path = [chatID]`
                    // below) at the same depth, and without an explicit
                    // identity SwiftUI keeps the old view's @State — a
                    // settled flag, scroll pins, the unread anchor and the
                    // divider of the PREVIOUS chat — and never restarts
                    // its `.task`, so the new chat opened with no opening
                    // routine at all (no first-page fetch on an empty
                    // cache, no anchored open, a stale jump-to-newest
                    // state). The Mac's sidebar has always done this —
                    // MacChatView's `.id(selectedChatID)` — for the same
                    // reason.
                    .id(chatID)
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
                    // A count of notes added since this device last showed
                    // the board — see AppSettings.boardSeenNoteID for why
                    // it is not the sync cursor.
                    //
                    // .badge on a toolbar item renders only on iOS 26+ —
                    // verified empirically: a silent no-op on an iOS 18.6
                    // simulator, drawn on 26.5 — so earlier systems get the
                    // chat row's badge capsule as an overlay instead. The
                    // overlay takes no hits, so the button's tap area is
                    // exactly what it always was.
                    if #available(iOS 26, *) {
                        boardButton
                            .badge(newNoteCount)
                    } else {
                        boardButton
                            .overlay(alignment: .topTrailing) {
                                if newNoteCount > 0 {
                                    Text("\(newNoteCount)")
                                        .font(.caption2.bold())
                                        .foregroundStyle(.white)
                                        .padding(.horizontal, 5)
                                        .padding(.vertical, 1)
                                        .background(.tint, in: Capsule())
                                        .offset(x: 8, y: -6)
                                        .allowsHitTesting(false)
                                        .accessibilityHidden(true)
                                }
                            }
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
            .sheet(isPresented: $showsBoard) {
                BoardView()
            }
            // Fires on both transitions, which is exactly what is wanted:
            // opening clears what is already pinned, closing catches
            // anything that arrived while the board was up.
            .onChange(of: showsBoard) { _, _ in markBoardSeen() }
            // Where a share INTO the app lands: pick a chat, and the files
            // stage in its composer — nothing is sent until Send there.
            // Dismissing without choosing discards the files (a no-op
            // after a choice, so onDismiss can say it unconditionally).
            .sheet(isPresented: $showsShareTarget, onDismiss: {
                session.discardPendingShareImport()
            }) {
                ShareTargetPicker { chatID in
                    session.chooseShareTarget(chatID: chatID)
                    showsShareTarget = false
                    path = [chatID]
                }
            }
        }
        .sheet(item: $linkingCall) { linking in
            CallRequestLinkSheet(contactIdentifier: linking.contactIdentifier, contactName: linking.contactName, handle: linking.handle) { userID in
                place(callTo: userID, video: linking.video)
            }
        }
        .alert("Couldn't place the call", isPresented: .init(
            get: { callRequestError != nil },
            set: { if !$0 { callRequestError = nil } })
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(callRequestError ?? "")
        }
        .onChange(of: session.pendingCallRequest) { _, _ in
            consumePendingCallRequest()
        }
        .task {
            consumePendingRoute() // parked before this view existed (cold start)
            consumePendingCallRequest()
            if session.pendingShareImport != nil { showsShareTarget = true }
        }
        .onChange(of: session.pendingPushRoute) { _, _ in
            consumePendingRoute() // arrived while the list is up (warm tap)
        }
        .onChange(of: session.pendingShareImport) { _, pending in
            // A share arrived while the app is up: everything else steps
            // aside so the picker is what the person sees.
            guard pending != nil else { return }
            showsNewChat = false
            showsSettings = false
            showsJoinRequests = false
            showsBoard = false
            showsShareTarget = true
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
    private var boardButton: some View {
        Button {
            showsBoard = true
        } label: {
            Label("Board", systemImage: "square.grid.2x2")
        }
    }

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

    /// Act on a call the system asked for (CallIntents), then clear it so
    /// it fires once. Who is meant is CallRequestRouter's decision; the
    /// roster gate is the pickers' — not me, not left, not deleted.
    private func consumePendingCallRequest() {
        guard let request = session.pendingCallRequest else { return }
        session.pendingCallRequest = nil
        // One call at a time (docs/protocol.md): a request that arrives
        // mid-call is refused out loud, not dropped — the manager would
        // only have returned false.
        guard calls.isIdle else {
            callRequestError = String(localized: "You're already on a call.", comment: "Alert shown when the Phone app or Siri asked Family to call somebody while a call is already in progress.")
            return
        }
        let links = ContactLinks.shared
        let directory = CallRequestRouter.Directory(
            isActiveMember: { id in
                members.contains { $0.userID == id && !$0.isCurrentUser && !$0.hasLeft && !$0.accountDeleted }
            },
            roster: {
                members.filter { !$0.isCurrentUser && !$0.hasLeft && !$0.accountDeleted }
                    .map { CallRequestRouter.Candidate(userID: $0.userID, name: $0.resolvedDisplayName) }
            },
            linkedMember: { links.userID(linkedTo: $0) },
            memberByPhone: { links.userID(matchingPhone: $0) },
            memberByEmail: { links.userID(matchingEmail: $0) })
        switch CallRequestRouter.resolve(request, in: directory) {
        case .member(let userID):
            place(callTo: userID, video: request.video)
        case .needsChoice(let contactIdentifier, let contactName):
            dismissSheets()
            linkingCall = LinkingCall(contactIdentifier: contactIdentifier, contactName: contactName, handle: request.handle, video: request.video)
        case .unknown:
            callRequestError = String(localized: "Family doesn't know who that is. Link the contact to a family member from the roster first.", comment: "Alert shown when the Phone app or Siri asked Family to call somebody the app cannot match to a family member.")
        }
    }

    /// Open (or create) the direct chat and ring: what the conversation's
    /// own call button does, from outside.
    private func place(callTo userID: Int64, video: Bool) {
        guard session.callsEnabled else {
            callRequestError = String(localized: "Calls are off on this server.")
            return
        }
        dismissSheets()
        Task {
            do {
                let chatID = try await coordinator.openDirectChat(with: userID)
                path = [chatID]
                if !calls.startCall(chatID: chatID, peerUserID: userID, video: video) {
                    callRequestError = String(localized: "You're already on a call.")
                }
            } catch {
                callRequestError = String(localized: "The server couldn't open a chat with them. Try again.")
            }
        }
    }

    /// Every sheet this list can have up: the call screen is a cover on
    /// RootView and cannot present over one of them. The share picker
    /// too — its dismissal discards the parked share, which is the lesser
    /// loss against a call that starts with no screen.
    private func dismissSheets() {
        showsNewChat = false
        showsSettings = false
        showsJoinRequests = false
        showsBoard = false
        showsShareTarget = false
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
                // The initials repeat the title; VoiceOver would read
                // the name twice.
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 2) {
                Text(chat.title)
                    .font(.body.weight(chat.unreadCount > 0 ? .semibold : .regular))
                    .lineLimit(1)
                // `??` makes this a String, and Text(String) is verbatim —
                // the fallback has to be localized by hand.
                Text(chat.lastMessagePreview ?? String(localized: "No messages yet"))
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
                        // A bare number means nothing read aloud.
                        .accessibilityLabel(Text("\(chat.unreadCount) unread"))
                }
            }
        }
        .padding(.vertical, 4)
        // One element per row — title, preview, when, unread — rather
        // than four stops per chat.
        .accessibilityElement(children: .combine)
    }
}


#endif
