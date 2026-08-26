//
//  MacConversationView.swift
//  FamilyConnect
//
//  One chat's thread on the Mac.
//
//  This used to say the Mac needed none of the phone's scroll-anchoring
//  apparatus, because a Mac window does not resize under a keyboard. The
//  premise was true and the conclusion was wrong, in two ways that showed
//  up as two different bugs:
//
//  1. A Mac window DOES resize — every time the sidebar is toggled, by the
//     240–380pt of the sidebar's width. A ScrollView keeps a point offset
//     across that, not a message, so the thread came back at the wrong
//     place.
//  2. An UNBOUNDED LazyVStack inside a ScrollView is not lazy where it
//     matters. Asking the scroll view for its height walks the whole
//     nested ForEach to estimate every row, and SwiftUI's own
//     `LazyLayoutViewCache.signalPrefetch` schedules another transaction
//     from inside that pass — so with a long thread the update never
//     converges. Two captured hang reports show 36 of 38 main-thread
//     samples inside `LazyStack.measureEstimates`, with an animation
//     demanding a fresh layout on every display frame. That is the
//     beachball, and it needed a Force Quit.
//
//  So the Mac now renders the same BOUNDED, NON-LAZY window the phone
//  does — real heights, an exact bottom anchor, every id a valid scroll
//  target — grown by a top sentinel, locally first and then over the
//  network. It keeps a bottom sentinel too, which is both the truth about
//  whether the reader is at the bottom and the target to put them back
//  there after a resize.
//
//  Sending: Return sends, Shift-Return makes a new line, which is what
//  every Mac chat app does and what a hardware keyboard makes possible.
//
//  Attachments render through the SAME AttachmentStore the phone uses;
//  only the picker differs (an open panel rather than PhotosPicker).
//

#if os(macOS)

import AppKit
import SwiftData
import SwiftUI

struct MacConversationView: View {
    let chatID: Int64

    @Environment(ChatSyncCoordinator.self) private var coordinator
    /// Watched only for `generation`: a card landing grows a bubble, and a
    /// bubble growing above the viewport moves everything below it.
    @Environment(LinkPreviewLoader.self) private var previewLoader
    /// `.key` when this window is the frontmost one. With several
    /// conversation windows open, this is what decides which chat is the
    /// one in front of the user — one of the three facts that make a chat
    /// READ (ChatPresence), and on its own not enough to read anything.
    @Environment(\.controlActiveState) private var windowActivation
    @Query private var messages: [MessageEntity]
    @Query private var chats: [ChatEntity]
    @Query private var members: [MemberEntity]

    @State private var draft = ""
    @State private var isSending = false
    @State private var mediaNotice: String?
    /// Prepared and waiting for Send. The Mac used to have no staging step
    /// at all — every door sent the moment it had bytes — and paste forced
    /// the question, because a paste that sent by itself would be a trap.
    /// Now every attachment door lands here first, which is also what lets
    /// a caption be written while looking at the thing.
    @State private var staged: StagedAttachment?
    /// The message being answered, while the composer is primed.
    @State private var replyDraft: ReplyToDTO?
    /// Live height of the composer, watched for the same reason the phone
    /// watches its input bar: the thread and the composer are siblings in
    /// one VStack, so anything that GROWS the composer — a reply banner, an
    /// edit banner, the recording row, a draft wrapping to another line —
    /// shrinks the thread's viewport under content that does not move, and
    /// the newest message slides away behind it.
    @State private var composerHeight: CGFloat = 0
    /// How the composer was primed, and what the reader could see the
    /// moment it happened — nil when nothing is primed.
    ///
    /// Captured in `beginReply` and `beginEdit`, BEFORE either banner
    /// exists, and that is the whole trick: growing the composer takes the
    /// bottom sentinel out of the viewport, so by the time the height
    /// change lands `isPinnedToBottom` has already flipped to false and can
    /// no longer answer "were they at the bottom a moment ago?". Without
    /// it, priming the composer from an old message throws the reader to
    /// the newest one — away from the very message they are answering or
    /// rewriting. Same reason as the phone's flag.
    ///
    /// A VALUE rather than a Bool because the Bool was set at only one of
    /// the two doors: `beginReply` captured it and `beginEdit` did not, so
    /// editing a message from last week yanked the thread to the bottom
    /// while the author was typing into it. Constructing this is the only
    /// way to prime the composer, and it takes the answer as an argument,
    /// so a third kind of banner cannot forget it either.
    @State private var composerPriming: ComposerPriming?
    /// The message being rewritten; mutually exclusive with a reply, the
    /// same rule the phone has — you are answering or rewriting, not both.
    @State private var editTarget: (messageID: Int64, original: String)?
    @Environment(\.openWindow) private var openWindow
    @FocusState private var composerFocused: Bool
    @State private var recorder = AudioRecorder()
    /// True while the poll form is up. A sheet, sized in the view itself:
    /// a macOS sheet cannot be resized by the person using it.
    @State private var showPollComposer = false
    /// One fix, on demand — never a running location service.
    @State private var locationProvider = LocationProvider()

    /// Side of the attach and send buttons — one box for both, so they sit
    /// level. Matches the composer's one-line height.
    private let composerControl: CGFloat = 24

    /// How many of the locally cached messages are RENDERED.
    ///
    /// The whole cure for the hang. See the header: an unbounded lazy stack
    /// estimates every row's height on every layout negotiation and never
    /// settles. A bounded window in a plain VStack has real heights, costs
    /// a page of rows to lay out, and makes every id a scroll target.
    @State private var visibleCount = MacConversationView.windowStep
    /// True while the newest message is on screen. Ground truth from the
    /// bottom sentinel's geometry, not from a guess about scroll offset.
    @State private var isPinnedToBottom = true
    /// The thread's width, watched because the SIDEBAR TOGGLE changes it.
    @State private var threadWidth: CGFloat = 0
    /// Set while an older page is in flight, so the sentinel does not ask
    /// again and the reader sees that something is happening.
    @State private var isLoadingOlder = false
    /// This member pressed Send, and the row it produces has not landed yet.
    ///
    /// The one hook that follows an own send used to INFER this, from
    /// `messages.last?.senderID == coordinator.currentUserID`, and that
    /// answers a different question: "is the newest row mine", not "did I
    /// just send one". The two come apart in this app, in three ways that
    /// all end with Send being pressed and nothing moving:
    ///
    /// - The optimistic row is stamped with THIS Mac's clock
    ///   (`ChatSyncCoordinator.enqueue`) while every acked row carries the
    ///   SERVER's (`upsert`), and the thread sorts on that key — so a
    ///   message somebody else sent a moment earlier can legitimately sort
    ///   after mine, on nothing worse than the two clocks disagreeing.
    /// - In the family chat the assistant answers a mention by streaming a
    ///   row of its own, which then IS the newest one.
    /// - Media and location travel through an upload first, so the row
    ///   appears seconds after the click, by which time anything may have
    ///   arrived.
    ///
    /// Declared at the door instead, where the answer is not in doubt, and
    /// spent by the arrival it was declared for. It is deliberately NOT
    /// `isPinnedToBottom`: that is an observation about the viewport, and
    /// every send spoils it before this can be read (see `ComposerPriming`
    /// for the same trap on the composer's side).
    @State private var owesSendPin = false

    /// Rows per window, and per widening step. The phone's number, for the
    /// same reasons — big enough that scrolling rarely reaches the top
    /// sentinel, small enough that laying all of it out is cheap.
    private static let windowStep = 60
    /// Most rows rendered at once. The stack is NON-lazy, so this is a real
    /// bound on main-thread layout work rather than a hint — which is the
    /// whole reason the window exists. Roughly five pages: far more history
    /// than a reader scrolls back through in one sitting, and still cheap
    /// enough to lay out. The phone caps its own jump window the same way.
    private static let maxWindow = 300
    /// The bottom sentinel's scroll id. A constant rather than the last
    /// message's, because aiming a pin at the last BUBBLE parks the row
    /// just off-screen and then reads as unpinned.
    private static let bottomAnchor = "mac-thread-bottom"


    init(chatID: Int64) {
        self.chatID = chatID
        _messages = Query(
            filter: #Predicate<MessageEntity> { $0.chatID == chatID },
            sort: [SortDescriptor(\MessageEntity.createdAt), SortDescriptor(\MessageEntity.localID)])
        _chats = Query(filter: #Predicate<ChatEntity> { $0.chatID == chatID })
    }

    private var chat: ChatEntity? { chats.first }

    /// Is either banner up — a reply being answered or a message being
    /// rewritten? What the priming above lives and dies with.
    private var composerIsPrimed: Bool { replyDraft != nil || editTarget != nil }

    private var avatarVersions: [Int64: Int64] {
        Dictionary(members.map { ($0.userID, $0.avatarVersion) }, uniquingKeysWith: { first, _ in first })
    }

    /// uniquingKeysWith, like `avatarVersions` right above it and like the
    /// phone's twin: `uniqueKeysWithValues` TRAPS on a duplicate key, so a
    /// roster that somehow held two rows for one person would crash the
    /// window rather than name a message twice. This map is read for every
    /// quote author and, now, for every face beside a poll option.
    private var memberNames: [Int64: String] {
        Dictionary(
            members.map { ($0.userID, $0.resolvedDisplayName) },
            uniquingKeysWith: { first, _ in first })
    }

    /// How many people a poll could hear from: the live roster, which is
    /// neither the people who have left nor the accounts that were deleted
    /// — a tally must not go on counting somebody who no longer exists.
    private var familyMemberCount: Int {
        members.filter { !$0.hasLeft && !$0.accountDeleted }.count
    }

    var body: some View {
        VStack(spacing: 0) {
            thread
            Divider()
            composer
        }
        // Narrower than this and a balloon has nowhere to go; the split
        // view honours it too, so the sidebar cannot squeeze the thread.
        .frame(minWidth: 420, minHeight: 320)
        // ⌘V, and deliberately NOT a ⌘V keyboard shortcut on a button: a
        // shortcut would outrank the field editor and steal every ordinary
        // text paste. This reaches the view only when the responder chain
        // gets past the composer's field — so words typed into the field
        // still paste as words, and ⌘V anywhere else in the window attaches.
        // The attach menu's Paste is the door that always works.
        .onPasteCommand(of: ClipboardAttachment.pasteCommandTypes) { _ in
            // The same guard the attach menu carries. Any door outside the
            // menu has to repeat it.
            guard editTarget == nil, !isSending else { return }
            pasteAttachment()
        }
        .sheet(isPresented: $showPollComposer) {
            PollComposerView(
                onCreate: { question, options in
                    showPollComposer = false
                    sendPoll(question: question, options: options)
                },
                onCancel: { showPollComposer = false })
        }
        .navigationTitle(chat?.title ?? "")
        .navigationSubtitle(typingLine ?? "")
        // Through ChatPresenceOpening rather than inline, and that is the
        // whole point of it: this task is cancelled the moment the window
        // closes or the sidebar selection changes, and a cancelled sleep
        // written `try?` would run the line after it in a view that is
        // already gone — re-claiming the chat it used to show, at the
        // optimistic `isPinnedToBottom` it started with, with nothing left
        // to release it. See the note there; it ends in a permanently and
        // silently read conversation.
        .task(id: chatID) {
            await ChatPresenceOpening.run(
                // Claim the chat, and claim NOTHING about what is visible:
                // `isPinnedToBottom` starts optimistically true so the first
                // pin has a target, and believing that guess here would read
                // the family chat on every cold start — before a single row
                // had been laid out. MacChatView auto-selects the family chat
                // when nothing is selected, so this ran on every launch.
                claim: { publishPresence(isAtNewest: false) },
                // One page is enough to open with; the phone's paging sentinel
                // is a scroll-position mechanism this window does not need yet.
                loadOlder: {
                    if messages.isEmpty { _ = await coordinator.loadOlder(chatID: chatID) }
                },
                // The thread opens at the bottom by construction, but "by
                // construction" is not "on screen". The wait defers the
                // question, it does not answer it, and the answer is still
                // geometry plus a key window.
                settled: { publishPresence(isAtNewest: isPinnedToBottom) })
        }
        // Presence is ONE value and a Mac can have several conversation
        // windows, so ownership has to be explicit: the frontmost window
        // claims it, and a window only ever releases a claim it still
        // holds. Without that guard, closing one window clears the claim of
        // another that is still on screen — and every message arriving in
        // it starts bumping the unread badge of a chat the user is looking
        // at.
        //
        // Becoming key no longer marks anything read by itself. It used to,
        // and that was the complaint in one sentence: a Mac keeps its
        // socket open in the background, so messages arrive live and light
        // the Dock badge — and clicking the Dock icon to see what arrived
        // destroyed the count in the same gesture. Now focus is only one of
        // the three facts, and the newest message still has to be on screen.
        .onChange(of: windowActivation) { _, _ in
            publishPresence(isAtNewest: isPinnedToBottom)
        }
        .onDisappear { coordinator.releasePresence(chatID: chatID) }
        // The suppression ends with the banner it was captured for, and it
        // is cleared HERE rather than beside each of the seven places a
        // reply draft or an edit is dropped (cancel, send, the swap between
        // the two, three handoffs) — an eighth would silently leave the
        // thread refusing to follow new messages for the rest of the
        // session. Keyed on "either banner is up" and not on the reply
        // alone: keyed on the reply, starting an edit FROM a primed reply
        // cleared the priming the edit had just captured.
        .onChange(of: composerIsPrimed) { _, isPrimed in
            if !isPrimed { composerPriming = nil }
        }
    }

    /// Day sections and sender runs, through the SAME rules the phone
    /// uses (MessagePresentation) — a Mac that grouped messages its own
    /// way would show the same conversation with different breaks in it.
    private var sections: [DaySection] {
        MessagePresentation.daySections(visibleMessages.map(MessageSnapshot.init))
    }

    /// The rendered slice — the newest `visibleCount` of what is cached.
    private var visibleMessages: ArraySlice<MessageEntity> {
        messages.suffix(visibleCount)
    }

    /// Is there anything older to reach, either in the cache or on the
    /// server? Decides whether the top sentinel is drawn at all.
    private var hasOlder: Bool {
        !messages.isEmpty && (visibleCount < messages.count || chat?.hasFullHistory != true)
    }

    /// One row's worth of presentation, worked out ONCE.
    ///
    /// Inline in the ForEach this was four index expressions per row and
    /// the type checker gave up on the whole body — "unable to type-check
    /// in reasonable time", which is what SwiftUI says instead of pointing
    /// at the arithmetic.
    private struct Row: Identifiable {
        let message: MessageSnapshot
        let isMine: Bool
        let showsSenderName: Bool
        let isRunStart: Bool
        let isRunEnd: Bool
        var id: String { message.localID }
    }

    private func rows(in section: DaySection) -> [Row] {
        let me = coordinator.currentUserID
        let isFamily = chat?.kind == "family"
        return section.messages.enumerated().map { index, message in
            let previous = index > 0 ? section.messages[index - 1] : nil
            let next = index < section.messages.count - 1 ? section.messages[index + 1] : nil
            return Row(
                message: message,
                isMine: message.senderID == me,
                showsSenderName: MessagePresentation.showsSenderName(
                    at: index,
                    in: section.messages,
                    isFamilyChat: isFamily,
                    currentUserID: me),
                isRunStart: previous?.senderID != message.senderID,
                // The last of a run carries the time, so a burst of four
                // messages is stamped once rather than four times.
                isRunEnd: next?.senderID != message.senderID)
        }
    }

    private var thread: some View {
        ScrollViewReader { proxy in
            ScrollView {
                // A plain VStack, NOT a LazyVStack, and that is the fix
                // rather than a preference. The window above bounds this to
                // about a page of rows, so laying all of them out is cheap
                // and their heights are REAL — which is what makes the
                // bottom anchor exact and every row's id something
                // `scrollTo` can actually reach. A lazy stack here has to
                // estimate the height of every row it has not built, on
                // every layout negotiation, and re-arms itself while doing
                // it (see the header).
                VStack(alignment: .leading, spacing: 0) {
                    if hasOlder { topSentinel(proxy: proxy) }
                    ForEach(sections) { section in
                        MacDayPill(day: section.day)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 10)
                        ForEach(rows(in: section)) { row in
                            MacMessageRow(
                                message: row.message,
                                senderName: senderName(for: row.message.senderID),
                                nameFor: quoteAuthorName,
                                avatarVersionFor: { avatarVersions[$0] ?? 0 },
                                isStreaming: row.message.serverID.map {
                                    coordinator.streamingMessageIDs.contains($0)
                                } ?? false,
                                isMine: row.isMine,
                                showsSenderName: row.showsSenderName,
                                showsTimestamp: row.isRunEnd,
                                isRunStart: row.isRunStart,
                                isRunEnd: row.isRunEnd,
                                onReply: { beginReply(row.message) },
                                onEdit: { beginEdit(row.message) },
                                onOpenAttachment: { attachment in
                                    if attachment.isFile {
                                        openFile(attachment)
                                    } else {
                                        // Its own window, which is what
                                        // makes it resizable.
                                        openWindow(
                                            id: MacWindow.attachment,
                                            value: attachment)
                                    }
                                },
                                memberCount: familyMemberCount)
                                .id(row.message.localID)
                        }
                    }
                    bottomSentinel
                }
                .padding(.horizontal, 18)
                .padding(.vertical, 12)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .defaultScrollAnchor(.bottom)
            // The sidebar toggle is a WIDTH change, and a ScrollView keeps
            // a point offset across one rather than a message. Watching the
            // width is what lets the thread be put back where the reader
            // left it (protocol-free — this is purely a client concern).
            .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width in
                guard threadWidth != 0, width != threadWidth else {
                    threadWidth = width
                    return
                }
                threadWidth = width
                // Only the bottom is restorable without tracking per-row
                // geometry, and it is the case that actually bit: a reader
                // at the newest message expects to still be there when the
                // sidebar comes back. Away from the bottom the bounded
                // window keeps any drift to genuine re-wrapping of a page
                // of rows, rather than thousands of re-estimated ones.
                if isPinnedToBottom {
                    proxy.scrollTo(Self.bottomAnchor, anchor: .bottom)
                }
            }
            .overlay {
                if messages.isEmpty {
                    ContentUnavailableView(
                        "No messages yet",
                        systemImage: "bubble.left.and.bubble.right",
                        description: Text("Say something to get started."))
                }
            }
            .onChange(of: composerHeight) {
                // The composer just grew or shrank into the thread. Re-pin,
                // so the newest message stays above it — this is the phone's
                // `onChange(of: inputBarHeight)`, which the Mac never had,
                // and its absence is why clicking Reply left the thread
                // parked behind the banner and — because the sentinel had
                // left the viewport — no longer following new messages at
                // all.
                //
                // Deliberately does NOT consult `isPinnedToBottom`: this
                // fires after the layout pass that already invalidated it.
                // The one case that must not pin is a composer primed from
                // history — answering or rewriting — and that was decided
                // before the banner appeared.
                guard composerPriming?.suppressesRePin != true else { return }
                proxy.scrollTo(Self.bottomAnchor, anchor: .bottom)
            }
            .onChange(of: previewLoader.generation) {
                // A link-preview card just landed: the bubble hosting it
                // GREW, and a bubble growing above the viewport slides the
                // visible region into older content. Re-pin, but only for a
                // reader who was at the bottom — the same rule, and the
                // same repair, the phone makes.
                guard isPinnedToBottom else { return }
                proxy.scrollTo(Self.bottomAnchor, anchor: .bottom)
            }
            .onChange(of: messages.count) { oldCount, newCount in
                // Grow the window over what just arrived, or new messages
                // fall outside the rendered slice and the thread looks
                // stuck. Only ever forward — a reader who paged back keeps
                // what they widened to — but CAPPED, which is the point of
                // having a window at all: this stack is non-lazy, so an
                // uncapped count would creep up over a long session until
                // the layout cost that caused the hang came back. Past the
                // cap the window SLIDES instead, keeping the newest rows.
                //
                // Only for a reader who is NOT at the bottom, which is the
                // phone's rule (ConversationView) and was the one piece of it
                // the Mac did not copy. For a reader at the bottom widening
                // buys nothing — the suffix already ends at the newest row —
                // and it costs a SECOND content change: this write lands in
                // the next update, so the pass that renders the arrival drops
                // the oldest rendered row and the pass after it puts that row
                // back, moving the thread twice for one message. Away from
                // the bottom it is still exactly right: arriving messages
                // widen the window instead of sliding the suffix out from
                // under somebody reading history.
                visibleCount = ThreadFollow.windowAfterArrival(
                    current: visibleCount,
                    cached: newCount,
                    arrived: newCount - oldCount,
                    isAtNewest: isPinnedToBottom,
                    cap: Self.maxWindow)
                // Follow the newest message for a reader who was ALREADY at
                // the bottom — yanking somebody out of history because
                // somebody else typed is the bug the phone fixed long ago —
                // OR when this member is the one who pressed Send. Pressing
                // Send and watching nothing happen is worse than either.
                // Re-ask what this reader can see: a message arriving for
                // somebody already at the bottom scrolls the thread WITHOUT
                // the sentinel ever leaving the viewport, so the geometry
                // hook may not fire, and this is what reads the message
                // they just watched land.
                publishPresence(isAtNewest: isPinnedToBottom)
                // Read and clear together: the declaration belongs to the
                // send that made it, and one left armed would yank the next
                // reader out of history for a message somebody else sent.
                let didSend = owesSendPin
                owesSendPin = false
                guard ThreadFollow.followsArrival(
                    isAtNewest: isPinnedToBottom, didSend: didSend) else { return }
                withAnimation(.easeOut(duration: 0.18)) {
                    proxy.scrollTo(Self.bottomAnchor, anchor: .bottom)
                }
            }
        }
    }

    /// Reaching the oldest rendered message asks for more.
    ///
    /// Geometry-triggered rather than `onAppear`, because in a NON-lazy
    /// stack every row appears at creation — an `onAppear` here would fire
    /// a history page on every open.
    private func topSentinel(proxy: ScrollViewProxy) -> some View {
        HStack {
            if isLoadingOlder {
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

    /// The truth about whether the newest message is on screen, and the
    /// target of every pin. One view, one geometry read — cheap enough to
    /// keep even though the window made everything else cheap.
    private var bottomSentinel: some View {
        Color.clear
            .frame(height: 1)
            .id(Self.bottomAnchor)
            .onGeometryChange(for: Bool.self) { geometry in
                // The rule itself is in ThreadFollow, where it is arithmetic
                // and has tests. Two things it decides that this closure used
                // to get wrong on its own: a missing viewport answers NO
                // rather than yes (this value marks a conversation READ, and
                // the server's marker is monotonic — the safe guess is the
                // one that reads nothing), and the band is bounded at BOTH
                // ends, so a sentinel far above the viewport is not reported
                // as "the newest message is on screen".
                ThreadFollow.isAtNewest(
                    sentinelMinY: geometry.frame(in: .scrollView).minY,
                    viewportHeight: geometry.bounds(of: .scrollView)?.height)
            } action: { visible in
                isPinnedToBottom = visible
                // The one fact only this geometry knows, and the one that
                // decides whether anything is read — published the moment
                // it moves, in both directions, and from the value just
                // computed rather than the state it was written to.
                publishPresence(isAtNewest: visible)
            }
    }

    /// Widen the window, and page over the network once the cache runs out.
    ///
    /// Local first: messages already downloaded but outside the rendered
    /// slice cost nothing to show, so a reader scrolling back gets them
    /// instantly and the server is only asked when there is nothing left.
    private func loadOlder(proxy: ScrollViewProxy) {
        guard !isLoadingOlder else { return }
        if visibleCount < messages.count {
            let anchorID = visibleMessages.first?.localID
            let widened = ThreadFollow.windowAfterPagingBack(
                current: visibleCount, cached: messages.count, step: Self.windowStep,
                cap: Self.maxWindow)
            // At the cap this branch widens by nothing, and it used to
            // schedule its restore scroll anyway: no row appeared, so
            // putting the oldest rendered row back at the TOP simply threw
            // the reader up there. The top sentinel is still on screen
            // afterwards (`hasOlder` stays true while the cache runs past
            // the cap), so it fired again, and again. Leave quietly instead
            // — the cap is a deliberate bound on layout cost, and refusing
            // to widen is the honest answer to it.
            guard widened > visibleCount else { return }
            visibleCount = widened
            if let anchorID {
                // Two turns, like the phone's: rows widened into existence
                // in this pass are not laid out yet, and a scrollTo at a
                // row that has no frame is a silent no-op.
                Task { @MainActor in
                    try? await Task.sleep(nanoseconds: 50_000_000)
                    proxy.scrollTo(anchorID, anchor: .top)
                }
            }
            return
        }
        guard chat?.hasFullHistory != true else { return }
        isLoadingOlder = true
        let anchorID = visibleMessages.first?.localID
        Task { @MainActor in
            let fetched = await coordinator.loadOlder(chatID: chatID)
            isLoadingOlder = false
            // Restore the reading position ONLY when rows really were
            // prepended: the sentinel can fire during the opening layout
            // pass, and scrolling the old top row back to the top would
            // yank a freshly opened chat away from the newest messages.
            guard fetched > 0 else { return }
            try? await Task.sleep(nanoseconds: 50_000_000)
            visibleCount = min(min(messages.count, visibleCount + fetched), Self.maxWindow)
            if let anchorID, !isPinnedToBottom {
                proxy.scrollTo(anchorID, anchor: .top)
            } else {
                proxy.scrollTo(Self.bottomAnchor, anchor: .bottom)
            }
        }
    }

    private var composer: some View {
        VStack(alignment: .leading, spacing: 6) {
            if recorder.isRecording {
                HStack(spacing: 10) {
                    Image(systemName: "waveform")
                        .foregroundStyle(.red)
                    Text(verbatim: AudioRecorder.timeLabel(recorder.elapsed))
                        .font(.callout.monospacedDigit())
                    Spacer(minLength: 0)
                    Button("Cancel") { recorder.cancel() }
                    Button("Stop") { finishRecording() }
                        .keyboardShortcut(.defaultAction)
                }
            }
            if let mediaNotice {
                Text(mediaNotice)
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            if let replyDraft {
                MacComposerBanner(
                    icon: "arrowshape.turn.up.left",
                    text: replyDraft.excerpt,
                    onCancel: { self.replyDraft = nil })
            }
            if editTarget != nil {
                MacComposerBanner(
                    icon: "pencil",
                    text: "Editing message",
                    onCancel: { cancelEdit() })
            }
            if let staged {
                StagedAttachmentChip(item: staged) { discardStaged() }
            }
            HStack(alignment: .bottom, spacing: 8) {
                Menu {
                    Button {
                        pickAttachment()
                    } label: {
                        Label("Attach a File…", systemImage: "doc")
                    }
                    // Inside the menu on purpose: the guard below disables
                    // attaching while an edit is in progress, and an item
                    // here inherits it for free.
                    Button {
                        pasteAttachment()
                    } label: {
                        Label("Paste", systemImage: "doc.on.clipboard")
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
                    // "Polls").
                    if chat?.kind == "family" {
                        Button {
                            showPollComposer = true
                        } label: {
                            Label("Poll", systemImage: "chart.bar")
                        }
                    }
                } label: {
                    Image(systemName: "paperclip")
                        .font(.system(size: 16))
                        // Same box as the send button, so the two are
                        // optically level with each other and with the last
                        // line of text — bottom-aligning glyphs of different
                        // sizes lines up their edges, not their centres.
                        .frame(width: composerControl, height: composerControl)
                        .contentShape(Rectangle())
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                .help("Attach a photo, video or file")
                // Editing borrows the composer to rewrite an existing
                // message; there is no second attachment to add, and
                // attaching would post it as a new message while leaving the
                // edit banner armed. iOS and Android already gated this.
                .disabled(editTarget != nil)

                if showsAssistantMention {
                    Button {
                        insertAssistantMention()
                    } label: {
                        Image(systemName: "sparkles")
                            .font(.system(size: 15))
                            .frame(width: composerControl, height: composerControl)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.borderless)
                    .help("Ask the assistant")
                    .disabled(editTarget != nil)
                }

                TextField("Message", text: $draft, axis: .vertical)
                    .textFieldStyle(.plain)
                    .font(.body)
                    .lineLimit(1...8)
                    .focused($composerFocused)
                    .onSubmit(send)

                Button(action: send) {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.system(size: 22))
                        .foregroundStyle(canSend ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
                        .frame(width: composerControl, height: composerControl)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.borderless)
                .keyboardShortcut(.return, modifiers: [])
                .disabled(!canSend)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(Color.appSecondaryFill.opacity(0.5)))
            .overlay(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .strokeBorder(composerFocused ? Color.accentColor.opacity(0.6) : .clear))
        }
        .padding(12)
        .onGeometryChange(for: CGFloat.self) { geometry in
            geometry.size.height
        } action: { height in
            composerHeight = height
        }
    }

    private var canSend: Bool {
        guard !isSending else { return false }
        // Something staged is enough on its own: an attachment does not
        // need a caption, which is the phone's rule too.
        if staged != nil { return true }
        return !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// Send where this Mac is, once.
    ///
    /// The same one-tap shape as the phone: "share my location" is what
    /// people mean, and a map to drag a pin around is a different feature.
    /// A sandboxed Mac additionally needs the
    /// `com.apple.security.personal-information.location` entitlement, or
    /// CoreLocation refuses without explaining itself.
    private func shareLocation() {
        guard !isSending else { return }
        isSending = true
        Task {
            defer { isSending = false }
            do {
                let fix = try await locationProvider.currentFix()
                // Read and clear together, so a caption typed while the fix
                // was arriving travels with the pin rather than being lost
                // — and so a primed reply is not left armed for the next
                // ordinary message. That pair of bugs has been fixed here
                // once already, on the media path.
                let caption = draft.trimmingCharacters(in: .whitespacesAndNewlines)
                let quote = replyDraft
                draft = ""
                replyDraft = nil
                owesSendPin = true
                let sent = await coordinator.sendLocation(
                    latitude: fix.latitude,
                    longitude: fix.longitude,
                    accuracyM: fix.accuracyM,
                    label: nil,
                    caption: caption,
                    replyTo: quote,
                    in: chatID)
                if !sent {
                    // Through the guarded restore, like every other send
                    // here: a location fix can take twenty seconds, and
                    // assigning unconditionally would wipe whatever was
                    // typed while it was being acquired.
                    restoreComposer(caption: caption, quote: quote)
                    mediaNotice = String(localized: "Could not share your location.")
                }
            } catch LocationProvider.Failure.denied {
                mediaNotice = String(
                    localized:
                        "Family needs permission to use your location. Turn it on in System Settings."
                )
            } catch {
                mediaNotice = String(localized: "Could not find your location.")
            }
        }
    }

    /// Offer the mention only where it does something: the family chat,
    /// on a server that actually has an assistant. An absent `assistant`
    /// on `GET /families/mine` is the whole capability check
    /// (docs/protocol.md, "Mentioning the assistant in the family chat").
    private var showsAssistantMention: Bool {
        chat?.kind == "family" && AppSettings.assistantUserID != nil
    }

    /// Put `@ai ` in the draft and put the caret back in the field.
    ///
    /// Appended rather than inserted at the caret, for the reason the phone
    /// has: SwiftUI's TextField publishes no selection, so "at the caret"
    /// is not knowable, and moving somebody's cursor is worse than adding
    /// to the end of what they were writing.
    private func insertAssistantMention() {
        let token = AssistantMention.token
        guard !AssistantMention.mentions(draft) else {
            composerFocused = true
            return
        }
        if draft.isEmpty {
            draft = "\(token) "
        } else if draft.hasSuffix(" ") {
            draft += "\(token) "
        } else {
            draft += " \(token) "
        }
        composerFocused = true
    }

    /// The name above a bubble in the family chat.
    ///
    /// The roster alone is not enough any more: the assistant answers here
    /// whenever somebody mentions it, and its reserved account belongs to
    /// no family, so it is in no roster by design. Without this the run
    /// head would draw a bubble with no name at all.
    private func senderName(for userID: Int64) -> String? {
        if let assistantID = AppSettings.assistantUserID, userID == assistantID {
            return AppSettings.assistantName ?? String(localized: "Assistant")
        }
        return memberNames[userID]
    }

    /// Who a quoted sender is, matching the phone's wording.
    private func quoteAuthorName(_ userID: Int64) -> String {
        if userID == coordinator.currentUserID { return String(localized: "You") }
        // In the FAMILY chat the assistant answers whenever somebody
        // mentions it, and there is no "the other participant" shortcut
        // there. Its id comes from `GET /families/mine`.
        if let assistantID = AppSettings.assistantUserID, userID == assistantID {
            return AppSettings.assistantName ?? String(localized: "Assistant")
        }
        // In its OWN chat the account is deliberately not in the roster, so
        // anyone who is not me is it.
        if chat?.kind == "ai" { return chat?.title ?? String(localized: "Someone") }
        return memberNames[userID] ?? String(localized: "Someone")
    }

    private var typingLine: String? {
        let ids = coordinator.typingUserIDs(in: chatID)
        guard !ids.isEmpty else { return nil }
        let names = ids.map { memberNames[$0] ?? "Someone" }
        return names.count == 1 ? "\(names[0]) is typing…" : "\(names.joined(separator: ", ")) are typing…"
    }

    /// Tell the coordinator what the person at this window can actually
    /// see — the only thing that marks a chat read (ChatPresence).
    ///
    /// `isAtNewest` is the bottom sentinel's geometry, and it is passed in
    /// rather than read from state so the sentinel can hand over the value
    /// it just computed. It is also why nothing calls this from
    /// `onChange(of: composerHeight)`: a reply banner GROWS the composer,
    /// which takes the sentinel out of the viewport before the re-pin puts
    /// it back, so for one layout pass the flag is false. False is the safe
    /// direction — it reads nothing — and the sentinel publishes the truth
    /// again the moment the re-pin lands.
    ///
    /// The guard is what makes several conversation windows work. Presence
    /// is ONE value, so a window that is not key may only speak when the
    /// claim is already its own or nobody holds one — otherwise a window
    /// sitting behind everything else would overwrite what the window the
    /// user IS looking at published, purely because a message landed in it,
    /// and stop that window reading anything.
    private func publishPresence(isAtNewest: Bool) {
        let isKey = windowActivation == .key
        guard isKey || coordinator.presence == nil || coordinator.presence?.chatID == chatID else {
            return
        }
        coordinator.updatePresence(chatID: chatID, isAtNewest: isAtNewest, isFrontmost: isKey)
    }

    private func send() {
        let body = draft.trimmingCharacters(in: .whitespacesAndNewlines)

        // Edit mode borrows the composer. The field clears only once the
        // server takes it — a refused edit leaves the text there to fix,
        // which is the phone's rule too.
        if let target = editTarget {
            guard !body.isEmpty else { return }
            Task {
                if await coordinator.edit(
                    messageServerID: target.messageID, in: chatID, body: body)
                {
                    editTarget = nil
                    draft = target.original
                }
            }
            return
        }

        // An attachment takes whatever is typed as its caption, so this
        // comes BEFORE the empty-body guard: an attachment on its own is a
        // perfectly good message.
        if let item = staged {
            sendStaged(item, caption: body)
            return
        }
        guard !body.isEmpty else { return }

        // Read and clear together: the quote belongs to the message being
        // sent, and leaving it primed would silently quote the next one.
        let quote = replyDraft
        draft = ""
        replyDraft = nil
        // Declared before the row exists, because that is the only moment
        // this is knowable without guessing (see `owesSendPin`).
        owesSendPin = true
        coordinator.send(body: body, in: chatID, replyTo: quote)
    }

    /// The poll door, and a send door like any other.
    ///
    /// Two things every other door on this window already does. The quote
    /// is read and cleared together, because a poll may be a reply and
    /// leaving the banner armed would silently quote the next ordinary
    /// message. And the pin is DECLARED here, not inferred later: the Mac
    /// follows an arrival only for somebody who pressed Send
    /// (`ThreadFollow.followsArrival`), so a poll created from up in
    /// history would otherwise land off-screen and pressing Create would
    /// look like nothing happened.
    ///
    /// Armed only on a localID: `sendPoll` refuses an empty question or a
    /// bad option list and returns nil, and a pin left armed with no row
    /// coming would spend itself on somebody else's next message — the
    /// case `restoreComposer` exists for.
    private func sendPoll(question: String, options: [String]) {
        let quote = replyDraft
        guard coordinator.sendPoll(
            question: question, options: options, in: chatID, replyTo: quote) != nil
        else { return }
        replyDraft = nil
        owesSendPin = true
    }

    private func beginReply(_ message: MessageSnapshot) {
        guard let serverID = message.serverID else { return }
        editTarget = nil
        // Decided HERE, before the banner grows the composer: afterwards the
        // sentinel has left the viewport and the answer would always be
        // "they were not at the bottom".
        composerPriming = .reply(isPinnedToBottom: isPinnedToBottom)
        replyDraft = ReplyToDTO(
            messageID: serverID,
            senderID: message.senderID,
            // Cut the way the server cuts (scalars, not graphemes) so the
            // quote does not visibly change length when the ack lands.
            excerpt: ReplyToSnapshot.excerpt(of: message.body))
        composerFocused = true
    }

    private func beginEdit(_ message: MessageSnapshot) {
        guard let serverID = message.serverID else { return }
        replyDraft = nil
        // The same capture a reply makes, for the same reason and at the
        // same moment: the edit banner and the prefilled (often multi-line)
        // draft grow the composer just as a reply banner does, and a reader
        // rewriting something from history must not be thrown to the newest
        // message. The phone has always done this; the Mac forgot.
        composerPriming = .edit(isPinnedToBottom: isPinnedToBottom)
        // The draft in progress is not thrown away: it comes back when the
        // edit is cancelled or lands.
        editTarget = (messageID: serverID, original: draft)
        draft = message.body
        composerFocused = true
    }

    private func cancelEdit() {
        guard let target = editTarget else { return }
        editTarget = nil
        draft = target.original
    }

    /// A file has nothing to view: hand it to whatever opens it.
    private func openFile(_ attachment: AttachmentDTO) {
        Task {
            mediaNotice = String(localized: "Preparing…")
            defer { mediaNotice = nil }
            guard let url = await coordinator.localFileURL(for: attachment) else { return }
            NSWorkspace.shared.open(url)
        }
    }

    /// An open panel rather than PhotosPicker: on the Mac what people
    /// attach lives in the file system, and the same panel covers photos,
    /// videos and documents alike.
    private func pickAttachment() {
        guard let url = MacFilePicker.pickOne() else { return }
        mediaNotice = String(localized: "Preparing…")
        Task {
            do {
                stage(try await MediaPrep.prepare(fileAt: url, limit: MediaPrep.sizeLimit))
            } catch MediaPrep.PrepError.tooLargeAfterCompression {
                mediaNotice = String(localized: "That file is over the 100 MB limit.")
            } catch {
                mediaNotice = String(localized: "Couldn't read that file.")
            }
        }
    }

    /// Attach whatever is on the clipboard.
    ///
    /// Staged, never sent: a paste that posted a message by itself would be
    /// a trap, because ⌘V is a reflex and the clipboard is not always what
    /// its owner thinks it is.
    private func pasteAttachment() {
        mediaNotice = String(localized: "Preparing…")
        Task {
            do {
                stage(try await ClipboardAttachment.prepare(limit: MediaPrep.sizeLimit))
            } catch ClipboardAttachment.Failure.nothingToPaste {
                mediaNotice = String(localized: "There's nothing to paste.")
            } catch MediaPrep.PrepError.tooLargeAfterCompression {
                // The same ceiling and the same wording every other door
                // uses; a pasted item is not a special kind of too big.
                mediaNotice = String(localized: "That file is over the 100 MB limit.")
            } catch {
                mediaNotice = String(localized: "Couldn't prepare that item.")
            }
        }
    }

    /// Stop recording and stage it, so a caption can be added and so a
    /// recording made by accident can still be discarded — the phone's
    /// behaviour, which the Mac could not have until it had a staged slot.
    private func finishRecording() {
        guard let url = recorder.stop() else {
            mediaNotice = String(localized: "That recording was too short.")
            return
        }
        mediaNotice = String(localized: "Preparing…")
        Task {
            do {
                // No name: a voice note's identity is its length, and the
                // scratch file it was recorded into is called
                // `fc-voice-<UUID>.m4a`.
                stage(try await MediaPrep.prepareAudio(from: url, limit: MediaPrep.sizeLimit))
            } catch MediaPrep.PrepError.tooLargeAfterCompression {
                mediaNotice = String(localized: "That file is over the 100 MB limit.")
                try? FileManager.default.removeItem(at: url)
            } catch {
                mediaNotice = String(localized: "Couldn't prepare that item.")
                try? FileManager.default.removeItem(at: url)
            }
        }
    }

    /// Hold prepared media in the composer, replacing anything already
    /// there — one attachment per message, so a second pick supersedes the
    /// first rather than queueing behind it.
    private func stage(_ prepared: MediaPrep.Prepared) {
        discardStaged()
        mediaNotice = nil
        staged = StagedAttachment(prepared: prepared)
        composerFocused = true
    }

    /// Throw away the staged attachment and its temp file.
    ///
    /// The file is ours: MediaPrep wrote it into a temp directory and
    /// nothing else will clean it up, because the `defer` that normally
    /// deletes it lives in `sendMedia` — which never ran.
    private func discardStaged() {
        guard let staged else { return }
        try? FileManager.default.removeItem(at: staged.prepared.fileURL)
        self.staged = nil
    }

    /// Commit the staged attachment with whatever the composer holds.
    ///
    /// Caption and quote are taken NOW, before the await, and put back only
    /// if the send never happens — the phone's rule
    /// (ConversationView.takeComposer). It fixed two Mac-only bugs when it
    /// lived at the picker, and both would come straight back if this read
    /// `draft` after the upload: anything typed while it ran was swallowed
    /// into the caption, and the primed reply was dropped entirely while
    /// its banner stayed armed, so the NEXT ordinary message silently
    /// became that reply.
    private func sendStaged(_ item: StagedAttachment, caption: String) {
        let quote = replyDraft
        draft = ""
        replyDraft = nil
        staged = nil
        mediaNotice = String(localized: "Sending…")
        // Declared at the door, because that is the only moment this is
        // knowable without guessing (see `owesSendPin`).
        owesSendPin = true
        Task {
            if await coordinator.sendMedia(
                item.prepared, caption: caption, replyTo: quote, in: chatID)
            {
                mediaNotice = nil
            } else {
                mediaNotice = String(localized: "Couldn't send that.")
                // Through the guarded restore, which is also what disarms
                // the pin: no row is coming, and one left armed would yank
                // the next reader out of history for somebody else's
                // message.
                restoreComposer(caption: caption, quote: quote)
                // Put the attachment back too: `sendMedia` deletes the temp
                // file only on the paths that consumed it, and a failed
                // send the user can retry is worth more than a tidy temp
                // dir.
                if FileManager.default.fileExists(atPath: item.prepared.fileURL.path) {
                    staged = item
                }
            }
        }
    }

    /// Put the composer back when the send never happened — but never over
    /// something typed in the meantime.
    private func restoreComposer(caption: String, quote: ReplyToDTO?) {
        if draft.isEmpty { draft = caption }
        if replyDraft == nil { replyDraft = quote }
        // No row is coming, so the pin declared at the door has nothing to
        // spend itself on. Left armed it would sit there until the NEXT
        // message arrived — somebody else's — and pull a reader out of
        // history for it. The upload doors are the only ones that can fail
        // this way; a text send is optimistic and always produces its row.
        owesSendPin = false
    }
}

/// "Replying to…" / "Editing message" above the field, with the way out.
private struct MacComposerBanner: View {
    let icon: String
    let text: String
    let onCancel: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: icon)
                .foregroundStyle(.tint)
            Text(text)
                .font(.callout)
                .lineLimit(1)
                .foregroundStyle(.secondary)
            Spacer()
            Button {
                onCancel()
            } label: {
                Image(systemName: "xmark.circle.fill")
            }
            .buttonStyle(.borderless)
        }
    }
}

/// "Today / Yesterday / Mon, Aug 17" between day sections — the same
/// labels the phone uses.
private struct MacDayPill: View {
    let day: Date

    var body: some View {
        Text(label)
            .font(.caption2.weight(.medium))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 10)
            .padding(.vertical, 3)
            .background(Color.appSecondaryFill.opacity(0.6), in: Capsule())
    }

    private var label: String {
        let calendar = Calendar.current
        if calendar.isDateInToday(day) { return String(localized: "Today") }
        if calendar.isDateInYesterday(day) { return String(localized: "Yesterday") }
        return day.formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day())
    }
}

#endif
