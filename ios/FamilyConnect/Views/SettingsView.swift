//
//  SettingsView.swift
//  FamilyConnect
//
//  Profile, server, family membership and session controls. Family
//  details (including the owner-only invite_code) come fresh from
//  GET /families/mine in .task rather than from cached session state,
//  because the invite code can be rotated from another device and stale
//  codes actively mislead.
//
//  Both destructive actions confirm first. Leave is special-cased for
//  the owner: the server answers 409 owner_cannot_leave unless they are
//  the sole member, and the alert explains that instead of parroting an
//  error code.
//

import Observation
import SwiftUI

@MainActor @Observable
final class SettingsModel {
    var family: FamilyDTO?
    var members: [MemberDTO] = []
    var isLoading = false
    var confirmLeave = false
    var confirmLogout = false
    var ownerBlockedAlert = false
    var errorText: String?

    func load(api: APIClient) async {
        isLoading = true
        defer { isLoading = false }
        do {
            let mine = try await api.myFamily()
            family = mine.family
            members = mine.members
        } catch {
            // Non-fatal: the cached session family still renders.
        }
    }
}

struct SettingsView: View {
    @Environment(AppSession.self) private var session
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss
    @State private var model = SettingsModel()

    var body: some View {
        NavigationStack {
            Form {
                profileSection
                familySection
                serverSection
                sessionSection
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
            .task {
                await model.load(api: coordinator.api)
            }
            .confirmationDialog(
                "Leave the family?",
                isPresented: Bindable(model).confirmLeave,
                titleVisibility: .visible
            ) {
                Button("Leave Family", role: .destructive) { leave() }
            } message: {
                Text("You'll lose access to the family chat and your direct chats. Your history returns if you rejoin.")
            }
            .confirmationDialog(
                "Log out?",
                isPresented: Bindable(model).confirmLogout,
                titleVisibility: .visible
            ) {
                Button("Log Out", role: .destructive) {
                    Task { await session.logout() }
                }
            } message: {
                Text("Messages stay on the family server; this device forgets its session.")
            }
            .alert("You're the owner", isPresented: Bindable(model).ownerBlockedAlert) {
                Button("OK", role: .cancel) {}
            } message: {
                Text("An owner can only leave once everyone else has left (the family is then deleted). Remove the other members first, or keep the family going.")
            }
        }
    }

    // MARK: - Sections

    private var profileSection: some View {
        Section("Profile") {
            if let user = session.currentUser {
                HStack(spacing: 12) {
                    InitialsAvatar(title: user.displayName)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(user.displayName)
                        Text("@\(user.username)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var familySection: some View {
        Section {
            if let family = model.family ?? session.family {
                LabeledContent("Family", value: family.name)
                LabeledContent("Join policy", value: family.joinPolicy == "open" ? "Open" : "Approval")
                if session.isOwner, let code = model.family?.inviteCode {
                    LabeledContent("Invite code") {
                        Text(code)
                            .font(.body.monospaced())
                            .textSelection(.enabled)
                    }
                    ShareLink(
                        item: "Join our family on Family Connect! Server: \(AppSettings.serverURL?.absoluteString ?? "") — invite code: \(code)"
                    ) {
                        Label("Share Invite", systemImage: "square.and.arrow.up")
                    }
                }
                if session.isOwner {
                    NavigationLink {
                        FamilyManageView(settingsModel: model)
                    } label: {
                        Label("Manage Family", systemImage: "person.2.badge.gearshape")
                    }
                }
            }
            Button("Leave Family", role: .destructive) {
                model.confirmLeave = true
            }
        } header: {
            Text("Family")
        } footer: {
            if let error = model.errorText {
                Label(error, systemImage: "xmark.circle")
                    .foregroundStyle(.red)
            }
        }
    }

    private var serverSection: some View {
        Section("Server") {
            LabeledContent("Address", value: AppSettings.serverURL?.absoluteString ?? "—")
        }
    }

    private var sessionSection: some View {
        Section {
            Button("Log Out", role: .destructive) {
                model.confirmLogout = true
            }
        }
    }

    // MARK: - Actions

    private func leave() {
        model.errorText = nil
        Task {
            do {
                try await session.leaveFamily()
                dismiss()
            } catch APIError.conflict(let code, _) where code == "owner_cannot_leave" {
                model.ownerBlockedAlert = true
            } catch {
                model.errorText = "Couldn't leave right now. Try again."
            }
        }
    }
}
