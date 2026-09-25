//
//  OpenPollsBadgeTests.swift
//  FamilyConnectTests
//
//  The open-polls badge rule (docs/protocol.md, "Finding the open ones").
//
//  The rule is "open polls this reader has not voted in", and every one of
//  the three obvious alternatives is wrong in a way a test can state:
//
//  - counting ALL open polls never clears by anything the reader can do, so
//    it stays lit until somebody else closes them and stops meaning anything;
//  - counting CLOSED ones asks for a decision that has already been made;
//  - counting somebody ELSE's outstanding polls is a badge about a person who
//    is not holding the phone.
//
//  The vectors here are mirrored by value in
//  android/…/util/OpenPollsBadgeTest.kt: same polls, same reader, same
//  numbers. Two ports that disagree about what a badge counts would show a
//  family two different numbers for the same chat.
//

import Foundation
import Testing
@testable import FamilyConnect

struct OpenPollsBadgeTests {

    private static let me: Int64 = 7
    private static let someoneElse: Int64 = 9

    private func poll(closed: Bool = false, votes: [Int64] = []) -> PollSnapshot {
        PollSnapshot(
            pollSeq: 1,
            closed: closed,
            options: [
                PollOptionSnapshot(id: 1, text: "Pizza", votes: votes),
                PollOptionSnapshot(id: 2, text: "Pasta", votes: []),
            ])
    }

    @Test("an open poll nobody has answered counts")
    func unansweredCounts() {
        #expect(OpenPollsBadge.count(polls: [poll()], currentUserID: Self.me) == 1)
    }

    @Test("voting clears it, which is the whole point of the rule")
    func votingClearsIt() {
        let answered = poll(votes: [Self.me])
        #expect(OpenPollsBadge.count(polls: [answered], currentUserID: Self.me) == 0)
        #expect(OpenPollsBadge.hasVoted(in: answered, userID: Self.me))
    }

    /// The badge is about the person holding the phone. Somebody else's vote
    /// answers nothing for this reader.
    @Test("somebody else's vote does not answer it for me")
    func anotherPersonsVoteDoesNotCount() {
        let theirs = poll(votes: [Self.someoneElse])
        #expect(OpenPollsBadge.count(polls: [theirs], currentUserID: Self.me) == 1)
        #expect(!OpenPollsBadge.hasVoted(in: theirs, userID: Self.me))
    }

    /// A closed poll is a RESULT. There is nothing left to ask of anybody,
    /// whether or not this reader ever answered it.
    @Test("a closed poll never counts, answered or not")
    func closedNeverCounts() {
        #expect(OpenPollsBadge.count(polls: [poll(closed: true)], currentUserID: Self.me) == 0)
        #expect(
            OpenPollsBadge.count(
                polls: [poll(closed: true, votes: [Self.me])], currentUserID: Self.me) == 0)
    }

    @Test("the count is over the whole set, and only the unanswered open ones")
    func theWholeSet() {
        let polls = [
            poll(),                                   // counts
            poll(votes: [Self.me]),                   // answered
            poll(votes: [Self.someoneElse]),          // counts
            poll(closed: true),                       // closed
            poll(closed: true, votes: [Self.me]),     // closed and answered
            poll(),                                   // counts
        ]
        #expect(OpenPollsBadge.count(polls: polls, currentUserID: Self.me) == 3)
    }

    @Test("no polls is no badge")
    func emptyIsZero() {
        #expect(OpenPollsBadge.count(polls: [PollSnapshot](), currentUserID: Self.me) == 0)
    }

    /// The rule is written once against both types the app holds a poll in —
    /// the snapshot a synced message carries, and the DTO the wire answers
    /// with — so the chat's badge and the surface's list cannot drift.
    @Test("the same rule reads a wire poll and a stored one alike")
    func bothTypesAgree() {
        let dto = PollDTO(
            pollSeq: 1,
            closed: false,
            options: [
                PollOptionDTO(id: 1, text: "Pizza", votes: [Self.someoneElse]),
                PollOptionDTO(id: 2, text: "Pasta", votes: []),
            ])
        #expect(OpenPollsBadge.count(polls: [dto], currentUserID: Self.me) == 1)
        #expect(OpenPollsBadge.count(polls: [dto], currentUserID: Self.someoneElse) == 0)
    }
}
