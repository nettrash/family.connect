//
//  MacConversationView.swift
//  FamilyConnect
//
//  One chat's thread on the Mac.
//
//  Structurally simpler than the iOS one, and legitimately so: a Mac
//  window does not resize under a keyboard, so the whole scroll-anchoring
//  apparatus the phone needs (windowed non-lazy rendering, a bottom
//  sentinel, a convergence loop — see ConversationView's header) has
//  nothing to fight here. A LazyVStack with `defaultScrollAnchor(.bottom)`
//  and one scroll-on-new-message is enough.
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
    /// `.key` when this window is the frontmost one. With several
    /// conversation windows open, this is what decides which chat counts
    /// as the one being READ.
    @Environment(\.controlActiveState) private var windowActivation
    @Query private var messages: [MessageEntity]
    @Query private var chats: [ChatEntity]
    @Query private var members: [MemberEntity]

    @State private var draft = ""
    @State private var isSending = false
    @State private var mediaNotice: String?
    /// The message being answered, while the composer is primed.
    @State private var replyDraft: ReplyToDTO?
    /// The message being rewritten; mutually exclusive with a reply, the
    /// same rule the phone has — you are answering or rewriting, not both.
    @State private var editTarget: (messageID: Int64, original: String)?
    @Environment(\.openWindow) private var openWindow
    @FocusState private var composerFocused: Bool

    /// Side of the attach and send buttons — one box for both, so they sit
    /// level. Matches the composer's one-line height.
    private let composerControl: CGFloat = 24


    init(chatID: Int64) {
        self.chatID = chatID
        _messages = Query(
            filter: #Predicate<MessageEntity> { $0.chatID == chatID },
            sort: [SortDescriptor(\MessageEntity.createdAt), SortDescriptor(\MessageEntity.localID)])
        _chats = Query(filter: #Predicate<ChatEntity> { $0.chatID == chatID })
    }

    private var chat: ChatEntity? { chats.first }

    private var avatarVersions: [Int64: Int64] {
        Dictionary(members.map { ($0.userID, $0.avatarVersion) }, uniquingKeysWith: { first, _ in first })
    }

    private var memberNames: [Int64: String] {
        Dictionary(uniqueKeysWithValues: members.map { ($0.userID, $0.displayName) })
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
        .navigationTitle(chat?.title ?? "")
        .navigationSubtitle(typingLine ?? "")
        .task(id: chatID) {
            claimActive()
            // One page is enough to open with; the phone's paging sentinel
            // is a scroll-position mechanism this window does not need yet.
            if messages.isEmpty { _ = await coordinator.loadOlder(chatID: chatID) }
        }
        // `activeChatID` is ONE value and a Mac can have several
        // conversation windows, so ownership has to be explicit: the
        // frontmost window claims it, and a window only ever releases a
        // claim it still holds. Without that guard, closing one window
        // clears the claim of another that is still on screen — and every
        // message arriving in it starts bumping the unread badge of a chat
        // the user is looking at.
        .onChange(of: windowActivation) { _, state in
            if state == .key {
                claimActive()
            } else {
                releaseActive()
            }
        }
        .onDisappear { releaseActive() }
    }

    /// Day sections and sender runs, through the SAME rules the phone
    /// uses (MessagePresentation) — a Mac that grouped messages its own
    /// way would show the same conversation with different breaks in it.
    private var sections: [DaySection] {
        MessagePresentation.daySections(messages.map(MessageSnapshot.init))
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
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(sections) { section in
                        MacDayPill(day: section.day)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 10)
                        ForEach(rows(in: section)) { row in
                            MacMessageRow(
                                message: row.message,
                                senderName: memberNames[row.message.senderID],
                                nameFor: quoteAuthorName,
                                avatarVersionFor: { avatarVersions[$0] ?? 0 },
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
                                })
                                .id(row.message.localID)
                        }
                    }
                }
                .padding(.horizontal, 18)
                .padding(.vertical, 12)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .defaultScrollAnchor(.bottom)
            .overlay {
                if messages.isEmpty {
                    ContentUnavailableView(
                        "No messages yet",
                        systemImage: "bubble.left.and.bubble.right",
                        description: Text("Say something to get started."))
                }
            }
            .onChange(of: messages.count) {
                // A Mac window does not move under a keyboard, so following
                // the newest message is the whole of what is needed here.
                guard let last = messages.last else { return }
                withAnimation(.easeOut(duration: 0.18)) {
                    proxy.scrollTo(last.localID, anchor: .bottom)
                }
            }
        }
    }

    private var composer: some View {
        VStack(alignment: .leading, spacing: 6) {
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
            HStack(alignment: .bottom, spacing: 8) {
                Button {
                    pickAttachment()
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
                .buttonStyle(.borderless)
                .help("Attach a photo, video or file")
                // Editing borrows the composer to rewrite an existing
                // message; there is no second attachment to add, and
                // attaching would post it as a new message while leaving the
                // edit banner armed. iOS and Android already gated this.
                .disabled(editTarget != nil)

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
    }

    private var canSend: Bool {
        !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !isSending
    }

    /// Who a quoted sender is, matching the phone's wording.
    private func quoteAuthorName(_ userID: Int64) -> String {
        if userID == coordinator.currentUserID { return String(localized: "You") }
        return memberNames[userID] ?? String(localized: "Someone")
    }

    private var typingLine: String? {
        let ids = coordinator.typingUserIDs(in: chatID)
        guard !ids.isEmpty else { return nil }
        let names = ids.map { memberNames[$0] ?? "Someone" }
        return names.count == 1 ? "\(names[0]) is typing…" : "\(names.joined(separator: ", ")) are typing…"
    }

    private func claimActive() {
        coordinator.activeChatID = chatID
    }

    /// Only if it is still ours — another window may have claimed it since.
    private func releaseActive() {
        if coordinator.activeChatID == chatID {
            coordinator.activeChatID = nil
        }
    }

    private func send() {
        let body = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !body.isEmpty else { return }

        // Edit mode borrows the composer. The field clears only once the
        // server takes it — a refused edit leaves the text there to fix,
        // which is the phone's rule too.
        if let target = editTarget {
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

        // Read and clear together: the quote belongs to the message being
        // sent, and leaving it primed would silently quote the next one.
        let quote = replyDraft
        draft = ""
        replyDraft = nil
        coordinator.send(body: body, in: chatID, replyTo: quote)
    }

    private func beginReply(_ message: MessageSnapshot) {
        guard let serverID = message.serverID else { return }
        editTarget = nil
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
            mediaNotice = "Preparing…"
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
        mediaNotice = "Preparing…"
        // Taken NOW, before the await, and put back only if the send never
        // happens — the phone's rule (ConversationView.takeComposer), and it
        // fixes two Mac-only bugs at once. Reading `draft` AFTER prepare
        // swallowed anything typed while it ran into the caption, and the
        // primed reply was dropped entirely while its banner stayed armed,
        // so the NEXT ordinary message silently became that reply.
        let caption = draft
        let quote = replyDraft
        draft = ""
        replyDraft = nil
        Task {
            defer { mediaNotice = nil }
            do {
                let prepared = try await MediaPrep.prepare(fileAt: url, limit: MediaPrep.sizeLimit)
                mediaNotice = "Sending…"
                if await coordinator.sendMedia(
                    prepared, caption: caption, replyTo: quote, in: chatID) == false
                {
                    mediaNotice = "Couldn't send that."
                    restoreComposer(caption: caption, quote: quote)
                }
            } catch MediaPrep.PrepError.tooLargeAfterCompression {
                mediaNotice = "That file is over the 100 MB limit."
                restoreComposer(caption: caption, quote: quote)
            } catch {
                mediaNotice = "Couldn't read that file."
                restoreComposer(caption: caption, quote: quote)
            }
        }
    }

    /// Put the composer back when the send never happened — but never over
    /// something typed in the meantime.
    private func restoreComposer(caption: String, quote: ReplyToDTO?) {
        if draft.isEmpty { draft = caption }
        if replyDraft == nil { replyDraft = quote }
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
