//
//  OpenPollsBadge.swift
//  FamilyConnect
//
//  What the open-polls badge counts, in ONE place (docs/protocol.md,
//  "Finding the open ones").
//
//  The rule is: **open polls this reader has not voted in**. Not "all open
//  polls", and not "polls created since you last looked".
//
//  Why not all of them. A badge is a claim that there is something for YOU to
//  do. A count of every open poll stays lit after you have answered all of
//  them, until somebody else gets round to closing them — so it stops meaning
//  anything within a day, and a family learns to ignore it. This one clears
//  itself the moment you vote, because voting is exactly the thing it was
//  asking for.
//
//  Why not a "seen" mark. The board needs one (`content_seq` and the mark
//  beside it) because a note is something to READ, and there is no act that
//  proves you have. A poll has one: the vote. So this needs no server field,
//  no DataStore key, no SwiftData column, no seed rule for a device that has
//  never looked, and nothing to keep in step — which is a whole apparatus
//  the board had to build and this does not.
//
//  Why it can be computed here at all. Every poll carries the full list of
//  user ids that chose each option, because a frame is serialised once and
//  sent to everybody: a field whose value depends on who is reading — "did I
//  vote" — cannot exist on the wire, and the protocol says in terms that
//  clients derive it from the list. This is that derivation, once, for both
//  Apple surfaces.
//
//  Android counterpart: util/OpenPollsBadge.kt — same rule, same vectors.
//

import Foundation

/// The two things the badge rule needs of a poll, whichever type is holding
/// it. `PollSnapshot` is what a synced message carries and `PollDTO` is what
/// the wire answers with; both say exactly this, so the rule is written once
/// against both rather than twice against each.
protocol PollVoteState {
    var closed: Bool { get }
    /// Everyone who has chosen any option.
    var voterIDs: [Int64] { get }
}

extension PollSnapshot: PollVoteState {
    var voterIDs: [Int64] { options.flatMap(\.votes) }
}

extension PollDTO: PollVoteState {
    var voterIDs: [Int64] { options.flatMap(\.votes) }
}

enum OpenPollsBadge {

    /// How many of these polls this reader still has to answer.
    ///
    /// A CLOSED poll never counts, whatever the reader did: a closed poll is
    /// a result, and there is nothing left to ask of anybody. The endpoint
    /// that feeds the surface returns only open ones, so that guard is belt
    /// and braces there — but the badge is computed over the polls the CHAT
    /// holds, where closed ones certainly do appear, and there it is the
    /// whole filter.
    static func count(polls: [some PollVoteState], currentUserID: Int64) -> Int {
        polls.filter { !$0.closed && !$0.voterIDs.contains(currentUserID) }.count
    }

    /// Whether this reader has voted in one poll.
    ///
    /// One choice per member, so any appearance is the appearance — but this
    /// deliberately does not assume that: it asks whether the id appears at
    /// all, which stays correct if the protocol ever grows multiple choice,
    /// and costs nothing today.
    static func hasVoted(in poll: some PollVoteState, userID: Int64) -> Bool {
        poll.voterIDs.contains(userID)
    }
}
