//
//  FamilyConnectApp.swift
//  FamilyConnect
//
//  Composition root. The SwiftData container is built as a Result in
//  init() (the exchange-ios pattern) so a corrupt on-disk store shows a
//  recoverable StoreErrorView instead of a fatalError crash loop. The two
//  long-lived objects — AppSession (phase machine) and ChatSyncCoordinator
//  (wire ↔ store) — are created here once and injected through the
//  SwiftUI Environment; the coordinator owns the APIClient and the
//  ChatSocket, and AppSession shares that same APIClient so a token set
//  at login is visible to the sync layer without any signalling.
//
//  CloudKit is explicitly off: the server is the sync authority for chat
//  data (protocol.md), and mirroring the same rows through a second
//  channel would only manufacture conflicts.
//
//  Push wiring also happens here: PushRegistrar owns the token
//  lifecycle, AppDelegate (the adaptor below) surfaces the UIKit-only
//  callbacks, and the two closure seams — the coordinator's resync hook
//  and the session's logout hook — are tied to the registrar so neither
//  of those objects grows a UIKit dependency.
//

import SwiftData
import SwiftUI

@main
struct FamilyConnectApp: App {
    /// Surfaces the APNs token callbacks and owns the notification-center
    /// delegate; see AppDelegate.swift.
    #if os(iOS)
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    #elseif os(macOS)
    @NSApplicationDelegateAdaptor(MacAppDelegate.self) private var appDelegate
    #endif

    /// Result of the SwiftData container construction, captured at init
    /// so the scene can choose between the app and a recoverable error
    /// view. Never fatalError.
    private let containerResult: Result<ModelContainer, Error>

    /// nil only when the container failed; the scene then shows
    /// StoreErrorView and never reaches the bound state.
    private let session: AppSession?
    private let coordinator: ChatSyncCoordinator?
    private let pushRegistrar: PushRegistrar?
    /// The voice-call state machine, one per process like the session.
    private let callManager: CallManager?
    #if os(iOS)
    /// CallKit and PushKit, iOS only: the Mac holds its socket and draws
    /// its own window (docs/protocol.md, "Incoming calls").
    private let callKit: CallKitController?
    private let voipRegistrar: VoIPPushRegistrar?
    /// Siri's call intents, answered from the roster (CallIntents).
    private let callIntentHandler: CallIntentHandler?
    #endif

    /// App-wide link-preview cache. Independent of the store, so it is
    /// built even when the container failed (the error view just never
    /// reads it).
    @State private var previewLoader = LinkPreviewLoader()

    /// App-wide profile-picture cache, sharing the coordinator's API
    /// client so it inherits the same server URL and session token. A
    /// stored property, not @State: rebuilding it per render would throw
    /// the cache away every time anything above it changed.
    private let avatars: AvatarStore?
    private let attachments: AttachmentStore?

    init() {
        // UI-test hook: launch with a clean slate so the smoke test can
        // assert the server-setup screen deterministically.
        if CommandLine.arguments.contains("--uitest-reset") {
            AppSettings.wipe(keepServerURL: false)
            try? KeychainStore.delete(account: KeychainStore.tokenAccount)
        }

        let schema = Schema([
            ChatEntity.self, MessageEntity.self, MemberEntity.self, NoteEntity.self,
            BlockEntity.self,
        ])
        let configuration = ModelConfiguration(
            schema: schema,
            isStoredInMemoryOnly: false,
            cloudKitDatabase: .none
        )
        let result = Result {
            try ModelContainer(for: schema, configurations: [configuration])
        }
        self.containerResult = result

        if let container = try? result.get() {
            let coordinator = ChatSyncCoordinator(modelContainer: container)
            let session = AppSession(api: coordinator.api)
            coordinator.bind(session: session)

            // Store side effects for the phase machine, wired as closures
            // so AppSession itself stays SwiftData-free (and testable).
            session.hasCachedChats = {
                let count = (try? container.mainContext.fetchCount(FetchDescriptor<ChatEntity>())) ?? 0
                return count > 0
            }
            session.applyBlockedIDs = { [weak coordinator] ids in
                coordinator?.replaceBlocks(with: ids)
            }
            session.clearChatStore = {
                let context = container.mainContext
                try? context.delete(model: MessageEntity.self)
                try? context.delete(model: ChatEntity.self)
                try? context.delete(model: MemberEntity.self)
                try? context.delete(model: NoteEntity.self)
                // The block list goes too, and that is safe rather than
                // lossy: it is server state, replaced wholesale from
                // `blocked_user_ids` on the very first `GET /me` after the
                // next sign-in. Keeping it would leave one account's blocks
                // in force for whoever signs in next on this device.
                try? context.delete(model: BlockEntity.self)
                try? context.save()
                // This path deletes the rows directly rather than through
                // the coordinator, so nothing here would otherwise take the
                // Mac's Dock badge down — leaving a count from a session
                // that no longer exists on an app sitting at the login
                // screen.
                UnreadBadge.show(0)
            }

            // Push: the registrar shares the coordinator's APIClient (same
            // token, same server) and meets the rest of the app only
            // through closures and the AppDelegate statics — set here,
            // before UIApplicationMain delivers any delegate callback.
            let registrar = PushRegistrar(api: coordinator.api)
            coordinator.ensurePushRegistration = { await registrar.ensureRegistered() }
            session.deregisterDevice = { await registrar.deregister() }
            #if os(iOS)
            AppDelegate.registrar = registrar
            AppDelegate.session = session
            #elseif os(macOS)
            MacAppDelegate.registrar = registrar
            MacAppDelegate.session = session
            #endif

            // Calls. The manager talks to the world through closures and
            // two protocols, wired here so it stays free of the store, the
            // socket and every OS framework — see CallManager.
            let calls = CallManager()
            calls.signaling = coordinator
            calls.iceServers = { try await coordinator.api.iceServers().iceServers }
            calls.resolvePeer = { userID in
                let descriptor = FetchDescriptor<MemberEntity>(predicate: #Predicate { $0.userID == userID })
                if let member = try? container.mainContext.fetch(descriptor).first {
                    return (member.resolvedDisplayName, member.avatarVersion)
                }
                return (String(localized: "Someone"), 0)
            }
            calls.ensureConnected = {
                if session.phase == .booting { await session.bootstrap() }
                await coordinator.ensureConnected()
            }
            calls.onEnded = { coordinator.callDidEnd() }
            coordinator.bind(callManager: calls)
            #if os(iOS)
            let callKit = CallKitController()
            callKit.manager = calls
            calls.systemBridge = callKit
            let voip = VoIPPushRegistrar()
            voip.pushRegistrar = registrar
            voip.callManager = calls
            voip.callKit = callKit
            voip.start()
            self.callKit = callKit
            self.voipRegistrar = voip
            // Siri resolves a spoken name against the ACTIVE roster — the
            // member pickers' gate, so a name cannot resolve to somebody
            // who left or deleted their account.
            let intents = CallIntentHandler()
            intents.roster = {
                let descriptor = FetchDescriptor<MemberEntity>(
                    predicate: #Predicate { !$0.isCurrentUser && !$0.hasLeft && !$0.accountDeleted })
                let members = (try? container.mainContext.fetch(descriptor)) ?? []
                return members.map { CallRequestRouter.Candidate(userID: $0.userID, name: $0.resolvedDisplayName) }
            }
            AppDelegate.callIntentHandler = intents
            self.callIntentHandler = intents
            #elseif os(macOS)
            // The Mac rings through Notification Center: a socket-delivered
            // offer while the app is behind other windows — or has none
            // open — is otherwise silent.
            calls.onPhaseChange = { [weak calls] phase in
                guard let calls, let callID = calls.callID else { return }
                switch phase {
                case .incoming:
                    ChatNotifier.announceIncomingCall(
                        callID: callID, callerName: calls.peerName, video: calls.isVideo)
                default:
                    ChatNotifier.dismissIncomingCall(callID: callID)
                }
            }
            MacAppDelegate.callManager = calls
            #endif
            self.callManager = calls

            self.session = session
            self.coordinator = coordinator
            self.pushRegistrar = registrar
            let avatars = AvatarStore(api: coordinator.api)
            let attachments = AttachmentStore(api: coordinator.api)
            coordinator.bind(attachmentStore: attachments)
            // Logout wipes the store; faces must go with it, or the next
            // account inherits this one's.
            session.clearAvatarCache = {
                avatars.clear()
                attachments.clear()
            }
            // A 401 on a picture is as final as a 401 on a message.
            avatars.onUnauthorized = { [weak session] in session?.handleUnauthorized() }
            self.avatars = avatars
            self.attachments = attachments
        } else {
            self.session = nil
            self.coordinator = nil
            self.pushRegistrar = nil
            self.callManager = nil
            #if os(iOS)
            self.callKit = nil
            self.voipRegistrar = nil
            self.callIntentHandler = nil
            #endif
            self.avatars = nil
            self.attachments = nil
        }
    }

    /// Everything a window needs, applied identically to every scene.
    ///
    /// Factored out when the Mac gained a second window kind: a
    /// conversation window that did not carry the same model container and
    /// the same stores would be a second app looking at the same server —
    /// its own cache, its own sync, its own idea of what has been read.
    @ViewBuilder
    private func windowContents(@ViewBuilder _ content: () -> some View) -> some View {
        switch containerResult {
        case .success(let container):
            if let session, let coordinator, let callManager {
                content()
                    .modelContainer(container)
                    .environment(session)
                    .environment(coordinator)
                    .environment(callManager)
                    // One preview cache for the app: a link posted in
                    // a busy chat is fetched once, not once per bubble.
                    .environment(previewLoader)
                    .environment(avatars ?? AvatarStore(api: coordinator.api))
                    .environment(attachments ?? AttachmentStore(api: coordinator.api))
            }
        case .failure(let error):
            StoreErrorView(error: error)
        }
    }

    var body: some Scene {
        WindowGroup {
            windowContents { RootView() }
        }
        #if os(macOS)
        // A Mac window opens at a size somebody can actually read a
        // conversation in, rather than the square SwiftUI would pick.
        .defaultSize(width: 1000, height: 680)
        .commands {
            // The menu bar is not decoration on a Mac: it is where the
            // keyboard shortcuts live and where people look for what an
            // app can do.
            CommandGroup(after: .toolbar) {
                Button("Refresh") {
                    NotificationCenter.default.post(name: .macRequestResync, object: nil)
                }
                .keyboardShortcut("r", modifiers: .command)
            }
        }
        #endif

        #if os(macOS)
        // One conversation in a window of its own — the thing a Mac can do
        // that a phone cannot. Opened from the sidebar's context menu, and
        // keyed BY CHAT ID, which is what makes asking for the same chat
        // twice raise the existing window instead of opening a duplicate.
        WindowGroup(id: MacWindow.conversation, for: Int64.self) { $chatID in
            windowContents {
                if let chatID {
                    MacConversationView(chatID: chatID)
                        .id(chatID)
                } else {
                    // A restored window whose chat has since gone (left the
                    // family, or a fresh install) — say so rather than
                    // showing an empty thread.
                    ContentUnavailableView(
                        "Conversation unavailable",
                        systemImage: "bubble.left.and.bubble.right")
                }
            }
        }
        .defaultSize(width: 720, height: 640)
        .windowResizability(.contentMinSize)

        // The board and the photo viewer are WINDOWS, not sheets, and that
        // is not a style choice: a macOS sheet cannot be resized by the
        // person using it. A board sized once at 640x480 cuts off the notes
        // that do not fit, and a photo gets a fixed rectangle to be cropped
        // by, with no way to open it out.

        // One board per family, so a Window rather than a WindowGroup —
        // asking for it twice raises the one that is already open.
        Window("Board", id: MacWindow.board) {
            windowContents { MacBoardView() }
        }
        .defaultSize(width: 900, height: 620)
        .windowResizability(.contentMinSize)

        // One call at a time, so one window: opened when the manager
        // leaves idle and closed when it returns (RootView), never by the
        // person — closing it does not hang up.
        Window("Call", id: MacWindow.call) {
            // Video needs room; a voice call just centers in it. The
            // window is resizable down to the minimum the frame sets —
            // .contentSize would pin it at the default forever.
            windowContents {
                CallView()
                    .frame(minWidth: 480, minHeight: 400)
            }
        }
        .defaultSize(width: 780, height: 560)
        .windowResizability(.contentMinSize)

        // Keyed BY ALBUM AND INDEX — the message's media plus the one that
        // was clicked — so two photos open as two windows and the same
        // photo twice raises the first. Paging inside the window does not
        // move its key.
        WindowGroup(id: MacWindow.attachment, for: AttachmentAlbum.self) { $album in
            windowContents {
                if let album {
                    MacAttachmentViewer(album: album)
                } else {
                    ContentUnavailableView("Attachment unavailable", systemImage: "photo")
                }
            }
        }
        .defaultSize(width: 900, height: 700)
        .windowResizability(.contentMinSize)
        #endif
    }
}

#if os(macOS)
/// Window identifiers, in one place so the opener and the scene cannot
/// drift apart on a string.
enum MacWindow {
    static let conversation = "conversation"
    static let board = "board"
    static let attachment = "attachment"
    static let call = "call"
}
#endif

#if os(macOS)
extension Notification.Name {
    /// ⌘R. A notification rather than a binding because the command lives
    /// in the scene and the thing that can act on it is several views
    /// down; SwiftUI has no environment path from one to the other.
    static let macRequestResync = Notification.Name("me.nettrash.FamilyConnect.requestResync")
}
#endif

/// Shown when the SwiftData store can't be opened — recoverable messaging
/// instead of a crash. Stock components + semantic colors so it renders
/// correctly in both appearances without a custom palette.
private struct StoreErrorView: View {
    let error: Error

    var body: some View {
        VStack(spacing: 24) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 56))
                .foregroundStyle(.orange)
            VStack(spacing: 12) {
                Text("Couldn't open the message store.")
                    .font(.headline)
                    .multilineTextAlignment(.center)
                Text(error.localizedDescription)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            .padding(.horizontal, 32)
            Text("Reinstall Family Connect to start fresh. Your messages are safe on the family server and will re-download.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 32)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.appBackground)
    }
}
