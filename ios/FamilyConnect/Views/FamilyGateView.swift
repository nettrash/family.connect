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

                // A server closed to NEW families (docs/protocol.md,
                // "Starting a family") shows the door shut, and where to
                // build one's own, instead of a Create row that would end
                // in a 403 after somebody has typed a name. Joining stays:
                // the families already here are what the server is for.
                if !session.familyRegistrationEnabled {
                    Section {
                        Label {
                            VStack(alignment: .leading, spacing: 4) {
                                Text("This server doesn't take new families.")
                                    .font(.headline)
                                Text("Family Connect is built for one family on a server of its own. To start yours, run your own server and invite everyone from there.")
                                    .font(.callout)
                                    .foregroundStyle(.secondary)
                            }
                        } icon: {
                            Image(systemName: "server.rack")
                        }
                        Link(destination: AppVersion.repositoryURL) {
                            Label("How to run your own server", systemImage: "arrow.up.right.square")
                        }
                    }
                }

                Section {
                    if session.familyRegistrationEnabled {
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
                } footer: {
                    // The deadline, said before it is met: this server
                    // removes an account that stays without a family past
                    // its grace (docs/protocol.md, "Accounts without a
                    // family"). 0 is a server that never does, and says nothing.
                    if session.familylessAccountTTLDays > 0 {
                        Text("An account that doesn't join a family within \(session.familylessAccountTTLDays) days is removed from this server.")
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
