//
//  NewChatView.swift
//  FamilyConnect
//
//  Member picker for starting (or reopening — POST /chats/direct is
//  get-or-create) a direct chat. Excludes self, members who left, and
//  deleted accounts (which are not people to chat with any more).
//  On success the sheet dismisses and hands the chatID to the caller,
//  which pushes the conversation onto the navigation path.
//

// iOS only — the Mac has its own views (MacViews/).
#if os(iOS)

import SwiftData
import SwiftUI

struct NewChatView: View {
    /// Called with the (created or existing) chat to open.
    var onOpen: (Int64) -> Void

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss
    // A deleted account is nobody to start a chat with: it receives
    // nothing and can never be signed into again (protocol.md, "Deleting
    // an account").
    @Query(filter: #Predicate<MemberEntity> { !$0.isCurrentUser && !$0.hasLeft && !$0.accountDeleted },
           sort: [SortDescriptor(\MemberEntity.displayName)])
    private var members: [MemberEntity]

    /// The picker, minus anyone this reader has blocked.
    ///
    /// Filtered AFTER the fetch because `blockedUserIDs` is not a SwiftData
    /// column and cannot go in the `#Predicate`. protocol.md's steer:
    /// leave a blocked member out of the picker rather than letting the tap
    /// answer `blocked` — a refusal the user can trigger has no innocent
    /// reading.
    private var selectableMembers: [MemberEntity] {
        members.filter { !coordinator.blockedUserIDs.contains($0.userID) }
    }

    @State private var isWorking = false
    @State private var errorText: String?

    var body: some View {
        NavigationStack {
            Group {
                if selectableMembers.isEmpty {
                    ContentUnavailableView(
                        "No one else yet",
                        systemImage: "person.2",
                        description: Text("Direct chats become available once another member joins the family."))
                } else {
                    List(selectableMembers) { member in
                        Button {
                            open(member)
                        } label: {
                            HStack(spacing: 12) {
                                InitialsAvatar(
                                    title: member.displayName,
                                    userID: member.userID,
                                    avatarVersion: member.avatarVersion)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(member.displayName)
                                        .foregroundStyle(.primary)
                                    Text("@\(member.username)")
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                                Spacer()
                                if member.role == "owner" {
                                    Text("Owner")
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                }
                            }
                        }
                        .disabled(isWorking)
                    }
                }
            }
            .navigationTitle("New Chat")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .keyboardShortcut(.cancelAction)
                }
            }
            .overlay {
                if isWorking { ProgressView() }
            }
            .alert("Couldn't open the chat", isPresented: .init(
                get: { errorText != nil },
                set: { if !$0 { errorText = nil } })
            ) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(errorText ?? "")
            }
        }
    }

    private func open(_ member: MemberEntity) {
        isWorking = true
        let userID = member.userID
        Task {
            defer { isWorking = false }
            do {
                let chatID = try await coordinator.openDirectChat(with: userID)
                dismiss()
                onOpen(chatID)
            } catch {
                errorText = String(localized: "The server couldn't open a chat with them. Try again.")
            }
        }
    }
}

#endif
