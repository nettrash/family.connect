//
//  ConnectionBanner.swift
//  FamilyConnect
//
//  The thin capsule that tells the truth about the wire. Hidden while
//  connected (the healthy state needs no chrome); "Connecting…" while the
//  socket's reconnect loop is working; "Offline" while deliberately
//  suspended. .thinMaterial so it reads over any list content without a
//  hard-coded background color.
//

import SwiftUI

struct ConnectionBanner: View {
    @Environment(ChatSyncCoordinator.self) private var coordinator

    var body: some View {
        if let text = label {
            HStack(spacing: 6) {
                if coordinator.connectionState == .connecting {
                    ProgressView()
                        .controlSize(.mini)
                } else {
                    Image(systemName: "wifi.slash")
                        .font(.caption2)
                }
                Text(text)
                    .font(.caption)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background(.thinMaterial, in: Capsule())
            .transition(.move(edge: .top).combined(with: .opacity))
        }
    }

    private var label: String? {
        switch coordinator.connectionState {
        case .connected: nil
        case .connecting: String(localized: "Connecting…")
        case .offline: String(localized: "Offline")
        }
    }
}
