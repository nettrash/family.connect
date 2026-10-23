//
//  FamilySwitcher.swift
//  FamilyConnect
//
//  The visible half of several families (docs/protocol.md, "Several
//  families", 2026-10-22): the switcher's contents, the invitation banner,
//  the two ways in to another family, and the owner's invitations. One
//  definition for both platforms — the phone opens the switcher from the
//  family name at the top of its chat list, the Mac from a menu in the
//  sidebar's header — so the two cannot drift apart. Every decision is
//  `FamilyRules`; these views only draw them.
//
//  An account in ONE family with nothing pending never sees the switcher or
//  the banner; it sees only "New Family…" and "Join Another Family…" on the
//  Family screen (`FamilyDoorButtons`).
//

import SwiftData
import SwiftUI

// MARK: - The switcher's contents

/// What the switcher lists: every family with its unread count, a pending
/// join request as a row of its own, every invitation with Join and
/// Decline, and the two ways in. Menu content — the phone's title menu and
/// the Mac's sidebar menu both host it.
struct FamilySwitcherMenuContent: View {
    @Environment(AppSession.self) private var session
    @Query private var chats: [ChatEntity]

    let onNewFamily: () -> Void
    let onJoinAnother: () -> Void
    /// What an invitation answer that failed has to say, for the host to
    /// show — a menu has nowhere of its own to put a sentence.
    let onFailure: (String) -> Void

    private var entries: [FamilyRules.SwitcherEntry] {
        FamilyRules.switcherEntries(
            families: session.families,
            chats: chats.map { FamilyRules.ChatCount(familyIDs: $0.familyIDs, unread: $0.unreadCount) },
            current: session.currentFamilyID)
    }

    var body: some View {
        Section("Families") {
            ForEach(entries) { entry in
                Button {
                    session.switchFamily(to: entry.id)
                } label: {
                    if entry.isCurrent {
                        Label {
                            entryText(entry)
                        } icon: {
                            Image(systemName: "checkmark")
                        }
                    } else {
                        entryText(entry)
                    }
                }
                .accessibilityLabel(Text("\(entry.name), \(entry.unread) unread"))
            }
            ForEach(session.pendingJoinRequests, id: \.familyID) { request in
                // A request still with its owner: shown, not switchable.
                Button {} label: {
                    Text(request.familyName)
                    Text("Waiting for approval")
                }
                .disabled(true)
            }
        }
        if !session.invitations.isEmpty {
            Section("Invitations") {
                ForEach(session.invitations) { invitation in
                    Menu(InvitationText.line(for: invitation)) {
                        Button("Join") { answer(invitation, accept: true) }
                        Button("Decline", role: .destructive) { answer(invitation, accept: false) }
                    }
                }
            }
        }
        if session.offersNewFamily || session.offersJoinAnotherFamily {
            Section {
                if session.offersNewFamily {
                    Button("New Family…", action: onNewFamily)
                }
                if session.offersJoinAnotherFamily {
                    Button("Join Another Family…", action: onJoinAnother)
                }
            }
        }
    }

    @ViewBuilder
    private func entryText(_ entry: FamilyRules.SwitcherEntry) -> some View {
        Text(entry.name)
        if entry.unread > 0 {
            Text("\(entry.unread) unread")
        }
    }

    private func answer(_ invitation: InvitationDTO, accept: Bool) {
        Task {
            if let failure = await InvitationAnswer.send(invitation, accept: accept, session: session) {
                onFailure(failure)
            }
        }
    }
}

/// The words of one invitation: "Anna invited you to “The Ivanovs”".
enum InvitationText {
    static func line(for invitation: InvitationDTO) -> String {
        String(localized: "\(invitation.invitedBy.displayName) invited you to “\(invitation.familyName)”")
    }
}

/// Join or Decline, and what to say when it did not happen.
@MainActor
enum InvitationAnswer {
    /// Nil when it happened; otherwise the sentence for the person.
    static func send(_ invitation: InvitationDTO, accept: Bool, session: AppSession) async -> String? {
        do {
            if accept {
                try await session.acceptInvitation(invitation)
            } else {
                try await session.declineInvitation(invitation)
            }
            return nil
        } catch APIError.unauthorized {
            session.handleUnauthorized()
            return nil
        } catch {
            if let said = FamilyRules.failureMessage(error, maxFamilies: session.maxFamiliesPerAccount) {
                return said
            }
            // An outcome nobody heard is not a refusal (docs/protocol.md,
            // "Transient or terminal").
            if (error as? APIError)?.isTransient == false {
                return String(localized: "The server refused it.")
            }
            return String(localized: "Can't reach the server. Try again.")
        }
    }
}

// MARK: - The invitation banner

/// Over the chat list while invitations wait: who invited you where, with
/// Join and Decline. Nothing at all when there are none.
struct InvitationBanner: View {
    @Environment(AppSession.self) private var session
    @State private var failure: String?
    @State private var answering: Set<Int64> = []

    var body: some View {
        if !session.invitations.isEmpty {
            VStack(spacing: 8) {
                ForEach(session.invitations) { invitation in
                    card(invitation)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .alert(
                "Invitations",
                isPresented: Binding(get: { failure != nil }, set: { if !$0 { failure = nil } })
            ) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(failure ?? "")
            }
        }
    }

    private func card(_ invitation: InvitationDTO) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Label(InvitationText.line(for: invitation), systemImage: "envelope.open")
                .font(.subheadline.weight(.semibold))
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 10) {
                Spacer(minLength: 0)
                Button("Decline", role: .destructive) { answer(invitation, accept: false) }
                    .buttonStyle(.bordered)
                Button("Join") { answer(invitation, accept: true) }
                    .buttonStyle(.borderedProminent)
            }
            .disabled(answering.contains(invitation.id))
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }

    private func answer(_ invitation: InvitationDTO, accept: Bool) {
        answering.insert(invitation.id)
        Task {
            defer { answering.remove(invitation.id) }
            failure = await InvitationAnswer.send(invitation, accept: accept, session: session)
        }
    }
}

// MARK: - The two ways in

/// "New Family…" and "Join Another Family…" on the Family screen — the way
/// in to a second family for an account that has only one, and therefore no
/// switcher. Each is offered only while the server and the cap allow it.
struct FamilyDoorButtons: View {
    @Environment(AppSession.self) private var session
    @Binding var showsNewFamily: Bool
    @Binding var showsJoinAnother: Bool

    var body: some View {
        if session.offersNewFamily {
            Button("New Family…") { showsNewFamily = true }
        }
        if session.offersJoinAnotherFamily {
            Button("Join Another Family…") { showsJoinAnother = true }
        }
    }
}

/// The two sheets behind the doors: the gate's own create and join screens,
/// in their "another family" form, each with a way out.
struct FamilyDoorSheets: ViewModifier {
    @Binding var showsNewFamily: Bool
    @Binding var showsJoinAnother: Bool

    func body(content: Content) -> some View {
        content
            .sheet(isPresented: $showsNewFamily) {
                FamilyDoorSheet { dismiss in
                    CreateFamilyView(anotherFamily: true, onFinished: dismiss)
                }
            }
            .sheet(isPresented: $showsJoinAnother) {
                FamilyDoorSheet { dismiss in
                    JoinFamilyView(anotherFamily: true, onFinished: dismiss)
                }
            }
    }
}

extension View {
    func familyDoorSheets(showsNewFamily: Binding<Bool>, showsJoinAnother: Binding<Bool>) -> some View {
        modifier(FamilyDoorSheets(showsNewFamily: showsNewFamily, showsJoinAnother: showsJoinAnother))
    }
}

/// A gate screen wrapped for presenting from inside the app: its own stack,
/// and Cancel.
private struct FamilyDoorSheet<Content: View>: View {
    @Environment(\.dismiss) private var dismiss
    @ViewBuilder let content: (@escaping () -> Void) -> Content

    var body: some View {
        NavigationStack {
            content { dismiss() }
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel") { dismiss() }
                            .keyboardShortcut(.cancelAction)
                    }
                }
        }
        #if os(macOS)
        .frame(minWidth: 420, minHeight: 300)
        #endif
    }
}

// MARK: - The owner's invitations

/// Loads and withdraws the owner's pending invitations for the family on
/// screen (`GET /families/{id}/invitations`), re-reading whenever an
/// invitation frame says the list moved.
@MainActor @Observable
final class OwnerInvitationsModel {
    var pending: [InvitationDTO] = []
    var errorText: String?

    func load(api: APIClient, family: Int64?) async {
        guard let family else {
            pending = []
            return
        }
        pending = (try? await api.invitations(family: family)) ?? pending
    }

    func invite(userID: Int64, api: APIClient, family: Int64?, maxFamilies: Int) async {
        guard let family else { return }
        errorText = nil
        do {
            let invitation = try await api.invite(userID: userID, family: family)
            if !pending.contains(where: { $0.id == invitation.id }) { pending.append(invitation) }
        } catch {
            errorText = FamilyRules.failureMessage(error, maxFamilies: maxFamilies)
                ?? String(localized: "The server refused it.")
        }
    }

    func withdraw(_ invitation: InvitationDTO, api: APIClient, family: Int64?) async {
        guard let family else { return }
        errorText = nil
        do {
            try await api.withdrawInvitation(id: invitation.id, family: family)
            pending.removeAll { $0.id == invitation.id }
        } catch {
            errorText = String(localized: "Can't reach the server. Try again.")
        }
    }
}

/// "Pending Invitations" on the owner's Family screen: who is invited and
/// has not answered, each with "Withdraw Invitation". Nothing at all while
/// there are none.
struct PendingInvitationsSection: View {
    @Environment(AppSession.self) private var session
    @Environment(ChatSyncCoordinator.self) private var coordinator
    let model: OwnerInvitationsModel

    var body: some View {
        if !model.pending.isEmpty {
            Section("Pending Invitations") {
                ForEach(model.pending) { invitation in
                    HStack {
                        Text(invitation.user.displayName)
                        Spacer(minLength: 8)
                        Button("Withdraw Invitation", role: .destructive) {
                            Task {
                                await model.withdraw(
                                    invitation, api: coordinator.api, family: session.apiFamilyID)
                            }
                        }
                        .buttonStyle(.borderless)
                    }
                }
            }
        }
    }
}

/// The picker behind "Invite…": the members of the owner's OTHER families
/// who are not in this one — and nobody else on the server — each invited
/// with a tap, and marked "Invited" while that invitation waits.
struct InviteToFamilyView: View {
    @Environment(AppSession.self) private var session
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss
    @Query private var members: [MemberEntity]
    let model: OwnerInvitationsModel

    private var invitees: [FamilyRules.Invitee] {
        guard let familyID = session.currentFamilyID else { return [] }
        let rows = members.map {
            FamilyRules.RosterRow(
                userID: $0.userID, name: $0.resolvedDisplayName,
                families: Set($0.familyRoles.keys), accountDeleted: $0.accountDeleted)
        }
        return FamilyRules.invitees(
            rows: rows, into: familyID, currentUserID: coordinator.currentUserID,
            pendingInvitedUserIDs: Set(model.pending.map(\.user.id)))
    }

    private var familyName: String { session.family?.name ?? "" }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    if invitees.isEmpty {
                        Text("Members of your other families who aren't in this one appear here.")
                            .foregroundStyle(.secondary)
                    } else {
                        ForEach(invitees) { invitee in
                            row(invitee)
                        }
                    }
                } footer: {
                    if let error = model.errorText {
                        Label(error, systemImage: "xmark.circle")
                            .foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle(Text("Invite to “\(familyName)”"))
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                        .keyboardShortcut(.cancelAction)
                }
            }
            .task { await model.load(api: coordinator.api, family: session.apiFamilyID) }
        }
        #if os(macOS)
        .frame(minWidth: 380, minHeight: 320)
        #endif
    }

    @ViewBuilder
    private func row(_ invitee: FamilyRules.Invitee) -> some View {
        if invitee.isInvited {
            LabeledContent(invitee.name) {
                Text("Invited")
                    .foregroundStyle(.secondary)
            }
        } else {
            Button {
                Task {
                    await model.invite(
                        userID: invitee.id, api: coordinator.api, family: session.apiFamilyID,
                        maxFamilies: session.maxFamiliesPerAccount)
                }
            } label: {
                Label(invitee.name, systemImage: "envelope")
            }
        }
    }
}
