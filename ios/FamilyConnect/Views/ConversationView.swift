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
//  The view sets coordinator.activeChatID on appear/disappear; while set,
//  inbound messages for this chat advance the read marker instead of the
//  unread badge (the coordinator owns that rule).
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

import SwiftData
import SwiftUI

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
    @Query private var messages: [MessageEntity]
    @Query private var chats: [ChatEntity]
    @Query private var members: [MemberEntity]
    @State private var model = ConversationModel()
    /// False until the opening layout settled at the newest message (the
    /// convergence loop in `.task`); a history-load scroll-restore before
    /// that would yank a freshly opened chat away from the bottom.
    @State private var hasSettled = false
    /// True while the bottom sentinel (the row after the newest message)
    /// is materialized — the ground-truth "the user is at the bottom"
    /// signal that the convergence loop and the re-pin hooks key on.
    @State private var isPinnedToBottom = false
    /// Focus of the input field. Focusing raises the keyboard; both that
    /// and a growing multi-line draft change the bottom INSET, which
    /// defaultScrollAnchor does not re-anchor for — the snaps below do.
    @FocusState private var inputFocused: Bool
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
    /// localID of the bubble the floating reaction picker is up over;
    /// nil = no picker. Set/cleared inside withAnimation so the capsule
    /// springs in and out.
    @State private var reactionPickerID: String?
    /// The bubble the "+" full emoji picker sheet is up for.
    @State private var fullPickerTarget: ReactionTarget?
    /// Text handed to the share sheet, nil while it is closed.
    @State private var shareText: ShareText?

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
    private static let jumpMargin = 15
    /// Most rows a quote tap may render at once. Roughly five windows —
    /// far enough for any quote a reader is plausibly following, short of
    /// laying out a whole family history.
    private static let maxJumpWindow = 300

    private var visibleMessages: ArraySlice<MessageEntity> {
        messages.suffix(visibleCount)
    }

    private var sections: [DaySection] {
        MessagePresentation.daySections(visibleMessages.map(MessageSnapshot.init))
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
                        }
                }
                .padding(.horizontal, 12)
                .padding(.top, 8)
            }
            .defaultScrollAnchor(.bottom)
            .scrollDismissesKeyboard(.interactively)
            .task { await settleAtBottom(proxy: proxy) }
            .onChange(of: messages.count) { oldCount, newCount in
                // A reader away from the bottom keeps their place: arriving
                // messages widen the render window instead of sliding the
                // suffix out from under them.
                if newCount > oldCount && !isPinnedToBottom {
                    visibleCount = min(newCount, visibleCount + (newCount - oldCount))
                }
                // Follow the bottom for new messages — for my own sends
                // from anywhere, otherwise only while actually pinned (a
                // reader deep in history must not be yanked down) — and
                // never while a history page is being prepended above.
                guard model.state == .idle, let last = messages.last else { return }
                guard last.senderID == currentUserID || isPinnedToBottom || !hasSettled else { return }
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
            .onChange(of: inputFocused) {
                // The keyboard rising is an inset change, and the system's
                // own avoidance is best-effort — deterministically pin the
                // newest message above it (twice: as the animation starts
                // and after it lands; standard messenger behavior).
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
        .sheet(item: $shareText) { share in
            ShareSheet(text: share.text)
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
        }
        .onAppear {
            coordinator.activeChatID = chatID
            if messages.isEmpty {
                // Fresh chat with no local window yet: pull the newest page.
                loadInitialIfNeeded()
            }
        }
        .onDisappear {
            if coordinator.activeChatID == chatID {
                coordinator.activeChatID = nil
            }
        }
        .onChange(of: model.draft) { _, newValue in
            guard !newValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
            coordinator.sendTyping(in: chatID)
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
                            MessageBubbleView(
                                message: message,
                                isMine: message.senderID == currentUserID,
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

    /// Opening convergence: defaultScrollAnchor positions from ESTIMATED
    /// row heights, and on a long multi-day history (especially on slower
    /// devices) the estimate can land the viewport in blank unmaterialized
    /// space, with follow-up layout passes drifting it further — one
    /// delayed snap loses that race. Keep snapping to the real newest row
    /// until the bottom sentinel reports the bottom is actually on screen
    /// (or ~2.4 s pass), then hand over to the pin hooks. Runs in `.task`,
    /// so a back-swipe cancels it with the view.
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
            HStack(alignment: .bottom, spacing: 8) {
                TextField("Message", text: Bindable(model).draft, axis: .vertical)
                    .focused($inputFocused)
                    .lineLimit(1...5)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 7)
                    .background(Color(.secondarySystemFill), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                Button {
                    send()
                } label: {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.system(size: 30))
                        .foregroundStyle(.tint)
                }
                .disabled(model.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .accessibilityLabel("Send")
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
        }
        .background(.bar)
        .onGeometryChange(for: CGFloat.self) { geometry in
            geometry.size.height
        } action: { height in
            inputBarHeight = height
        }
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
        guard needed <= Self.maxJumpWindow else { return }
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

    private var typingLine: String? {
        let ids = coordinator.typingUserIDs(in: chatID)
        guard !ids.isEmpty else { return nil }
        let names = ids.map { displayName(for: $0) ?? "Someone" }
        switch names.count {
        case 1: return "\(names[0]) is typing…"
        default: return "\(names.joined(separator: ", ")) are typing…"
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
                        let menuSize = MessageContextMenu.size(canReply: canReply, canEdit: canEdit)
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
                                    shareText = ShareText(text: message.body)
                                },
                                canReply: canReply,
                                canEdit: canEdit)
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
        guard !body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        if let target = editTarget {
            submitEdit(target: target, body: body)
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

    private func toggleReaction(localID: String, emoji: String) {
        Task { await coordinator.toggleReaction(localID: localID, emoji: emoji) }
    }

    private func loadOlder(proxy: ScrollViewProxy) {
        guard model.state == .idle else { return }
        // Local-first: older messages already cached but outside the
        // render window just widen it — instant, no network.
        if visibleCount < messages.count {
            let anchorID = visibleMessages.first?.localID
            visibleCount = min(messages.count, visibleCount + Self.windowStep)
            if hasSettled, let anchorID {
                Task {
                    try? await Task.sleep(nanoseconds: 50_000_000)
                    proxy.scrollTo(anchorID, anchor: .top)
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
                // and widen the window over the fetched page so it renders.
                try? await Task.sleep(nanoseconds: 50_000_000)
                visibleCount = min(messages.count, visibleCount + fetched)
                if hasSettled, let anchorID {
                    // A real scroll-up: put the previous top bubble back
                    // at the top so the reading position holds.
                    proxy.scrollTo(anchorID, anchor: .top)
                } else {
                    // Fired during the opening pass: the user never left
                    // the bottom — keep them there over the grown content.
                    pinToBottom(proxy, animated: false)
                }
            }
            model.state = (chat?.hasFullHistory == true) ? .atStart : .idle
        }
    }

    private func loadInitialIfNeeded() {
        guard model.state == .idle else { return }
        model.state = .loadingOlder
        Task {
            _ = await coordinator.loadOlder(chatID: chatID)
            model.state = (chat?.hasFullHistory == true) ? .atStart : .idle
        }
    }

    private func displayName(for userID: Int64) -> String? {
        members.first(where: { $0.userID == userID })?.displayName
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

    /// userID → display name for every known member, feeding the "who
    /// reacted" rows. Tolerates duplicate rows (first wins) rather than
    /// trusting the store never to produce one.
    private var memberNames: [Int64: String] {
        Dictionary(members.map { ($0.userID, $0.displayName) }, uniquingKeysWith: { first, _ in first })
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
