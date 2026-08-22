//
//  RootView.swift
//  FamilyConnect
//
//  The phase switch: exactly one subtree per AppSession.Phase, so every
//  screen below can assume its preconditions (a token exists, a family
//  exists, …) instead of re-checking them. Also the single owner of the
//  scenePhase → coordinator lifecycle mapping: background suspends the
//  socket, active resumes it and resyncs (the socket is a live wire —
//  everything missed while suspended comes back over REST), and
//  becoming active always clears the app badge — the number meant
//  "unread while you were away", and the server recomputes it fresh on
//  its next push anyway.
//

import SwiftUI
import UserNotifications

struct RootView: View {
    @Environment(AppSession.self) private var session
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        Group {
            switch session.phase {
            case .booting:
                BootingView()
            case .needsServer:
                ServerSetupView()
            case .needsAuth:
                AuthView()
            case .needsFamily:
                FamilyGateView()
            case .pendingApproval:
                PendingApprovalView()
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
            if newPhase == .active {
                // Regardless of session phase — a badge on the icon of a
                // logged-out app would be just as stale.
                Task { try? await UNUserNotificationCenter.current().setBadgeCount(0) }
            }
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
