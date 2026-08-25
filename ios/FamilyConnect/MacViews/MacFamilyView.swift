//
//  MacFamilyView.swift
//  FamilyConnect
//
//  The family on the Mac: who is in it, who is asking to join, the
//  invite code and the assistant's two family settings — plus starting a
//  direct chat with any of them.
//
//  One sheet rather than the phone's two screens (New Chat, Manage
//  Family). On a phone those are separate destinations because a phone
//  shows one thing at a time; in a window there is room for the roster
//  and the owner's controls together, and starting a chat with somebody
//  is the same gesture as looking at them.
//
//  Owner-only controls are HIDDEN rather than disabled for members, which
//  is what the phone does — a greyed-out row invites a tap that will never
//  work.
//

#if os(macOS)

import SwiftData
import SwiftUI

struct MacFamilyView: View {
    /// Called with the chat to open once a direct chat exists.
    var onOpenChat: (Int64) -> Void

    @Environment(AppSession.self) private var session
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss

    @Query(filter: #Predicate<MemberEntity> { !$0.hasLeft },
           sort: [SortDescriptor(\MemberEntity.displayName)])
    private var members: [MemberEntity]

    @State private var requests: [JoinRequestDTO] = []
    @State private var inviteCode: String?
    @State private var busy = false
    @State private var errorText: String?
    @State private var resetting: MemberDTO?
    /// The member whose birthday the owner is editing; nil while closed.
    @State private var editingBirthday: MemberDTO?

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            List {
                if session.isOwner {
                    inviteSection
                    if !requests.isEmpty { requestsSection }
                    if let family = session.family {
                        // The same two sections the phone shows, from the
                        // same file — a setting changed on one device has
                        // to be findable on the other.
                        FamilyAssistantSettings(family: family) { updated in
                            session.applyFamily(updated)
                        }
                    }
                }
                membersSection
            }
            if let errorText {
                Label(errorText, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout)
                    .foregroundStyle(.red)
                    .padding(.horizontal, 12)
                    .padding(.top, 8)
            }
            Divider()
            HStack {
                if busy { ProgressView().controlSize(.small) }
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
            .padding(12)
        }
        .frame(width: 520, height: 520)
        .task { await reload() }
        .sheet(item: $resetting) { member in
            ResetPasswordView(member: member)
                .frame(width: 420)
        }
        .sheet(item: $editingBirthday) { member in
            // The roster draws SwiftData, and applyMemberBirthday writes
            // there, so there is nothing further for this screen to patch.
            MemberBirthdayView(member: member) { _ in }
                .frame(width: 420)
        }
    }

    /// The family, named, with a count — a roster sheet that opens with
    /// "Members" and nothing else could be anybody's.
    private var header: some View {
        HStack(spacing: 12) {
            InitialsAvatar(
                title: session.family?.name ?? "Family",
                isFamily: true,
                size: 44)
            VStack(alignment: .leading, spacing: 1) {
                Text(session.family?.name ?? "Family")
                    .font(.title3.weight(.semibold))
                Text("\(members.count) members")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            Spacer()
        }
        .padding(16)
    }

    private var inviteSection: some View {
        Section("Invite code") {
            HStack {
                Text(inviteCode ?? "…")
                    .font(.title3.monospaced())
                    .textSelection(.enabled)
                Spacer()
                Button("Copy") {
                    guard let inviteCode else { return }
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(inviteCode, forType: .string)
                }
                .disabled(inviteCode == nil)
                Button("Rotate") { rotate() }
                    .disabled(busy)
            }
            Text("Rotating invalidates the current code immediately.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var requestsSection: some View {
        Section("Join requests") {
            ForEach(requests) { request in
                HStack(spacing: 10) {
                    InitialsAvatar(
                        title: request.user.displayName,
                        userID: request.user.id,
                        avatarVersion: 0,
                        size: 28)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(request.user.displayName)
                        Text("@\(request.user.username)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Approve") { decide(request, approve: true) }
                    Button("Decline") { decide(request, approve: false) }
                        .foregroundStyle(.red)
                }
                .disabled(busy)
            }
        }
    }

    private var membersSection: some View {
        Section("Members") {
            ForEach(members) { member in
                HStack(spacing: 10) {
                    InitialsAvatar(
                        title: member.displayName,
                        userID: member.userID,
                        avatarVersion: member.avatarVersion,
                        size: 28)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(member.displayName)
                        // The username, and the birthday under it when
                        // there is one — day and month in the reader's
                        // locale, never a year and never an age.
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
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.tint)
                            .padding(.horizontal, 7)
                            .padding(.vertical, 2)
                            .background(.tint.opacity(0.15), in: Capsule())
                    }
                    if !member.isCurrentUser {
                        Button("Message") { openDirect(member) }
                    }
                    // Owner tools. The birthday editor is offered on
                    // EVERY row, the owner's own included, because the
                    // roster endpoint accepts their id (protocol.md,
                    // "Birthdays") — the other two are never aimed at the
                    // owner, who changes their own password in Settings,
                    // with the current one.
                    if session.isOwner {
                        Menu {
                            Button("Birthday…") { editingBirthday = member.dto }
                            if !member.isCurrentUser, member.role != "owner" {
                                Divider()
                                Button("Reset Password…") { resetting = member.dto }
                                Button("Remove from Family", role: .destructive) {
                                    remove(member)
                                }
                            }
                        } label: {
                            Image(systemName: "ellipsis.circle")
                        }
                        .menuStyle(.borderlessButton)
                        .fixedSize()
                    }
                }
                .disabled(busy)
            }
        }
    }

    // MARK: - Actions

    private func reload() async {
        guard session.isOwner else { return }
        // Re-read the family rather than trusting the cached one: the
        // invite code can be rotated from another device, and so can the
        // two assistant settings drawn above.
        if let mine = try? await coordinator.api.myFamily() {
            session.applyFamily(mine.family)
        }
        inviteCode = session.family?.inviteCode
        requests = (try? await coordinator.api.joinRequests()) ?? []
    }

    private func openDirect(_ member: MemberEntity) {
        run { [userID = member.userID] in
            let chat = try await coordinator.api.createDirectChat(userID: userID)
            // The chat may be new to this device; make sure it is in the
            // store before the window tries to select it.
            await coordinator.resync()
            dismiss()
            onOpenChat(chat.id)
        }
    }

    private func rotate() {
        run {
            inviteCode = try await coordinator.api.rotateInviteCode()
        }
    }

    private func decide(_ request: JoinRequestDTO, approve: Bool) {
        run {
            if approve {
                _ = try await coordinator.api.approveJoinRequest(id: request.id)
            } else {
                try await coordinator.api.rejectJoinRequest(id: request.id)
            }
            await coordinator.resync()
            requests = (try? await coordinator.api.joinRequests()) ?? []
        }
    }

    private func remove(_ member: MemberEntity) {
        run { [userID = member.userID] in
            try await coordinator.api.removeMember(userID: userID)
            await coordinator.resync()
        }
    }

    /// One busy flag and one error line for every action here — each is a
    /// single request whose only interesting outcome is "it failed".
    private func run(_ work: @escaping () async throws -> Void) {
        busy = true
        errorText = nil
        Task {
            defer { busy = false }
            do {
                try await work()
            } catch {
                errorText = "That didn't work. Try again."
            }
        }
    }
}

#endif
