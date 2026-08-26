//
//  RootView.swift
//  FamilyConnect
//
//  The phase switch: exactly one subtree per AppSession.Phase, so every
//  screen below can assume its preconditions (a token exists, a family
//  exists, …) instead of re-checking them. Also the single owner of the
//  scenePhase → coordinator lifecycle mapping: background suspends the
//  socket AND revokes the authority to mark anything read (see
//  ChatSyncCoordinator.enterBackground), active resumes the socket and
//  resyncs — the socket is a live wire, so everything missed while
//  suspended comes back over REST.
//
//  It is also where the app-icon badge is taken over from the system, once
//  per launch and before anything else runs. Until something SAVES, no
//  code in this app has touched the icon: on iOS it is still showing
//  whatever the last APNs push left there, which is a count of a server
//  state this device may never have seen; on the Mac it is showing
//  nothing, because `dockTile.badgeLabel` is per-process and starts nil.
//  Here, rather than in FamilyConnectApp.init, because `NSApp` does not
//  exist yet at init time and the Mac's half would silently do nothing.
//  See UnreadBadge for why the seed is the store's number and not a clear.
//

import SwiftUI

struct RootView: View {
    @Environment(AppSession.self) private var session
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        Group {
            switch session.phase {
            // Everything before the chat itself is the phone's screen, held
            // to a readable column on the Mac (see setupColumn).
            case .booting:
                BootingView()
                    .setupColumn()
            case .needsServer:
                ServerSetupView()
                    .setupColumn()
            case .needsAuth:
                AuthView()
                    .setupColumn()
            case .needsFamily:
                FamilyGateView()
                    .setupColumn()
            case .pendingApproval:
                PendingApprovalView()
                    .setupColumn()
            case .active:
                // The one case that differs: iOS pushes a list, the Mac
                // shows a sidebar and a thread side by side.
                #if os(iOS)
                ChatListView()
                #else
                MacChatView()
                #endif
            }
        }
        .task {
            // Before bootstrap on purpose: this is the number that has to
            // be right on a launch with no network at all, where the store
            // is the only truth there is and no resync is coming to
            // correct it. It can be LOWER than a correct pushed number for
            // one round trip — the store cannot know about messages that
            // arrived while the process was dead — and that is invisible,
            // because the icon is behind the app that is drawing it.
            coordinator.refreshUnreadBadge()
            await session.bootstrap()
        }
        .onChange(of: session.phase) { oldPhase, newPhase in
            if newPhase == .active {
                Task { await coordinator.activate() }
            } else if oldPhase == .active {
                coordinator.deactivate()
            }
        }
        .onChange(of: scenePhase) { _, newPhase in
            // Nothing clears the badge here any more, on either platform.
            // iOS used to wipe it on every single foreground, on the theory
            // that the number belonged to the push payload and the server
            // would send a fresh one. It does not: the server pushes only
            // to a device with no live socket, so a foregrounded phone is
            // never sent another badge and the icon stayed bare while the
            // chat list still showed three unread. Both platforms now
            // derive the number from the store instead (UnreadBadge, fed
            // from saveContext), which needs no permission on the Mac and
            // no push on the phone — and coming back to an app is not
            // reading anything anyway (ChatPresence).
            guard session.phase == .active else { return }
            switch newPhase {
            case .background:
                coordinator.enterBackground()
            case .active:
                coordinator.resumeForeground()
            default:
                break
            }
        }
    }
}

/// The `.booting` subtree: a spinner while bootstrap runs; if it failed
/// with no cached chats to fall back on, the error plus a Retry button.
private struct BootingView: View {
    @Environment(AppSession.self) private var session

    var body: some View {
        VStack(spacing: 16) {
            if let error = session.bootError {
                Image(systemName: "wifi.exclamationmark")
                    .font(.system(size: 44))
                    .foregroundStyle(.secondary)
                Text("Can't reach the family server")
                    .font(.headline)
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 32)
                Button("Retry") {
                    Task { await session.retryBootstrap() }
                }
                .buttonStyle(.borderedProminent)
            } else {
                ProgressView()
                Text("Connecting…")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.appBackground)
    }
}

#Preview {
    RootView()
}
