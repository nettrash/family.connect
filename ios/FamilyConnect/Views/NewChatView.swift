//
//  NewChatView.swift
//  FamilyConnect
//
//  Member picker for starting (or reopening — POST /chats/direct is
//  get-or-create) a direct chat. Excludes self and members who left.
//  On success the sheet dismisses and hands the chatID to the caller,
//  which pushes the conversation onto the navigation path.
//

import SwiftData
import SwiftUI

struct NewChatView: View {
    /// Called with the (created or existing) chat to open.
    var onOpen: (Int64) -> Void

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss
    @Query(filter: #Predicate<MemberEntity> { !$0.isCurrentUser && !$0.hasLeft },
           sort: [SortDescriptor(\MemberEntity.displayName)])
    private var members: [MemberEntity]

    @State private var isWorking = false
    @State private var errorText: String?

    var body: some View {
        NavigationStack {
            Group {
                if members.isEmpty {
                    ContentUnavailableView(
                        "No one else yet",
                        systemImage: "person.2",
                        description: Text("Direct chats become available once another member joins the family."))
                } else {
                    List(members) { member in
                        Button {
                            open(member)
                        } label: {
                            HStack(spacing: 12) {
                                InitialsAvatar(title: member.displayName)
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
                errorText = "The server couldn't open a chat with them. Try again."
            }
        }
    }
}
