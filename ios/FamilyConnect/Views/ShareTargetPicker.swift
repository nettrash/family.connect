//
//  ShareTargetPicker.swift
//  FamilyConnect
//
//  "Send to": the chat a share INTO the app should land in.
//
//  Shown by ChatListView (iOS) and MacChatView (macOS) when the share
//  extension has staged files and parked them on the session
//  (AppSession.pendingShareImport). Choosing a chat routes there and the
//  files land STAGED in that chat's composer — the person still presses
//  Send, because sharing into a family chat is choosing to say
//  something, not having said it.
//
//  The list is every chat a file may land in: the family chat pinned
//  first, direct chats by recency — the chat list's own order — and
//  never the assistant's chat, which takes no attachments
//  (ShareImport.isEligible; the server would refuse the send).
//
//  One file, both platforms: the list IS the view, and a sheet with a
//  title and a cancel button needs no per-platform architecture.
//

import SwiftData
import SwiftUI

struct ShareTargetPicker: View {
    /// Called with the chosen chat. The caller routes and consumes the
    /// parked files; this view only asks the question.
    var onChoose: (Int64) -> Void

    @Environment(\.dismiss) private var dismiss
    @Query private var chats: [ChatEntity]
    @Query private var members: [MemberEntity]

    /// Eligible chats in the chat list's order: family first (pinRank),
    /// then recency, then stable id.
    private var eligibleChats: [ChatEntity] {
        chats
            .filter { ShareImport.isEligible(chatKind: $0.kind) }
            .sorted { a, b in
                if a.pinRank != b.pinRank { return a.pinRank < b.pinRank }
                let dateA = a.lastMessageDate ?? .distantPast
                let dateB = b.lastMessageDate ?? .distantPast
                if dateA != dateB { return dateA > dateB }
                return a.chatID < b.chatID
            }
    }

    private var avatarVersions: [Int64: Int64] {
        Dictionary(members.map { ($0.userID, $0.avatarVersion) }, uniquingKeysWith: { first, _ in first })
    }

    var body: some View {
        NavigationStack {
            List(eligibleChats) { chat in
                Button {
                    onChoose(chat.chatID)
                } label: {
                    HStack(spacing: 12) {
                        InitialsAvatar(
                            title: chat.title,
                            isFamily: chat.kind == "family",
                            userID: chat.peerUserID,
                            avatarVersion: chat.peerUserID
                                .flatMap { avatarVersions[$0] } ?? 0)
                        Text(chat.title)
                            .foregroundStyle(.primary)
                        Spacer(minLength: 0)
                    }
                }
            }
            .navigationTitle("Send to")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            #if os(macOS)
            // A macOS sheet draws no title bar, so the navigation title
            // above never renders there — the same reason PollComposerView
            // hand-rolls its "New poll" headline. Say the question.
            .safeAreaInset(edge: .top, spacing: 0) {
                Text("Send to")
                    .font(.headline)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(16)
                    .background(.bar)
            }
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
        #if os(macOS)
        // A macOS sheet cannot be resized by the person using it, so it
        // sizes itself: wide enough for a name, tall enough for a family
        // (the extra height carries the headline row above the list).
        .frame(width: 360, height: 460)
        #endif
    }
}
