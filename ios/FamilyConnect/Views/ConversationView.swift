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
//    stay correct, which hand-rolled bottom bars always get wrong.
//
//  The view sets coordinator.activeChatID on appear/disappear; while set,
//  inbound messages for this chat advance the read marker instead of the
//  unread badge (the coordinator owns that rule).
//
//  Long-pressing a bubble opens a compact sheet: the quick-reaction row
//  for messages the server knows (a reaction needs a server message id),
//  and Retry/Delete for failed ones (which never have a server id — the
//  two gates are disjoint). Pending-but-unfailed bubbles get no sheet.
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

    /// Identity of the bubble the long-press sheet is up for. A tiny
    /// Identifiable wrapper because `.sheet(item:)` wants one and the
    /// stable identity of a bubble is its localID string.
    private struct ReactionTarget: Identifiable {
        let localID: String
        var id: String { localID }
    }

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Query private var messages: [MessageEntity]
    @Query private var chats: [ChatEntity]
    @Query private var members: [MemberEntity]
    @State private var model = ConversationModel()
    @State private var reactionTarget: ReactionTarget?

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

    private var sections: [DaySection] {
        MessagePresentation.daySections(messages.map(MessageSnapshot.init))
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 4) {
                    if model.state != .atStart && chat?.hasFullHistory != true && !messages.isEmpty {
                        // Top sentinel: appearing means the user scrolled
                        // to the oldest loaded message.
                        HStack {
                            if model.state == .loadingOlder {
                                ProgressView().controlSize(.small)
                            } else {
                                Color.clear.frame(height: 1)
                            }
                        }
                        .frame(maxWidth: .infinity)
                        .frame(height: 24)
                        .onAppear { loadOlder(proxy: proxy) }
                    }

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
                                    reactionTarget = ReactionTarget(localID: message.localID)
                                })
                                .id(message.localID)
                        }
                    }
                }
                .padding(.horizontal, 12)
                .padding(.top, 8)
            }
            .defaultScrollAnchor(.bottom)
            .scrollDismissesKeyboard(.interactively)
            .onChange(of: messages.count) {
                // Follow the bottom for new messages — but never while a
                // history page is being prepended above.
                guard model.state == .idle, let last = messages.last else { return }
                withAnimation(.easeOut(duration: 0.2)) {
                    proxy.scrollTo(last.localID, anchor: .bottom)
                }
            }
        }
        .safeAreaInset(edge: .bottom) {
            inputBar
        }
        .sheet(item: $reactionTarget) { target in
            reactionSheet(for: target)
        }
        .navigationTitle(chat?.title ?? "")
        .navigationBarTitleDisplayMode(.inline)
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

    // MARK: - Input bar

    private var inputBar: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let typingLine {
                Text(typingLine)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 16)
                    .padding(.bottom, 4)
                    .transition(.opacity)
            }
            HStack(alignment: .bottom, spacing: 8) {
                TextField("Message", text: Bindable(model).draft, axis: .vertical)
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

    // MARK: - Reaction sheet

    /// The long-press sheet's content, resolved against the LIVE entity
    /// (not a captured snapshot) so a state change while the sheet is up
    /// — the ack landing, say — shows current truth.
    @ViewBuilder
    private func reactionSheet(for target: ReactionTarget) -> some View {
        let message = messages.first { $0.localID == target.localID }
        VStack(spacing: 20) {
            if let message {
                if message.serverID != nil {
                    // Reacting needs a server message id to PUT against.
                    HStack(spacing: 8) {
                        let mine = message.reactionList.first { $0.userID == currentUserID }?.emoji
                        ForEach(MessagePresentation.quickReactions, id: \.self) { emoji in
                            Button {
                                toggleReaction(localID: message.localID, emoji: emoji)
                                reactionTarget = nil
                            } label: {
                                Text(emoji)
                                    .font(.system(size: 32))
                                    .padding(8)
                                    .background {
                                        if mine == emoji {
                                            Circle().fill(.tint.opacity(0.2))
                                        }
                                    }
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
                if message.state == .failed {
                    // The failed-bubble actions (no server id, so the two
                    // gates never show together).
                    VStack(spacing: 12) {
                        Button {
                            coordinator.retry(localID: message.localID)
                            reactionTarget = nil
                        } label: {
                            Label("Try Again", systemImage: "arrow.clockwise")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.bordered)
                        Button(role: .destructive) {
                            coordinator.deleteLocalMessage(localID: message.localID)
                            reactionTarget = nil
                        } label: {
                            Label("Delete", systemImage: "trash")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.bordered)
                    }
                    .padding(.horizontal, 24)
                }
            }
        }
        .frame(maxWidth: .infinity)
        .presentationDetents([.height(200)])
        .presentationDragIndicator(.visible)
    }

    // MARK: - Actions

    private func send() {
        let body = model.draft
        model.draft = ""
        coordinator.send(body: body, in: chatID)
    }

    private func toggleReaction(localID: String, emoji: String) {
        Task { await coordinator.toggleReaction(localID: localID, emoji: emoji) }
    }

    private func loadOlder(proxy: ScrollViewProxy) {
        guard model.state == .idle else { return }
        model.state = .loadingOlder
        // The bubble to restore to: the current topmost message.
        let anchorID = messages.first?.localID
        Task {
            _ = await coordinator.loadOlder(chatID: chatID)
            // Give the @Query one runloop tick to reflect the inserts,
            // then put the previous top bubble back at the top.
            try? await Task.sleep(nanoseconds: 50_000_000)
            if let anchorID {
                proxy.scrollTo(anchorID, anchor: .top)
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
