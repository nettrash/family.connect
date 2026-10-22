//
//  GroupViews.swift
//  FamilyConnect
//
//  The screens a GROUP needs beyond a conversation (docs/protocol.md,
//  "Groups", 2026-10-22; the decided design is docs/groups-2026-10-21.md):
//
//  - New Group — a name and the members to start it with. In the phone's
//    New Chat sheet, and behind a button in the Mac's toolbar.
//  - Group Info — the name, the members with the admin marked, and what the
//    reader may do: the admin renames, adds, removes and deletes (D1), and
//    every member may leave.
//
//  One file, both platforms, for ShareTargetPicker's reason: these are forms
//  in a sheet, and a sheet with a title, a list and two buttons needs no
//  per-platform architecture. What may be OFFERED is `GroupRules`, shared
//  with the tests; the server is the one that decides, and its refusals are
//  said in the sentences `GroupRules` maps them to.
//

import SwiftData
import SwiftUI

// MARK: - New Group

struct NewGroupView: View {
    /// Called with the created group, after the sheet dismisses, to open it.
    var onCreate: (Int64) -> Void

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss
    /// The family as it is now: never the reader, never somebody who has
    /// left, never a deleted account — the server refuses all three
    /// (`not_same_family`). A blocked member IS offered: a block is never a
    /// reason to refuse an add, in either direction (docs/protocol.md,
    /// "Groups").
    @Query(filter: #Predicate<MemberEntity> { !$0.isCurrentUser && !$0.hasLeft && !$0.accountDeleted },
           sort: [SortDescriptor(\MemberEntity.displayName)])
    private var members: [MemberEntity]

    @State private var title = ""
    @State private var chosen: Set<Int64> = []
    @State private var isWorking = false
    @State private var problem: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Group Name", text: $title)
                        .onSubmit { create() }
                }
                Section("Members") {
                    GroupMemberChecklist(members: members, chosen: $chosen)
                }
                if let problem {
                    Section {
                        Text(problem)
                            .foregroundStyle(.red)
                    }
                }
            }
            .macSheetForm()
            .disabled(isWorking)
            .navigationTitle("New Group")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .keyboardShortcut(.cancelAction)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Create") { create() }
                        .disabled(isWorking)
                }
            }
            .overlay {
                if isWorking { ProgressView() }
            }
        }
        .macSheetFrame(MacSheetSize.newGroup)
    }

    /// Checked here before anything is sent, in the order the form is
    /// filled in; the server checks the same and is the one that decides.
    private func create() {
        guard !isWorking else { return }
        if let reason = GroupRules.creationProblem(title: title, memberIDs: chosen) {
            problem = reason
            return
        }
        guard let name = GroupRules.validTitle(title) else { return }
        problem = nil
        isWorking = true
        // In the roster's order, which is the order a person ticked them
        // against; the server keeps the order they joined, and they all
        // join at once.
        let ids = members.map(\.userID).filter { chosen.contains($0) }
        Task {
            defer { isWorking = false }
            do {
                let chatID = try await coordinator.createGroup(title: name, memberIDs: ids)
                dismiss()
                onCreate(chatID)
            } catch {
                problem = GroupRules.createFailure(error)
            }
        }
    }
}

/// The family's members as rows that tick on and off — New Group's list and
/// Add Members' list.
private struct GroupMemberChecklist: View {
    let members: [MemberEntity]
    @Binding var chosen: Set<Int64>

    var body: some View {
        ForEach(members) { member in
            Button {
                if chosen.contains(member.userID) {
                    chosen.remove(member.userID)
                } else {
                    chosen.insert(member.userID)
                }
            } label: {
                HStack(spacing: 12) {
                    InitialsAvatar(
                        title: member.resolvedDisplayName,
                        userID: member.userID,
                        avatarVersion: member.avatarVersion,
                        size: 32)
                        .accessibilityHidden(true)
                    Text(member.resolvedDisplayName)
                        .foregroundStyle(.primary)
                    Spacer(minLength: 0)
                    Image(systemName: chosen.contains(member.userID) ? "checkmark.circle.fill" : "circle")
                        .foregroundStyle(chosen.contains(member.userID) ? Color.accentColor : Color.secondary)
                        .imageScale(.large)
                        .accessibilityHidden(true)
                }
                // The whole row is the target, not only the name.
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityAddTraits(chosen.contains(member.userID) ? .isSelected : [])
        }
    }
}

// MARK: - Group Info

struct GroupInfoView: View {
    let chatID: Int64

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss
    @Query private var chats: [ChatEntity]
    @Query private var members: [MemberEntity]

    @State private var isWorking = false
    @State private var problem: String?
    @State private var renaming = false
    @State private var draftTitle = ""
    @State private var addingMembers = false
    @State private var removing: Int64?
    @State private var confirmsLeave = false
    @State private var confirmsDelete = false

    init(chatID: Int64) {
        self.chatID = chatID
        _chats = Query(filter: #Predicate<ChatEntity> { $0.chatID == chatID })
    }

    private var chat: ChatEntity? { chats.first }
    private var memberIDs: [Int64] { chat?.groupMemberIDs ?? [] }
    private var adminID: Int64? { chat?.groupAdminUserID }
    private var me: Int64 { coordinator.currentUserID }
    private var groupName: String { chat?.title ?? "" }

    private func may(_ action: GroupRules.Action) -> Bool {
        GroupRules.may(action, userID: me, adminUserID: adminID, memberIDs: memberIDs)
    }

    private func member(_ userID: Int64) -> MemberEntity? {
        members.first { $0.userID == userID }
    }

    /// What a row calls somebody: "You" for the reader, the app's name for
    /// everybody else (a deleted account's translated placeholder
    /// included), and "Someone" for an id this device has no row for yet.
    private func name(of userID: Int64) -> String {
        if userID == me { return String(localized: "You") }
        return member(userID)?.resolvedDisplayName ?? String(localized: "Someone")
    }

    /// Who the admin may still add: the family as it is now, minus the
    /// group. A block is no reason to leave somebody out (docs/protocol.md,
    /// "Groups").
    private var addable: [MemberEntity] {
        let inGroup = Set(memberIDs)
        return members
            .filter { !$0.isCurrentUser && !$0.hasLeft && !$0.accountDeleted && !inGroup.contains($0.userID) }
            .sorted { $0.resolvedDisplayName.localizedStandardCompare($1.resolvedDisplayName) == .orderedAscending }
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text(groupName)
                        .font(.title3.weight(.semibold))
                    Text("Members: \(memberIDs.count)")
                        .foregroundStyle(.secondary)
                }
                Section("Members") {
                    ForEach(memberIDs, id: \.self) { userID in
                        memberRow(userID)
                    }
                }
                if may(.rename) || may(.addMembers) {
                    Section {
                        if may(.rename) {
                            Button("Rename Group") {
                                draftTitle = groupName
                                renaming = true
                            }
                        }
                        if may(.addMembers) {
                            Button("Add Members") { addingMembers = true }
                        }
                    }
                }
                Section {
                    if may(.leave) {
                        Button("Leave Group", role: .destructive) { confirmsLeave = true }
                    }
                    if may(.delete) {
                        Button("Delete Group", role: .destructive) { confirmsDelete = true }
                    }
                }
                if let problem {
                    Section {
                        Text(problem)
                            .foregroundStyle(.red)
                    }
                }
            }
            .macSheetForm()
            .disabled(isWorking)
            .navigationTitle("Group Info")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                        .keyboardShortcut(.defaultAction)
                }
            }
            .overlay {
                if isWorking { ProgressView() }
            }
            .modifier(GroupInfoPrompts(
                groupName: groupName,
                renaming: $renaming,
                draftTitle: $draftTitle,
                confirmsLeave: $confirmsLeave,
                confirmsDelete: $confirmsDelete,
                removing: $removing,
                removingName: removing.map(name(of:)) ?? "",
                onRename: rename,
                onLeave: leave,
                onDelete: deleteGroup,
                onRemove: remove))
            .sheet(isPresented: $addingMembers) {
                AddGroupMembersView(candidates: addable) { ids in
                    add(ids)
                }
            }
        }
        .macSheetFrame(MacSheetSize.groupInfo)
        // The group went while this was open — left or deleted here, or
        // removed by `chat_removed` from anywhere: there is nothing to show.
        .onChange(of: chat == nil) { _, gone in
            if gone { dismiss() }
        }
    }

    private func memberRow(_ userID: Int64) -> some View {
        HStack(spacing: 12) {
            InitialsAvatar(
                title: name(of: userID),
                userID: userID,
                avatarVersion: member(userID)?.avatarVersion ?? 0,
                size: 32)
                .accessibilityHidden(true)
            Text(name(of: userID))
            Spacer(minLength: 0)
            if userID == adminID {
                Text("Admin")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            if GroupRules.mayRemove(userID, userID: me, adminUserID: adminID, memberIDs: memberIDs) {
                // Its own target, so a tap on the row is not a removal.
                Button {
                    removing = userID
                } label: {
                    Image(systemName: "person.badge.minus")
                }
                .buttonStyle(.borderless)
                .accessibilityLabel(Text("Remove from Group"))
                .help(Text("Remove from Group"))
            }
        }
        .accessibilityElement(children: .combine)
    }

    // MARK: - The five changes

    private func rename() {
        guard let title = GroupRules.validTitle(draftTitle) else {
            problem = String(localized: "A group's name can be 1 to 64 characters.")
            return
        }
        guard title != groupName else { return }
        run { try await coordinator.renameGroup(chatID: chatID, title: title) }
    }

    private func add(_ ids: [Int64]) {
        guard !ids.isEmpty else { return }
        run { try await coordinator.addGroupMembers(chatID: chatID, userIDs: ids) }
    }

    private func remove(_ userID: Int64) {
        run { try await coordinator.removeGroupMember(chatID: chatID, userID: userID) }
    }

    private func leave() {
        run { try await coordinator.leaveGroup(chatID: chatID) }
    }

    private func deleteGroup() {
        run { try await coordinator.deleteGroup(chatID: chatID) }
    }

    /// One change at a time, its failure said under the form. A leave or a
    /// delete that succeeds drops the chat, and `onChange` above closes the
    /// sheet.
    private func run(_ change: @escaping () async throws -> Void) {
        guard !isWorking else { return }
        problem = nil
        isWorking = true
        Task {
            defer { isWorking = false }
            do {
                try await change()
            } catch {
                problem = GroupRules.changeFailure(error)
            }
        }
    }
}

/// Group Info's rename prompt and its three confirmations, as one modifier so
/// the form's own chain stays short enough to read.
private struct GroupInfoPrompts: ViewModifier {
    let groupName: String
    @Binding var renaming: Bool
    @Binding var draftTitle: String
    @Binding var confirmsLeave: Bool
    @Binding var confirmsDelete: Bool
    @Binding var removing: Int64?
    let removingName: String
    let onRename: () -> Void
    let onLeave: () -> Void
    let onDelete: () -> Void
    let onRemove: (Int64) -> Void

    func body(content: Content) -> some View {
        content
            .alert("Rename Group", isPresented: $renaming) {
                TextField("Group Name", text: $draftTitle)
                Button("Save") { onRename() }
                Button("Cancel", role: .cancel) {}
            }
            // The protocol's own sentences (docs/protocol.md, "Groups"):
            // deleting is final and for everyone, and leaving cannot be
            // undone from this side.
            .confirmationDialog(
                Text("Leave “\(groupName)”? You can come back only if its admin adds you again."),
                isPresented: $confirmsLeave,
                titleVisibility: .visible
            ) {
                Button("Leave Group", role: .destructive) { onLeave() }
                Button("Cancel", role: .cancel) {}
            }
            .confirmationDialog(
                Text("Delete “\(groupName)”? Its messages are deleted for everyone."),
                isPresented: $confirmsDelete,
                titleVisibility: .visible
            ) {
                Button("Delete Group", role: .destructive) { onDelete() }
                Button("Cancel", role: .cancel) {}
            }
            .confirmationDialog(
                Text(verbatim: removingName),
                isPresented: Binding(
                    get: { removing != nil },
                    set: { if !$0 { removing = nil } }),
                titleVisibility: .visible,
                presenting: removing
            ) { userID in
                Button("Remove from Group", role: .destructive) { onRemove(userID) }
                Button("Cancel", role: .cancel) {}
            }
    }
}

/// The admin's picker: the family members not in the group yet, ticked and
/// added together.
private struct AddGroupMembersView: View {
    let candidates: [MemberEntity]
    let onAdd: ([Int64]) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var chosen: Set<Int64> = []

    var body: some View {
        NavigationStack {
            Group {
                if candidates.isEmpty {
                    ContentUnavailableView(
                        "Add Members",
                        systemImage: "person.3",
                        description: Text("Everyone in the family is already in this group."))
                } else {
                    Form {
                        GroupMemberChecklist(members: candidates, chosen: $chosen)
                    }
                    .macSheetForm()
                }
            }
            .navigationTitle("Add Members")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .keyboardShortcut(.cancelAction)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        let ids = candidates.map(\.userID).filter { chosen.contains($0) }
                        dismiss()
                        onAdd(ids)
                    }
                    .disabled(chosen.isEmpty)
                }
            }
        }
        .macSheetFrame(MacSheetSize.groupInfo)
    }
}

// MARK: - The way in

/// A group's name in a conversation header, as the button that opens Group
/// Info — and every other chat's name as plain text. Its own view, owning
/// its own sheet, so a conversation adds nothing to a modifier chain that is
/// already at the type-checker's limit (ConversationView.body).
struct ConversationHeaderTitle: View {
    let chatID: Int64
    let title: String
    let isGroup: Bool
    /// "Anna is typing…", under the title, while somebody is.
    let typingLine: String?

    @State private var showsInfo = false

    var body: some View {
        Group {
            if isGroup {
                Button {
                    showsInfo = true
                } label: {
                    label
                }
                .buttonStyle(.plain)
                .accessibilityHint(Text("Group Info"))
                .sheet(isPresented: $showsInfo) {
                    GroupInfoView(chatID: chatID)
                }
            } else {
                label
            }
        }
        .animation(.easeInOut(duration: 0.15), value: typingLine)
    }

    private var label: some View {
        VStack(spacing: 0) {
            Text(title)
                .font(.headline)
                .lineLimit(1)
            if let typingLine {
                // One line: three people typing at once wrapped
                // the caption and pushed the title off its bar.
                Text(typingLine)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .transition(.opacity)
            }
        }
    }
}

/// The Mac's way in: a toolbar button beside the conversation's own items,
/// which a Mac reader looks for where a phone reader taps the title.
struct GroupInfoToolbarButton: View {
    let chatID: Int64

    @State private var showsInfo = false

    var body: some View {
        Button {
            showsInfo = true
        } label: {
            Label("Group Info", systemImage: "info.circle")
        }
        .help(Text("Group Info"))
        .sheet(isPresented: $showsInfo) {
            GroupInfoView(chatID: chatID)
        }
    }
}
