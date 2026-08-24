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
    @State private var recorder = AudioRecorder()
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
                                })
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
            .onChange(of: messages.count) { oldCount, newCount in
                // Grow the window over what just arrived, or new messages
                // fall outside the rendered slice and the thread looks
                // stuck. Only ever forward — a reader who paged back keeps
                // what they widened to — but CAPPED, which is the point of
                // having a window at all: this stack is non-lazy, so an
                // uncapped count would creep up over a long session until
                // the layout cost that caused the hang came back. Past the
                // cap the window SLIDES instead, keeping the newest rows.
                if newCount > oldCount {
                    visibleCount = min(
                        min(newCount, visibleCount + (newCount - oldCount)), Self.maxWindow)
                }
                // Follow the newest message for a reader who was ALREADY at
                // the bottom — yanking somebody out of history because
                // somebody else typed is the bug the phone fixed long ago —
                // OR when the newest message is this member's OWN. Pressing
                // Send and watching nothing happen is worse than either.
                let mine = messages.last?.senderID == coordinator.currentUserID
                guard isPinnedToBottom || mine else { return }
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
                guard let viewport = geometry.bounds(of: .scrollView) else { return true }
                let y = geometry.frame(in: .scrollView).minY
                return y <= viewport.height + 40
            } action: { visible in
                isPinnedToBottom = visible
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
            visibleCount = min(min(messages.count, visibleCount + Self.windowStep), Self.maxWindow)
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
            HStack(alignment: .bottom, spacing: 8) {
                Menu {
                    Button {
                        pickAttachment()
                    } label: {
                        Label("Attach a File…", systemImage: "doc")
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
    }

    private var canSend: Bool {
        !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !isSending
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

    /// Stop recording and send it. The Mac composer has no staging step
    /// yet, so a voice note goes as its own message with whatever is typed
    /// — the same shape `pickAttachment` uses.
    private func finishRecording() {
        guard let url = recorder.stop() else {
            mediaNotice = "That recording was too short."
            return
        }
        mediaNotice = "Sending…"
        let caption = draft
        let quote = replyDraft
        draft = ""
        replyDraft = nil
        Task {
            defer { mediaNotice = nil }
            do {
                let prepared = try await MediaPrep.prepareAudio(
                    from: url, limit: MediaPrep.sizeLimit)
                if await coordinator.sendMedia(
                    prepared, caption: caption, replyTo: quote, in: chatID) == false
                {
                    mediaNotice = "Couldn't send that."
                    restoreComposer(caption: caption, quote: quote)
                }
            } catch {
                mediaNotice = "Couldn't prepare that item."
                try? FileManager.default.removeItem(at: url)
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
