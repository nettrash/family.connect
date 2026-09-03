//
//  ConversationView.swift
//  FamilyConnect
//
//  One chat's message thread. Structure:
//
//    ScrollView + LazyVStack + defaultScrollAnchor(.bottom): opens at the
//    newest message like every messenger; LazyVStack keeps long histories
//    cheap. ScrollViewReader drives the two programmatic scrolls we need:
//    "stay in place after older messages are prepended" and "follow the
//    bottom when something new arrives".
//
//    History pagination: an invisible sentinel row above the oldest
//    message; when it appears, one before_id page is fetched and the view
//    scrolls back to the previously-topmost bubble (anchor: .top), so the
//    content the user was reading doesn't jump. A short page flips the
//    chat's hasFullHistory and the sentinel stops rendering — that's the
//    `atStart` state.
//
//    The input bar lives in safeAreaInset(edge: .bottom) on .bar material
//    — it rides the keyboard for free and the scroll view's own insets
//    stay correct, which hand-rolled bottom bars always get wrong. The
//    bar's height must stay CONSTANT: anything that grows it (the typing
//    line used to) grows the scroll view's bottom inset, and inset
//    changes are not content changes, so defaultScrollAnchor(.bottom)
//    does not re-anchor — the newest messages slide under the bar. The
//    typing indicator therefore renders as the nav-bar subtitle (fixed
//    height, and where the Android app already shows it).
//
//  The view publishes coordinator.updatePresence — which chat is open,
//  whether the bottom sentinel says the newest message is actually on
//  screen, and whether the scene is active. That triple is the ONLY thing
//  that marks a chat read (ChatPresence); the view never decides for
//  itself, and appearing is not one of the three.
//
//  Long-pressing a bubble floats a Tapback-style menu directly over it:
//  the quick-reaction capsule (+ "+" into the full EmojiPickerView sheet)
//  for messages the server knows (a reaction needs a server message id),
//  and Retry/Delete for failed ones (which never have a server id — the
//  two gates are disjoint). Pending-but-unfailed bubbles get no menu.
//  Mechanically: every bubble publishes its frame through BubbleAnchorKey
//  and the overlay below resolves the pressed one's rect, dims the rest,
//  and springs the menu out of the bubble — above it normally, below it
//  when the bubble hugs the top edge, always clamped on-screen. The menu
//  reads the LIVE entity (not a captured snapshot), so a state change
//  while it is up — the ack landing, say — shows current truth.
//

// iOS only — the Mac has its own views (MacViews/).
#if os(iOS)

import PhotosUI
import QuickLook
import SwiftData
import SwiftUI
import UniformTypeIdentifiers

@MainActor @Observable
final class ConversationModel {
    /// Pagination state machine: idle (sentinel armed) → loadingOlder
    /// (one page in flight, sentinel ignored) → back to idle, or atStart
    /// once the top of history is reached (sentinel gone for good).
    enum State: Equatable {
        case idle
        case loadingOlder
        case atStart
    }

    var state: State = .idle
    var draft = ""
}

struct ConversationView: View {
    let chatID: Int64

    /// Identity of the bubble the full emoji picker is up for. A tiny
    /// Identifiable wrapper because `.sheet(item:)` wants one and the
    /// stable identity of a bubble is its localID string.
    private struct ReactionTarget: Identifiable {
        let localID: String
        var id: String { localID }
    }

    /// One message's text on its way to the share sheet — a wrapper
    /// because `.sheet(item:)` wants something Identifiable.
    private struct ShareText: Identifiable {
        let text: String
        var id: String { text }
    }

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(LinkPreviewLoader.self) private var previewLoader
    /// For `callsEnabled` — whether this server rings anybody at all.
    @Environment(AppSession.self) private var session
    @Environment(CallManager.self) private var calls
    /// Owns a media send from the moment it starts; see MediaOutbox.
    @Environment(MediaOutbox.self) private var outbox
    /// The "app is frontmost" half of ChatPresence. Read here rather than
    /// taken from RootView because this is where the other two facts are,
    /// and all three have to be published together.
    @Environment(\.scenePhase) private var scenePhase
    @Query private var messages: [MessageEntity]
    @Query private var chats: [ChatEntity]
    @Query private var members: [MemberEntity]
    @State private var model = ConversationModel()
    /// False until the opening layout settled where this open was meant to
    /// land (the convergence loop in `.task`, or the anchored scroll that
    /// replaces it); a history-load scroll-restore before that would yank a
    /// freshly opened chat away from where it was going.
    @State private var hasSettled = false
    /// Where this open landed, decided ONCE by UnreadAnchor before the
    /// first scroll and never recomputed — nil only until it is decided.
    ///
    /// Never derived per pass, and that is the whole design: both of its
    /// inputs move while the reader reads. The count is zeroed by the read
    /// path the instant they reach the bottom and the marker advances
    /// behind it, so a divider computed on every body evaluation would
    /// disappear at exactly the moment the person it was drawn for arrived
    /// at it.
    @State private var openAnchor: OpenAnchor?

    /// The two ways a chat can open.
    private enum OpenAnchor: Equatable {
        /// The ordinary open: at the newest message.
        case newest
        /// At the oldest unread message, under a divider carrying the
        /// count as the store held it at open.
        case unread(serverID: Int64, count: Int)
    }

    /// Did this open deliberately land the reader in history? Read by
    /// every hook that would otherwise pin the thread to the bottom.
    ///
    /// True from the moment the anchor is decided, which is BEFORE any
    /// scrolling happens — so the opening window is covered too, which is
    /// where the fight actually is.
    private var opensInHistory: Bool {
        if case .unread = openAnchor { return true }
        return false
    }

    /// Is an anchored open still LANDING? True only between the anchor
    /// being decided and the opening scroll finishing.
    ///
    /// The window where nothing else may move the thread: the opening
    /// scroll has already run (so it will not run again), the reader has
    /// not touched anything, and any content change in between — a
    /// prepended history page, above all — slides the viewport away from
    /// the divider with nothing left to put it back.
    private var openingAnchorPending: Bool { opensInHistory && !hasSettled }
    /// True while the bottom sentinel (the row after the newest message)
    /// is materialized — the ground-truth "the user is at the bottom"
    /// signal that the convergence loop and the re-pin hooks key on.
    @State private var isPinnedToBottom = false
    /// The thread's own width, watched because on an iPad it CHANGES —
    /// the split view's sidebar toggles and the device rotates. See the
    /// geometry hook on the scroll view (PIN HOOK 4 of 4).
    @State private var threadWidth: CGFloat = 0
    /// Focus of the input field. Focusing raises the keyboard; both that
    /// and a growing multi-line draft change the bottom INSET, which
    /// defaultScrollAnchor does not re-anchor for — the snaps below do.
    @FocusState private var inputFocused: Bool

    /// Side of the attach and send buttons.
    ///
    /// It matches the one-line height of the text capsule (body line height
    /// plus its 7pt of padding each side), which is what makes the two icons
    /// optically level with the last line of text rather than sitting below
    /// it against the capsule's bottom edge. Scaled, because the capsule
    /// grows with Dynamic Type and a hard-coded 36 would drift apart from it.
    @ScaledMetric(relativeTo: .body) private var composerControl: CGFloat = 36

    /// Live height of the input bar; changes exactly when the draft wraps
    /// to more (or fewer) lines.
    @State private var inputBarHeight: CGFloat = 0
    /// The message being answered, while the composer is primed. Held as
    /// the wire snapshot so send() can hand it straight to the coordinator
    /// and the pending bubble can draw its quote before the server answers.
    @State private var replyDraft: ReplyToDTO?
    /// What the message being replied to carries, looked up once per
    /// reply target rather than per keystroke — the family composer's
    /// picture strip reads it (`mentionPictureNotice`), and a scan of the
    /// chat's rows inside a body property would run on every character.
    @State private var quotedAttachments: [AttachmentDTO] = []
    /// The message being rewritten, while the composer is in edit mode.
    /// Mutually exclusive with replyDraft: you are either answering a
    /// message or rewriting one.
    @State private var editTarget: (messageID: Int64, original: String)?
    /// Briefly tinted after a jump, so the eye lands on the right bubble.
    @State private var highlightedMessageID: String?
    /// Rows peeked at, keyed by `message.localID`.
    ///
    /// A REVEAL IS A PEEK: per row, per device, never on the wire and never
    /// stored. It lives here rather than in the bubble for two reasons —
    /// `@State` inside a bubble is destroyed when the ~60-row window slides
    /// the row out and back, which the protocol forbids ("a revealed row
    /// stays revealed … including when newer messages arrive after it"),
    /// and the jump gate needs to read it. `.id(chatID)` on the
    /// navigationDestination tears this whole view down on a chat switch,
    /// which is exactly "once on demand".
    @State private var revealedMessageIDs: Set<String> = []
    /// Quote levels peeked at, keyed by HOST + LEVEL — never by the quoted
    /// message's id. The same blocked message quoted by three bubbles is
    /// three independent peeks, and revealing a quote must not reveal the
    /// row it points at.
    @State private var revealedQuoteIDs: Set<String> = []
    /// True while a reply that was STARTED FROM HISTORY is being composed.
    ///
    /// Both pin rules below fire on the two things beginReply does — it
    /// focuses the field (opening the keyboard) and adds the banner
    /// (growing the bar) — so without this, tapping Reply on an old
    /// message throws the reader to the newest one, away from the very
    /// message they are answering. Suppressed only when they were not at
    /// the bottom already; ends with the draft.
    @State private var replyStartedFromHistory = false
    /// The picked photos and videos, while they are being prepared. A
    /// LIST since the picker went multi-select (up to the message cap);
    /// they stage in the order picked, which is the order sent.
    @State private var pickedMedia: [PhotosPickerItem] = []
    @State private var showPhotoPicker = false
    @State private var showCamera = false
    @State private var recorder = AudioRecorder()
    @State private var showFilePicker = false
    @State private var mediaState: MediaSendState = .idle
    /// A one-line answer to something the person just tried that this
    /// composer could not do — a ⌘V while an upload is running, a pasted
    /// wall of text over the body limit, an empty clipboard.
    ///
    /// Deliberately NOT `mediaState = .failed`. That state IS the upload:
    /// overwriting it would both lose "Sending…" and un-gate
    /// `composerIsBusy`, which reads it — so the guard that produced the
    /// message would be gone the moment the message appeared.
    @State private var composerNotice: String?
    /// One fix, on demand — never a running location service.
    @State private var locationProvider = LocationProvider()
    /// True while the poll form is up. A sheet rather than a banner over
    /// the field: a question plus up to ten options is a form, not a line.
    @State private var showPollComposer = false
    /// The album being viewed full-screen: the message's media and the
    /// one that was tapped. A lone photo is an album of one.
    @State private var viewingAlbum: AttachmentAlbum?
    /// A downloaded file on its way to Quick Look.
    @State private var previewedFile: URL?

    /// What the composer is doing with a picked photo or video. Sending is
    /// NOT optimistic: the message appears once the server has the bytes,
    /// because a bubble pointing at an upload that failed is worse than a
    /// composer that is visibly busy.
    enum MediaSendState: Equatable {
        case idle
        case preparing
        /// A send in flight. The associated text is the multi-item
        /// progress ("Uploading 2 of 5…"); nil keeps the plain
        /// "Sending…". Progress rides INSIDE this case, never through
        /// `.working`, because `.working` is the download/share state
        /// and leaves the composer usable — only a send closes the
        /// attach and paste doors.
        case uploading(String?)
        /// A download/save the user asked for, with its own wording — the
        /// same strip reports it, since it is the one place this screen
        /// already says what it is busy with.
        case working(String)
        case failed(String)

        /// Whether this state closes the attachment doors — the pure
        /// rule behind `composerIsBusy`, split out so it can be tested.
        /// A send blocks (staging more mid-send would allow a second,
        /// concurrent send); a download or a dismissible failure does not.
        var blocksComposer: Bool {
            switch self {
            case .preparing, .uploading: return true
            case .idle, .working, .failed: return false
            }
        }
    }

    /// Files on their way to the share sheet.
    private struct SharePayload: Identifiable {
        let id = UUID()
        let items: [Any]
    }
    /// localID of the bubble the floating reaction picker is up over;
    /// nil = no picker. Set/cleared inside withAnimation so the capsule
    /// springs in and out.
    @State private var reactionPickerID: String?
    /// Which page the message menu is showing. Owned here, not by the
    /// menu, so `MessageContextMenu.size` and the menu itself agree.
    @State private var menuPage: MessageContextMenu.Page = .main
    /// The member a report sheet is open for, if any.
    @State private var reportTarget: ReportTarget?
    /// The bubble the "+" full emoji picker sheet is up for.
    @State private var fullPickerTarget: ReactionTarget?
    /// Text handed to the share sheet, nil while it is closed.
    @State private var shareText: ShareText?
    /// An attachment (and any caption) handed to the share sheet.
    @State private var sharePayload: SharePayload?

    /// Media that is prepared and waiting for the user to press Send —
    /// up to StagedAttachment.maxPerMessage of it, in the order staged,
    /// which is the order the message will carry (docs/protocol.md,
    /// "Photos, videos, audio, files and locations"). A pick APPENDS; at
    /// the cap a brief notice says so. The type and its chips are shared
    /// with the Mac composer — see Views/StagedAttachment.swift.
    @State private var staged: [StagedAttachment] = []

    init(chatID: Int64) {
        self.chatID = chatID
        // Ordering is the dual-id design from MessageEntity: server time
        // first, stable localID as the tiebreaker.
        _messages = Query(
            filter: #Predicate<MessageEntity> { $0.chatID == chatID },
            sort: [SortDescriptor(\MessageEntity.createdAt), SortDescriptor(\MessageEntity.localID)])
        _chats = Query(filter: #Predicate<ChatEntity> { $0.chatID == chatID })
    }

    private var chat: ChatEntity? { chats.first }
    private var isFamilyChat: Bool { chat?.kind == "family" }

    /// Whether there is somebody to ring from here.
    private var canCall: Bool {
        session.callsEnabled && chat?.kind == "direct" && chat?.peerUserID != nil
            // The blocker calling somebody they blocked is refused with
            // `blocked`, so the button does not offer it. Defence in depth
            // behind the direct-chat prune: if the prune landed this is
            // unreachable, and if it did not this stops the one visible
            // refusal.
            && !(chat?.peerUserID.map { coordinator.blockedUserIDs.contains($0) } ?? false)
    }

    private func startCall() {
        guard let chat, let peerUserID = chat.peerUserID else { return }
        calls.startCall(chatID: chat.chatID, peerUserID: peerUserID, video: false)
    }

    private func startVideoCall() {
        guard let chat, let peerUserID = chat.peerUserID else { return }
        calls.startCall(chatID: chat.chatID, peerUserID: peerUserID, video: true)
    }

    /// The assistant's own chat. Its replies come from a reserved account
    /// that is deliberately not in the roster, so the sender is named from
    /// the chat rather than looked up (docs/protocol.md, "The assistant").
    private var isAssistantChat: Bool { chat?.kind == "ai" }
    private var currentUserID: Int64 { AppSettings.currentUserID ?? -1 }

    /// How many of the locally cached messages are RENDERED. The thread
    /// deliberately renders a bounded window (grown by the top sentinel —
    /// locally first, then over the network): an unbounded lazy stack over
    /// a long history full of very tall bubbles estimates its heights so
    /// badly that the opening bottom-anchor can land the viewport in blank
    /// space — and `scrollTo` cannot repair it, because an unmaterialized
    /// row's id is not a scroll target. A ~60-row window keeps estimates
    /// honest, every pin target materialized, and rendering cheap.
    @State private var visibleCount = ConversationView.windowStep

    private static let windowStep = 60
    /// Rows kept ABOVE a jumped-to message. Enough that the target does
    /// not land against the top sentinel, whose appearance triggers a
    /// history page whose scroll restore would undo the jump.
    ///
    /// One number, in one place: the anchored open needs exactly this and
    /// for exactly this reason, and the Mac — which has no quote jump —
    /// needs it too.
    private static let jumpMargin = UnreadAnchor.margin
    /// Most rows rendered at once, by ANY path that widens the window.
    ///
    /// The stack is non-lazy, so this is a real bound on main-thread
    /// layout work rather than a hint — it is the Mac's `maxWindow` and it
    /// is a hang fix (see MacConversationView's header: two captured hang
    /// reports ending in a Force Quit). Roughly five pages.
    ///
    /// It used to bound the quote jump alone, under the name
    /// `maxJumpWindow`, while the three paths that widen the window on
    /// their own — an arrival, a local page back, a fetched page back —
    /// were uncapped on the phone and capped on the Mac. Over a long
    /// session that is the same drift back towards the unbounded stack the
    /// window exists to prevent, and starting the window MUCH wider (which
    /// is what an anchored open does) makes it materially worse.
    private static let maxWindow = 300
    /// The readable column the thread's content is capped to (#21/#44),
    /// named because the floating jump-to-newest button has to line up
    /// with the SAME column — see the overlay for why that is not a
    /// cosmetic choice.
    static let threadMaxWidth: CGFloat = 560

    private var visibleMessages: ArraySlice<MessageEntity> {
        messages.suffix(visibleCount)
    }

    private var sections: [DaySection] {
        // An explicit closure rather than `map(MessageSnapshot.init)`: the
        // bridge initializer reads MainActor-bound @Model properties, and
        // handing it over UNAPPLIED makes it a nonisolated function value —
        // which the compiler has been warning about here for a while. The
        // closure is inferred with this view's isolation, so the call is
        // the same call, made where it was always being made.
        MessagePresentation.daySections(
            visibleMessages.map { MessageSnapshot($0) },
            firstUnreadID: unreadDividerServerID)
    }

    /// The server id the "N new messages" rule is drawn above, and the
    /// count on it — both nil/absent unless this open resolved to an
    /// unread anchor.
    private var unreadDividerServerID: Int64? {
        if case .unread(let serverID, _) = openAnchor { return serverID }
        return nil
    }

    private var unreadDividerCount: Int? {
        if case .unread(_, let count) = openAnchor { return count }
        return nil
    }

    /// Is the floating jump-to-newest button up? The rule is shared with
    /// the Mac (and, in intent, with Android) — see ThreadFollow.
    private var showsJumpToNewest: Bool {
        ThreadFollow.showsJumpToNewest(isAtNewest: isPinnedToBottom, hasSettled: hasSettled)
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                // A plain (non-lazy) VStack, on purpose: the window bounds
                // it to ~a page of rows, and real heights are what make
                // opening at the bottom EXACT. A lazy stack here estimates
                // unmaterialized row heights, and with tall pasted-text
                // bubbles the estimates are wrong by screens — the bottom
                // anchor then lands the viewport in blank space, and
                // scrollTo cannot repair it because an unmaterialized
                // row's id is not a scroll target.
                VStack(spacing: 4) {
                    if !messages.isEmpty
                        && (visibleCount < messages.count
                            || (model.state != .atStart && chat?.hasFullHistory != true)) {
                        // Top sentinel: scrolling it into view means the
                        // reader reached the oldest rendered message.
                        // Geometry-triggered — in a non-lazy stack every
                        // row "appears" at creation, so onAppear would
                        // fire a page load on every open.
                        HStack {
                            if model.state == .loadingOlder {
                                ProgressView().controlSize(.small)
                            } else {
                                Color.clear.frame(height: 1)
                            }
                        }
                        .frame(maxWidth: .infinity)
                        .frame(height: 24)
                        .onGeometryChange(for: Bool.self) { geometry in
                            guard let viewport = geometry.bounds(of: .scrollView) else { return false }
                            let y = geometry.frame(in: .scrollView).maxY
                            return y >= -200 && y <= viewport.height + 200
                        } action: { visible in
                            if visible { loadOlder(proxy: proxy) }
                        }
                    }

                    bubbleRows(proxy: proxy)

                    // Bottom sentinel: drives `isPinnedToBottom` and is the
                    // TARGET of every pin-to-bottom scroll (aiming at the
                    // last bubble would park this row just off-screen and
                    // read as unpinned). Visibility comes from GEOMETRY in
                    // the scroll view's space — onAppear would lie: a short
                    // thread materializes ALL rows, sentinel included, no
                    // matter where the viewport actually is.
                    Color.clear
                        .frame(height: 1)
                        .id(Self.bottomSentinelID)
                        .onGeometryChange(for: Bool.self) { geometry in
                            guard let viewport = geometry.bounds(of: .scrollView) else { return false }
                            let y = geometry.frame(in: .scrollView).minY
                            return y >= -32 && y <= viewport.height + 32
                        } action: { visible in
                            isPinnedToBottom = visible
                            // The one fact only this geometry knows. It is
                            // also the fact that decides whether anything is
                            // read, so it is published the moment it moves.
                            publishPresence()
                        }
                }
                .padding(.horizontal, 12)
                .padding(.top, 8)
                // A readable column on a big screen, the second half of
                // #21. The CONTAINER is capped, never the bubble: the
                // greedy-frame ordering question stays out of
                // MessageBubbleView (which has shipped a full-width-slab
                // regression twice) and ReplyBalloonLayout — shared with
                // macOS, carrying no #if os(iOS) — is untouched. 560 sits
                // above every iPhone PORTRAIT width, so the bubble subtree
                // there sees exactly the proposal it ships with today.
                .frame(maxWidth: Self.threadMaxWidth)
                .frame(maxWidth: .infinity)
            }
            .defaultScrollAnchor(.bottom)
            .scrollDismissesKeyboard(.interactively)
            // The Mac's empty state, on the phone: a thread with nothing
            // in it was a blank screen here. An OVERLAY only — the scroll
            // geometry and the opening routine are untouched, and it
            // takes no hits so it cannot swallow a gesture meant for the
            // thread. While the first page is still on its way
            // (`loadInitial` parks the model in `.loadingOlder`) a
            // spinner, not a claim there is nothing to see.
            .overlay {
                if messages.isEmpty {
                    Group {
                        if model.state == .loadingOlder {
                            ProgressView()
                        } else {
                            ContentUnavailableView(
                                "No messages yet",
                                systemImage: "bubble.left.and.bubble.right",
                                description: Text("Say something to get started."))
                        }
                    }
                    .allowsHitTesting(false)
                }
            }
            // The way back down, for a reader the anchored open has
            // deliberately left in history — and for anyone who scrolled
            // there themselves. Android's button, Android's icon, and the
            // trigger rule ported to what an Apple thread can actually
            // observe (ThreadFollow.showsJumpToNewest).
            //
            // The animation is scoped to this container rather than hung on
            // the scroll view: `value:` on the ScrollView itself would put
            // every content change that happens to coincide with it inside
            // an animation transaction, which is how a thread starts
            // sliding for reasons nobody asked for.
            .overlay(alignment: .bottom) {
                // Aligned to the trailing edge of the THREAD COLUMN, not
                // of the viewport.
                //
                // The thread's content is capped to a readable column
                // (#21/#44) and centred, so on an iPad's detail column the
                // viewport's trailing edge is 100pt-odd of empty margin
                // away from the last bubble — a control pinned there is
                // nowhere near the thing it acts on, which is the same
                // "phone app stretched across a display" complaint issue
                // #43 is about. On the phone NOTHING moves: every iPhone
                // is narrower than the cap, so the column IS the viewport
                // (measured, iPhone 17: the button's frame is
                // 353.75,731.75 before and after, to the quarter-point).
                //
                // The button's own hit shape is a separate matter and a
                // real bug; it is fixed where it lives, in
                // JumpToNewestButton.
                HStack(spacing: 0) {
                    Spacer(minLength: 0)
                    ZStack {
                        if showsJumpToNewest {
                            JumpToNewestButton { pinToBottom(proxy, animated: true) }
                                .transition(.scale.combined(with: .opacity))
                        }
                    }
                    .animation(.spring(duration: 0.25), value: showsJumpToNewest)
                }
                .frame(maxWidth: Self.threadMaxWidth)
            }
            .task {
                // Un-park a draft an earlier identity of this chat's view
                // stashed on its way out — before the opening routine, so
                // the composer is whole by the time the thread lands.
                // Recovery FIRST: a failed set carries files and a caption,
                // the parked draft carries only text, and `ComposerDrafts`
                // hands its draft over exactly once. Taking the text first
                // would spend that one chance and leave the caption behind.
                recoverFailedSend()
                if model.draft.isEmpty, let parked = ComposerDrafts.take(for: chatID) {
                    model.draft = parked
                }
                // Files shared into the app and addressed to THIS chat
                // land in the composer, staged — the ordinary arrival,
                // since the picker routes here and this view is then
                // built fresh (`.id(chatID)`).
                consumeShareImport()
                await openThread(proxy: proxy)
            }
            .onChange(of: messages.count) { oldCount, newCount in
                // A reader away from the bottom keeps their place: arriving
                // messages widen the render window instead of sliding the
                // suffix out from under them — CAPPED, because this stack
                // is not lazy and an uncapped count creeps up over a long
                // session until the layout cost the window exists to bound
                // comes back. Past the cap the suffix SLIDES, which is also
                // what eventually carries an old divider off the top of a
                // very long session; see `openThread`.
                visibleCount = ThreadFollow.windowAfterArrival(
                    current: visibleCount,
                    cached: newCount,
                    arrived: newCount - oldCount,
                    isAtNewest: isPinnedToBottom,
                    cap: Self.maxWindow)
                // Re-ask what this reader can see: a message arriving for
                // somebody already at the bottom scrolls the thread WITHOUT
                // the sentinel ever leaving the viewport, so the geometry
                // hook above may not fire, and this is what reads the
                // message they just watched land.
                publishPresence()
                // Follow the bottom for new messages — for my own sends
                // from anywhere, otherwise only while actually pinned (a
                // reader deep in history must not be yanked down) — and
                // never while a history page is being prepended above.
                //
                // PIN HOOK 1 of 4. The `!hasSettled` clause is the one that
                // had to be gated: it exists so a message landing during
                // the opening window keeps a normal open at the bottom
                // (where `isPinnedToBottom` is still false because no
                // geometry has been reported yet), and it would just as
                // happily undo an anchored open — which spends its whole
                // opening window deliberately NOT at the bottom. An own
                // send still wins from anywhere, as it must.
                guard model.state == .idle, let last = messages.last else { return }
                guard last.senderID == currentUserID
                    || isPinnedToBottom
                    || (!hasSettled && !opensInHistory) else { return }
                pinToBottom(proxy, animated: true)
            }
            .onChange(of: previewLoader.generation) {
                // A link-preview card just landed: the bubble hosting it
                // grew, and a bubble above the viewport growing slides
                // the visible region into older content — same problem
                // as reaction catch-up below, same repair.
                guard isPinnedToBottom else { return }
                pinToBottom(proxy, animated: false)
            }
            .onChange(of: chats.first?.maxReactionSeq) {
                // Reaction catch-up landed: chip rows above the viewport
                // grew, and the preserved pixel offset would slide the
                // visible region into older content. While pinned, re-pin
                // over the new heights.
                guard isPinnedToBottom else { return }
                pinToBottom(proxy, animated: false)
            }
            .onChange(of: chats.first?.maxPollSeq) {
                // Poll catch-up landed, and it grows bubbles for the same
                // reason: an option gaining its FIRST vote adds a row of
                // faces under it. Same repair as the reaction cursor above.
                guard isPinnedToBottom else { return }
                pinToBottom(proxy, animated: false)
            }
            .onChange(of: coordinator.blockedUserIDs) {
                // A live `member_blocked` frame COLLAPSES every row of that
                // sender in the window in one layout pass — the same
                // problem as a link card landing, in reverse and at scale.
                // The preserved pixel offset then throws a reader who was
                // at the bottom into the middle of nowhere.
                guard isPinnedToBottom else { return }
                pinToBottom(proxy, animated: false)
            }
            .onChange(of: revealedMessageIDs) {
                // And a reveal is the same thing the other way: a thin row
                // becomes a full bubble with an album, a link card and two
                // chip rows, which dwarfs every growth the three hooks
                // above exist for.
                guard isPinnedToBottom else { return }
                pinToBottom(proxy, animated: false)
            }
            .onChange(of: inputFocused) {
                // The keyboard rising is an inset change, and the system's
                // own avoidance is best-effort — deterministically pin the
                // newest message above it (twice: as the animation starts
                // and after it lands; standard messenger behavior).
                //
                // PIN HOOK 3 of 4, and deliberately NOT gated on the
                // opening anchor. Nothing focuses this field on open, so it
                // cannot fire during the opening window; when it does fire
                // a person has just tapped into the composer, and a person
                // who taps the composer is done reading history. Answering
                // from an old message keeps its own suppression
                // (`replyStartedFromHistory`), which is the case where that
                // is not true.
                guard inputFocused, !replyStartedFromHistory else { return }
                Task {
                    try? await Task.sleep(nanoseconds: 100_000_000)
                    pinToBottom(proxy, animated: true)
                    try? await Task.sleep(nanoseconds: 350_000_000)
                    pinToBottom(proxy, animated: false)
                }
            }
            .onChange(of: inputBarHeight) {
                // The draft wrapped to another line: the bar grew into the
                // thread — keep the newest message above it while writing.
                guard inputFocused, !replyStartedFromHistory else { return }
                pinToBottom(proxy, animated: false)
            }
            // The thread can now be RESIZED HORIZONTALLY under a reader:
            // on an iPad it lives in a split view's detail column (issue
            // #43), so hiding or showing the sidebar and rotating the
            // device both change its width. A ScrollView keeps a POINT
            // OFFSET across that, not a message, and every row re-wraps at
            // the new width — so a reader sitting at the newest message
            // comes back somewhere above it.
            //
            // The Mac hit this first and fixed it the same way; its
            // header spells out the reasoning (MacConversationView, "A Mac
            // window DOES resize"). Only the BOTTOM is restorable without
            // tracking per-row geometry, and it is the case that actually
            // bites; away from the bottom the bounded window keeps any
            // drift to genuine re-wrapping of a page of rows.
            //
            // The measure is the width CONTENT can use, not the view's
            // own — and on this platform they are different things. The
            // Mac watches `size.width` because there a sidebar toggle
            // really does resize the detail view. On iPadOS 26 the sidebar
            // floats OVER a detail column that stays the full width of the
            // window (verified: the thread's ScrollView reports
            // {0,0,1032,1376} with the sidebar both shown and hidden), and
            // what moves is the safe area. Subtracting the horizontal
            // insets is what makes this hook see the toggle at all; it
            // also still sees a rotation and a Split View resize, which
            // move the frame itself.
            //
            // PIN HOOK 4 of 4. Inert on the phone by construction: its
            // width changes only on rotation, and the guard fires only
            // for a reader who is ALREADY at the newest message, whose
            // request is "be at the newest message".
            .onGeometryChange(for: CGFloat.self) {
                $0.size.width - $0.safeAreaInsets.leading - $0.safeAreaInsets.trailing
            } action: { width in
                // The first report is the opening layout, not a resize:
                // record it and do nothing, or this would scroll a thread
                // that is still deciding where to open.
                guard threadWidth != 0, width != threadWidth else {
                    threadWidth = width
                    return
                }
                threadWidth = width
                guard isPinnedToBottom else { return }
                pinToBottom(proxy, animated: false)
            }
        }
        .safeAreaInset(edge: .bottom) {
            inputBar
        }
        .overlayPreferenceValue(BubbleAnchorKey.self) { anchors in
            reactionPickerOverlay(anchors: anchors)
        }
        // A selection tick when the floating picker opens (not when it
        // closes — that's just the scrim fading).
        .sensoryFeedback(.selection, trigger: reactionPickerID) { _, newValue in
            newValue != nil
        }
        // Grouped into one modifier: the chain here is long enough that
        // the type checker gives up on it as separate steps.
        .modifier(attachmentSurfaces)
        .sheet(item: $reportTarget) { target in
            ReportSheet(
                target: target,
                onSubmit: { reason in
                    reportTarget = nil
                    Task {
                        await coordinator.report(
                            reportedUserID: target.senderID,
                            // The RAW value: the untranslated wire string,
                            // never the label somebody reads.
                            reason: reason.rawValue,
                            messageID: target.messageID)
                    }
                },
                onCancel: { reportTarget = nil })
        }
        .sheet(item: $shareText) { share in
            ShareSheet(text: share.text)
        }
        .sheet(item: $sharePayload) { payload in
            ShareSheet(items: payload.items)
        }
        .sheet(isPresented: $showPollComposer) {
            PollComposerView(
                onCreate: { question, options in
                    showPollComposer = false
                    sendPoll(question: question, options: options)
                },
                onCancel: { showPollComposer = false })
        }
        .sheet(item: $fullPickerTarget) { target in
            EmojiPickerView { emoji in
                toggleReaction(localID: target.localID, emoji: emoji)
                fullPickerTarget = nil
            }
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
        }
        .navigationTitle(chat?.title ?? "")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            // Title + typing subtitle in the fixed-height nav bar: the
            // indicator can come and go without moving any chat content
            // (see the header comment; Android shows it the same way).
            ToolbarItem(placement: .principal) {
                VStack(spacing: 0) {
                    Text(chat?.title ?? "")
                        .font(.headline)
                    if let typingLine {
                        Text(typingLine)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .transition(.opacity)
                    }
                }
                .animation(.easeInOut(duration: 0.15), value: typingLine)
            }
            // A direct chat's peer can be rung (docs/protocol.md, "Voice
            // calls"); the family chat has no single person to ring, and
            // the button hides behind the server's `calls_enabled`.
            if canCall {
                // The video button sits beside the phone one and hides
                // behind its OWN server switch (`video_calls_enabled`,
                // docs/protocol.md, "Video") — an operator whose relay is
                // sized for voice can say no to video alone.
                if session.videoCallsEnabled {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button {
                            startVideoCall()
                        } label: {
                            Image(systemName: "video.fill")
                        }
                        .accessibilityLabel("Video Call")
                        .disabled(!calls.isIdle)
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        startCall()
                    } label: {
                        Image(systemName: "phone.fill")
                    }
                    .accessibilityLabel("Call")
                    .disabled(!calls.isIdle)
                }
            }
        }
        .onAppear {
            // Claims the chat, and claims NOTHING about having seen it:
            // `hasSettled` is still false, so this publishes "not at the
            // newest message" no matter where the opening layout happens to
            // have landed. Appearing used to be the whole test, which meant
            // a chat was read by being pushed onto the stack.
            publishPresence()
            // The first page is fetched by `openThread` rather than here.
            // It has to be: the anchor is arithmetic over the CACHED rows,
            // and a chat opened with an empty cache would resolve every
            // time to "fewer cached rows than the count" and give up — so
            // the one open that most needs an anchor would never get one.
        }
        .onDisappear {
            coordinator.releasePresence(chatID: chatID)
            // The view's identity dies with the route (`.id(chatID)` in
            // ChatListView) and the draft is @State: park it, or a tapped
            // notification for another chat discards a half-typed message.
            ComposerDrafts.stash(model.draft, for: chatID)
        }
        // A send for this chat failed. Whichever composer is on screen
        // adopts it — including this one immediately after its own send
        // failed, which is how the in-place restore now happens.
        .onChange(of: outbox.failedCount(for: chatID)) {
            recoverFailedSend()
        }
        .onChange(of: hasSettled) {
            // The opening convergence finished: whatever the sentinel says
            // now is real, and if it says the newest message is on screen
            // then the reader has genuinely seen it.
            publishPresence()
        }
        .onChange(of: scenePhase) { previous, _ in
            // Backgrounding revokes the authority to read (the coordinator
            // does that centrally, because onDisappear does NOT fire here);
            // coming back re-establishes it from the same geometry, without
            // the act of returning reading anything by itself.
            publishPresence()
            // Coming back from the BACKGROUND specifically — not from the
            // inactive flicker a Control Centre pull or an alert causes —
            // is the one moment iOS has certainly thrown away a permission
            // alert that was up, and it does not put it back. Tell the
            // provider, so a share parked on that prompt stops waiting for
            // an answer that can no longer arrive. It re-reads the status
            // first, so a permission granted in Settings while we were away
            // still continues the share.
            if previous == .background { locationProvider.promptWasAbandoned() }
        }
        .onChange(of: model.draft) { _, newValue in
            guard !newValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
            coordinator.sendTyping(in: chatID)
        }
        // The share picker chose THIS chat while it was already open —
        // `.id(chatID)` rebuilds nothing then, so `.task` will not run
        // again and this is what stages the files instead.
        .onChange(of: session.shareImportTarget) {
            consumeShareImport()
        }
    }

    /// Stage files shared into the app, if any are addressed to this
    /// chat. `takeShareImport` hands them over exactly once and only for
    /// the chat the picker chose, so a conversation that merely appears
    /// while a share is pending cannot swallow it. Nothing auto-sends —
    /// the files sit staged until Send.
    private func consumeShareImport() {
        guard let urls = session.takeShareImport(for: chatID), !urls.isEmpty else { return }
        mediaState = .preparing
        Task {
            for url in urls {
                do {
                    let prepared = try await MediaPrep.prepare(fileAt: url, limit: MediaPrep.sizeLimit)
                    // `prepare` may hand back the source itself (a video
                    // that already fits); only delete the import — the
                    // whole per-import `fc-shared-<id>` directory, so no
                    // empty husk survives — when a new file was made from
                    // it. (When the source IS the staged file, its
                    // directory lives until the file is consumed.)
                    if prepared.fileURL != url {
                        try? FileManager.default.removeItem(at: url.deletingLastPathComponent())
                    }
                    // The cap (and its notice) lives in `stage`.
                    stage(prepared)
                } catch MediaPrep.PrepError.tooLargeAfterCompression {
                    mediaState = .failed(String(localized: "That file is over the 100 MB limit."))
                    try? FileManager.default.removeItem(at: url.deletingLastPathComponent())
                } catch {
                    mediaState = .failed(String(localized: "Couldn't read that file."))
                    try? FileManager.default.removeItem(at: url.deletingLastPathComponent())
                }
            }
            if mediaState == .preparing { mediaState = .idle }
        }
    }

    /// The message rows — split out so the scroll container above stays
    /// type-checkable at a glance.
    @ViewBuilder
    private func bubbleRows(proxy: ScrollViewProxy) -> some View {
                    ForEach(sections) { section in
                        DayPill(day: section.day)
                            .padding(.vertical, 6)
                        ForEach(Array(section.messages.enumerated()), id: \.element.id) { index, message in
                            // "N new messages", above the oldest message
                            // this reader has not seen. Which row that is
                            // comes from the section (one shared builder
                            // for both Apple threads); the count is the
                            // one captured at open.
                            if section.unreadDividerAbove == message.localID,
                                let unreadDividerCount {
                                UnreadDivider(count: unreadDividerCount)
                                    .id(UnreadDivider.scrollID)
                            }
                            MessageBubbleView(
                                message: message,
                                isMine: message.senderID == currentUserID,
                                isStreaming: coordinator.isAwaitingAssistant(
                                    message, isAssistantChat: isAssistantChat),
                                assistantFailed: message.serverID.map {
                                    coordinator.assistantAnswerFailed(messageID: $0)
                                } ?? false,
                                showsSenderName: MessagePresentation.showsSenderName(
                                    at: index,
                                    in: section.messages,
                                    isFamilyChat: isFamilyChat,
                                    currentUserID: currentUserID,
                                    blockedUserIDs: coordinator.blockedUserIDs),
                                senderName: displayName(for: message.senderID),
                                senderID: message.senderID,
                                senderAvatarVersion: avatarVersion(for: message.senderID),
                                isRead: MessagePresentation.isRead(
                                    message,
                                    othersReadUpTo: chat?.othersReadUpTo ?? 0,
                                    isFamilyChat: isFamilyChat),
                                reactionChips: MessagePresentation.reactionChips(
                                    message.reactions,
                                    currentUserID: currentUserID),
                                reactionDetails: MessagePresentation.reactionDetails(
                                    message.reactions,
                                    names: memberNames,
                                    currentUserID: currentUserID,
                                    // The CHIPS above are deliberately not
                                    // given this: their count must not move.
                                    blockedUserIDs: coordinator.blockedUserIDs),
                                avatarVersions: avatarVersions,
                                memberNames: memberNames,
                                currentUserID: currentUserID,
                                onTapQuote: { jumpToMessage($0, proxy: proxy) },
                                onOpenAttachment: { attachment in
                                    if attachment.isFile {
                                        openFile(attachment)
                                    } else {
                                        // The whole message's media, opened
                                        // at the one that was tapped — the
                                        // viewer pages through the rest.
                                        let media = AttachmentAlbum.media(of: message.attachments)
                                        let index = media.firstIndex { $0.id == attachment.id } ?? 0
                                        viewingAlbum = AttachmentAlbum(items: media, index: index)
                                    }
                                },
                                onVote: { optionID in
                                    Task {
                                        await coordinator.vote(
                                            localID: message.localID, optionID: optionID)
                                    }
                                },
                                onClosePoll: {
                                    Task { await coordinator.closePoll(localID: message.localID) }
                                },
                                memberCount: familyMemberCount,
                                onCallBack: canCall ? { startCall() } : nil,
                                onRetry: { coordinator.retry(localID: message.localID) },
                                onDelete: { coordinator.deleteLocalMessage(localID: message.localID) },
                                onToggleReaction: { emoji in
                                    toggleReaction(localID: message.localID, emoji: emoji)
                                },
                                onLongPress: {
                                    // Nothing to offer on a pending bubble:
                                    // no server id to react to, no failure
                                    // to retry.
                                    guard message.serverID != nil || message.state == .failed else { return }
                                    // A hidden row opens nothing. One
                                    // missing clause here is three leaks
                                    // at once: the capsule floats over a
                                    // placeholder, Copy puts the blocked
                                    // body on the pasteboard, and Share
                                    // hands the blocked attachment to the
                                    // share sheet. Reveal is a TAP.
                                    guard
                                        !MessagePresentation.isHiddenByBlock(
                                            message,
                                            blockedUserIDs: coordinator.blockedUserIDs,
                                            currentUserID: currentUserID)
                                            || revealedMessageIDs.contains(message.localID)
                                    else { return }
                                    withAnimation(.spring(duration: 0.3, bounce: 0.25)) {
                                        reactionPickerID = message.localID
                                    }
                                },
                                publishesAnchor: reactionPickerID == message.localID,
                                blockedUserIDs: coordinator.blockedUserIDs,
                                isHiddenByBlock: MessagePresentation.isHiddenByBlock(
                                    message,
                                    blockedUserIDs: coordinator.blockedUserIDs,
                                    currentUserID: currentUserID),
                                isRevealed: revealedMessageIDs.contains(message.localID),
                                // Never persisted, never sent, never
                                // written to SwiftData: a peek is not a
                                // fact anybody else may read.
                                onReveal: { revealedMessageIDs.insert(message.localID) },
                                isReplyQuoteRevealed: revealedQuoteIDs.contains(
                                    "\(message.localID)#reply"),
                                isParentQuoteRevealed: revealedQuoteIDs.contains(
                                    "\(message.localID)#parent"),
                                onRevealQuote: { level in
                                    revealedQuoteIDs.insert(
                                        level == .reply
                                            ? "\(message.localID)#reply"
                                            : "\(message.localID)#parent")
                                })
                                .id(message.localID)
                                // A jumped-to bubble is briefly tinted, so
                                // the eye lands on the right one in a wall
                                // of text.
                                .background(
                                    RoundedRectangle(cornerRadius: 12)
                                        .fill(Color.accentColor.opacity(
                                            highlightedMessageID == message.localID ? 0.14 : 0))
                                        .padding(.horizontal, -4))
                                .animation(.easeOut(duration: 0.25), value: highlightedMessageID)
                        }
                    }
    }

    /// Open the chat: fetch a first page if there is nothing cached,
    /// decide ONCE where this open lands, and put the thread there.
    ///
    /// Runs in `.task`, so a back-swipe cancels it with the view; every
    /// sleep below is followed by a cancellation check for the reason
    /// ChatPresenceOpening's header spells out at length — a `try?` sleep
    /// in a torn-down view runs the line after it anyway.
    ///
    /// WHAT HAPPENS TO THE DIVIDER OVER A LONG SESSION. The window is a
    /// suffix, arrivals widen it from the TOP, and once it reaches
    /// `maxWindow` it slides — so after a few hundred messages in one
    /// sitting the divider's row leaves the rendered window and the
    /// divider goes with it. That is deliberate and it is the only
    /// coherent answer: the divider marks one row, the row is no longer
    /// drawn, and re-anchoring it to something else would be inventing a
    /// boundary rather than remembering one. The anchor is not recomputed
    /// to chase it, and the reader is not moved.
    private func openThread(proxy: ScrollViewProxy) async {
        // ONCE per open — and "once" is two questions, not one, because
        // `.task` runs again every time this view re-appears (a full-screen
        // attachment viewer closing is enough). The anchor must not be
        // re-decided; the opening scroll must not be re-run once it has
        // landed; but a scroll that was CANCELLED half way through has to be
        // finished, and asking only "is there an anchor" cannot tell that
        // apart from a completed open. See ThreadFollow.openingStep.
        let step = ThreadFollow.openingStep(
            hasSettled: hasSettled, hasAnchor: openAnchor != nil)
        guard step != .done else { return }

        if messages.isEmpty {
            await loadInitial()
            // One runloop tick for the @Query to reflect the inserts — the
            // same tick the pagination path takes, and here for a sharper
            // reason: the anchor below is arithmetic over exactly these
            // rows, and over an empty array it always gives up.
            try? await Task.sleep(nanoseconds: 50_000_000)
        }
        guard !Task.isCancelled else { return }

        // Decided ONCE, before anything scrolls, and never re-decided by a
        // re-appear: by then the chat may have been read, and the answer
        // would be `.newest` — silently erasing a divider still in use.
        if step == .decideAndPlace {
            let chat = self.chat
            let unreadCount = chat?.unreadCount ?? 0
            let target = UnreadAnchor.openAnchor(
                unreadCount: unreadCount,
                myLastReadID: chat?.myLastReadID ?? 0,
                // The suffix, not the whole cache: anything further back
                // than the cap is refused by the rule anyway, and this is
                // a per-row read through SwiftData.
                cachedNewestFirst: messages.suffix(Self.maxWindow).reversed().map {
                    UnreadAnchor.Row(serverID: $0.serverID, senderID: $0.senderID)
                },
                myUserID: currentUserID,
                cap: Self.maxWindow)
            switch target {
            case .newest:
                openAnchor = .newest
            case .message(let serverID):
                openAnchor = .unread(serverID: serverID, count: unreadCount)
            }
        }

        if case .unread(let serverID, _) = openAnchor {
            await settleAtUnread(serverID: serverID, proxy: proxy)
        } else {
            await settleAtBottom(proxy: proxy)
        }
    }

    /// Put the thread at the oldest unread message, with its divider at the
    /// top of the viewport.
    ///
    /// The shape is the quote jump's, copied rather than reinvented, and
    /// each step of it is a bug that has already happened here:
    ///
    /// - WIDEN FIRST. The window is a bounded SUFFIX, so a row far enough
    ///   back is not in the view tree at all, and `scrollTo` at an
    ///   unmaterialized id is a SILENT no-op — no error, no log, nothing
    ///   moves and nothing says why.
    /// - WITH MARGIN. Landing the target against the top sentinel fires a
    ///   history page whose own restore scroll fights this one.
    /// - YIELD, THEN SCROLL, TWICE. Rows widened into existence in this
    ///   turn have no frames yet; the second pass is for the layout that
    ///   lands after the first.
    /// - AT THE DIVIDER, not the message. `.top` on the message parks the
    ///   divider one row above the edge, and the reader arrives at their
    ///   unread messages with nothing saying where the boundary was.
    ///
    /// Anything that cannot be reached falls back to the ordinary open.
    private func settleAtUnread(serverID: Int64, proxy: ScrollViewProxy) async {
        guard let index = messages.firstIndex(where: { $0.serverID == serverID }) else {
            openAnchor = .newest
            await settleAtBottom(proxy: proxy)
            return
        }
        // BEFORE the widen and before the highlight, and the order is
        // load-bearing: the widen is the "page history in" half and the
        // tint is the "flash" half, so a check placed after it fixes only
        // one of the two. Without this, tapping a masked quote materialises
        // up to 300 non-lazy rows and then tints a thin "Hidden"
        // placeholder for 1.6 seconds — pointing the eye at exactly the
        // thing the feature is hiding.
        let target = messages[index]
        // Bridged through the seam rather than hand-writing
        // `blockedUserIDs.contains(target.senderID)`, which would drop the
        // `senderID != currentUserID` guard `isHiddenByBlock` carries.
        if MessagePresentation.isHiddenByBlock(
            MessageSnapshot(target),
            blockedUserIDs: coordinator.blockedUserIDs, currentUserID: currentUserID),
            !revealedMessageIDs.contains(target.localID)
        {
            return
        }
        let needed = UnreadAnchor.rowsToRender(distanceFromNewest: messages.count - 1 - index)
        guard needed <= Self.maxWindow else {
            openAnchor = .newest
            await settleAtBottom(proxy: proxy)
            return
        }
        if needed > visibleCount {
            visibleCount = min(needed, messages.count)
        }

        try? await Task.sleep(nanoseconds: 50_000_000)
        guard !Task.isCancelled else { return }
        proxy.scrollTo(UnreadDivider.scrollID, anchor: .top)
        try? await Task.sleep(nanoseconds: 350_000_000)
        guard !Task.isCancelled else { return }
        proxy.scrollTo(UnreadDivider.scrollID, anchor: .top)
        // Only now: `hasSettled` is what re-arms the pin hooks and what
        // lets the bottom sentinel's geometry mean anything. It is set even
        // though this open is NOT at the newest message — it means "the
        // opening scroll has finished", not "the reader is at the bottom",
        // and `publishPresence` still asks the sentinel where they are.
        hasSettled = true
    }

    /// Opening convergence: defaultScrollAnchor positions from ESTIMATED
    /// row heights, and on a long multi-day history (especially on slower
    /// devices) the estimate can land the viewport in blank unmaterialized
    /// space, with follow-up layout passes drifting it further — one
    /// delayed snap loses that race. Keep snapping to the real newest row
    /// until the bottom sentinel reports the bottom is actually on screen
    /// (or ~2.4 s pass), then hand over to the pin hooks.
    private func settleAtBottom(proxy: ScrollViewProxy) async {
        pinToBottom(proxy, animated: false)
        for _ in 0..<16 {
            if isPinnedToBottom || Task.isCancelled { break }
            try? await Task.sleep(nanoseconds: 150_000_000)
            if Task.isCancelled { break }
            if model.state != .loadingOlder {
                pinToBottom(proxy, animated: false)
            }
        }
        hasSettled = true
    }

    /// Tell the coordinator what this reader can actually see — the only
    /// thing that marks a chat read (ChatPresence). Called from every hook
    /// that can move one of the three facts, and from nowhere that cannot.
    ///
    /// The guard is for the iPad, where the app can have two scenes: a
    /// scene that is not frontmost may only speak when the claim is already
    /// its own or nobody holds one, or a thread sitting in a background
    /// window would overwrite what the scene the user IS looking at
    /// published — purely because a message landed in it.
    /// **There is deliberately no block filter anywhere in this function,
    /// and none in `markRead`.** The read marker advances THROUGH hidden
    /// rows exactly as if they were visible.
    ///
    /// It is satisfied by construction — presence is three geometric and
    /// lifecycle facts with no message identity in it, and `markRead`
    /// targets the chat's newest server id, which is sender-agnostic — so
    /// the absence of code here looks like an oversight and is not. The
    /// failure mode is a well-meaning addition: filter hidden rows out of
    /// the rendered list so the unread divider "looks right", or hold the
    /// marker at an unrevealed row, and the client rebuilds the exact
    /// oracle the server refuses one layer above it. A marker that freezes
    /// at the id before a blocked message and then leaps forward the
    /// moment a third member posts is a perfect, repeatable signal for the
    /// blocked person watching the other end (protocol.md, "Blocking a
    /// member").
    private func publishPresence() {
        let isFrontmost = scenePhase == .active
        guard isFrontmost || coordinator.presence == nil
                || coordinator.presence?.chatID == chatID else { return }
        coordinator.updatePresence(
            chatID: chatID,
            // `hasSettled` matters as much as the sentinel does: until the
            // convergence loop above has finished pinning, `isPinnedToBottom`
            // describes a layout that is still moving, and the server's read
            // marker never comes back once it has been advanced on a guess.
            isAtNewest: hasSettled && isPinnedToBottom,
            isFrontmost: isFrontmost)
    }

    /// Scroll target for every "be at the newest message" path: the bottom
    /// sentinel, so being pinned and reporting pinned coincide.
    static let bottomSentinelID = "bottom-sentinel"

    private func pinToBottom(_ proxy: ScrollViewProxy, animated: Bool) {
        guard !messages.isEmpty else { return }
        if animated {
            withAnimation(.easeOut(duration: 0.2)) {
                proxy.scrollTo(Self.bottomSentinelID, anchor: .bottom)
            }
        } else {
            proxy.scrollTo(Self.bottomSentinelID, anchor: .bottom)
        }
    }

    // MARK: - Input bar

    private var inputBar: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let replyDraft {
                replyBanner(replyDraft)
            }
            if editTarget != nil {
                editBanner()
            }
            if mediaState != .idle || composerNotice != nil {
                mediaStrip
            }
            if recorder.isRecording {
                recordingStrip
            }
            if !staged.isEmpty {
                stagedRow
            }
            // Two strips, never both: the assistant's own chat says what a
            // staged photo will meet; the family chat says the same for a
            // photo an `@ai` draft is about to carry (#56).
            if let notice = pictureNotice ?? mentionPictureNotice {
                assistantPictureNotice(notice)
            }
            HStack(alignment: .bottom, spacing: 8) {
                // A Menu rather than two buttons: the composer is narrow,
                // and "attach" is one intent with two sources.
                Menu {
                    // In the assistant's own chat the picture doors are the
                    // vision gate, and they are ABSENT rather than disabled
                    // when it is shut: a server with no vision deployment,
                    // or a family whose owner has not turned `ai_vision`
                    // on, must show no surface at all rather than one that
                    // lies about what would happen (protocol.md,
                    // "Pictures"). Everywhere else they are unconditional,
                    // exactly as they have always been.
                    if !isAssistantChat || showsPictureAttach {
                        Button {
                            showPhotoPicker = true
                        } label: {
                            // In the assistant's chat this door NAMES what
                            // it does. The switch lives on a settings
                            // screen somebody read once; this is where the
                            // photograph is actually chosen, and it is the
                            // last place the consequence can be said before
                            // it happens. A video never reaches the model
                            // either — the server sends photographs and
                            // nothing else — so the wording is honest about
                            // that too. Same sentence the Mac's panel uses.
                            //
                            // Two literals rather than one ternary so
                            // `check-strings.py` can see both keys: it
                            // reads source text, and a key inside a
                            // conditional expression is invisible to it.
                            if isAssistantChat {
                                Label("Show the Assistant a Photo…", systemImage: "photo")
                            } else {
                                Label("Photo or Video", systemImage: "photo.on.rectangle")
                            }
                        }
                    }
                    Button {
                        showFilePicker = true
                    } label: {
                        Label("File", systemImage: "doc")
                    }
                    // Inside the menu on purpose: the guard below disables
                    // attaching while an edit or an upload is in flight, and
                    // an item here inherits it for free. It is also the door
                    // that works when the field is NOT focused, which is
                    // where a keyboard ⌘V cannot reach.
                    //
                    // Through `pasteFromClipboard` like every other door,
                    // and that is the fix: this item used to call
                    // `pasteAttachment` directly, so it answered "There's
                    // nothing to paste." to a clipboard full of words and
                    // attached the picture out of a clipboard the rule says
                    // is text.
                    Button {
                        pasteFromClipboard()
                    } label: {
                        Label("Paste", systemImage: "doc.on.clipboard")
                    }
                    // Hidden rather than disabled where there is no camera
                    // (Simulator, camera-less device): presenting the picker
                    // there shows an empty black sheet.
                    if CameraPicker.isAvailable, !isAssistantChat || showsPictureAttach {
                        Button {
                            showCamera = true
                        } label: {
                            Label("Camera", systemImage: "camera")
                        }
                    }
                    Button {
                        Task { await startRecording() }
                    } label: {
                        Label("Record Audio", systemImage: "mic")
                    }
                    Button {
                        shareLocation()
                    } label: {
                        Label("Location", systemImage: "mappin.and.ellipse")
                    }
                    // The family chat only, and the server agrees: a poll
                    // anywhere else is `invalid_poll` (docs/protocol.md,
                    // "Polls"). A poll is a family deciding something
                    // together; between two people it is a question, and
                    // the answer is the next message.
                    if isFamilyChat {
                        Button {
                            showPollComposer = true
                        } label: {
                            Label("Poll", systemImage: "chart.bar")
                        }
                    }
                } label: {
                    Image(systemName: "paperclip")
                        .font(.system(size: 20))
                        .foregroundStyle(.tint)
                        .frame(width: composerControl, height: composerControl)
                        .contentShape(Rectangle())
                }
                // Editing borrows the composer to rewrite an existing
                // message, which has no second attachment to add — and
                // attaching would have posted the edit as a new message
                // while leaving the banner armed. Android already gated
                // this; iOS did not.
                .disabled(composerIsBusy)
                .accessibilityLabel("Attach a photo, video or file")
                .photosPicker(
                    isPresented: $showPhotoPicker,
                    selection: $pickedMedia,
                    maxSelectionCount: StagedAttachment.maxPerMessage,
                    // No `photoLibrary:` — see the note in SettingsView. The
                    // out-of-process picker needs no PhotoKit reference and
                    // no usage description, and nothing here reads a PHAsset.
                    matching: isAssistantChat ? .images : .any(of: [.images, .videos]))
                if showsAssistantMention {
                    Button {
                        insertAssistantMention()
                    } label: {
                        Image(systemName: "sparkles")
                            .font(.system(size: 20))
                            .foregroundStyle(.tint)
                            .frame(width: composerControl, height: composerControl)
                            .contentShape(Rectangle())
                    }
                    .disabled(editTarget != nil)
                    .accessibilityLabel("Ask the assistant")
                }
                if showsPictureRequest {
                    Button {
                        insertDrawToken()
                    } label: {
                        Image(systemName: "paintbrush")
                            .font(.system(size: 19))
                            .foregroundStyle(.tint)
                            .frame(width: composerControl, height: composerControl)
                            .contentShape(Rectangle())
                    }
                    .disabled(editTarget != nil)
                    .accessibilityLabel("Ask for a picture")
                }
                TextField("Message", text: Bindable(model).draft, axis: .vertical)
                    .focused($inputFocused)
                    // The backstop for the door this side does not own:
                    // the field's OWN paste — the edit menu's Paste item,
                    // a drag into the field — happens inside UIKit, and
                    // reaching it would mean replacing this composer with a
                    // UIViewRepresentable. The draft it leaves behind can
                    // always be seen, so the protocol's 4000-character
                    // ceiling is applied here for those, with the same
                    // sentence the paste doors use.
                    .onChange(of: model.draft) { _, draft in
                        guard let clamped = ComposerText.clamping(draft) else { return }
                        model.draft = clamped
                        composerNotice = String(
                            localized: "A message can be at most \(ComposerText.bodyLimit) characters.")
                    }
                    .lineLimit(1...5)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 7)
                    .background(Color(.secondarySystemFill), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                Button {
                    send()
                } label: {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.system(size: 26))
                        .foregroundStyle(.tint)
                        .frame(width: composerControl, height: composerControl)
                        .contentShape(Rectangle())
                }
                .disabled(!canSend)
                .accessibilityLabel("Send")
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
        }
        .background(.bar)
        .background { pasteShortcut }
        .onGeometryChange(for: CGFloat.self) { geometry in
            geometry.size.height
        } action: { height in
            inputBarHeight = height
        }
        // On the input bar rather than the thread's own chain, which is
        // already at the type-checker's limit; it is the bar that reads it.
        .onChange(of: replyDraft?.messageID, initial: true) { _, messageID in
            quotedAttachments = quotedAttachmentList(for: messageID)
        }
    }

    /// ⌘V from a hardware keyboard, for the pastes a text field will not
    /// take.
    ///
    /// Zero-sized and unlabelled: the DISCOVERABLE door is the Paste item
    /// in the attach menu, and this exists only so the gesture people
    /// already have in their fingers does the obvious thing on an iPad.
    ///
    /// It is written to be correct under BOTH possible orders of the
    /// responder chain, because which one UIKit picks is not something this
    /// side can assert. If the focused field claims ⌘V — which is what a
    /// standard edit action does against a key command declared further up
    /// the chain — this never runs and an ordinary text paste is untouched.
    /// If it runs instead, the rule answers `.type` for a clipboard of
    /// words and `pasteText` appends them, so a text paste is not swallowed
    /// either way.
    ///
    /// NOT `.disabled` during an edit or an upload any more, and that is a
    /// fix rather than a relaxation. Disabled, ⌘V of a picture mid-upload
    /// did nothing and said nothing — the one outcome a keyboard gesture
    /// must never have. The guard now lives in the rule, where it gates the
    /// ATTACHMENT branch only: `.busy` says so out loud, and words still
    /// land in the draft, which during an edit is precisely what was meant.
    private var pasteShortcut: some View {
        Button {
            pasteFromClipboard()
        } label: {
            // A real title, clipped to nothing: iPadOS reads the label when
            // it draws the ⌘-held shortcut list, and a blank row there
            // would be worse than no row.
            Text("Paste")
                .frame(width: 0, height: 0)
                .clipped()
                .opacity(0)
        }
        .buttonStyle(.plain)
        .keyboardShortcut("v", modifiers: .command)
        .accessibilityHidden(true)
    }

    /// "Replying to X" above the field, with the way out. Its appearance
    /// changes the input bar's height, which the thread already watches
    /// (onChange(of: inputBarHeight)) and re-pins for — the same path a
    /// multi-line draft takes.
    @ViewBuilder
    private func replyBanner(_ quote: ReplyToDTO) -> some View {
        HStack(spacing: 8) {
            RoundedRectangle(cornerRadius: 1.5)
                .fill(Color.accentColor)
                .frame(width: 3, height: 32)
            VStack(alignment: .leading, spacing: 1) {
                Text("Replying to \(quoteAuthorName(quote.senderID))")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tint)
                Text(quote.excerpt)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
            Button {
                // Animated, so the banner leaves the way it arrived; a
                // bare assignment plays no transition on the way out.
                withAnimation(.spring(duration: 0.25)) {
                    replyDraft = nil
                }
                replyStartedFromHistory = false
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .foregroundStyle(.secondary)
                    // Tap slack only, the poll's idiom: a negative inset,
                    // so the banner's height stays exactly what it is —
                    // it feeds the inputBarHeight re-pin.
                    .contentShape(Rectangle().inset(by: -12))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Cancel reply")
        }
        .padding(.horizontal, 12)
        .padding(.top, 8)
        .transition(.move(edge: .bottom).combined(with: .opacity))
    }

    /// "Editing message" above the field, with the way out. Cancelling
    /// puts the draft back the way it was — the composer was borrowed for
    /// the edit, and giving it back unchanged is the least surprising
    /// thing it can do.
    @ViewBuilder
    private func editBanner() -> some View {
        HStack(spacing: 8) {
            Image(systemName: "pencil")
                .font(.caption)
                .foregroundStyle(.tint)
            Text("Editing message")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.tint)
            Spacer(minLength: 0)
            Button {
                cancelEdit()
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .foregroundStyle(.secondary)
                    // Tap slack only — see the reply banner.
                    .contentShape(Rectangle().inset(by: -12))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Cancel editing")
        }
        .padding(.horizontal, 12)
        .padding(.top, 8)
        .transition(.move(edge: .bottom).combined(with: .opacity))
    }

    /// Borrow the composer to rewrite an existing message: prefill it with
    /// what is there now, so an edit starts from the text rather than from
    /// nothing.
    private func beginEdit(serverID: Int64?, body: String) {
        guard let serverID else { return }
        replyStartedFromHistory = !isPinnedToBottom
        withAnimation(.spring(duration: 0.25)) {
            replyDraft = nil
            editTarget = (messageID: serverID, original: model.draft)
        }
        model.draft = body
        inputFocused = true
    }

    private func cancelEdit() {
        let restored = editTarget?.original ?? ""
        withAnimation(.spring(duration: 0.25)) {
            editTarget = nil
        }
        model.draft = restored
        replyStartedFromHistory = false
    }

    private func quoteAuthorName(_ senderID: Int64) -> String {
        if senderID == currentUserID { return String(localized: "You") }
        return displayName(for: senderID) ?? String(localized: "Someone")
    }

    /// Scroll to a quoted message when this device actually holds it.
    ///
    /// Best-effort by design: the thread renders a bounded window and the
    /// quoted message may be thousands of rows back, so rather than paging
    /// the whole history to chase it, this widens the window once to cover
    /// what is cached and scrolls if the row is there. A quote that cannot
    /// be reached simply does nothing — the excerpt is already on screen,
    /// which is most of what the tap was asking for.
    private func jumpToMessage(_ serverID: Int64, proxy: ScrollViewProxy) {
        guard let index = messages.firstIndex(where: { $0.serverID == serverID }) else { return }
        let target = messages[index]

        // Widen with MARGIN, not to exactly the target: the top sentinel
        // triggers pagination when it appears, so parking the target one
        // row below it fires a history load whose own scroll restore
        // fights the jump.
        let needed = messages.count - index + Self.jumpMargin
        // The window is a SUFFIX and the rows are non-lazy, so reaching a
        // message thousands back would materialize everything after it in
        // one layout pass. Past the cap the jump simply does not happen —
        // the excerpt is already on screen, which is most of what the tap
        // was asking for. (Android gives up the same way, after
        // MAX_JUMP_PAGES.)
        guard needed <= Self.maxWindow else { return }
        if needed > visibleCount {
            visibleCount = min(needed, messages.count)
        }

        // Scrolling in the same turn as the window change silently
        // no-ops: the rows the widened window adds have not been laid out
        // yet, and scrollTo to an id that does not exist does nothing.
        // Twice, like the opening pin: once as layout lands, once after
        // it settles.
        Task { @MainActor in
            withAnimation { proxy.scrollTo(target.localID, anchor: .center) }
            highlight(target.localID)
            try? await Task.sleep(nanoseconds: 350_000_000)
            proxy.scrollTo(target.localID, anchor: .center)
        }
    }

    /// Tint the jumped-to bubble, then let it fade. The token guards the
    /// clear so a second jump before the first fades does not wipe the
    /// newer highlight.
    private func highlight(_ localID: String) {
        highlightedMessageID = localID
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(1.6))
            if highlightedMessageID == localID { highlightedMessageID = nil }
        }
    }

    /// Prime the composer to answer this message, and put the keyboard up:
    /// the user chose Reply because they intend to type.
    private func beginReply(serverID: Int64?, senderID: Int64, body: String) {
        guard let serverID else { return }
        // Decided BEFORE the keyboard opens: afterwards the thread has
        // already moved and the answer would always be "pinned".
        replyStartedFromHistory = !isPinnedToBottom
        withAnimation(.spring(duration: 0.25)) {
            replyDraft = ReplyToDTO(
                messageID: serverID,
                senderID: senderID,
                // Cut exactly as the server will, so the banner and the
                // final bubble agree.
                excerpt: ReplyToSnapshot.excerpt(of: body))
        }
        inputFocused = true
    }

    /// What the composer shows while a photo or video is on its way.
    @ViewBuilder
    /// While a voice note is being recorded: a counter, and the two ways
    /// out. Stop STAGES it rather than sending — so a caption can be added,
    /// and so a recording made by accident can still be discarded.
    private var recordingStrip: some View {
        HStack(spacing: 10) {
            Image(systemName: "waveform")
                .foregroundStyle(.red)
                .symbolEffect(.variableColor.iterative, isActive: true)
            Text(verbatim: AudioRecorder.timeLabel(recorder.elapsed))
                .font(.callout.monospacedDigit())
            Spacer(minLength: 0)
            Button("Cancel") { recorder.cancel() }
                .font(.callout)
            Button("Stop") { finishRecording() }
                .font(.callout.weight(.semibold))
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .transition(.move(edge: .bottom).combined(with: .opacity))
    }

    private func finishRecording() {
        guard let url = recorder.stop() else {
            mediaState = .failed(String(localized: "That recording was too short."))
            return
        }
        mediaState = .preparing
        Task {
            do {
                stage(try await MediaPrep.prepareAudio(from: url, limit: MediaPrep.sizeLimit))
            } catch MediaPrep.PrepError.tooLargeAfterCompression {
                mediaState = .failed(String(localized: "That file is over the 100 MB limit."))
                try? FileManager.default.removeItem(at: url)
            } catch {
                mediaState = .failed(String(localized: "Couldn't prepare that item."))
                try? FileManager.default.removeItem(at: url)
            }
        }
    }

    /// The guard every attachment door carries: the composer is borrowed
    /// for an edit, or already busy with a file. Named once so the attach
    /// menu, the ⌘V door and the rule's `busy` branch cannot drift apart.
    private var composerIsBusy: Bool {
        mediaState.blocksComposer || editTarget != nil
    }

    /// Whether Send has anything to do: words, or something staged, or both.
    private var canSend: Bool {
        if !staged.isEmpty { return true }
        return !model.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// The staged attachments sitting above the field, each with its own
    /// way out — the familiar chip for one, a scrolling row for several.
    @ViewBuilder
    private var stagedRow: some View {
        StagedAttachmentRow(items: staged) { item in
            withAnimation(.spring(duration: 0.25)) { discardStaged(item) }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .transition(.move(edge: .bottom).combined(with: .opacity))
    }

    /// The one line this composer uses to say what it is doing or what
    /// went wrong.
    ///
    /// A paste-time notice takes precedence over the media state and does
    /// not replace it: "you can't attach that while this is uploading" is
    /// said WHILE the upload is running, and dismissing it has to leave the
    /// upload exactly where it was. Every failure here has a way out — an
    /// error nothing clears sits in the composer for the rest of the
    /// conversation.
    private var mediaStrip: some View {
        HStack(spacing: 8) {
            if let composerNotice {
                Image(systemName: "exclamationmark.circle")
                    .foregroundStyle(.red)
                Text(composerNotice)
                    .font(.caption)
                    .foregroundStyle(.red)
            } else {
                switch mediaState {
                case .preparing:
                    ProgressView()
                        .controlSize(.small)
                    Text("Preparing…")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                case .uploading(let progress):
                    ProgressView()
                        .controlSize(.small)
                    Text(progress ?? String(localized: "Sending…"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                case .working(let label):
                    ProgressView()
                        .controlSize(.small)
                    Text(label)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                case .failed(let reason):
                    Image(systemName: "exclamationmark.circle")
                        .foregroundStyle(.red)
                    Text(reason)
                        .font(.caption)
                        .foregroundStyle(.red)
                case .idle:
                    EmptyView()
                }
            }
            Spacer(minLength: 0)
            if composerNotice != nil {
                Button("Dismiss") { composerNotice = nil }
                    .font(.caption)
            } else if case .failed = mediaState {
                Button("Dismiss") { mediaState = .idle }
                    .font(.caption)
            }
        }
        .padding(.horizontal, 12)
        .padding(.top, 8)
    }

    /// The three things attachments hang off the thread: the photo
    /// picker's result, the document picker, and the two ways an
    /// attachment opens (Quick Look for a file, the viewer for media).
    private var attachmentSurfaces: some ViewModifier {
        AttachmentSurfaces(
            pickedMedia: $pickedMedia,
            showFilePicker: $showFilePicker,
            previewedFile: $previewedFile,
            viewingAlbum: $viewingAlbum,
            showCamera: $showCamera,
            onPickedMedia: stagePickedMedia,
            onPickedFiles: stagePickedFiles,
            onCapturedPhoto: stageCapturedPhoto,
            onCapturedVideo: stageCapturedVideo,
            onImportFailed: { mediaState = .failed(String(localized: "Couldn't read that file.")) },
            onShareAttachment: { shareAttachment($0, caption: "") })
    }

    /// Share an attachment as a FILE, downloading it first if this device
    /// does not have the bytes yet.
    ///
    /// A URL rather than a UIImage: it is what lets the sheet offer Save
    /// to Files, AirDrop and every document app, and it keeps the original
    /// bytes rather than re-encoding them on the way out.
    func shareAttachment(_ attachment: AttachmentDTO, caption: String) {
        Task {
            mediaState = .working(String(localized: "Preparing…"))
            guard let url = await coordinator.localFileURL(for: attachment) else {
                mediaState = .failed(String(localized: "Couldn't download that to share."))
                return
            }
            mediaState = .idle
            var items: [Any] = [url]
            if !caption.isEmpty { items.append(caption) }
            sharePayload = SharePayload(items: items)
        }
    }

    /// What the composer was holding when a media send started.
    ///
    /// Read and CLEARED up front, exactly as a text send does. Reading the
    /// draft after the upload instead meant anything typed while it ran was
    /// wiped when the photo posted, and a primed reply was dropped on the
    /// floor with its banner left armed — so the next ordinary message
    /// silently became the reply.
    private struct ComposerHandoff {
        let caption: String
        let replyTo: ReplyToDTO?
    }

    private func takeComposer() -> ComposerHandoff {
        let handoff = ComposerHandoff(caption: model.draft, replyTo: replyDraft)
        model.draft = ""
        replyDraft = nil
        replyStartedFromHistory = false
        return handoff
    }

    /// Put it back when the send never happened, so nothing is lost.
    private func restore(_ handoff: ComposerHandoff) {
        if model.draft.isEmpty { model.draft = handoff.caption }
        if replyDraft == nil { replyDraft = handoff.replyTo }
    }

    /// Start a voice note, and SAY SO IF IT DOES NOT START.
    ///
    /// The recorder has always recorded its failure; nothing read it, so a
    /// denied microphone made this button do nothing at all. The two causes
    /// get different sentences on purpose — pointing somebody at Settings
    /// for a permission they already granted wastes their time, and the
    /// location path next door already draws exactly this distinction.
    private func startRecording() async {
        await recorder.start()
        if let failure = recorder.failure {
            mediaState = .failed(AudioRecorder.message(for: failure))
        }
    }

    /// EVERY paste door on this screen, and the only one.
    ///
    /// The attach menu's Paste item and the hardware ⌘V both end up here,
    /// and neither repeats a word of the policy: `ClipboardAttachment.door`
    /// reads the free probes, applies the one rule, and names the single
    /// thing to do. A door that decided for itself is what produced the two
    /// behaviours this replaces — a menu item that said "There's nothing to
    /// paste." to a clipboard of words, and a ⌘V that went silent whenever
    /// the composer was busy.
    ///
    /// The payload is read exactly once, after the answer, by whichever of
    /// `prepare` or `pendingText` the answer names — which is what keeps
    /// iOS's "Allow Paste?" alert down to one, at the moment somebody
    /// actually asked for a paste.
    private func pasteFromClipboard() {
        switch ClipboardAttachment.door(composerIsBusy: composerIsBusy) {
        case .attach:
            pasteAttachment()
        case .type:
            pasteText()
        case .busy:
            // Which busy it is, because the two have different ways out.
            composerNotice = editTarget != nil
                ? String(localized: "Finish editing before attaching something.")
                : String(localized: "Wait until the current attachment is done.")
        case .nothing:
            composerNotice = String(localized: "There's nothing to paste.")
        }
    }

    /// The clipboard's words, into the draft.
    ///
    /// Appended rather than inserted at the caret, for the reason
    /// `insertAssistantMention` has: SwiftUI's TextField publishes no
    /// selection before iOS 18, the deployment target is 17, and appending
    /// is what this composer already does everywhere else.
    ///
    /// The ceiling is the protocol's (docs/protocol.md, "Limits": 4000
    /// characters). Refusing here is the whole difference between a
    /// sentence saying why and a Send that comes back `message_too_long`
    /// long after the clipboard has moved on.
    private func pasteText() {
        guard let text = ClipboardAttachment.pendingText, !text.isEmpty else {
            composerNotice = String(localized: "There's nothing to paste.")
            return
        }
        switch ComposerText.appending(text, to: model.draft) {
        case .appended(let draft):
            model.draft = draft
            composerNotice = nil
        case .truncated(let draft):
            model.draft = draft
            composerNotice = String(
                localized: "A message can be at most \(ComposerText.bodyLimit) characters. The rest wasn't pasted.")
        case .full:
            composerNotice = String(
                localized: "The message is already at the \(ComposerText.bodyLimit)-character limit.")
        }
        inputFocused = true
    }

    /// Attach whatever is on the clipboard.
    ///
    /// Reached only once the rule has said `.attachment`: `prepare` takes
    /// the best representation it can find, which on a clipboard holding
    /// both words and a picture is the picture — so leaving it to arbitrate
    /// would quietly contradict the rule.
    ///
    /// Through `stage`, like every other door: one attachment per message,
    /// nothing is sent until Send is pressed, and the caption can be
    /// written while looking at the chip.
    private func pasteAttachment() {
        composerNotice = nil
        mediaState = .preparing
        Task {
            do {
                stage(try await ClipboardAttachment.prepare(limit: MediaPrep.sizeLimit))
            } catch ClipboardAttachment.Failure.nothingToPaste {
                mediaState = .failed(String(localized: "There's nothing to paste."))
            } catch MediaPrep.PrepError.tooLargeAfterCompression {
                // The same ceiling and the same wording every other door
                // uses; a pasted item is not a special kind of too big.
                mediaState = .failed(String(localized: "That file is over the 100 MB limit."))
            } catch {
                mediaState = .failed(String(localized: "Couldn't prepare that item."))
            }
        }
    }

    /// Stage picked documents, in the order picked. Nothing is re-encoded
    /// — a file goes as it is. Sequential on purpose: preparing is a copy
    /// (an iCloud download, possibly), and ten at once would stampede.
    private func stagePickedFiles(_ urls: [URL]) {
        mediaState = .preparing
        Task {
            for url in urls {
                do {
                    stage(try await MediaPrep.prepareFile(from: url, limit: MediaPrep.sizeLimit))
                } catch MediaPrep.PrepError.tooLargeAfterCompression {
                    // A document cannot be compressed the way a video can,
                    // so the advice is different: there is nothing to try.
                    mediaState = .failed(String(localized: "That file is over the 100 MB limit."))
                } catch {
                    mediaState = .failed(String(localized: "Couldn't read that file."))
                }
            }
        }
    }

    /// A still from the camera. It arrives as encoded bytes rather than a
    /// file — the one source that has no URL — so it goes straight into the
    /// same photo path a library pick uses.
    private func stageCapturedPhoto(_ data: Data) {
        mediaState = .preparing
        Task {
            do {
                stage(try await MediaPrep.preparePhoto(from: data, limit: MediaPrep.sizeLimit))
            } catch MediaPrep.PrepError.tooLargeAfterCompression {
                mediaState = .failed(String(localized: "Still too large after compressing — try a shorter clip."))
            } catch {
                mediaState = .failed(String(localized: "Couldn't prepare that item."))
            }
        }
    }

    /// A clip from the camera: a real file in tmp, same as a library video.
    private func stageCapturedVideo(_ url: URL) {
        mediaState = .preparing
        Task {
            do {
                let prepared = try await MediaPrep.prepareVideo(from: url, limit: MediaPrep.sizeLimit)
                // prepareVideo hands back the source itself when it already
                // fits; only delete the capture when it made a new file.
                if prepared.fileURL != url {
                    try? FileManager.default.removeItem(at: url)
                }
                stage(prepared)
            } catch MediaPrep.PrepError.tooLargeAfterCompression {
                mediaState = .failed(String(localized: "Still too large after compressing — try a shorter clip."))
                try? FileManager.default.removeItem(at: url)
            } catch {
                mediaState = .failed(String(localized: "Couldn't prepare that item."))
                try? FileManager.default.removeItem(at: url)
            }
        }
    }

    /// Hold prepared media in the composer, APPENDING behind whatever is
    /// already staged — a message carries up to ten attachments, in the
    /// order staged. At the cap the pick is refused with a notice (the
    /// house style for something the composer could not do) and the
    /// prepared file cleaned up, because nothing else owns it now.
    private func stage(_ prepared: MediaPrep.Prepared) {
        guard StagedAttachment.canAdd(to: staged.count) else {
            try? FileManager.default.removeItem(at: prepared.fileURL)
            mediaState = .idle
            composerNotice = String(
                localized: "You can attach up to \(StagedAttachment.maxPerMessage) items.")
            return
        }
        mediaState = .idle
        composerNotice = nil
        withAnimation(.spring(duration: 0.25)) {
            staged.append(StagedAttachment(prepared: prepared))
        }
        inputFocused = true
    }

    /// Commit the staged set with whatever the composer holds.
    ///
    /// The composer is taken atomically FIRST — caption, quote and the
    /// staged files together — so nothing typed during the upload is
    /// swallowed into the caption and a primed reply cannot leak onto the
    /// next message. Everything goes back if the send never lands.
    private func sendStaged(_ items: [StagedAttachment], caption: String) {
        let handoff = takeComposer()
        withAnimation(.spring(duration: 0.25)) { staged = [] }
        mediaState = .uploading(nil)
        // The outbox owns the set from HERE, before the first byte goes
        // out — not from the failure branch. The window this closes is the
        // one where the set belongs to nobody but a running Task, so a
        // reader who leaves mid-upload loses the caption, the reply and the
        // files with nothing left holding them.
        let token = outbox.begin(
            chatID: chatID,
            caption: handoff.caption,
            replyTo: handoff.replyTo,
            prepared: items.map(\.prepared))
        Task {
            let sent = await coordinator.sendMedia(
                items.map(\.prepared),
                caption: handoff.caption,
                replyTo: handoff.replyTo,
                in: chatID,
                onItemStart: { index, total in
                    // The existing strip carries the progress; one item
                    // keeps the plain "Sending…" it has always shown.
                    // Progress stays INSIDE `.uploading` so the composer
                    // still counts as busy — `.working` would reopen the
                    // attach menu and the paste door mid-send.
                    guard total > 1 else { return }
                    mediaState = .uploading(
                        String(localized: "Uploading \(index) of \(total)…"))
                })
            if sent {
                // `sendMedia` consumed the files on its way out; the entry
                // is all that is left to drop.
                outbox.finish(token)
                mediaState = .idle
            } else {
                // Mark it failed and DO NOT take it here. A take removes
                // the entry whether or not this view is still alive, and if
                // it is not, the restore that follows writes into @State
                // nobody owns — losing the set exactly as before, with its
                // files now unowned too. Recovery is driven by observation
                // instead: `.onChange` below fires on whichever composer
                // for this chat is actually on screen, and that one takes it.
                outbox.fail(token)
                mediaState = .failed(String(localized: "Couldn't send that — try again."))
            }
        }
    }

    /// Adopt a send that failed with nobody left to hand it back to.
    ///
    /// The composer that started it is gone — the reader left the chat
    /// while the upload ran — so its restore went into `@State` that no
    /// longer exists. The set is still owned by the outbox; this takes it
    /// back so the photos, the caption and the reply are where the reader
    /// left them.
    ///
    /// Only when this composer is empty. A reader who has since staged
    /// something else is mid-thought, and merging an old set into it would
    /// send photos they did not choose alongside the ones they did.
    private func recoverFailedSend() {
        // Only `staged` gates this. An earlier version also required an
        // empty draft, which lost the photos of anyone who typed their next
        // message while the upload ran — and the code this replaced put the
        // set back unconditionally, so that was a regression, not a
        // tightening. Restoring into `staged` sends nothing on its own:
        // `send()` still needs a deliberate press.
        guard staged.isEmpty,
              // Never over a share import: those arrive into this same
              // composer and are what the reader is actually looking at.
              session.shareImportTarget == nil,
              let waiting = outbox.mostRecentFailed(for: chatID),
              let recovered = outbox.take(waiting.id)
        else { return }
        staged = recovered.prepared.map { StagedAttachment(prepared: $0) }
        if model.draft.isEmpty { model.draft = recovered.caption }
        if replyDraft == nil { replyDraft = recovered.replyTo }
        mediaState = .failed(String(localized: "Couldn't send that — try again."))
    }

    /// Throw away ONE staged item and its temp file.
    ///
    /// The file is ours: `MediaPrep` wrote it into a temp directory and
    /// nothing else will clean it up, because the delete that normally
    /// consumes it lives in `sendMedia` — which never ran.
    private func discardStaged(_ item: StagedAttachment) {
        try? FileManager.default.removeItem(at: item.prepared.fileURL)
        staged.removeAll { $0.id == item.id }
    }

    /// Throw away the whole staged set.
    private func discardStaged() {
        for item in staged {
            try? FileManager.default.removeItem(at: item.prepared.fileURL)
        }
        staged = []
    }

    /// Fetch a file's bytes and hand them to Quick Look, which previews
    /// what it can and offers Share for everything else.
    private func openFile(_ attachment: AttachmentDTO) {
        Task {
            guard let url = await coordinator.localFileURL(for: attachment) else {
                mediaState = .failed(String(localized: "Couldn't download that file."))
                return
            }
            previewedFile = url
        }
    }

    /// Prepare and stage the picked photos and videos, in picked order.
    ///
    /// Preparation happens off the main actor: re-encoding a video is
    /// seconds of work, and the thread has to keep scrolling while it
    /// runs. Sequential rather than concurrent — several 4K clips
    /// re-encoding at once is how a phone falls over.
    private func stagePickedMedia(_ items: [PhotosPickerItem]) {
        mediaState = .preparing
        Task {
            defer { pickedMedia = [] }
            let limit = MediaPrep.sizeLimit
            for item in items {
                // Decide from what the item SAYS it is, rather than trying a
                // movie transfer and reading the failure as "must be a photo" —
                // a transfer can fail for reasons that have nothing to do with
                // the kind (iCloud, cancellation), and that path would then
                // hand a video's bytes to the photo decoder.
                let isVideo = item.supportedContentTypes.contains { $0.conforms(to: .movie) }
                let prepared: MediaPrep.Prepared
                do {
                    if isVideo {
                        guard let movie = try await item.loadTransferable(type: PickedMovie.self) else {
                            mediaState = .failed(String(localized: "Couldn't read that video."))
                            continue
                        }
                        prepared = try await MediaPrep.prepareVideo(from: movie.url, limit: limit)
                        // prepareVideo returns the source itself when it already
                        // fits; only delete the copy when it made a new file.
                        if prepared.fileURL != movie.url {
                            try? FileManager.default.removeItem(at: movie.url)
                        }
                    } else if let data = try await item.loadTransferable(type: Data.self) {
                        prepared = try await MediaPrep.preparePhoto(from: data, limit: limit)
                    } else {
                        mediaState = .failed(String(localized: "Couldn't read that item."))
                        continue
                    }
                } catch MediaPrep.PrepError.tooLargeAfterCompression {
                    // The one case the user has to act on: compression was not
                    // enough, so say what would help rather than just refusing.
                    mediaState = .failed(String(localized: "Still too large after compressing — try a shorter clip."))
                    continue
                } catch {
                    mediaState = .failed(String(localized: "Couldn't prepare that item."))
                    continue
                }

                stage(prepared)
            }
        }
    }

    private var typingLine: String? {
        let ids = coordinator.typingUserIDs(in: chatID)
        guard !ids.isEmpty else { return nil }
        let names = ids.map { displayName(for: $0) ?? String(localized: "Someone") }
        switch names.count {
        case 1: return String(localized: "\(names[0]) is typing…")
        default: return String(localized: "\(names.joined(separator: ", ")) are typing…")
        }
    }

    // MARK: - Floating reaction picker

    /// The Tapback-style layer over the thread: empty until a bubble is
    /// long-pressed, then a dimmed scrim (tap = dismiss) plus one
    /// floating menu over the pressed bubble — the reaction capsule for
    /// messages the server knows, Retry/Delete for failed ones (no
    /// server id, so the two gates never show together).
    @ViewBuilder
    private func reactionPickerOverlay(anchors: [String: Anchor<CGRect>]) -> some View {
        GeometryReader { proxy in
            ZStack(alignment: .topLeading) {
                if let localID = reactionPickerID,
                   let message = messages.first(where: { $0.localID == localID }),
                   let anchor = anchors[localID] {
                    Color.black.opacity(0.2)
                        .ignoresSafeArea()
                        .onTapGesture { dismissReactionPicker() }
                        .transition(.opacity)

                    let rect = proxy[anchor]
                    if message.state == .failed {
                        floatingMenu(size: FailedMessageMenu.size, over: rect, in: proxy) {
                            FailedMessageMenu(
                                onRetry: {
                                    coordinator.retry(localID: localID)
                                    dismissReactionPicker()
                                },
                                onDelete: {
                                    coordinator.deleteLocalMessage(localID: localID)
                                    dismissReactionPicker()
                                })
                        }
                    } else if message.serverID != nil {
                        // Reacting needs a server message id to PUT against.
                        let mine = message.reactionList.first { $0.userID == currentUserID }?.emoji
                        let emojis = capsuleEmojis(mine: mine)
                        let capsuleSize = ReactionCapsule.size(emojiCount: emojis.count)
                        floatingMenu(size: capsuleSize, over: rect, in: proxy) {
                            ReactionCapsule(
                                emojis: emojis,
                                selected: mine,
                                onPick: { emoji in
                                    toggleReaction(localID: localID, emoji: emoji)
                                    dismissReactionPicker()
                                },
                                onMore: {
                                    dismissReactionPicker()
                                    fullPickerTarget = ReactionTarget(localID: localID)
                                })
                        }
                        // The actions go UNDER the bubble, so the capsule
                        // above it stays where the thumb already expects
                        // it — and under the capsule when that had to
                        // flip down too, so the two never overlap.
                        let canReply = message.serverID != nil
                        let canEdit = message.serverID != nil && message.senderID == currentUserID
                        let attachment = message.attachmentSnapshot
                        // A photo sent without a caption has nothing to copy.
                        let canCopy = !message.body.isEmpty
                        // Somebody else's message, from a real member.
                        // NEVER the assistant: its reserved account is
                        // deliberately absent from the roster, so blocking
                        // it would name a non-member and the server would
                        // refuse — a VISIBLE refusal in a feature whose
                        // whole design is that refusals look innocent.
                        let isOther = message.senderID != currentUserID
                        let isAssistantSender = isAssistantChat
                            || message.senderID == AppSettings.assistantUserID
                        let canReport = isOther && !isAssistantSender && message.serverID != nil
                        let blockState: MessageContextMenu.BlockState? =
                            (isOther && !isAssistantSender)
                            ? (coordinator.blockedUserIDs.contains(message.senderID)
                                ? .blocked : .notBlocked)
                            : nil
                        // The SAME two values feed the size call and the
                        // initializer below. Out of step, the overlay
                        // places one menu and draws another.
                        let menuSize = MessageContextMenu.size(
                            canReply: canReply,
                            canEdit: canEdit,
                            canCopy: canCopy,
                            canReport: canReport,
                            blockState: blockState,
                            page: menuPage)
                        floatingMenu(
                            size: menuSize,
                            over: rect,
                            in: proxy,
                            atCenterY: menuCenterY(
                                menu: menuSize,
                                capsule: capsuleSize,
                                over: rect,
                                in: proxy)
                        ) {
                            MessageContextMenu(
                                onReply: {
                                    dismissReactionPicker()
                                    beginReply(
                                        serverID: message.serverID,
                                        senderID: message.senderID,
                                        body: message.body)
                                },
                                onEdit: {
                                    dismissReactionPicker()
                                    beginEdit(serverID: message.serverID, body: message.body)
                                },
                                onCopy: {
                                    UIPasteboard.general.string = message.body
                                    dismissReactionPicker()
                                },
                                onShare: {
                                    dismissReactionPicker()
                                    if let attachment {
                                        // The file, plus the caption when
                                        // there is one — sharing a photo's
                                        // empty body would share nothing.
                                        shareAttachment(attachment, caption: message.body)
                                    } else {
                                        shareText = ShareText(text: message.body)
                                    }
                                },
                                canReply: canReply,
                                canEdit: canEdit,
                                canCopy: canCopy,
                                canReport: canReport,
                                blockState: blockState,
                                onReport: {
                                    dismissReactionPicker()
                                    reportTarget = ReportTarget(
                                        senderID: message.senderID,
                                        senderName: displayName(for: message.senderID)
                                            ?? String(localized: "Someone"),
                                        messageID: message.serverID)
                                },
                                // Through the coordinator's wrappers, never
                                // `coordinator.api`: they own the
                                // request-then-write ordering and the
                                // direct-chat prune.
                                onBlock: {
                                    dismissReactionPicker()
                                    let userID = message.senderID
                                    Task { await coordinator.block(userID: userID) }
                                },
                                onUnblock: {
                                    dismissReactionPicker()
                                    let userID = message.senderID
                                    Task { await coordinator.unblock(userID: userID) }
                                },
                                page: menuPage,
                                onPage: { menuPage = $0 })
                        }
                    }
                }
            }
        }
    }

    /// Places one floating menu of a known size against the pressed
    /// bubble's rect: above it (below when the bubble hugs the top
    /// edge), centered on it but clamped inside the horizontal bounds,
    /// and springing out of the nearest point of the bubble.
    /// `atCenterY` overrides the default placement for a panel that has
    /// to be positioned relative to another one (see menuCenterY).
    private func floatingMenu<Content: View>(
        size: CGSize,
        over rect: CGRect,
        in proxy: GeometryProxy,
        atCenterY explicitY: CGFloat? = nil,
        @ViewBuilder content: () -> Content
    ) -> some View {
        let margin: CGFloat = 8
        let placeBelow = explicitY != nil
            ? explicitY! > rect.midY
            : rect.minY - size.height - margin < proxy.safeAreaInsets.top + margin
        let x = min(max(rect.midX, size.width / 2 + margin), proxy.size.width - size.width / 2 - margin)
        let y = explicitY ?? panelCenterY(size: size, over: rect, in: proxy)
        // The unit point of the menu nearest the bubble: the scale grows
        // from there, so the menu sprouts out of the bubble even after
        // the horizontal clamp shifted it.
        let anchorX = min(max((rect.midX - (x - size.width / 2)) / size.width, 0), 1)
        return content()
            .transition(
                .scale(scale: 0.1, anchor: UnitPoint(x: anchorX, y: placeBelow ? 0 : 1))
                    .combined(with: .opacity))
            .position(x: x, y: y)
    }

    /// Vertical centre of a panel placed against the bubble: above it,
    /// below it when the bubble hugs the top edge, and never past the
    /// bottom of the viewport.
    private func panelCenterY(size: CGSize, over rect: CGRect, in proxy: GeometryProxy) -> CGFloat {
        let margin: CGFloat = 8
        let placeBelow = rect.minY - size.height - margin < proxy.safeAreaInsets.top + margin
        return placeBelow
            ? min(rect.maxY + margin + size.height / 2, proxy.size.height - size.height / 2 - margin)
            : rect.minY - margin - size.height / 2
    }

    /// Vertical centre of the action menu: under the bubble, but never
    /// overlapping the capsule. A bubble taller than the viewport pins
    /// BOTH panels against the same edge — the menu is drawn second, so
    /// without this it would simply cover the capsule and take the
    /// reactions with it. Stack under the capsule when there is room,
    /// otherwise above it.
    private func menuCenterY(
        menu: CGSize,
        capsule: CGSize,
        over rect: CGRect,
        in proxy: GeometryProxy
    ) -> CGFloat {
        let margin: CGFloat = 8
        let gap: CGFloat = 8
        let capsuleCenter = panelCenterY(size: capsule, over: rect, in: proxy)
        let capsuleTop = capsuleCenter - capsule.height / 2
        let capsuleBottom = capsuleCenter + capsule.height / 2
        let preferred = min(
            rect.maxY + margin + menu.height / 2,
            proxy.size.height - menu.height / 2 - margin)
        let overlaps = preferred - menu.height / 2 < capsuleBottom + gap
            && preferred + menu.height / 2 > capsuleTop - gap
        guard overlaps else { return preferred }
        if capsuleBottom + gap + menu.height <= proxy.size.height - margin {
            return capsuleBottom + gap + menu.height / 2
        }
        return max(margin + menu.height / 2, capsuleTop - gap - menu.height / 2)
    }

    /// The capsule's items: the quick set, plus the user's current
    /// reaction appended when it is not a quick one — so it is always
    /// visible and can be tapped off.
    private func capsuleEmojis(mine: String?) -> [String] {
        var emojis = MessagePresentation.quickReactions
        if let mine, !emojis.contains(mine) {
            emojis.append(mine)
        }
        return emojis
    }

    /// Apply an edit. The field is cleared only once the server has taken
    /// it: a refused edit (too long, no longer the author's, message gone)
    /// leaves the text in the composer to fix rather than dropping it.
    private func submitEdit(target: (messageID: Int64, original: String), body: String) {
        Task { @MainActor in
            let ok = await coordinator.edit(messageServerID: target.messageID, in: chatID, body: body)
            guard ok else { return }
            withAnimation(.spring(duration: 0.25)) {
                editTarget = nil
            }
            model.draft = target.original
            replyStartedFromHistory = false
        }
    }

    private func dismissReactionPicker() {
        withAnimation(.spring(duration: 0.3, bounce: 0.25)) {
            reactionPickerID = nil
        }
        // Every menu opens on its first page. Without this a reader who
        // left it on Safety last time reopens into Safety on an unrelated
        // message, one tap from Block.
        menuPage = .main
    }

    // MARK: - Actions

    private func send() {
        let body = model.draft
        // An attachment can travel with no words at all — that is how a
        // photo is normally sent — so an empty draft is only a reason to
        // stop when there is nothing staged either.
        guard !body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !staged.isEmpty else {
            return
        }
        if let target = editTarget {
            submitEdit(target: target, body: body)
            return
        }
        if !staged.isEmpty {
            sendStaged(staged, caption: body)
            return
        }
        coordinator.send(body: body, in: chatID, replyTo: replyDraft)
        withAnimation(.spring(duration: 0.25)) {
            replyDraft = nil
        }
        // Sending moves the thread to my own new message by the normal
        // path, so the suppression ends here.
        replyStartedFromHistory = false
        // Cleared on the NEXT main-actor turn, not synchronously in the
        // button action: clearing a focused multi-line field in the same
        // turn can leave the sent text sitting in the field — UIKit may
        // re-commit in-flight marked text / autocorrect over a same-turn
        // programmatic clear, and a repeat write of the same "" would not
        // invalidate the view. One deferred write lands after those
        // events and actually empties the field.
        Task { @MainActor in
            model.draft = ""
        }
    }

    /// The poll door, and a send door like any other — so it reads the
    /// quote and clears it together.
    ///
    /// A poll may be a reply (the server takes `reply_to_message_id` beside
    /// `poll`), and leaving the banner armed is the failure `takeComposer`
    /// exists to prevent: the quote was dropped on the floor and the next
    /// ordinary message silently became that reply. The draft is NOT
    /// touched — the question came from the sheet, and anything typed in
    /// the composer is still going somewhere else.
    private func sendPoll(question: String, options: [String]) {
        let quote = replyDraft
        coordinator.sendPoll(question: question, options: options, in: chatID, replyTo: quote)
        withAnimation(.spring(duration: 0.25)) {
            replyDraft = nil
        }
        replyStartedFromHistory = false
    }

    private func toggleReaction(localID: String, emoji: String) {
        Task { await coordinator.toggleReaction(localID: localID, emoji: emoji) }
    }

    private func loadOlder(proxy: ScrollViewProxy) {
        guard model.state == .idle else { return }
        // Local-first: older messages already cached but outside the
        // render window just widen it — instant, no network.
        if visibleCount < messages.count {
            let anchorID = visibleMessages.first?.localID
            // Capped, through the Mac's rule. At the cap this widens by
            // nothing and the restore below must not run either: no row
            // appeared, so putting the oldest rendered row back at the TOP
            // simply throws the reader up there — and the top sentinel,
            // still on screen, asks again.
            let widened = ThreadFollow.windowAfterPagingBack(
                current: visibleCount, cached: messages.count, step: Self.windowStep,
                cap: Self.maxWindow)
            guard widened > visibleCount else { return }
            visibleCount = widened
            if hasSettled, let anchorID {
                Task {
                    try? await Task.sleep(nanoseconds: 50_000_000)
                    proxy.scrollTo(anchorID, anchor: .top)
                }
            } else if openingAnchorPending {
                // Widening a SUFFIX adds rows above the viewport, which
                // slides the visible region into older content — the same
                // movement the restore above exists to undo. During an
                // anchored open there is no reading position to restore
                // yet, and the opening scroll has already run, so nothing
                // else would put the divider back. Re-assert it.
                Task {
                    try? await Task.sleep(nanoseconds: 50_000_000)
                    proxy.scrollTo(UnreadDivider.scrollID, anchor: .top)
                }
            }
            return
        }
        model.state = .loadingOlder
        // The bubble to restore to: the current topmost rendered message.
        let anchorID = visibleMessages.first?.localID
        Task {
            let fetched = await coordinator.loadOlder(chatID: chatID)
            // Restore the reading position ONLY when rows were actually
            // prepended. An empty page must not scroll: the sentinel can
            // fire during the OPENING layout pass (before the bottom
            // anchor lands), and scrolling the old top row "back" to the
            // top would yank a freshly opened chat away from the newest
            // messages.
            if fetched > 0 {
                // Give the @Query one runloop tick to reflect the inserts,
                // and widen the window over the fetched page so it renders
                // — capped, like every other path that widens it.
                try? await Task.sleep(nanoseconds: 50_000_000)
                visibleCount = min(min(messages.count, visibleCount + fetched), Self.maxWindow)
                if hasSettled, let anchorID {
                    // A real scroll-up: put the previous top bubble back
                    // at the top so the reading position holds.
                    proxy.scrollTo(anchorID, anchor: .top)
                } else if openingAnchorPending {
                    // Same repair as the local branch: prepended rows moved
                    // the viewport, and during an anchored open the divider
                    // is what it has to be moved back to.
                    proxy.scrollTo(UnreadDivider.scrollID, anchor: .top)
                } else {
                    // Fired during the opening pass: the user never left
                    // the bottom — keep them there over the grown content.
                    //
                    // PIN HOOK 2 of 4. Gated, because an anchored open
                    // spends its whole opening window away from the bottom
                    // ON PURPOSE, and this branch is exactly the one that
                    // fires then: the top sentinel is much more likely to
                    // be on screen when the thread opens in history, so a
                    // page lands and this used to throw the reader down to
                    // the newest message — the bug the feature exists to
                    // fix, arriving a fraction of a second later.
                    pinToBottom(proxy, animated: false)
                }
            }
            model.state = (chat?.hasFullHistory == true) ? .atStart : .idle
        }
    }

    /// The first page of a chat with nothing cached.
    ///
    /// Awaited by `openThread` rather than fired and forgotten from
    /// `onAppear`, because the opening anchor is arithmetic over the rows
    /// this returns: deciding before it lands means deciding over an empty
    /// cache, which always gives up to `.newest`.
    private func loadInitial() async {
        guard model.state == .idle else { return }
        model.state = .loadingOlder
        _ = await coordinator.loadOlder(chatID: chatID)
        model.state = (chat?.hasFullHistory == true) ? .atStart : .idle
    }

    /// Send where this device is, once.
    ///
    /// One tap rather than a map to drag a pin around: "share my location"
    /// is what people mean, and a picker for anywhere else is a different
    /// feature. The composer shows it working, and a refusal says which
    /// kind of refusal it was — a denied permission and a fix that never
    /// arrived need different things from the reader.
    private func shareLocation() {
        // The gate every other attachment door uses, rather than
        // `== .idle`: a dismissible `.failed` left over from the last try
        // must not swallow the next tap. It did — only the strip's Dismiss
        // button returns this to `.idle` — so a reader told to turn the
        // permission on in Settings, who did exactly that and came back and
        // tapped Location, got silence.
        guard !composerIsBusy else { return }
        Task {
            // Settle permission FIRST, outside everything that says the
            // composer is busy. The prompt is a system alert somebody may
            // take a minute over, or never answer at all, and `.preparing`
            // closes the attach menu and the paste door for as long as it
            // is set. Android has never had this problem: `ChatScreen.kt`
            // asks for the permission and only calls the view model once it
            // is held, so its composer stays live under the dialog. This is
            // that shape — and it needs no timeout, because nothing here is
            // waiting on a machine.
            switch await locationProvider.requestPermission() {
            case .allowed:
                break
            case .denied:
                mediaState = .failed(String(
                    localized:
                        "Family needs permission to use your location. Turn it on in Settings."))
                return
            case .unanswered:
                // The alert went away unanswered — backgrounding tears it
                // down, and Location Services switched off system-wide may
                // mean it never appeared. Nothing was taken from the
                // composer and nothing is in flight, so there is nothing to
                // report and nothing to restore: Location is still in the
                // menu, still enabled, and tapping it asks again. A line of
                // copy here would be nagging somebody for an answer they
                // deliberately withheld — and any copy that said "try
                // again" would have to be a lie the guard above refused.
                return
            }
            // That wait was a real suspension. Whatever the composer was
            // doing when the alert went up may have finished or started in
            // the meantime, so the door is checked again on the way in.
            guard !composerIsBusy else { return }
            mediaState = .preparing
            do {
                let fix = try await locationProvider.currentFix()
                mediaState = .uploading(nil)
                // Take-then-restore, like every other send here: whatever
                // was typed travels with the pin, and comes back if the
                // send never happened.
                let composer = takeComposer()
                let sent = await coordinator.sendLocation(
                    latitude: fix.latitude,
                    longitude: fix.longitude,
                    accuracyM: fix.accuracyM,
                    label: nil,
                    caption: composer.caption,
                    replyTo: composer.replyTo,
                    in: chatID)
                if sent {
                    mediaState = .idle
                } else {
                    restore(composer)
                    mediaState = .failed(String(localized: "Could not share your location."))
                }
            } catch LocationProvider.Failure.denied {
                // Not the prompt any more — that was settled above. This is
                // the switch being thrown in Settings, or an "Allow Once"
                // lapsing, while the hunt was running.
                mediaState = .failed(String(
                    localized:
                        "Family needs permission to use your location. Turn it on in Settings."))
            } catch {
                mediaState = .failed(String(localized: "Could not find your location."))
            }
        }
    }

    /// Offer the mention only where it does something.
    ///
    /// The family chat is the only place `@ai` is answered (a direct chat
    /// is two people who each already have a private assistant, and the
    /// assistant's own chat answers everything), and only when the server
    /// actually has one — which is exactly what an absent `assistant` on
    /// `GET /families/mine` says (docs/protocol.md).
    ///
    /// A button rather than an autocomplete popup: there is precisely one
    /// name to complete, and a menu of one is a worse way to type three
    /// characters than a button that types them.
    private var showsAssistantMention: Bool {
        chat?.kind == "family" && AppSettings.assistantUserID != nil
    }

    /// Both locks, and only in the assistant's own chat.
    ///
    /// The operator configured a deployment that can SEE
    /// (`assistant.vision`), and this family's owner turned `ai_vision` on.
    /// Neither is consent for a particular photograph — that is the third
    /// thing, and it is the member attaching it to this one question, which
    /// is deliberately not a setting anywhere (protocol.md, "Pictures").
    ///
    /// `@ai` in the FAMILY chat is not here, and that is a statement about
    /// this DOOR rather than about what travels: since #56 a photo on an
    /// `@ai` message, or on the message it replies to, goes to the model
    /// under the same two locks, through the ordinary photo picker and the
    /// reply affordance the family composer has always had (protocol.md,
    /// "Showing the assistant a picture from the family chat"). What still
    /// never goes is a photo the member did not point the assistant at —
    /// somebody else's picture elsewhere in the window stays `[photo]`.
    private var showsPictureAttach: Bool {
        AssistantSurfaces.offersPictureAttach(
            isAssistantChat: isAssistantChat,
            serverCanSee: AppSettings.assistantVision,
            familyAllows: session.family?.aiVision == true)
    }

    /// The `/draw` affordance: this server can generate a picture, and we
    /// are somewhere it would be answered.
    ///
    /// No family switch, deliberately — what leaves on such a request is
    /// the words after the token and nothing else, which is a SMALLER
    /// disclosure than an ordinary text question.
    private var showsPictureRequest: Bool {
        isAssistantChat && AppSettings.offersPictureRequests
    }

    /// What the composer has to say out loud before a photograph goes.
    ///
    /// The switch lives on a settings screen somebody read once; the
    /// photograph is chosen here, later, possibly by somebody else. So the
    /// sentence is said HERE, at the moment it matters, and it names the
    /// two things a reader can act on: that the picture leaves for whatever
    /// model this server talks to, and anything of theirs that will NOT go.
    private var pictureNotice: String? {
        guard isAssistantChat else { return nil }
        // EVERY staged photograph, before any bound is applied — the
        // sentence is about what the member is looking at, so what they are
        // looking at is what it counts.
        let photos = staged.filter { $0.prepared.kind == AttachmentDTO.Kind.photo }
        guard !photos.isEmpty else { return nil }
        guard showsPictureAttach else {
            // Not an error and not refused — the server takes it, stores
            // it, and tells the assistant a photo was attached that it was
            // not shown. Saying so is the whole of the honesty here.
            return String(localized: "The assistant on this server can't look at pictures, so it will be told a photo is here but won't be shown it.")
        }
        // The server's own rules, applied to what will actually be ON THE
        // WIRE: JPEG or PNG, and 5 MiB at most, judged on the PREVIEW where
        // there is one because that is the copy the server prefers
        // (protocol.md, "Pictures"). Without this the notice below promised
        // that a photograph the server will leave out "goes to the model".
        let carried = AssistantPictureLimits.carried(
            photos,
            kind: { $0.prepared.kind },
            mime: { $0.assistantWireMIME },
            bytes: { $0.assistantWireBytes })
        if carried.count < photos.count {
            // Ahead of the cap below, on the rare send that trips both: a
            // member who staged five photos half expects a limit, and
            // nobody expects a photograph to be left out for its bytes.
            return String(localized: "A photo here is too large, or in a format the model can't read, so it will be told it's here but won't be shown it.")
        }
        if photos.count > AssistantPictureLimits.maxPerQuestion {
            // Named rather than silently dropped, the same way the server
            // names them to the model.
            return String(localized: "The first \(AssistantPictureLimits.maxPerQuestion) photos go to the model your server is set up to use. The rest are named to it, not shown.")
        }
        return String(localized: "This goes to the model your server is set up to use, with your question. Nothing else from this chat does.")
    }

    /// The FAMILY composer's own sentence, for the case that did not exist
    /// before #56: an `@ai` draft with a photo staged on it, or replying to
    /// a message that carries one, is about to send that photo to the model
    /// under the same two locks a private question needs. The doctrine is
    /// the one above — say it here, now, where the photo is — and the rule
    /// is `MentionPictureNotice`, shared with the Mac and pinned by tests:
    /// absent with either lock shut, absent without a mention, absent for
    /// `@ai /draw`, and counting the four exactly as the server does.
    ///
    /// With the owner's third switch on (`ai_history_photos`, and
    /// `ai_history` with it) the same rule also says that the chat's most
    /// recent photos may go — "up to N", N being what the draft and the
    /// quote left of the four — and it then shows for an `@ai` draft with
    /// no photo of its own at all, since that is the mention on which every
    /// one of the four may be somebody else's picture (protocol.md, "Recent
    /// photos from the family chat").
    private var mentionPictureNotice: String? {
        guard isFamilyChat else { return nil }
        return MentionPictureNotice.of(
            draft: model.draft,
            staged: staged.map(\.assistantPictureCandidate),
            quoted: quotedAttachments.map(AssistantPictureCandidate.init(attachment:)),
            serverCanSee: AppSettings.assistantVision,
            familyAllows: session.family?.aiVision == true,
            familyHistory: session.family?.aiHistory == true,
            familyHistoryPhotos: session.family?.aiHistoryPhotos == true,
            serverCanDraw: AppSettings.offersPictureRequests
        )?.sentence
    }

    /// The attachments of the message a reply is primed on — from this
    /// chat's own rows, because `ReplyToDTO` carries an excerpt and nothing
    /// else. [] when nothing is primed, or the row is not held here.
    private func quotedAttachmentList(for serverID: Int64?) -> [AttachmentDTO] {
        guard let serverID else { return [] }
        return messages.first { $0.serverID == serverID }?.attachmentList ?? []
    }

    /// The notice itself. Not red and not an error: it is what is about to
    /// happen, said plainly.
    private func assistantPictureNotice(_ text: String) -> some View {
        HStack(alignment: .top, spacing: 6) {
            Image(systemName: "eye")
                .font(.caption)
                .foregroundStyle(.secondary)
            Text(text)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.top, 6)
        .accessibilityElement(children: .combine)
    }

    /// Put `/draw ` at the FRONT of the draft and give the field back.
    ///
    /// At the front rather than appended, which is the one place this
    /// differs from the mention button: the token is only a request when it
    /// is the first thing in the body, so appending it would type something
    /// the server will not act on (protocol.md, "Pictures").
    private func insertDrawToken() {
        let token = AssistantMention.drawToken
        guard !AssistantMention.asksForPicture(model.draft) else {
            inputFocused = true
            return
        }
        let rest = model.draft.trimmingCharacters(in: .whitespacesAndNewlines)
        model.draft = rest.isEmpty ? "\(token) " : "\(token) \(rest)"
        inputFocused = true
    }

    /// Put `@ai ` in the draft and give the field back to the keyboard.
    ///
    /// Appended rather than inserted at the caret: SwiftUI's TextField
    /// publishes no selection, so "at the caret" is not knowable here, and
    /// silently moving somebody's cursor would be worse than adding to the
    /// end of what they were writing.
    private func insertAssistantMention() {
        let token = AssistantMention.token
        var draft = model.draft
        guard !AssistantMention.mentions(draft) else {
            inputFocused = true
            return
        }
        if draft.isEmpty {
            draft = "\(token) "
        } else if draft.hasSuffix(" ") {
            draft += "\(token) "
        } else {
            draft += " \(token) "
        }
        model.draft = draft
        inputFocused = true
    }

    private func displayName(for userID: Int64) -> String? {
        // The assistant's account is not in the roster on purpose, so a
        // lookup would come back "Someone". In its own chat there are
        // exactly two participants, and anyone who is not me is it.
        if isAssistantChat, userID != coordinator.currentUserID {
            return chat?.title
        }
        // In the FAMILY chat there is no such shortcut — many senders, and
        // the assistant answers there whenever somebody mentions it. Its id
        // comes from `GET /families/mine`, which is the only place a client
        // is told it (docs/protocol.md).
        if let assistantID = AppSettings.assistantUserID, userID == assistantID {
            return AppSettings.assistantName
        }
        // resolvedDisplayName, not displayName: a deleted account's
        // stored name is the server's English placeholder, and this is
        // where the translated one is drawn instead.
        return members.first(where: { $0.userID == userID })?.resolvedDisplayName
    }

    /// 0 when the sender has no picture — or has left the family, so the
    /// roster no longer holds them.
    private func avatarVersion(for userID: Int64) -> Int64 {
        members.first(where: { $0.userID == userID })?.avatarVersion ?? 0
    }

    /// userID → picture version for every known member, feeding the
    /// faces in the "who reacted" rows.
    private var avatarVersions: [Int64: Int64] {
        Dictionary(members.map { ($0.userID, $0.avatarVersion) }, uniquingKeysWith: { first, _ in first })
    }

    /// How many people a poll could hear from: the live roster, which is
    /// neither the people who have left nor the accounts that were
    /// deleted — a tally must not go on counting somebody who no longer
    /// exists (docs/protocol.md, "Deleting an account").
    private var familyMemberCount: Int {
        members.filter { !$0.hasLeft && !$0.accountDeleted }.count
    }

    /// userID → display name for every known member, feeding the "who
    /// reacted" rows. Tolerates duplicate rows (first wins) rather than
    /// trusting the store never to produce one.
    private var memberNames: [Int64: String] {
        Dictionary(members.map { ($0.userID, $0.resolvedDisplayName) }, uniquingKeysWith: { first, _ in first })
    }
}

/// The "Today / Yesterday / Mon, Aug 17" capsule between day sections.
private struct DayPill: View {
    let day: Date

    var body: some View {
        Text(label)
            .font(.caption2.weight(.medium))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 10)
            .padding(.vertical, 4)
            .background(Color(.secondarySystemFill), in: Capsule())
    }

    private var label: String {
        let calendar = Calendar.current
        if calendar.isDateInToday(day) { return String(localized: "Today") }
        if calendar.isDateInYesterday(day) { return String(localized: "Yesterday") }
        return day.formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day())
    }
}

/// The pickers and viewers attachments need, as one modifier.
///
/// Split out of ConversationView's body for a mundane reason: applied
/// inline, the modifier chain there grew past what the Swift type checker
/// will solve in reasonable time. Grouping them also keeps the four
/// bindings that only attachments touch in one place.
private struct AttachmentSurfaces: ViewModifier {
    @Binding var pickedMedia: [PhotosPickerItem]
    @Binding var showFilePicker: Bool
    @Binding var previewedFile: URL?
    @Binding var viewingAlbum: AttachmentAlbum?
    @Binding var showCamera: Bool

    let onPickedMedia: ([PhotosPickerItem]) -> Void
    let onPickedFiles: ([URL]) -> Void
    let onCapturedPhoto: (Data) -> Void
    let onCapturedVideo: (URL) -> Void
    let onImportFailed: () -> Void
    let onShareAttachment: (AttachmentDTO) -> Void

    func body(content: Content) -> some View {
        content
            .onChange(of: pickedMedia) { _, items in
                guard !items.isEmpty else { return }
                onPickedMedia(items)
            }
            .fileImporter(
                isPresented: $showFilePicker,
                allowedContentTypes: [.item],
                allowsMultipleSelection: true
            ) { result in
                // .item, not a list of types: the whole point of files is
                // that the family is never told what they may send.
                switch result {
                case .success(let urls):
                    guard !urls.isEmpty else { return }
                    onPickedFiles(urls)
                case .failure:
                    onImportFailed()
                }
            }
            .fullScreenCover(isPresented: $showCamera) {
                CameraPicker(onPhoto: onCapturedPhoto, onVideo: onCapturedVideo)
                    .ignoresSafeArea()
            }
            .quickLookPreview($previewedFile)
            .fullScreenCover(item: $viewingAlbum) { album in
                AttachmentViewer(album: album, onShare: onShareAttachment)
            }
    }
}

#endif
