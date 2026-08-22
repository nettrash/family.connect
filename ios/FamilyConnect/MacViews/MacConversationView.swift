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
    /// The attachment open at full size.
    @State private var viewingAttachment: AttachmentDTO?
    @FocusState private var composerFocused: Bool

    init(chatID: Int64) {
        self.chatID = chatID
        _messages = Query(
            filter: #Predicate<MessageEntity> { $0.chatID == chatID },
            sort: [SortDescriptor(\MessageEntity.createdAt), SortDescriptor(\MessageEntity.localID)])
        _chats = Query(filter: #Predicate<ChatEntity> { $0.chatID == chatID })
    }

    private var chat: ChatEntity? { chats.first }

    private var memberNames: [Int64: String] {
        Dictionary(uniqueKeysWithValues: members.map { ($0.userID, $0.displayName) })
    }

    var body: some View {
        VStack(spacing: 0) {
            thread
            Divider()
            composer
        }
        .navigationTitle(chat?.title ?? "")
        .navigationSubtitle(typingLine ?? "")
        .task(id: chatID) {
            coordinator.activeChatID = chatID
            // One page is enough to open with; the phone's paging sentinel
            // is a scroll-position mechanism this window does not need yet.
            if messages.isEmpty { _ = await coordinator.loadOlder(chatID: chatID) }
        }
        .onDisappear { coordinator.activeChatID = nil }
        .sheet(item: $viewingAttachment) { attachment in
            MacAttachmentViewer(attachment: attachment)
        }
    }

    private var thread: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 2) {
                    ForEach(messages) { message in
                        MacMessageRow(
                            message: message,
                            senderName: memberNames[message.senderID],
                            isMine: message.senderID == coordinator.currentUserID,
                            onReply: { beginReply(message) },
                            onEdit: { beginEdit(message) },
                            onOpenAttachment: { attachment in
                                if attachment.isFile {
                                    openFile(attachment)
                                } else {
                                    viewingAttachment = attachment
                                }
                            })
                            .id(message.localID)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .defaultScrollAnchor(.bottom)
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
                }
                .buttonStyle(.borderless)
                .help("Attach a photo, video or file")

                TextField("Message", text: $draft, axis: .vertical)
                    .textFieldStyle(.plain)
                    .lineLimit(1...8)
                    .focused($composerFocused)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(Color.appSecondaryFill, in: RoundedRectangle(cornerRadius: 8))
                    // Return sends, Shift-Return newlines — the Mac idiom,
                    // and the reason the field is multi-line at all.
                    .onSubmit(send)

                Button(action: send) {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.system(size: 20))
                }
                .buttonStyle(.borderless)
                .keyboardShortcut(.return, modifiers: [])
                .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || isSending)
            }
        }
        .padding(12)
    }

    private var typingLine: String? {
        let ids = coordinator.typingUserIDs(in: chatID)
        guard !ids.isEmpty else { return nil }
        let names = ids.map { memberNames[$0] ?? "Someone" }
        return names.count == 1 ? "\(names[0]) is typing…" : "\(names.joined(separator: ", ")) are typing…"
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

    private func beginReply(_ message: MessageEntity) {
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

    private func beginEdit(_ message: MessageEntity) {
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
        Task {
            defer { mediaNotice = nil }
            do {
                let prepared = try await MediaPrep.prepare(fileAt: url, limit: MediaPrep.sizeLimit)
                mediaNotice = "Sending…"
                let caption = draft
                draft = ""
                if await coordinator.sendMedia(prepared, caption: caption, in: chatID) == false {
                    mediaNotice = "Couldn't send that."
                    draft = caption
                }
            } catch MediaPrep.PrepError.tooLargeAfterCompression {
                mediaNotice = "That file is over the 100 MB limit."
            } catch {
                mediaNotice = "Couldn't read that file."
            }
        }
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

#endif
