//
//  MacSettingsView.swift
//  FamilyConnect
//
//  Settings on the Mac: profile, family, server, session.
//
//  A sheet rather than a Settings scene, because most of what is here is
//  account state (who you are, which family, which server) rather than
//  preferences — and because it has to be reachable while the app is on
//  its own window, which a Settings scene is not on every macOS version.
//

#if os(macOS)

import SwiftUI

struct MacSettingsView: View {
    @Environment(AppSession.self) private var session
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss

    @State private var changingPassword = false
    @State private var confirmLogout = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Form {
                Section("Profile") {
                    if let user = session.currentUser {
                        LabeledContent("Name", value: user.displayName)
                        LabeledContent("Username", value: "@\(user.username)")
                    }
                    Button("Change Password…") { changingPassword = true }
                }
                Section("Family") {
                    LabeledContent("Family", value: session.family?.name ?? "—")
                    if session.isOwner {
                        Text("You are the owner.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                Section("Server") {
                    LabeledContent(
                        "Address",
                        value: AppSettings.serverURL?.absoluteString ?? "—")
                }
                Section {
                    Button("Log Out", role: .destructive) { confirmLogout = true }
                }
            }
            .formStyle(.grouped)
            Divider()
            HStack {
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
            .padding(12)
        }
        .frame(width: 460, height: 420)
        .sheet(isPresented: $changingPassword) {
            ChangePasswordView()
                .frame(width: 420)
        }
        .confirmationDialog(
            "Log out?",
            isPresented: $confirmLogout,
            titleVisibility: .visible
        ) {
            Button("Log Out", role: .destructive) {
                Task { await session.logout() }
            }
        } message: {
            Text("Local messages are removed from this Mac.")
        }
    }
}

#endif
