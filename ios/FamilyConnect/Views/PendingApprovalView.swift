//
//  PendingApprovalView.swift
//  FamilyConnect
//
//  The waiting room after a join request against an approval-policy
//  family. There is no push in v1, so the client polls GET /me every 5 s
//  while this screen is foregrounded (the .task cancels with the view,
//  so backgrounding stops the polling for free) plus pull-to-refresh for
//  the impatient. AppSession.pollPending routes all three outcomes:
//  approved → active, still pending → stay, vanished → declined message
//  on the family gate.
//

import SwiftUI

struct PendingApprovalView: View {
    @Environment(AppSession.self) private var session

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 20) {
                    Image(systemName: "hourglass")
                        .font(.system(size: 52))
                        .foregroundStyle(.tint)
                        .padding(.top, 60)
                    Text("Waiting for approval")
                        .font(.title2.bold())
                    Text("The family owner needs to approve your request. This screen updates automatically — or pull down to check right now.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 32)
                    ProgressView()
                        .padding(.top, 8)

                    Button("Log out", role: .destructive) {
                        Task { await session.logout() }
                    }
                    .font(.footnote)
                    .padding(.top, 32)
                }
                .frame(maxWidth: .infinity)
            }
            .refreshable {
                await session.pollPending()
            }
            .task {
                // Foreground poll loop; cancelled automatically when the
                // view disappears (approval navigates away, backgrounding
                // tears the task down).
                while !Task.isCancelled {
                    await session.pollPending()
                    try? await Task.sleep(nanoseconds: 5_000_000_000)
                }
            }
            .navigationTitle("Almost There")
        }
    }
}
