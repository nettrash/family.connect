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
    /// localID of the bubble the floating reaction picker is up over;
    /// nil = no picker. Set/cleared inside withAnimation so the capsule
    /// springs in and out.
    @State private var reactionPickerID: String?
    /// The bubble the "+" full emoji picker sheet is up for.
    @State private var fullPickerTarget: ReactionTarget?

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
                guard inputFocused else { return }
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
                guard inputFocused else { return }
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
                        floatingMenu(size: ReactionCapsule.size(emojiCount: emojis.count), over: rect, in: proxy) {
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
                    }
                }
            }
        }
    }

    /// Places one floating menu of a known size against the pressed
    /// bubble's rect: above it (below when the bubble hugs the top
    /// edge), centered on it but clamped inside the horizontal bounds,
    /// and springing out of the nearest point of the bubble.
    private func floatingMenu<Content: View>(
        size: CGSize,
        over rect: CGRect,
        in proxy: GeometryProxy,
        @ViewBuilder content: () -> Content
    ) -> some View {
        let margin: CGFloat = 8
        let placeBelow = rect.minY - size.height - margin < proxy.safeAreaInsets.top + margin
        let x = min(max(rect.midX, size.width / 2 + margin), proxy.size.width - size.width / 2 - margin)
        let y = placeBelow
            ? rect.maxY + margin + size.height / 2
            : rect.minY - margin - size.height / 2
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

    private func dismissReactionPicker() {
        withAnimation(.spring(duration: 0.3, bounce: 0.25)) {
            reactionPickerID = nil
        }
    }

    // MARK: - Actions

    private func send() {
        let body = model.draft
        guard !body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        coordinator.send(body: body, in: chatID)
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
