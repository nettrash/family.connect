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
    /// The stepper's own value while the debounced write is in flight, and
    /// nil whenever the server's answer is the one to trust.
    @State private var capDraft: Int?
    /// The pending cap write, cancelled and replaced on every tap.
    @State private var capCommit: Task<Void, Never>?
    /// The member being reported from the roster, if any.
    @State private var reportingMember: MemberDTO?
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @State private var model = FamilyManageModel()
    /// The member whose password the owner is resetting; nil while closed.
    @State private var resettingPassword: MemberDTO?
    /// The member whose birthday the owner is editing; nil while closed.
    @State private var editingBirthday: MemberDTO?
    /// The member being linked to a device contact; nil while the system
    /// picker is closed (ContactPicker).
    @State private var linkingContact: MemberDTO?
    /// Bumped on every link change so the captions re-read ContactLinks.
    @State private var linksGeneration = 0

    var body: some View {
        List {
            // Owner-only: join requests, the invite code and the join
            // policy are all owner endpoints on the server. A plain
            // member opens this screen for the roster below.
            if session.isOwner {
                requestsSection
                inviteSection
                reportsSection
                policySection
                capSection
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
        .sheet(item: $reportingMember) { member in
            ReportSheet(
                target: ReportTarget(senderID: member.id, senderName: member.displayName, messageID: nil),
                onSubmit: { reason in report(member, reason: reason) },
                onCancel: { reportingMember = nil })
        }
        .sheet(item: $resettingPassword) { member in
            ResetPasswordView(member: member)
        }
        .sheet(item: $linkingContact) { member in
            // The system picker: no Contacts permission, just the one
            // contact the person chooses (ContactPicker).
            ContactPicker(
                onPick: { link in
                    ContactLinks.shared.link(userID: member.id, to: link)
                    linksGeneration += 1
                    linkingContact = nil
                },
                onCancel: { linkingContact = nil })
            .ignoresSafeArea()
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
                Text("Nobody").tag("closed")
            }
            .pickerStyle(.menu)
        } header: {
            Text("Join policy")
        } footer: {
            Text(policyFooter)
        }
    }

    /// One line per policy, because "Nobody" needs two things said that
    /// the other two do not: the code stops working, and the requests
    /// already waiting are untouched (docs/protocol.md,
    /// `POST /families/join-requests/{id}/approve`). An owner who closed
    /// the family to stop new arrivals should not be left wondering
    /// whether they have just silently rejected the people queueing.
    private var policyFooter: LocalizedStringKey {
        switch settingsModel.family?.joinPolicy {
        case "approval": "With approval, join requests wait here until you approve them."
        case "closed": "The invite code stops working — nobody new can join. Requests already waiting are unaffected, and you can still approve them."
        default: "Anyone with the invite code joins straight away."
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

    /// The owner's moderation inbox.
    ///
    /// A link rather than an inline list: a report carries a whole frozen
    /// message body, and an owner with several of them would push the
    /// invite code and the policy controls off the screen.
    private var reportsSection: some View {
        Section {
            NavigationLink {
                ReportInboxView()
            } label: {
                Label("Reports", systemImage: "flag")
            }
        } footer: {
            Text("Members can report a message or a person to you.")
        }
    }

    /// The operator's ceiling — the most this server lets ANY family hold.
    /// An owner's own cap may only be lower (docs/protocol.md, "Limits").
    /// Absent on a server too old to report it, in which case there is
    /// nothing sensible to bound a stepper by and the section stays away
    /// rather than inventing a number.
    private var ceiling: Int? { session.maxFamilyMembers }

    @ViewBuilder
    private var capSection: some View {
        if let ceiling {
            Section {
                Toggle("Limit members", isOn: capEnabledBinding)
                if let cap = draftCap {
                    Stepper(value: capBinding(ceiling: ceiling), in: 1...ceiling) {
                        LabeledContent("Most members", value: cap.formatted())
                    }
                }
            } header: {
                Text("Member limit")
            } footer: {
                Text(capFooter(ceiling: ceiling))
            }
        }
    }

    /// The cap as the stepper is holding it — which is not always what the
    /// server holds, because the stepper commits on a delay (see
    /// `commitCap`). Seeded from the family and re-seeded whenever the
    /// server's answer changes under it.
    private var draftCap: Int? { capDraft ?? settingsModel.family?.maxMembers }

    private var capEnabledBinding: Binding<Bool> {
        Binding(
            get: { draftCap != nil },
            set: { on in
                guard let ceiling else { return }
                // Turning it ON freezes the family where it stands, which
                // is what "limit members" almost always means in the
                // moment somebody reaches for it. A cap at or below the
                // current size is legal and acts as a freeze rather than
                // being refused, so this needs no clamping upward
                // (docs/protocol.md, `PATCH /families/mine`).
                let seed = MemberCap.seed(memberCount: settingsModel.members.count, ceiling: ceiling)
                capDraft = on ? seed : nil
                commitCap(on ? seed : nil)
            })
    }

    private func capBinding(ceiling: Int) -> Binding<Int> {
        Binding(
            get: { draftCap ?? MemberCap.seed(memberCount: settingsModel.members.count, ceiling: ceiling) },
            set: { value in
                let clamped = MemberCap.clamp(value, ceiling: ceiling)
                capDraft = clamped
                commitCap(clamped)
            })
    }

    /// Send the cap, but not on every tap of the stepper. Each tap
    /// replaces the pending write, so holding the button down is one PATCH
    /// at the end rather than fifteen on the way there.
    private func commitCap(_ cap: Int?) {
        capCommit?.cancel()
        capCommit = Task {
            try? await Task.sleep(for: .milliseconds(600))
            guard !Task.isCancelled else { return }
            do {
                let updated = try await coordinator.api.setMemberCap(cap)
                settingsModel.family = FamilyDTO(
                    id: updated.id,
                    name: updated.name,
                    joinPolicy: updated.joinPolicy,
                    createdAt: updated.createdAt,
                    // Same merge as `setPolicy`: PATCH answers without the
                    // invite code unless the caller is the owner.
                    inviteCode: updated.inviteCode ?? settingsModel.family?.inviteCode,
                    language: updated.language,
                    aiHistory: updated.aiHistory,
                    aiVision: updated.aiVision,
                    maxMembers: updated.maxMembers)
                // The server's answer is the truth; drop the draft so the
                // stepper follows it again.
                capDraft = nil
            } catch {
                capDraft = nil
                model.errorText = String(localized: "Couldn't change the member limit. Try again.")
            }
        }
    }

    private func capFooter(ceiling: Int) -> LocalizedStringKey {
        // The branching is in MemberCap so the Mac cannot drift from it;
        // only the words live here.
        switch MemberCap.state(
            cap: draftCap, memberCount: settingsModel.members.count, ceiling: ceiling) {
        case .openToCeiling(let ceiling):
            "No limit of your own. This server allows up to \(ceiling) members in a family."
        case .frozen(let count):
            "\(count) members now. Nobody new can join until somebody leaves; no one is removed."
        case .room(let count, let cap):
            "\(count) of \(cap) seats used."
        }
    }

    /// Read through `linksGeneration` so a link made or removed in this
    /// view redraws the caption (ContactLinks is plain defaults, not
    /// observable).
    private func contactLink(for member: MemberDTO) -> ContactLink? {
        _ = linksGeneration
        return ContactLinks.shared.link(for: member.id)
    }

    @ViewBuilder
    private var membersSection: some View {
        // The error rides HERE, not on the owner-only join-requests
        // section it used to: Report and Block are the two actions on this
        // screen a plain member can take, and a failure they cannot see is
        // a block they believe is in force and is not.
        Section {
            membersRows
        } header: {
            Text("Members")
        } footer: {
            if let error = model.errorText {
                Label(error, systemImage: "xmark.circle")
                    .foregroundStyle(.red)
            }
        }
    }

    @ViewBuilder
    private var membersRows: some View {
        Group {
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
                        // The device contact this member is, on this
                        // phone (ContactLinks) — what lets the Phone app's
                        // Favorites and a contact card call them on Family.
                        if let link = contactLink(for: member) {
                            Label(String(localized: "Linked to \(link.contactName)", comment: "Caption under a family member naming the device contact they are linked to; the argument is the contact's name."), systemImage: "person.crop.circle.badge.checkmark")
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
                .contextMenu {
                    // Not for oneself: a link is how OTHER people reach
                    // this member through the Phone app.
                    if member.id != AppSettings.currentUserID {
                        Button {
                            linkingContact = member
                        } label: {
                            Label("Link to a Contact…", systemImage: "person.crop.circle.badge.plus")
                        }
                        if contactLink(for: member) != nil {
                            Button(role: .destructive) {
                                ContactLinks.shared.unlink(userID: member.id)
                                linksGeneration += 1
                            } label: {
                                Label("Unlink Contact", systemImage: "person.crop.circle.badge.xmark")
                            }
                        }
                        Divider()
                        // Grouped under Safety, matching the message menu.
                        // Report and Block are NOT owner actions, and this
                        // is the one place in this screen where that
                        // matters: the whole point of them is a member with
                        // no other recourse, and the person they need them
                        // for may BE the owner. Any member may block any
                        // other, the owner included (protocol.md,
                        // "Blocking a member").
                        //
                        // Reachable from the roster as well as from a
                        // message, because a member row is where somebody
                        // looks for what they can do about a PERSON — and
                        // it is the only way to report one without singling
                        // out a message of theirs.
                        Menu {
                            Button {
                                reportingMember = member
                            } label: {
                                Label("Report…", systemImage: "flag")
                            }
                            if coordinator.blockedUserIDs.contains(member.id) {
                                Button {
                                    setBlocked(member, blocked: false)
                                } label: {
                                    Label("Unblock", systemImage: "lock.open")
                                }
                            } else {
                                Button(role: .destructive) {
                                    setBlocked(member, blocked: true)
                                } label: {
                                    Label("Block", systemImage: "nosign")
                                }
                            }
                        } label: {
                            Label("Safety", systemImage: "shield")
                        }
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
            } catch APIError.conflict(let code, _) where code == "family_full" {
                // The cap is re-checked at approval, because the roster can
                // fill between a request and the decision. The request stays
                // PENDING — a full family is a temporary condition, not a
                // decision — so say that rather than "pull to refresh",
                // which does nothing about it (docs/protocol.md,
                // `POST /families/join-requests/{id}/approve`).
                model.errorText = String(localized: "The family is full. Raise the member limit or wait for somebody to leave — the request is still waiting.", comment: "Error when approving a join request would exceed the family's member cap.")
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
                        aiHistory: family.aiHistory,
                        // Rotating a code changes nothing about the cap or
                        // the picture switch: carry the ones we hold, or
                        // this rebuild clears them.
                        aiVision: family.aiVision,
                        maxMembers: family.maxMembers)
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
                    aiHistory: updated.aiHistory,
                    aiVision: updated.aiVision,
                    // From the RESPONSE, not the held copy: unlike the
                    // invite code, the cap is not owner-gated, so the
                    // server's answer is complete and authoritative — and
                    // an absent key here genuinely means "no cap".
                    maxMembers: updated.maxMembers)
            } catch {
                model.errorText = String(localized: "Couldn't change the policy. Try again.")
            }
        }
    }

    private func setBlocked(_ member: MemberDTO, blocked: Bool) {
        Task {
            let ok = blocked
                ? await coordinator.block(userID: member.id)
                : await coordinator.unblock(userID: member.id)
            // Never optimistic: a block that failed silently would hide
            // rows the reader does not know are hidden, in a feature with
            // no error surface and no badge to notice it by.
            if !ok {
                model.errorText = String(localized: "Couldn't change that right now. Try again.")
            }
        }
    }

    private func report(_ member: MemberDTO, reason: ReportReason) {
        reportingMember = nil
        Task {
            // A PERSON, not a message: `messageID` is nil, which is what
            // makes this the only way to report somebody without singling
            // out one thing they said.
            let ok = await coordinator.report(
                reportedUserID: member.id, reason: reason.rawValue, messageID: nil)
            if !ok {
                model.errorText = String(localized: "Couldn't send the report. Try again.")
            }
        }
    }

    private func remove(_ member: MemberDTO) {
        Task {
            do {
                try await coordinator.api.removeMember(userID: member.id)
                settingsModel.members.removeAll { $0.id == member.id }
                ContactLinks.shared.unlink(userID: member.id)
                await coordinator.resync()
            } catch {
                model.errorText = String(localized: "Couldn't remove \(member.displayName). Try again.")
            }
        }
    }
}

#endif
