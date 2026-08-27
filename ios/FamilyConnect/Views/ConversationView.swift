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
    /// The message being rewritten, while the composer is in edit mode.
    /// Mutually exclusive with replyDraft: you are either answering a
    /// message or rewriting one.
    @State private var editTarget: (messageID: Int64, original: String)?
    /// Briefly tinted after a jump, so the eye lands on the right bubble.
    @State private var highlightedMessageID: String?
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
    /// The attachment being viewed full-screen.
    @State private var viewingAttachment: AttachmentDTO?
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
            .overlay(alignment: .bottomTrailing) {
                ZStack {
                    if showsJumpToNewest {
                        JumpToNewestButton { pinToBottom(proxy, animated: true) }
                            .transition(.scale.combined(with: .opacity))
                    }
                }
                .animation(.spring(duration: 0.25), value: showsJumpToNewest)
            }
            .task {
                // Un-park a draft an earlier identity of this chat's view
                // stashed on its way out — before the opening routine, so
                // the composer is whole by the time the thread lands.
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
                // PIN HOOK 1 of 3. The `!hasSettled` clause is the one that
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
            .onChange(of: inputFocused) {
                // The keyboard rising is an inset change, and the system's
                // own avoidance is best-effort — deterministically pin the
                // newest message above it (twice: as the animation starts
                // and after it lands; standard messenger behavior).
                //
                // PIN HOOK 3 of 3, and deliberately NOT gated on the
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
        .onChange(of: hasSettled) {
            // The opening convergence finished: whatever the sentinel says
            // now is real, and if it says the newest message is on screen
            // then the reader has genuinely seen it.
            publishPresence()
        }
        .onChange(of: scenePhase) {
            // Backgrounding revokes the authority to read (the coordinator
            // does that centrally, because onDisappear does NOT fire here);
            // coming back re-establishes it from the same geometry, without
            // the act of returning reading anything by itself.
            publishPresence()
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
                                isStreaming: message.serverID.map {
                                    coordinator.streamingMessageIDs.contains($0)
                                } ?? false,
                                showsSenderName: MessagePresentation.showsSenderName(
                                    at: index,
                                    in: section.messages,
                                    isFamilyChat: isFamilyChat,
                                    currentUserID: currentUserID),
                                senderName: displayName(for: message.senderID),
                                senderID: message.senderID,
                                senderAvatarVersion: avatarVersion(for: message.senderID),
                                isRead: MessagePresentation.isRead(
                                    message,
                                    othersReadUpTo: chat?.othersReadUpTo ?? 0),
                                reactionChips: MessagePresentation.reactionChips(
                                    message.reactions,
                                    currentUserID: currentUserID),
                                reactionDetails: MessagePresentation.reactionDetails(
                                    message.reactions,
                                    names: memberNames,
                                    currentUserID: currentUserID),
                                avatarVersions: avatarVersions,
                                memberNames: memberNames,
                                currentUserID: currentUserID,
                                onTapQuote: { jumpToMessage($0, proxy: proxy) },
                                onOpenAttachment: { attachment in
                                    if attachment.isFile {
                                        openFile(attachment)
                                    } else {
                                        viewingAttachment = attachment
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
                                    withAnimation(.spring(duration: 0.3, bounce: 0.25)) {
                                        reactionPickerID = message.localID
                                    }
                                },
                                publishesAnchor: reactionPickerID == message.localID)
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
            HStack(alignment: .bottom, spacing: 8) {
                // A Menu rather than two buttons: the composer is narrow,
                // and "attach" is one intent with two sources.
                Menu {
                    Button {
                        showPhotoPicker = true
                    } label: {
                        Label("Photo or Video", systemImage: "photo.on.rectangle")
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
                    if CameraPicker.isAvailable {
                        Button {
                            showCamera = true
                        } label: {
                            Label("Camera", systemImage: "camera")
                        }
                    }
                    Button {
                        Task { await recorder.start() }
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
                    matching: .any(of: [.images, .videos]),
                    photoLibrary: .shared())
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
            viewingAttachment: $viewingAttachment,
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
                mediaState = .idle
            } else {
                mediaState = .failed(String(localized: "Couldn't send that — try again."))
                restore(handoff)
                // Put the attachments back too — ALL of them: `sendMedia`
                // consumes the temp files only on whole-send success, so
                // after a failure every file is still on disk and the
                // retry offers exactly the set that was composed. Ids
                // already uploaded but never claimed are the server's 24h
                // sweep's problem, not ours.
                withAnimation(.spring(duration: 0.25)) {
                    staged = items + staged
                }
            }
        }
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
                        let menuSize = MessageContextMenu.size(
                            canReply: canReply,
                            canEdit: canEdit,
                            canCopy: canCopy)
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
                                canCopy: canCopy)
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
                    // PIN HOOK 2 of 3. Gated, because an anchored open
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
        guard mediaState == .idle else { return }
        mediaState = .preparing
        Task {
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
    @Binding var viewingAttachment: AttachmentDTO?
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
            .fullScreenCover(item: $viewingAttachment) { attachment in
                AttachmentViewer(
                    attachment: attachment,
                    onShare: { onShareAttachment(attachment) })
            }
    }
}

#endif
