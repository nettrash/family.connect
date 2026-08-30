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

    // Live members only: a deleted account is in the store so old
    // messages can still be given a sender, and is a member of nothing.
    @Query(filter: #Predicate<MemberEntity> { !$0.hasLeft && !$0.accountDeleted },
           sort: [SortDescriptor(\MemberEntity.displayName)])
    private var members: [MemberEntity]

    @State private var requests: [JoinRequestDTO] = []
    @State private var inviteCode: String?
    @State private var busy = false
    @State private var errorText: String?
    @State private var resetting: MemberDTO?
    /// The owner's moderation list, oldest first. Stays empty for a plain
    /// member: `reload` returns early before it would ever be fetched.
    @State private var reports: [ReportDTO] = []
    @State private var resolvingReports: Set<Int64> = []
    /// The stepper's own cap while the debounced write is in flight; nil
    /// whenever the server's answer is the one to trust.
    @State private var capDraft: Int?
    /// The pending cap write, cancelled and replaced on every tap.
    @State private var capCommit: Task<Void, Never>?
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
                    if !reports.isEmpty { reportsSection }
                    policySection
                    capSection
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
                title: session.family?.name ?? String(localized: "Family"),
                isFamily: true,
                size: 44)
            VStack(alignment: .leading, spacing: 1) {
                Text(session.family?.name ?? String(localized: "Family"))
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

    /// Who may join, and how many. Net-new on the Mac: this window has
    /// shown the invite code since it was written but never what the code
    /// DOES, so an owner working from a desk could hand out a code and had
    /// no way to say who it let in — or to shut the door.
    private var policySection: some View {
        Section("Join policy") {
            Picker("New members", selection: policyBinding) {
                Text("Join immediately").tag("open")
                Text("Need approval").tag("approval")
                Text("Nobody").tag("closed")
            }
            Text(policyFooter)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var policyBinding: Binding<String> {
        Binding(
            get: { session.family?.joinPolicy ?? "open" },
            set: { policy in
                guard policy != session.family?.joinPolicy else { return }
                run {
                    let updated = try await coordinator.api.setJoinPolicy(policy)
                    await MainActor.run { session.applyFamily(merged(updated)) }
                }
            })
    }

    /// One line per policy — "Nobody" needs the two things the others do
    /// not: the code stops working, and the requests already waiting are
    /// untouched (docs/protocol.md,
    /// `POST /families/join-requests/{id}/approve`).
    private var policyFooter: LocalizedStringKey {
        switch session.family?.joinPolicy {
        case "approval": "With approval, join requests wait here until you approve them."
        case "closed": "The invite code stops working — nobody new can join. Requests already waiting are unaffected, and you can still approve them."
        default: "Anyone with the invite code joins straight away."
        }
    }

    @ViewBuilder
    private var capSection: some View {
        if let ceiling = session.maxFamilyMembers {
            Section("Member limit") {
                Toggle("Limit members", isOn: capEnabledBinding(ceiling: ceiling))
                if let cap = draftCap {
                    Stepper(value: capBinding(ceiling: ceiling), in: 1...ceiling) {
                        LabeledContent("Most members", value: cap.formatted())
                    }
                }
                Text(capFooter(ceiling: ceiling))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var draftCap: Int? { capDraft ?? session.family?.maxMembers }

    private func capEnabledBinding(ceiling: Int) -> Binding<Bool> {
        Binding(
            get: { draftCap != nil },
            set: { on in
                // Turning it on freezes the family where it stands. A cap
                // at or below the current size is legal and acts as a
                // freeze rather than being refused (docs/protocol.md,
                // `PATCH /families/mine`).
                let seed = MemberCap.seed(memberCount: members.count, ceiling: ceiling)
                capDraft = on ? seed : nil
                commitCap(on ? seed : nil)
            })
    }

    private func capBinding(ceiling: Int) -> Binding<Int> {
        Binding(
            get: { draftCap ?? MemberCap.seed(memberCount: members.count, ceiling: ceiling) },
            set: { value in
                let clamped = MemberCap.clamp(value, ceiling: ceiling)
                capDraft = clamped
                commitCap(clamped)
            })
    }

    /// One PATCH at the end of a stepper drag, not one per tap.
    private func commitCap(_ cap: Int?) {
        capCommit?.cancel()
        capCommit = Task {
            try? await Task.sleep(for: .milliseconds(600))
            guard !Task.isCancelled else { return }
            do {
                let updated = try await coordinator.api.setMemberCap(cap)
                session.applyFamily(merged(updated))
                capDraft = nil
            } catch {
                capDraft = nil
                errorText = String(localized: "Couldn't change the member limit. Try again.")
            }
        }
    }

    private func capFooter(ceiling: Int) -> LocalizedStringKey {
        switch MemberCap.state(cap: draftCap, memberCount: members.count, ceiling: ceiling) {
        case .openToCeiling(let ceiling):
            "No limit of your own. This server allows up to \(ceiling) members in a family."
        case .frozen(let count):
            "\(count) members now. Nobody new can join until somebody leaves; no one is removed."
        case .room(let count, let cap):
            "\(count) of \(cap) seats used."
        }
    }

    /// A PATCH answers without the invite code unless the caller is the
    /// owner, so keep the one already held rather than blanking the field
    /// this window draws.
    private func merged(_ updated: FamilyDTO) -> FamilyDTO {
        FamilyDTO(
            id: updated.id,
            name: updated.name,
            joinPolicy: updated.joinPolicy,
            createdAt: updated.createdAt,
            inviteCode: updated.inviteCode ?? session.family?.inviteCode,
            language: updated.language,
            aiHistory: updated.aiHistory,
            maxMembers: updated.maxMembers)
    }

    /// The owner's moderation inbox, inline rather than behind a sheet: a
    /// report carries a whole frozen message body, and a Mac sheet cannot
    /// be resized to read one.
    ///
    /// Never shows who blocked whom. Blocking and reporting are
    /// independent, and a family owner is often a parent while the blocked
    /// person is often in the same house — the exact case the silence
    /// exists for (docs/protocol.md, "Reporting a member").
    private var reportsSection: some View {
        Section("Reports") {
            ForEach(reports) { report in
                VStack(alignment: .leading, spacing: 4) {
                    Text(ReportReason(rawValue: report.reason)?.label ?? ReportReason.other.label)
                        .font(.headline)
                    Text("\(report.reporter.displayName) reported \(report.reported.displayName)")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                    // Drawn ALWAYS when present, and never truncated: it is
                    // frozen precisely because the author may edit the body
                    // away and retention will sweep the message, and an
                    // owner judging a message has to see all of it.
                    if let excerpt = report.messageExcerpt, !excerpt.isEmpty {
                        Text(excerpt)
                            .font(.callout)
                            .padding(6)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(.quaternary, in: RoundedRectangle(cornerRadius: 6))
                            .textSelection(.enabled)
                    }
                    HStack {
                        Spacer()
                        // Says nothing about what the owner DID: this
                        // protocol has removing a member, resetting a
                        // password and closing the family, not deleting
                        // somebody else's message.
                        Button("Mark as handled") { resolve(report) }
                            .disabled(busy || resolvingReports.contains(report.id))
                    }
                }
                .padding(.vertical, 2)
            }
        }
    }

    private func resolve(_ report: ReportDTO) {
        resolvingReports.insert(report.id)
        run {
            try await coordinator.api.resolveReport(id: report.id)
            await MainActor.run {
                reports.removeAll { $0.id == report.id }
                resolvingReports.remove(report.id)
            }
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
        // Owner-only, like the requests above: a plain member never gets
        // here (`reload` returns early for them), so the 403 is unreachable
        // rather than swallowed.
        reports = (try? await coordinator.api.reports()) ?? []
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

    /// Its own Task rather than `run`, which flattens every failure into
    /// one sentence. Approval has a refusal worth naming: the cap is
    /// re-checked here because the roster can fill between a request and
    /// the decision, and the request stays PENDING when it does — a full
    /// family is a temporary condition, not a decision (docs/protocol.md,
    /// `POST /families/join-requests/{id}/approve`).
    private func decide(_ request: JoinRequestDTO, approve: Bool) {
        busy = true
        errorText = nil
        Task {
            defer { busy = false }
            do {
                if approve {
                    _ = try await coordinator.api.approveJoinRequest(id: request.id)
                } else {
                    try await coordinator.api.rejectJoinRequest(id: request.id)
                }
                await coordinator.resync()
                requests = (try? await coordinator.api.joinRequests()) ?? []
            } catch APIError.conflict(let code, _) where code == "family_full" {
                errorText = String(localized: "The family is full. Raise the member limit or wait for somebody to leave — the request is still waiting.", comment: "Error when approving a join request would exceed the family's member cap.")
            } catch {
                errorText = String(localized: "That didn't work. Try again.")
            }
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
                errorText = String(localized: "That didn't work. Try again.")
            }
        }
    }
}

#endif
