//
//  OpenPollsView.swift
//  FamilyConnect
//
//  The family's open polls, on a surface of their own (docs/protocol.md,
//  "Finding the open ones").
//
//  WHY THIS EXISTS. A poll is an ordinary message, which is what makes it
//  cheap — the question is the body, so previews, pushes, reply excerpts and
//  the assistant's transcript all read it with no new case between them. It
//  is also what loses it: a poll is drawn where it was sent, and once the
//  family has talked past it there is no way back but scrolling. A decision
//  nobody can find is a decision nobody makes.
//
//  WHY A SURFACE AND NOT A BANNER. A banner above the thread was the obvious
//  cheaper answer and is the wrong one twice over. The thread renders a
//  bounded, deliberately NON-lazy window (two captured hang reports paid for
//  that bound), and the opening unread anchor plus its "N new" divider are
//  decided ONCE at open — a strip inserted above changes the geometry that
//  decision was made against, which is the exact class of bug the two-pass
//  scroll dance already exists to work around. And there is no cap on how
//  many polls may be open, so a banner can only ever show one of N.
//
//  WHY IT REUSES PollBubbleView. Voting from here must be the same act as
//  voting in the chat — same rules, same bar, same faces, same "N of M
//  voted", same author-only close. Drawing a second, simpler poll control
//  here would be a second place for those rules to drift.
//
//  ONE VIEW, PRESENTED AS A SHEET EVERYWHERE — and deliberately not the
//  board's three-shape treatment. The board needs the screen on an iPad and a
//  window on the Mac because its notes are POSITIONED: a wall squeezed into a
//  540pt box is a different wall. This is a list of questions, which reads the
//  same at any width, so a sheet on all three is the honest answer rather than
//  a smaller one.
//
//  Android counterpart: ui/polls/OpenPollsScreen.kt
//

import SwiftData
import SwiftUI

struct OpenPollsView: View {
    /// The family chat. Polls exist nowhere else, so this surface is only
    /// ever opened from it and always has one.
    let chatID: Int64

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss
    /// The roster, for the faces and the "N of M voted" denominator — the
    /// same source `ConversationView` draws a poll from, so a poll looks the
    /// same on both surfaces.
    @Query private var members: [MemberEntity]

    @State private var messages: [MessageDTO] = []
    @State private var isLoading = true
    @State private var errorText: String?
    /// Rows this reader has peeked at. Per row, per screen, never stored and
    /// never on the wire — the same peek a hidden row in the thread gets
    /// (protocol.md, "Blocking a member").
    @State private var revealed: Set<Int64> = []

    var body: some View {
        NavigationStack {
            content
                .navigationTitle("Open polls")
                .inlineNavigationTitle()
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Done") { dismiss() }
                            .keyboardShortcut(.cancelAction)
                    }
                }
        }
        #if os(macOS)
        // A Mac sheet cannot be resized by the person using it, so it is
        // sized here or it is wrong for everybody. Taller than the shared
        // form sheets because this one is a LIST of questions.
        .frame(width: 460, height: 520)
        #endif
        .task { await load() }
    }

    @ViewBuilder
    private var content: some View {
        if isLoading && messages.isEmpty {
            ProgressView()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let errorText, messages.isEmpty {
            ContentUnavailableView(
                "Couldn't load the polls",
                systemImage: "exclamationmark.triangle",
                description: Text(errorText))
        } else if messages.isEmpty {
            // Not a failure, and said as such: an empty list here means the
            // family has decided everything, which is the good outcome.
            ContentUnavailableView(
                "Nothing to decide",
                systemImage: "checkmark.circle",
                description: Text("Open polls stay here until they're closed. There are none right now."))
        } else {
            List(messages, id: \.id) { message in
                row(for: message)
            }
        }
    }

    /// The block rule, unchanged from the thread's `MessagePresentation
    /// .isHiddenByBlock`: not your own message, and the sender is on your
    /// list. A poll by somebody you blocked is a hidden row on this surface
    /// exactly as it is in the chat, or the block has a second door
    /// (protocol.md, "Finding the open ones").
    private func isHiddenByBlock(_ message: MessageDTO) -> Bool {
        message.senderID != currentUserID
            && coordinator.blockedUserIDs.contains(message.senderID)
            && !revealed.contains(message.id)
    }

    @ViewBuilder
    private func row(for message: MessageDTO) -> some View {
        if isHiddenByBlock(message) {
            // The placeholder and nothing else — no question, no name, no
            // options. A TAP reveals, as in the thread.
            HStack(spacing: 6) {
                Image(systemName: "hand.raised.slash")
                    .font(.caption)
                Text("Hidden — blocked member")
                    .font(.caption)
            }
            .foregroundStyle(.secondary)
            .padding(.vertical, 6)
            .contentShape(Rectangle())
            .onTapGesture { revealed.insert(message.id) }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(Text("Hidden message from a blocked member. Double tap to show it."))
            .accessibilityAddTraits(.isButton)
        } else if let poll = message.poll {
            VStack(alignment: .leading, spacing: 8) {
                // The QUESTION is the message body — it is not a field on the
                // poll, and that is the whole reason the endpoint returns
                // messages rather than polls.
                Text(message.body)
                    .font(.headline)
                Text(memberNames[message.senderID] ?? String(localized: "Someone"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                PollBubbleView(
                    poll: snapshot(of: poll),
                    currentUserID: currentUserID,
                    // Only the author may close, here as in the chat: the
                    // family owner does not outrank authorship.
                    isAuthor: message.senderID == currentUserID,
                    memberCount: familyMemberCount,
                    memberNames: memberNames,
                    avatarVersions: avatarVersions,
                    blockedUserIDs: coordinator.blockedUserIDs,
                    // The tap on the option you already hold is a RETRACT,
                    // here as in the chat: PollBubbleView draws that row as
                    // yours and says a tap clears it, and the coordinator
                    // decides the same way (`PollPresentation.tapRetracts`).
                    // Always PUTting instead was a silent no-op on this one
                    // surface — the server treats re-PUTting a held option
                    // as nothing, so nothing moved.
                    onVote: { optionID in
                        let snapshot = snapshot(of: poll)
                        if PollPresentation.tapRetracts(
                            optionID: optionID, in: snapshot, currentUserID: currentUserID)
                        {
                            retract(message.id)
                        } else {
                            vote(message.id, optionID)
                        }
                    },
                    onClose: { close(message.id) })
            }
            .padding(.vertical, 6)
        }
    }

    // MARK: - The same context the chat draws a poll with

    private var currentUserID: Int64 { AppSettings.currentUserID ?? -1 }

    /// How many people a poll could hear from: the live roster, neither the
    /// people who have left nor the accounts that were deleted — a tally must
    /// not go on counting somebody who no longer exists. The same rule
    /// `ConversationView` uses, so the two surfaces cannot disagree.
    private var familyMemberCount: Int {
        members.filter { !$0.hasLeft && !$0.accountDeleted }.count
    }

    private var memberNames: [Int64: String] {
        Dictionary(members.map { ($0.userID, $0.resolvedDisplayName) }, uniquingKeysWith: { first, _ in first })
    }

    private var avatarVersions: [Int64: Int64] {
        Dictionary(members.map { ($0.userID, $0.avatarVersion) }, uniquingKeysWith: { first, _ in first })
    }

    /// The wire object as the shared bubble wants it. Both carry exactly the
    /// same three things, because a `Poll` is `poll_seq`, `closed` and the
    /// options and nothing else — the question lives on the message.
    private func snapshot(of poll: PollDTO) -> PollSnapshot {
        PollSnapshot(
            pollSeq: poll.pollSeq,
            closed: poll.closed,
            options: poll.options.map { option in
                PollOptionSnapshot(id: option.id, text: option.text, votes: option.votes)
            })
    }

    // MARK: - Work

    private func load() async {
        errorText = nil
        do {
            messages = try await coordinator.api.openPolls(chatID: chatID)
        } catch {
            errorText = String(localized: "Try again in a moment.")
        }
        isLoading = false
    }

    /// Vote, then re-read.
    ///
    /// The vote endpoint answers with the poll's whole new state and the
    /// socket fans the same state to every other device, so this could patch
    /// one row in place — but a poll may have been CLOSED by its author while
    /// this list was open, and a closed poll belongs off this list. Re-reading
    /// is one request and gets both right.
    private func vote(_ messageID: Int64, _ optionID: Int64) {
        Task {
            do {
                _ = try await coordinator.api.vote(
                    chatID: chatID, messageID: messageID, optionID: optionID)
            } catch {
                errorText = String(localized: "Try again in a moment.")
            }
            await load()
        }
    }

    private func retract(_ messageID: Int64) {
        Task {
            do {
                _ = try await coordinator.api.retractVote(chatID: chatID, messageID: messageID)
            } catch {
                errorText = String(localized: "Try again in a moment.")
            }
            await load()
        }
    }

    /// Closing removes it from this list, which is the point of the list.
    private func close(_ messageID: Int64) {
        Task {
            do {
                _ = try await coordinator.api.closePoll(chatID: chatID, messageID: messageID)
            } catch {
                errorText = String(localized: "Try again in a moment.")
            }
            await load()
        }
    }
}
