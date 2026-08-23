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

    /// Who you are, at the top, with a face — the rows underneath are then
    /// only the things you can DO, which is what a settings sheet is for.
    private var identityHeader: some View {
        HStack(spacing: 12) {
            InitialsAvatar(
                title: session.currentUser?.displayName ?? "?",
                userID: session.currentUser?.id,
                avatarVersion: session.currentUser?.avatarVersion ?? 0,
                size: 56)
            VStack(alignment: .leading, spacing: 2) {
                Text(session.currentUser?.displayName ?? "—")
                    .font(.title3.weight(.semibold))
                if let user = session.currentUser {
                    Text("@\(user.username)")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                if session.isOwner {
                    Text("Owner")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.tint)
                        .padding(.horizontal, 7)
                        .padding(.vertical, 2)
                        .background(.tint.opacity(0.15), in: Capsule())
                        .padding(.top, 2)
                }
            }
            Spacer()
        }
        .padding(16)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            identityHeader
            Divider()
            Form {
                Section("Profile") {
                    Button("Change Password…") { changingPassword = true }
                }
                Section("Family") {
                    LabeledContent("Family", value: session.family?.name ?? "—")
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
        .background(Color.appGroupedBackground)
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
