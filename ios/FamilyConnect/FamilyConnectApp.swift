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
import os

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
        //
        // DEBUG-only, because what it does is destructive and it is driven
        // by a launch argument — a thing a person can pass to a shipped
        // app. It wipes every default, deletes the keychain token AND
        // deletes the message store (below), so on a Release build it is a
        // one-flag "sign me out, forget my server and throw the cache
        // away" that nothing in the UI offers. Nothing legitimate needs
        // it there: `DEBUG` is defined only by the project-level Debug
        // configuration, and both schemes' TestAction builds Debug, so
        // every UI test that passes this argument still gets it.
        #if DEBUG
        let uiTestReset = CommandLine.arguments.contains("--uitest-reset")
        if uiTestReset {
            AppSettings.wipe(keepServerURL: false)
            try? KeychainStore.delete(account: KeychainStore.tokenAccount)
        }
        #endif

        let schema = Schema([
            ChatEntity.self, MessageEntity.self, MemberEntity.self, NoteEntity.self,
            PendingMediaItemEntity.self,
            BlockEntity.self,
        ])
        let configuration = ModelConfiguration(
            schema: schema,
            isStoredInMemoryOnly: false,
            cloudKitDatabase: .none
        )
        // …AND the message cache, which the wipe above cannot reach: it
        // clears UserDefaults and the keychain token, both of which live
        // somewhere else entirely (#55).
        //
        // The gap was not academic. `--uitest-reset` is documented, and
        // used, as "a clean slate"; what it actually produced was a
        // signed-out app still holding the previous fixture's rows, and
        // SwiftData has no reason to distrust them. A chat id reused by a
        // different seed therefore rendered the OTHER fixture's thread —
        // one store-screenshot run photographed 1500 "Message number N"
        // bubbles inside "The Harpers", and the images looked entirely
        // plausible.
        //
        // Deleting the FILE rather than the rows, and here rather than
        // after the container opens, because this must also clear a store
        // whose schema no longer matches — the case where opening it is
        // the thing that fails. Same three files as the corruption path
        // below (`.store`, `-wal`, `-shm`); a missing one is not an error.
        #if DEBUG
        if uiTestReset { Self.deleteStore(at: configuration.url) }
        #endif
        // One delete-and-retry before giving up. A store that will not open
        // is not a disaster here: every row in it is a CACHE of something
        // the server still has, which is exactly what the error view has
        // always told people. What WAS a disaster is the state this
        // replaces — a permanent dead end on every launch, whose only
        // suggested escape (reinstall) does not even clear the store on
        // macOS, where the app is sandboxed and its container survives
        // deleting the app.
        //
        // Deliberately unconditional on the error: SwiftData reports a
        // corrupt file, a failed migration and an unreadable directory in
        // ways that are not worth pattern-matching, and the recovery is the
        // same for all of them. Anything genuinely unrecoverable — a full
        // disk, a broken sandbox — fails the retry too and still lands on
        // the error view.
        var result = Result {
            try ModelContainer(for: schema, configurations: [configuration])
        }
        if case .failure(let first) = result {
            AppLog.app.error(
                "Message store would not open (\(String(describing: first), privacy: .public)); deleting it and retrying once")
            Self.deleteStore(at: configuration.url)
            result = Result {
                try ModelContainer(for: schema, configurations: [configuration])
            }
            if case .failure(let second) = result {
                AppLog.app.error(
                    "Message store still would not open after a reset (\(String(describing: second), privacy: .public))")
            } else {
                AppLog.app.info("Message store reset; the cache will re-download")
            }
        }
        self.containerResult = result

        // The Share Extension's leftovers, once per launch (issue #35).
        // A hand-off that never completed — the open was dropped, this
        // process was killed mid-import, the app was simply not opened —
        // leaves its staged files in the App Group container, and nothing
        // else in either process will ever look at them again.
        //
        // Detached, so the launch path neither waits for it nor can be
        // failed by it: this is IO on a path that already does IO, and
        // there is nothing here that needs its answer. Order against an
        // incoming hand-off does not matter either — a share this app is
        // about to import is seconds old, and the sweep only takes what
        // is a day old (ShareHandoff.stagingGrace).
        //
        // Deliberately OUTSIDE the store branch below: the group
        // container is not the message store's, and a launch that cannot
        // open the store is still a launch that should not leak.
        Task.detached(priority: .utility) { _ = ShareHandoff.sweepOrphanedStaging() }

        if let container = try? result.get() {
            let coordinator = ChatSyncCoordinator(modelContainer: container)
            let session = AppSession(api: coordinator.api)
            coordinator.bind(session: session)
            // BEFORE the first sync, and that ordering is the whole point:
            // `blocked_user_ids` only arrives with `GET /me`, so a cold
            // start — offline, or just slow — would draw every blocked
            // member's messages in full until it landed. The store already
            // holds the answer from last time; this is what reads it.
            coordinator.loadBlocksFromStore()

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
                // The unfinished half of any queued media send goes with
                // the messages it belonged to. Its FILES are removed
                // separately (`clearMediaOutbox`): deleting rows here
                // would otherwise strand directories nothing names.
                try? context.delete(model: PendingMediaItemEntity.self)
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
            // Composer litter: files staged for a set that was never sent,
            // orphaned by a kill. A process that has just started owns
            // none of them. This must NOT touch PendingMediaStaging —
            // those bytes belong to messages somebody pressed Send on.
            MediaOutbox.sweepOrphans()
            // Staging directories no row names any more: a send that was
            // delivered while the process died between deleting its rows
            // and deleting its files, or a store the app had to recreate.
            // AFTER the container is open, because it needs the rows to
            // know what to keep.
            let liveItemIDs = Set(((try? container.mainContext.fetch(
                FetchDescriptor<PendingMediaItemEntity>())) ?? []).map(\.itemID))
            PendingMediaStaging.sweepOrphans(keeping: liveItemIDs)
            // A queued media send composed in one account must never reach
            // the next: rows AND the bytes they own.
            session.clearMediaOutbox = {
                let context = container.mainContext
                let staged = (try? context.fetch(
                    FetchDescriptor<PendingMediaItemEntity>())) ?? []
                for item in staged { PendingMediaStaging.remove(itemID: item.itemID) }
                try? context.delete(model: PendingMediaItemEntity.self)
                try? context.save()
            }
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
        #if os(macOS)
        // THE `id:` IS THE FIX FOR #52, and it is not cosmetic.
        //
        // macOS saves and restores windows by a per-window restoration
        // identifier, and for a SwiftUI WindowGroup WITHOUT an id that
        // identifier is the mangled Swift type name of the window's content.
        // This app's content type is a `_ConditionalContent` over
        // `windowContents`, and it contains two types with no stable mangled
        // name — the fileprivate `StoreErrorView` and SwiftData's own
        // `PassthroughModelContainerViewModifier` — which the runtime spells
        // as `(unknown context at $103eaa038)`: a RUN-TIME ADDRESS. Measured
        // on this tree, that made the saved identifier different on every
        // single launch, so AppKit asked SwiftUI to restore a window it could
        // no longer recognise and got nil back:
        //
        //     restoreWindowWithIdentifier:…-AppWindow-1
        //         className=SwiftUI.AppWindowsController
        //     …_block_invoke … window=0x0 error=(null)
        //
        // AppKit opens NOTHING in place of a restore that returns nil, so a
        // launch could reach a live run loop with a menu bar and zero
        // windows — no UI to click, and a Dock icon that is already running.
        // With this id the identifier is simply `main-AppWindow-1`, and the
        // same measurement shows the restore handing back a real window.
        //
        // macOS only, and the group is spelled twice for that reason: the id
        // buys nothing on iOS, where there is one scene and no AppKit window
        // restoration, and changing the identity of the iPhone app's only
        // scene is a risk with no matching return.
        WindowGroup(id: MacWindow.main) {
            windowContents { RootView() }
        }
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
        #else
        WindowGroup {
            windowContents { RootView() }
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

        // ⌘, and App menu → "Settings…", which is where a Mac's settings
        // live and which this app simply did not answer: the panel existed
        // only as a sheet on the main window, so the two standard doors
        // were dead keys. A Settings scene is also the only one of these
        // scenes macOS creates the menu item for by itself.
        //
        // `windowContents` for the same reason every other scene here uses
        // it: this window shows the signed-in identity, the family and the
        // server, and a settings window built on a second container would
        // be a second app's idea of all three.
        //
        // .defaultSize AND .contentMinSize, and both are load-bearing.
        // Measured on macOS 26: a Settings scene ignores the panel's ideal
        // frame and opens at the system default, 882x444 — twice the width
        // the grouped Form wants, and short enough to push its last section
        // (Delete Account) below the fold. .defaultSize is what opens it at
        // the 460x530 the panel was tuned to; .contentMinSize is what then
        // lets a person resize it, down to the floor MacSettingsView names.
        // .contentSize would pin it there forever, which is the sheet's own
        // defect wearing a title bar.
        Settings {
            windowContents { MacSettingsView() }
        }
        .defaultSize(width: 460, height: 530)
        .windowResizability(.contentMinSize)
        #endif
    }
}

#if os(macOS)
/// Window identifiers, in one place so the opener and the scene cannot
/// drift apart on a string.
enum MacWindow {
    /// The main window. Unlike the others this id is never passed to
    /// `openWindow` — it exists so the window has a STABLE macOS restoration
    /// identifier (#52). Changing this string retires every saved window
    /// people already have, exactly once; there is no reason to.
    static let main = "main"
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
/// Remove a SwiftData store and its sidecars.
///
/// SQLite keeps a write-ahead log and a shared-memory file beside the
/// database — `default.store-wal` and `default.store-shm`, the suffix
/// appended to the WHOLE filename rather than replacing the extension —
/// and leaving either behind can reproduce the very failure the delete is
/// meant to clear. A missing one is not an error.
///
/// This throws away nothing that cannot be re-downloaded: the store holds
/// chats, messages, members, notes and blocks, every one of which the
/// server can send again. That is the same promise the error view has
/// always made to the reader.
extension FamilyConnectApp {
    static func storeFiles(for url: URL) -> [URL] {
        let directory = url.deletingLastPathComponent()
        let name = url.lastPathComponent
        return [url] + ["-wal", "-shm"].map {
            directory.appendingPathComponent(name + $0)
        }
    }

    static func deleteStore(at url: URL) {
        for path in storeFiles(for: url) {
            try? FileManager.default.removeItem(at: path)
        }
    }
}

/// What to tell someone whose message store will not open, even after the
/// app has already deleted it and rebuilt it once.
///
/// Platform-specific because the escape hatch is. On iOS, deleting the app
/// takes its container with it. On macOS the app is sandboxed and its store
/// lives in ~/Library/Containers, which SURVIVES deleting the app — so the
/// reinstall advice this replaces was not merely unhelpful there, it was
/// wrong, and it was the only thing the view offered.
///
/// Not nested in the view: the view is private, and this sentence is worth a
/// test — getting it wrong is invisible until somebody is already stuck.
enum StoreErrorAdvice {
    static var text: String {
        #if os(macOS)
        String(localized: "Family Connect already tried resetting the store and it still won't open. Quit and reopen the app; if that doesn't help, remove its folder in ~/Library/Containers. Your messages are safe on the family server and will re-download.")
        #else
        String(localized: "Family Connect already tried resetting the store and it still won't open. Reinstall the app to start fresh. Your messages are safe on the family server and will re-download.")
        #endif
    }
}

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
            Text(StoreErrorAdvice.text)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 32)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.appBackground)
    }
}
