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
    @State private var editingBirthday = false
    @State private var showingStatistics = false
    @State private var confirmLogout = false
    @State private var deletingAccount = false
    /// Mirrors AppSettings — defaults are not observable, so the view
    /// holds its own copy and writes through on change.
    @State private var mapPreviewsEnabled = AppSettings.mapPreviewsEnabled
    @State private var linkPreviewsEnabled = AppSettings.linkPreviewsEnabled

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

    /// The birthday as the family sees it, or the plain fact that there
    /// isn't one.
    private var birthdayText: String {
        session.currentUser?.birthday?.formatted() ?? String(localized: "Not set")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            identityHeader
            Divider()
            Form {
                Section("Profile") {
                    // Day and month, no year — so it can be shown to the
                    // family without publishing an age (protocol.md,
                    // "Birthdays").
                    LabeledContent("Birthday", value: birthdayText)
                    Button(
                        session.currentUser?.birthday == nil
                            ? "Add Birthday…" : "Change Birthday…"
                    ) { editingBirthday = true }
                    Button("Change Password…") { changingPassword = true }
                }
                Section("Family") {
                    LabeledContent("Family", value: session.family?.name ?? "—")
                }
                Section {
                    Button("Statistics…") { showingStatistics = true }
                }
                // The Mac's only setting that changes who the app talks
                // to, so it says so plainly. Link previews are not drawn
                // here at all, so there is nothing to switch for them; a
                // map on a shared location IS drawn, and drawing one asks
                // Apple for tiles.
                Section {
                    Toggle("Link Previews", isOn: $linkPreviewsEnabled)
                        .onChange(of: linkPreviewsEnabled) { _, newValue in
                            AppSettings.linkPreviewsEnabled = newValue
                        }
                    Toggle("Map Previews", isOn: $mapPreviewsEnabled)
                        .onChange(of: mapPreviewsEnabled) { _, newValue in
                            AppSettings.mapPreviewsEnabled = newValue
                        }
                } header: {
                    Text("Privacy")
                } footer: {
                    Text("Shows a preview under links in messages, and a map on a shared location. Building either asks somebody else for it — the linked website for its title and image, Apple for the map — so they see a request from this Mac. With maps off, a shared location still shows its pin and opens in Maps when you click it.")
                }

                Section("Server") {
                    LabeledContent(
                        "Address",
                        value: AppSettings.serverURL?.absoluteString ?? "—")
                }
                Section {
                    Button("Log Out", role: .destructive) { confirmLogout = true }
                    // The shared sheet does the explaining (see
                    // DeleteAccountView); this is the door, next to Log
                    // Out where somebody looks for it.
                    Button("Delete Account…", role: .destructive) { deletingAccount = true }
                } footer: {
                    Text(verbatim: AppVersion.settingsLine)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .center)
                        .padding(.top, 8)
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
        // Grown from 420 to make room for the two birthday rows, and
        // again to 530 for the Delete Account row. The Form would scroll
        // rather than clip them, but a settings sheet that has to be
        // scrolled to reach its last section is a settings sheet whose
        // last section nobody finds — and a Mac sheet cannot be resized
        // by the person using it.
        .frame(width: 460, height: 530)
        .background(Color.appGroupedBackground)
        .sheet(isPresented: $changingPassword) {
            ChangePasswordView()
                .frame(width: 420)
        }
        .sheet(isPresented: $editingBirthday) {
            MyBirthdayView()
                .frame(width: 420)
        }
        .sheet(isPresented: $showingStatistics) {
            StatisticsView()
        }
        .sheet(isPresented: $deletingAccount) {
            DeleteAccountView()
                .frame(width: 460)
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
