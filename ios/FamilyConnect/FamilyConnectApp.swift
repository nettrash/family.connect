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
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    /// Result of the SwiftData container construction, captured at init
    /// so the scene can choose between the app and a recoverable error
    /// view. Never fatalError.
    private let containerResult: Result<ModelContainer, Error>

    /// nil only when the container failed; the scene then shows
    /// StoreErrorView and never reaches the bound state.
    private let session: AppSession?
    private let coordinator: ChatSyncCoordinator?
    private let pushRegistrar: PushRegistrar?

    /// App-wide link-preview cache. Independent of the store, so it is
    /// built even when the container failed (the error view just never
    /// reads it).
    @State private var previewLoader = LinkPreviewLoader()

    /// App-wide profile-picture cache, sharing the coordinator's API
    /// client so it inherits the same server URL and session token. A
    /// stored property, not @State: rebuilding it per render would throw
    /// the cache away every time anything above it changed.
    private let avatars: AvatarStore?

    init() {
        // UI-test hook: launch with a clean slate so the smoke test can
        // assert the server-setup screen deterministically.
        if CommandLine.arguments.contains("--uitest-reset") {
            AppSettings.wipe(keepServerURL: false)
            try? KeychainStore.delete(account: KeychainStore.tokenAccount)
        }

        let schema = Schema([ChatEntity.self, MessageEntity.self, MemberEntity.self, NoteEntity.self])
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
            session.clearChatStore = {
                let context = container.mainContext
                try? context.delete(model: MessageEntity.self)
                try? context.delete(model: ChatEntity.self)
                try? context.delete(model: MemberEntity.self)
                try? context.delete(model: NoteEntity.self)
                try? context.save()
            }

            // Push: the registrar shares the coordinator's APIClient (same
            // token, same server) and meets the rest of the app only
            // through closures and the AppDelegate statics — set here,
            // before UIApplicationMain delivers any delegate callback.
            let registrar = PushRegistrar(api: coordinator.api)
            coordinator.ensurePushRegistration = { await registrar.ensureRegistered() }
            session.deregisterDevice = { await registrar.deregister() }
            AppDelegate.registrar = registrar
            AppDelegate.session = session

            self.session = session
            self.coordinator = coordinator
            self.pushRegistrar = registrar
            let avatars = AvatarStore(api: coordinator.api)
            // Logout wipes the store; faces must go with it, or the next
            // account inherits this one's.
            session.clearAvatarCache = { avatars.clear() }
            // A 401 on a picture is as final as a 401 on a message.
            avatars.onUnauthorized = { [weak session] in session?.handleUnauthorized() }
            self.avatars = avatars
        } else {
            self.session = nil
            self.coordinator = nil
            self.pushRegistrar = nil
            self.avatars = nil
        }
    }

    var body: some Scene {
        WindowGroup {
            switch containerResult {
            case .success(let container):
                if let session, let coordinator {
                    RootView()
                        .modelContainer(container)
                        .environment(session)
                        .environment(coordinator)
                        // One preview cache for the app: a link posted in
                        // a busy chat is fetched once, not once per bubble.
                        .environment(previewLoader)
                        .environment(avatars ?? AvatarStore(api: coordinator.api))
                }
            case .failure(let error):
                StoreErrorView(error: error)
            }
        }
    }
}

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
        .background(Color(.systemBackground))
    }
}
