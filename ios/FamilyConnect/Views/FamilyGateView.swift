//
//  FamilyGateView.swift
//  FamilyConnect
//
//  The "signed in, no family yet" fork: create one (become the owner) or
//  join one by invite code. A NavigationStack with two plain
//  NavigationLinks — the fork itself has no state worth a model. This is
//  also where a declined join request is surfaced: AppSession sets
//  `joinDeclined` when a previously pending request vanished from /me
//  (the protocol's only rejection signal), and we show it once, here,
//  because this is exactly the screen the user lands back on.
//

import SwiftUI

struct FamilyGateView: View {
    @Environment(AppSession.self) private var session

    var body: some View {
        NavigationStack {
            List {
                if session.joinDeclined {
                    Section {
                        Label {
                            Text("Your request to join was declined. You can ask for a new invite code and try again.")
                        } icon: {
                            Image(systemName: "hand.raised")
                                .foregroundStyle(.orange)
                        }
                        .font(.callout)
                    }
                }

                Section {
                    NavigationLink {
                        CreateFamilyView()
                    } label: {
                        Label {
                            VStack(alignment: .leading, spacing: 2) {
                                Text("Create a family")
                                Text("Start fresh — you'll be the owner and can invite everyone else.")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        } icon: {
                            Image(systemName: "house")
                        }
                    }

                    NavigationLink {
                        JoinFamilyView()
                    } label: {
                        Label {
                            VStack(alignment: .leading, spacing: 2) {
                                Text("Join a family")
                                Text("Enter the invite code a family member shared with you.")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        } icon: {
                            Image(systemName: "person.badge.key")
                        }
                    }
                } header: {
                    if let user = session.currentUser {
                        Text("Hi, \(user.displayName)")
                    }
                }

                Section {
                    Button("Log out", role: .destructive) {
                        Task { await session.logout() }
                    }
                    .font(.footnote)
                }
            }
            .navigationTitle("Your Family")
        }
    }
}
