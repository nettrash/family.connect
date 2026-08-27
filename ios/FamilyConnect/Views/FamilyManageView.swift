//
//  FamilyManageView.swift
//  FamilyConnect
//
//  Owner-only console: pending join requests (approve/reject), invite
//  code rotation (confirmed — the old code dies immediately and anyone
//  mid-typing it is out of luck), join policy, the assistant's two family
//  settings, and the member roster with swipe-to-remove. The owner's own
//  row is protected from removal and from a password reset — the server
//  would 409 cannot_remove_owner anyway, but not offering the swipe is
//  better than explaining the refusal — and is deliberately NOT protected
//  from the birthday editor, which the protocol lets the owner point at
//  themselves precisely so this screen needs no special case.
//
//  Shares SettingsModel with its parent so a rotate/policy change is
//  reflected immediately when the user swipes back.
//

// iOS only — the Mac has its own views (MacViews/).
#if os(iOS)

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
    /// The member whose password the owner is resetting; nil while closed.
    @State private var resettingPassword: MemberDTO?
    /// The member whose birthday the owner is editing; nil while closed.
    @State private var editingBirthday: MemberDTO?

    var body: some View {
        List {
            // Owner-only: join requests, the invite code and the join
            // policy are all owner endpoints on the server. A plain
            // member opens this screen for the roster below.
            if session.isOwner {
                requestsSection
                inviteSection
                policySection
                if let family = settingsModel.family {
                    FamilyAssistantSettings(family: family) { updated in
                        settingsModel.family = updated
                        session.applyFamily(updated)
                    }
                }
            }
            membersSection
        }
        .navigationTitle(session.isOwner ? "Manage Family" : "Family Members")
        .sheet(item: $resettingPassword) { member in
            ResetPasswordView(member: member)
        }
        .sheet(item: $editingBirthday) { member in
            MemberBirthdayView(member: member) { birthday in
                guard let index = settingsModel.members.firstIndex(where: { $0.id == member.id }) else { return }
                settingsModel.members[index] = settingsModel.members[index].withBirthday(birthday)
            }
        }
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
                        // Initials on purpose: a pending requester is not
                        // in the family yet, so the server answers 404 for
                        // their picture — asking would only cache them as
                        // pictureless past approval.
                        InitialsAvatar(
                            title: request.user.displayName,
                            userID: request.user.id)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(request.user.displayName)
                            Text("@\(request.user.username)")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        // 44pt targets on glyph-only buttons, and a label
                        // that says WHOSE request — two unlabelled icons
                        // per row is what VoiceOver read before.
                        Button {
                            reject(request)
                        } label: {
                            Image(systemName: "xmark.circle")
                                .foregroundStyle(.red)
                                .frame(minWidth: 44, minHeight: 44)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.borderless)
                        .accessibilityLabel(Text("Reject \(request.user.displayName)"))
                        Button {
                            approve(request)
                        } label: {
                            Image(systemName: "checkmark.circle.fill")
                                .foregroundStyle(.tint)
                                .frame(minWidth: 44, minHeight: 44)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.borderless)
                        .accessibilityLabel(Text("Approve \(request.user.displayName)"))
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
                    InitialsAvatar(
                        title: member.displayName,
                        userID: member.id,
                        avatarVersion: member.avatarVersion)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(member.displayName)
                        // The username, and the birthday under it when
                        // there is one — day and month in the reader's
                        // locale, never a year and never an age, because
                        // there is nothing here to compute one from.
                        Text("@\(member.username)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        if let birthday = member.birthday {
                            Label(birthday.formatted(), systemImage: "birthday.cake")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    Spacer()
                    if member.role == "owner" {
                        Text("Owner")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                    // Remove and Password are owner actions, and the owner
                    // row is protected from both: an owner changes their
                    // own password from Settings, with the current one.
                    if session.isOwner, member.role != "owner" {
                        Button(role: .destructive) {
                            remove(member)
                        } label: {
                            Label("Remove", systemImage: "person.badge.minus")
                        }
                        Button {
                            resettingPassword = member
                        } label: {
                            Label("Password", systemImage: "key")
                        }
                        .tint(.orange)
                    }
                    // Birthday is the exception, and on purpose: the
                    // roster endpoint accepts the owner's own id, so this
                    // row needs no special case (protocol.md, "Birthdays").
                    if session.isOwner {
                        Button {
                            editingBirthday = member
                        } label: {
                            Label("Birthday", systemImage: "birthday.cake")
                        }
                        .tint(.pink)
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
        // Join requests are an owner-only endpoint: asking as a plain
        // member is a guaranteed 403, and the section is hidden anyway.
        async let requests = session.isOwner ? try? coordinator.api.joinRequests() : nil
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
                model.errorText = String(localized: "Couldn't approve the request. Pull to refresh.")
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
                model.errorText = String(localized: "Couldn't reject the request. Pull to refresh.")
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
                        inviteCode: newCode,
                        language: family.language,
                        aiHistory: family.aiHistory)
                }
            } catch {
                model.errorText = String(localized: "Couldn't rotate the code. Try again.")
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
                    inviteCode: updated.inviteCode ?? settingsModel.family?.inviteCode,
                    language: updated.language,
                    aiHistory: updated.aiHistory)
            } catch {
                model.errorText = String(localized: "Couldn't change the policy. Try again.")
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
                model.errorText = String(localized: "Couldn't remove \(member.displayName). Try again.")
            }
        }
    }
}

#endif
