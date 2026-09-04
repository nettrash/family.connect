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
//  TWO SHAPES, ONE STATE (issue #43). On the phone this is what it has
//  always been: a NavigationStack whose path is the open chat. On the
//  iPad the same list is the SIDEBAR of a NavigationSplitView and the
//  conversation fills the detail column, because a plain full-width list
//  on a 13-inch display is a row of avatars with a disclosure chevron a
//  thousand points away and three quarters of the screen blank.
//
//  `path` stays the single source of truth for BOTH shapes, and that is
//  the whole reason this conversion is small: `path.last` IS the split
//  view's selection (see `selection` below), so every one of the eight
//  places that write the path — the New Chat sheet, the share picker, a
//  vanished chat, a Siri/Recents call, and the four push routes — keeps
//  working unread and unedited, on both shapes, including the cold-start
//  ones that are hardest to test. Nothing about routing moved; only the
//  container the route lands in did.
//
//  The shape is chosen by IDIOM, not by size class, and deliberately:
//  a size-class test would swap NavigationStack for NavigationSplitView
//  under a running conversation — a different view identity, so the
//  thread is torn down and rebuilt — every time a Pro Max phone is
//  rotated or an iPad enters Slide Over. Choosing by idiom makes the
//  phone's tree literally the one it has today, and leaves the iPad's
//  narrow states to NavigationSplitView's own collapsing (which is a
//  stack, presented by the framework rather than by us).
//

// iOS only — the Mac has its own views (MacViews/).
#if os(iOS)

import SwiftData
import SwiftUI
// UIDevice.userInterfaceIdiom, for the shape decision above. SwiftUI has
// no environment key for the idiom — only for the size class, which is
// the thing that must NOT decide this.
import UIKit

struct ChatListView: View {
    /// Read for ONE thing only — whether the sidebar has a title — and
    /// never for the shape (see the header). At regular width the sidebar
    /// bar holds four buttons and iOS 26 squeezes an inline title beside
    /// them to "Ch…", so it is left blank there; collapsed to a stack in a
    /// compact width (Slide Over) the same bar has room and the title is
    /// also the detail's Back label.
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
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
    @State private var showsReports = false
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

    /// iPad or phone — see the header for why this is the IDIOM and not
    /// the horizontal size class.
    private var usesSplitView: Bool {
        UIDevice.current.userInterfaceIdiom == .pad
    }

    /// The split view's selection, projected onto the SAME `path` the
    /// stack navigates — the projection itself is `ChatSelection`, which
    /// is where it can be tested without a screen.
    private var selection: Binding<Int64?> {
        Binding(
            get: { ChatSelection.openChat(in: path) },
            set: { path = ChatSelection.path(opening: $0) })
    }

    /// "Chats", on both shapes, and no longer the family's name. A large
    /// title cannot wrap and cannot shrink, so any family name past
    /// ~16 characters came out as "The Harper-Whittington…" on a phone
    /// and was dropped altogether from the iPad sidebar, whose bar holds
    /// four buttons and had no room left for a title at all. The family's
    /// name now lives where it can be read whole: on the family chat's
    /// own row (two lines allowed, `ChatRowView`), on the iPad's empty
    /// detail column, and in Settings.
    private var navigationTitleText: String {
        String(localized: "Chats")
    }

    /// What the family is called, for the places that draw it whole.
    private var familyName: String {
        session.family?.name ?? String(localized: "Family")
    }

    /// Members still in the family, for the detail column's placeholder.
    private var activeMemberCount: Int {
        members.filter { !$0.hasLeft && !$0.accountDeleted }.count
    }

    var body: some View {
        Group {
            if usesSplitView {
                splitShape
            } else {
                stackShape
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

    // MARK: - The two shapes

    /// The phone. This is the tree this view has always had, verbatim —
    /// same container, same list, same NavigationLink, same destination,
    /// same modifier order — because "the iPhone is unchanged" is a claim
    /// worth being able to make by reading rather than by testing.
    private var stackShape: some View {
        NavigationStack(path: $path) {
            presentations(
                on: chatList(selectable: false)
                    .navigationTitle(navigationTitleText)
                    .navigationDestination(for: Int64.self) { chatID in
                        ConversationView(chatID: chatID)
                            // Rebuild cleanly when the routed chat changes IN
                            // PLACE: a notification tap while another chat is
                            // open replaces the path's element (`path =
                            // [chatID]` in `consumePendingRoute`) at the same
                            // depth, and without an explicit identity SwiftUI
                            // keeps the old view's @State — a settled flag,
                            // scroll pins, the unread anchor and the divider
                            // of the PREVIOUS chat — and never restarts its
                            // `.task`, so the new chat opened with no opening
                            // routine at all (no first-page fetch on an empty
                            // cache, no anchored open, a stale jump-to-newest
                            // state). The Mac's sidebar has always done this —
                            // MacChatView's `.id(selectedChatID)` — for the
                            // same reason, and the split shape below does it
                            // too.
                            .id(chatID)
                    }
                    .toolbar { chatListToolbar }
                    .safeAreaInset(edge: .top) {
                        ConnectionBanner()
                    }
                    .refreshable {
                        await coordinator.resync()
                    })
        }
    }

    /// The iPad. MacChatView's shape, with the Mac's column width, so the
    /// two big-screen clients do not drift apart.
    private var splitShape: some View {
        NavigationSplitView {
            presentations(
                on: chatList(selectable: true)
                    .navigationTitle(horizontalSizeClass == .compact ? navigationTitleText : "")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar { chatListToolbar }
                    // In a COMPACT window (Slide Over, a third of Split
                    // View) the split view is a stack and the detail — and
                    // the banner it carries — is off screen while the list
                    // shows. The phone's placement, for the phone's shape.
                    .safeAreaInset(edge: .top) {
                        if horizontalSizeClass == .compact {
                            ConnectionBanner()
                        }
                    }
                    .refreshable {
                        await coordinator.resync()
                    })
                // Wider than the Mac's 280: the same rows are set in the
                // iPad's 17pt body, not the Mac's 13pt, and at 280 a
                // two-word family name and its time shared 248pt. The
                // minimum is what an iPad mini gets in portrait, and 280
                // is where "Whittington Family" stops losing its last word.
                .navigationSplitViewColumnWidth(min: 280, ideal: 320, max: 400)
        } detail: {
            detailColumn
                // The banner is on the DETAIL column, top edge, where the
                // phone has it — and not at the foot of the sidebar, where
                // the Mac puts it. The sidebar is hidden whenever the
                // reader toggles it away or the iPad is in Slide Over, and
                // a banner in a hidden column tells nobody the wire has
                // dropped; the detail is on screen in every state.
                .safeAreaInset(edge: .top) {
                    ConnectionBanner()
                }
        }
    }

    /// The right-hand column: the open thread, or the family's name over
    /// an invitation to pick one.
    @ViewBuilder
    private var detailColumn: some View {
            // The path's own open chat, NOT `chats.first(where:)`: a
            // push-routed chat
            // may not be cached yet after a reinstall, and routing to one
            // must still open the thread — ConversationView and the resync
            // handle a not-yet-known id between them. That is the phone's
            // rule (see `consumePendingRoute`), and the split shape must
            // not quietly tighten it into the Mac's. A chat that VANISHES
            // is a different case and is already handled, by the
            // `chats.map(\.chatID)` hook clearing the path.
            if let chatID = ChatSelection.openChat(in: path) {
                ConversationView(chatID: chatID)
                    // The reason the stack's destination gives, for the
                    // same reason: a route that replaces the selection in
                    // place must rebuild the thread, not reuse it.
                    .id(chatID)
            } else {
                // Deliberately NOT auto-selecting the first chat the way
                // MacChatView does. Its own comment allows it because a Mac
                // window can be behind every other window, so selecting a
                // chat is not showing it; an iPad detail column is in front
                // of the reader with `scenePhase == .active`, so a launch
                // that auto-selected the family chat would satisfy all
                // three of ChatPresence's conditions and mark it read
                // before anybody had asked to see it.
                // The family's name, whole, where the iPad has the room
                // for it — this column is most of the screen and used to
                // hold nothing but a generic glyph. Not a stock
                // ContentUnavailableView: its title is one line.
                VStack(spacing: 14) {
                    InitialsAvatar(title: familyName, isFamily: true, size: 88)
                        .accessibilityHidden(true)
                    Text(familyName)
                        .font(.title2.weight(.semibold))
                        .multilineTextAlignment(.center)
                        .lineLimit(3)
                        .minimumScaleFactor(0.7)
                    Text("\(activeMemberCount) members", comment: "Under the family name on the iPad's empty detail column; %lld is the number of members still in the family.")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                    Text("Pick a chat from the sidebar.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .padding(.top, 6)
                }
                .padding(40)
                .frame(maxWidth: 420)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color.appBackground)
                .accessibilityElement(children: .combine)
            }
    }

    // MARK: - Shared pieces

    /// The list itself. One row definition, two selection mechanisms:
    /// the stack pushes through a NavigationLink, the sidebar tags rows
    /// for the List's own selection (a link in a sidebar would push the
    /// thread ON TOP of the sidebar instead of filling the detail column).
    @ViewBuilder
    private func chatList(selectable: Bool) -> some View {
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
        } else if selectable {
            List(selection: selection) {
                ForEach(sortedChats) { chat in
                    chatRow(chat, isNarrow: true)
                        .tag(chat.chatID)
                }
            }
            // The iPad sidebar look — inset rows and a real selection
            // highlight, so the open chat is visibly the open one. The
            // phone keeps `.plain`, which is what it has today.
            .listStyle(.sidebar)
        } else {
            List(sortedChats) { chat in
                NavigationLink(value: chat.chatID) {
                    chatRow(chat)
                }
            }
            .listStyle(.plain)
        }
    }

    private func chatRow(_ chat: ChatEntity, isNarrow: Bool = false) -> some View {
        ChatRowView(
            chat: chat,
            peerAvatarVersion: chat.peerUserID
                .flatMap { avatarVersions[$0] } ?? 0,
            isNarrow: isNarrow)
    }

    @ToolbarContentBuilder
    private var chatListToolbar: some ToolbarContent {
        ToolbarItem(placement: .topBarTrailing) {
            Button {
                showsNewChat = true
            } label: {
                Label("New Chat", systemImage: "square.and.pencil")
            }
            // The Mac's shortcuts, for an iPad with a keyboard attached;
            // harmless on a phone, which has no way to press them.
            .keyboardShortcut("n", modifiers: .command)
        }
        // iOS 26 groups adjacent trailing items into ONE glass pill, which
        // read as "compose and its option". New Chat and Board are two
        // unrelated places; a fixed spacer gives each its own pill.
        if #available(iOS 26.0, *) {
            ToolbarSpacer(.fixed, placement: .topBarTrailing)
        }
        ToolbarItem(placement: .topBarLeading) {
            Button {
                showsSettings = true
            } label: {
                Label("Settings", systemImage: "gearshape")
            }
            .keyboardShortcut(",", modifiers: .command)
        }
        ToolbarItem(placement: .topBarTrailing) {
            // What is new to READ on the board since this device
            // last showed it: notes pinned, and notes whose text
            // changed. A drag, a resize and a recolour are not
            // news and never count — BoardBadge holds the rule,
            // shared with the Mac so the two cannot disagree.
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

    /// Everything this screen can put on top of itself, in one place so
    /// the two shapes present exactly the same set from exactly the same
    /// host — the list on the phone, the sidebar on the iPad. Keeping
    /// them attached to the LIST (rather than to the split view) is what
    /// makes `dismissSheets` and the push-route handlers below true for
    /// both shapes without knowing which one they are running under.
    private func presentations(on content: some View) -> some View {
        content
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
            .sheet(isPresented: $showsReports) {
                ReportsSheet()
            }
            // The board is a CANVAS of sticky notes, and on an iPad a
            // .sheet is a centred form sheet about a third of the screen —
            // the notes sat in a 540pt box with a 13-inch display around
            // it. The Mac gives the board a window of its own; the iPad
            // gives it the screen. The phone keeps its sheet, which is
            // full-height there anyway. The idiom never changes while the
            // app runs, so the other presenter is simply never armed.
            .sheet(isPresented: usesSplitView ? .constant(false) : $showsBoard) {
                BoardView()
            }
            .fullScreenCover(isPresented: usesSplitView ? $showsBoard : .constant(false)) {
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

    /// The way into the board; the badge beside it is `newNoteCount`.
    private var boardButton: some View {
        Button {
            showsBoard = true
        } label: {
            Label("Board", systemImage: "square.grid.2x2")
        }
        .keyboardShortcut("b", modifiers: [.command, .shift])
    }

    private var newNoteCount: Int {
        BoardBadge.unreadCount(notes: notes, marks: AppSettings.boardMarks)
    }

    /// The board is on screen, so everything on it has been shown.
    private func markBoardSeen() {
        AppSettings.boardMarks = BoardBadge.marksAfterShowing(
            notes: notes, marks: AppSettings.boardMarks)
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
            // NOT folded into the gate above: a blocked member is still
            // active — still on the roster, still nameable — and the two
            // questions have different right answers everywhere else.
            isBlocked: { coordinator.blockedUserIDs.contains($0) },
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
        case .blocked(let userID):
            // The OS goes on offering a handle the app will not act on —
            // a Recents row, a Favorites entry, a contact card's call
            // button all outlive the block. Saying so is safe: this
            // refusal reaches nobody but the person who set the block.
            let name = members.first { $0.userID == userID }?.resolvedDisplayName
                ?? String(localized: "this person", comment: "Stands in for a blocked member whose name is not known.")
            callRequestError = String(
                localized: "You've blocked \(name). Unblock them from the family roster to call them.",
                comment: "Alert shown when the Phone app or Siri asked Family to call a blocked member; %@ is their display name.")
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
        showsReports = false
        showsBoard = false
        showsShareTarget = false
    }

    /// A route that opens a SHEET over the list clears the pushed thread
    /// on the phone, so the reader comes back to the list the sheet was
    /// opened over. On the iPad the sheet floats over the split view and
    /// the thread beside it is not in the way — clearing the path there
    /// closed the conversation somebody was reading just to show a board
    /// beside where it had been.
    private func popToListForSheet() {
        guard !usesSplitView else { return }
        path = []
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
            showsReports = false
            // The board too: on the iPad it is a full-screen cover, and a
            // thread routed in underneath it would be marked read unseen.
            showsBoard = false
            path = [chatID]
        case .board:
            showsNewChat = false
            showsSettings = false
            showsJoinRequests = false
            showsReports = false
            popToListForSheet()
            showsBoard = true
        case .joinRequests:
            // Owner-only screen; a member who somehow gets this push
            // falls back to the list itself.
            guard session.isOwner else { return }
            showsNewChat = false
            showsSettings = false
            showsReports = false
            popToListForSheet()
            showsJoinRequests = true
        case .reports:
            // Owner-only, like the requests above: a member who somehow
            // gets this push stays on the list rather than being sent to a
            // screen the server would refuse them.
            guard session.isOwner else { return }
            showsNewChat = false
            showsSettings = false
            showsJoinRequests = false
            popToListForSheet()
            showsReports = true
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
                            .keyboardShortcut(.cancelAction)
                    }
                }
        }
    }
}

/// The owner's report inbox as a sheet: its own stack, an inline title
/// (a large one inside a 540pt iPad form sheet took a third of it), and a
/// Done — a sheet with no button out is a trap for anyone on a trackpad.
private struct ReportsSheet: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ReportInboxView()
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Done") { dismiss() }
                            .keyboardShortcut(.cancelAction)
                    }
                }
        }
    }
}

/// One chat row: initials avatar, title, preview, relative time, unread
/// badge. Stock components + semantic colors throughout.
struct ChatRowView: View {
    @Environment(ChatSyncCoordinator.self) private var coordinator
    let chat: ChatEntity

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
    /// Profile-picture version of the direct chat's peer; 0 for the
    /// family chat and for anyone without a picture.
    var peerAvatarVersion: Int64 = 0
    /// The iPad sidebar's arrangement: two lines rather than three
    /// side-by-side columns.
    ///
    /// The phone's row spreads title+preview, the time and the unread
    /// badge across the full width of a screen. A 280pt sidebar cannot
    /// hold that — measured at 248pt of usable row: the title truncated
    /// to "The Sm…", the preview to "Message number…", and the relative
    /// date wrapped onto two lines, all in one 90pt row. This is
    /// MacChatRow's arrangement instead (title and time on the first
    /// line, preview and unread on the second), which is what the Mac
    /// sidebar has always used at this width — one shape for the two
    /// big-screen clients, which is the point of issue #43.
    var isNarrow = false

    /// The family chat is titled with the family's name, which is the
    /// one title in this list that is not a person's — "The
    /// Harper-Whittington Family", "Семья Ивановых-Петровых" — and the one
    /// the navigation bar can no longer carry (see `navigationTitleText`).
    /// It gets a second line on the phone's wide row and a third in the
    /// narrow sidebar row, whose title shares its measure with the time
    /// and, on an iPad mini in portrait, has ~150pt to wrap in ("The
    /// Harper-" / "Whittington" / "Family"). A person's name keeps one.
    private var titleLines: Int {
        guard chat.kind == "family" else { return 1 }
        return isNarrow ? 3 : 2
    }

    var body: some View {
        if isNarrow {
            narrowBody
        } else {
            wideBody
        }
    }

    private var wideBody: some View {
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
                    .lineLimit(titleLines)
                // `??` makes this a String, and Text(String) is verbatim —
                // the fallback has to be localized by hand.
                // A preview from a blocked sender draws the placeholder
                // and no sender name — protocol.md settles this in the
                // `GET /chats` row, and it is the app's most-visited
                // screen, so leaving it unmasked defeats the block outright.
                // NOT revealable from the list: the peek belongs to the
                // thread, and no gesture is wanted here.
                Text(previewText(for: chat))
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

    private var narrowBody: some View {
        HStack(spacing: 10) {
            InitialsAvatar(
                title: chat.title,
                isFamily: chat.kind == "family",
                userID: chat.peerUserID,
                avatarVersion: peerAvatarVersion,
                size: 34)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(chat.title)
                        .font(.body.weight(chat.unreadCount > 0 ? .semibold : .regular))
                        .lineLimit(titleLines)
                    Spacer(minLength: 0)
                    if let date = chat.lastMessageDate {
                        // NUMERIC and narrow, not the phone's named style:
                        // "5m" fits beside a name in a sidebar where
                        // "5 minutes ago" wraps to two lines and shoves
                        // the title into an ellipsis. The Mac's choice,
                        // for the Mac's reason.
                        Text(date, format: .relative(presentation: .numeric, unitsStyle: .narrow))
                            .font(.caption)
                            .foregroundStyle(.tertiary)
                            .lineLimit(1)
                    }
                }
                HStack(spacing: 6) {
                    // The blocked-sender placeholder, for the reason the
                    // wide row gives.
                    // Subheadline, not the Mac's caption: this is a
                    // 17pt-body platform, and a 12pt preview beside a
                    // 17pt title read as a footnote to it.
                    Text(previewText(for: chat))
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    Spacer(minLength: 0)
                    if chat.unreadCount > 0 {
                        Text("\(chat.unreadCount)")
                            .font(.caption2.weight(.semibold).monospacedDigit())
                            .foregroundStyle(.white)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 1)
                            .background(.tint, in: Capsule())
                            .accessibilityLabel(Text("\(chat.unreadCount) unread"))
                    }
                }
            }
        }
        .padding(.vertical, 3)
        .accessibilityElement(children: .combine)
    }
}


#endif
