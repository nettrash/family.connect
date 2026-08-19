//
//  FamilyManageView.swift
//  FamilyConnect
//
//  Owner-only console: pending join requests (approve/reject), invite
//  code rotation (confirmed — the old code dies immediately and anyone
//  mid-typing it is out of luck), join policy, and the member roster
//  with swipe-to-remove. The owner's own row is protected — the server
//  would 409 cannot_remove_owner anyway, but not offering the swipe is
//  better than explaining the refusal.
//
//  Shares SettingsModel with its parent so a rotate/policy change is
//  reflected immediately when the user swipes back.
//

import Observation
import SwiftUI

@MainActor @Observable
final class FamilyManageModel {
    var requests: [JoinRequestDTO] = []
    var isLoading = false
    var confirmRotate = false
    var errorText: String?
    /// Request ids with an approve/reject in flight (disables the row).
    var workingRequestIDs: Set<Int64> = []
}

struct FamilyManageView: View {
    /// The parent Settings screen's model — family + members live there.
    @Bindable var settingsModel: SettingsModel

    @Environment(AppSession.self) private var session
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @State private var model = FamilyManageModel()

    var body: some View {
        List {
            requestsSection
            inviteSection
            policySection
            membersSection
        }
        .navigationTitle("Manage Family")
        .navigationBarTitleDisplayMode(.inline)
        .task {
            await reload()
        }
        .refreshable {
            await reload()
        }
        .confirmationDialog(
            "Rotate the invite code?",
            isPresented: Bindable(model).confirmRotate,
            titleVisibility: .visible
        ) {
            Button("Rotate Code", role: .destructive) { rotate() }
        } message: {
            Text("The current code stops working immediately. Pending requests survive.")
        }
    }

    // MARK: - Sections

    @ViewBuilder
    private var requestsSection: some View {
        Section {
            if model.requests.isEmpty {
                Text("No pending requests")
                    .foregroundStyle(.secondary)
                    .font(.callout)
            } else {
                ForEach(model.requests) { request in
                    HStack(spacing: 12) {
                        InitialsAvatar(title: request.user.displayName)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(request.user.displayName)
                            Text("@\(request.user.username)")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button {
                            reject(request)
                        } label: {
                            Image(systemName: "xmark.circle")
                                .foregroundStyle(.red)
                        }
                        .buttonStyle(.borderless)
                        Button {
                            approve(request)
                        } label: {
                            Image(systemName: "checkmark.circle.fill")
                                .foregroundStyle(.tint)
                        }
                        .buttonStyle(.borderless)
                    }
                    .disabled(model.workingRequestIDs.contains(request.id))
                }
            }
        } header: {
            Text("Join requests")
        } footer: {
            if let error = model.errorText {
                Label(error, systemImage: "xmark.circle")
                    .foregroundStyle(.red)
            }
        }
    }

    @ViewBuilder
    private var inviteSection: some View {
        Section("Invite code") {
            if let code = settingsModel.family?.inviteCode {
                LabeledContent("Code") {
                    Text(code)
                        .font(.body.monospaced())
                        .textSelection(.enabled)
                }
            }
            Button {
                model.confirmRotate = true
            } label: {
                Label("Rotate Code", systemImage: "arrow.triangle.2.circlepath")
            }
        }
    }

    @ViewBuilder
    private var policySection: some View {
        Section {
            Picker("New members", selection: policyBinding) {
                Text("Join immediately").tag("open")
                Text("Need approval").tag("approval")
            }
            .pickerStyle(.menu)
        } header: {
            Text("Join policy")
        } footer: {
            Text("With approval, join requests wait here until you approve them.")
        }
    }

    private var policyBinding: Binding<String> {
        Binding(
            get: { settingsModel.family?.joinPolicy ?? "open" },
            set: { newPolicy in
                guard newPolicy != settingsModel.family?.joinPolicy else { return }
                setPolicy(newPolicy)
            })
    }

    @ViewBuilder
    private var membersSection: some View {
        Section("Members") {
            ForEach(settingsModel.members, id: \.id) { member in
                HStack(spacing: 12) {
                    InitialsAvatar(title: member.displayName)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(member.displayName)
                        Text("@\(member.username)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    if member.role == "owner" {
                        Text("Owner")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                    // The owner row is protected: no remove offered.
                    if member.role != "owner" {
                        Button(role: .destructive) {
                            remove(member)
                        } label: {
                            Label("Remove", systemImage: "person.badge.minus")
                        }
                    }
                }
            }
        }
    }

    // MARK: - Actions

    private func reload() async {
        model.isLoading = true
        defer { model.isLoading = false }
        model.errorText = nil
        async let requests = try? coordinator.api.joinRequests()
        await settingsModel.load(api: coordinator.api)
        model.requests = (await requests) ?? []
    }

    private func approve(_ request: JoinRequestDTO) {
        model.workingRequestIDs.insert(request.id)
        Task {
            defer { model.workingRequestIDs.remove(request.id) }
            do {
                _ = try await coordinator.api.approveJoinRequest(id: request.id)
                model.requests.removeAll { $0.id == request.id }
                await coordinator.resync() // roster + chats pick up the new member
            } catch {
                model.errorText = "Couldn't approve the request. Pull to refresh."
            }
        }
    }

    private func reject(_ request: JoinRequestDTO) {
        model.workingRequestIDs.insert(request.id)
        Task {
            defer { model.workingRequestIDs.remove(request.id) }
            do {
                try await coordinator.api.rejectJoinRequest(id: request.id)
                model.requests.removeAll { $0.id == request.id }
            } catch {
                model.errorText = "Couldn't reject the request. Pull to refresh."
            }
        }
    }

    private func rotate() {
        Task {
            do {
                let newCode = try await coordinator.api.rotateInviteCode()
                // Patch the shared model in place so Settings shows the
                // fresh code without a refetch.
                if let family = settingsModel.family {
                    settingsModel.family = FamilyDTO(
                        id: family.id,
                        name: family.name,
                        joinPolicy: family.joinPolicy,
                        createdAt: family.createdAt,
                        inviteCode: newCode)
                }
            } catch {
                model.errorText = "Couldn't rotate the code. Try again."
            }
        }
    }

    private func setPolicy(_ policy: String) {
        Task {
            do {
                let updated = try await coordinator.api.setJoinPolicy(policy)
                // PATCH answers with the family sans invite_code unless
                // owner — merge, keeping the code we already hold.
                settingsModel.family = FamilyDTO(
                    id: updated.id,
                    name: updated.name,
                    joinPolicy: updated.joinPolicy,
                    createdAt: updated.createdAt,
                    inviteCode: updated.inviteCode ?? settingsModel.family?.inviteCode)
            } catch {
                model.errorText = "Couldn't change the policy. Try again."
            }
        }
    }

    private func remove(_ member: MemberDTO) {
        Task {
            do {
                try await coordinator.api.removeMember(userID: member.id)
                settingsModel.members.removeAll { $0.id == member.id }
                await coordinator.resync()
            } catch {
                model.errorText = "Couldn't remove \(member.displayName). Try again."
            }
        }
    }
}
