//
//  MessageIntentPlanTests.swift
//  FamilyConnectTests
//
//  The two questions a Shortcut cannot ask out loud, pinned (issue #118):
//  is there anything to send, and who is meant. Both are decided in
//  MessageIntentPlan precisely so they can be tested here — the intent
//  framework itself is not testable on the simulator, which is the same
//  division CallRequestRouterTests works to.
//
//  Every branch of `decide` is pinned, in the order the plan asks them, and
//  the picker's ids are pinned to the handles the call side already mints:
//  a Shortcut and a Siri call must not disagree about who `7` is.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Shortcuts message plan")
struct MessageIntentPlanTests {

    /// The store the plan is told about: one family chat (id 1, "The
    /// Smiths"), direct chats only for the ids listed, and a roster.
    private func directory(
        roster: [CallRequestRouter.Candidate] = [
            CallRequestRouter.Candidate(userID: 7, name: "Anna Smith"),
            CallRequestRouter.Candidate(userID: 8, name: "Bob"),
        ],
        blocked: [Int64] = [],
        directChats: [Int64: Int64] = [7: 70],
        familyChat: Int64? = 1,
        familyChatTitle: String? = "The Smiths"
    ) -> MessageIntentPlan.Directory {
        MessageIntentPlan.Directory(
            isActiveMember: { id in roster.contains { $0.userID == id } },
            isBlocked: { blocked.contains($0) },
            roster: { roster },
            directChatID: { directChats[$0] },
            familyChatID: { familyChat })
    }

    private func decide(
        _ recipient: MessageIntentPlan.Recipient,
        body: String?,
        attachments: Int = 0,
        signedIn: Bool = true,
        in directory: MessageIntentPlan.Directory
    ) -> MessageIntentPlan.Decision {
        MessageIntentPlan.decide(
            recipient: recipient, body: body, attachmentCount: attachments,
            signedIn: signedIn, in: directory)
    }

    // MARK: - Is there anything to send

    @Test("nothing goes while the app is signed out, whatever else is wrong")
    func signedOutAnswersFirst() {
        // Signed out AND empty AND over the cap: the one refusal the user
        // can fix by opening the app is the one they get.
        let refusal = decide(
            .familyChat, body: nil,
            attachments: ShareHandoff.maxAttachmentsPerMessage + 1,
            signedIn: false, in: directory())
        #expect(refusal == .refuse(.signedOut))
    }

    @Test("an empty message is refused, but an empty message with a file is sent")
    func emptyNeedsAnAttachmentToCount() {
        let held = directory()
        #expect(decide(.familyChat, body: nil, in: held) == .refuse(.nothingToSend))
        #expect(decide(.familyChat, body: "", in: held) == .refuse(.nothingToSend))
        #expect(
            decide(.familyChat, body: " \n\t ", in: held) == .refuse(.nothingToSend),
            "whitespace is not a message")
        #expect(
            decide(.familyChat, body: "   ", attachments: 1, in: held) == .send(chatID: 1, peerUserID: nil),
            "a voice note with no caption is still something to send")
        #expect(decide(.familyChat, body: "Hi", in: held) == .send(chatID: 1, peerUserID: nil))
    }

    @Test("the cap is the composer's, and it is refused rather than truncated")
    func attachmentCap() {
        let cap = ShareHandoff.maxAttachmentsPerMessage
        let held = directory()
        #expect(
            decide(.familyChat, body: nil, attachments: cap, in: held) == .send(chatID: 1, peerUserID: nil),
            "the cap itself is allowed")
        #expect(
            decide(.familyChat, body: nil, attachments: cap + 1, in: held) == .refuse(.tooManyAttachments),
            "half a Shortcut's files arriving silently is the bad answer")
    }

    // MARK: - Who is meant

    @Test("the family chat goes to the chat this device holds")
    func familyChat() {
        #expect(decide(.familyChat, body: "Dinner at eight", in: directory()) == .send(chatID: 1, peerUserID: nil))
        #expect(
            decide(.familyChat, body: "Dinner at eight", in: directory(familyChat: nil)) == .refuse(.noFamilyChat),
            "a device in no family has nothing to post to")
    }

    @Test("a picker id is exact: held chat, or the chat has to be opened")
    func memberIDIsExact() {
        let held = directory()
        #expect(decide(.member(userID: 7), body: "Hi", in: held) == .send(chatID: 70, peerUserID: 7))
        #expect(
            decide(.member(userID: 8), body: "Hi", in: held) == .openDirectThenSend(userID: 8),
            "Bob has no direct chat here yet")
    }

    @Test("an id that outlived the membership is refused, not sent")
    func staleOrLeftMember() {
        let held = directory()
        #expect(
            decide(.member(userID: 99), body: "Hi", in: held) == .refuse(.unknownRecipient),
            "nobody here by that id")
        let gone = directory(roster: [CallRequestRouter.Candidate(userID: 8, name: "Bob")])
        #expect(
            decide(.member(userID: 7), body: "Hi", in: gone) == .refuse(.unknownRecipient),
            "Anna left, or deleted her account: the roster gate says so")
    }

    @Test("a blocked member is refused out loud, by id and by name alike")
    func blocked() {
        let held = directory(blocked: [7])
        #expect(decide(.member(userID: 7), body: "Hi", in: held) == .refuse(.blocked(7)))
        #expect(decide(.named("Anna"), body: "Hi", in: held) == .refuse(.blocked(7)))
    }

    @Test("a name is matched the way Siri's is: whole name first, then a word of it")
    func namedMatches() {
        let held = directory()
        #expect(decide(.named("Anna Smith"), body: "Hi", in: held) == .send(chatID: 70, peerUserID: 7))
        #expect(
            decide(.named("anna"), body: "Hi", in: held) == .send(chatID: 70, peerUserID: 7),
            "case and the surname being absent do not matter")
        #expect(
            decide(.named("Smith"), body: "Hi", in: held) == .send(chatID: 70, peerUserID: 7),
            "a word of the name is enough")
        #expect(decide(.named("Zoe"), body: "Hi", in: held) == .refuse(.unknownRecipient))
        #expect(
            decide(.named("   "), body: "Hi", in: held) == .refuse(.unknownRecipient),
            "a name that is only spaces names nobody")
    }

    @Test("two members sharing a name is a refusal, unless blocking has answered it")
    func ambiguousName() {
        let twoAnnas = directory(
            roster: [
                CallRequestRouter.Candidate(userID: 7, name: "Anna"),
                CallRequestRouter.Candidate(userID: 9, name: "Anna"),
            ],
            directChats: [7: 70, 9: 90])
        let annas = [
            CallRequestRouter.Candidate(userID: 7, name: "Anna"),
            CallRequestRouter.Candidate(userID: 9, name: "Anna"),
        ]
        #expect(decide(.named("Anna"), body: "Hi", in: twoAnnas) == .refuse(.ambiguous(annas)))
        #expect(
            decide(.named("Anna"), body: "Hi", in: directory(
                roster: twoAnnas.roster(), blocked: [9], directChats: [7: 70, 9: 90])) == .send(chatID: 70, peerUserID: 7),
            "one callable Anna out of two is not a question")
        #expect(
            decide(.named("Anna"), body: "Hi", in: directory(
                roster: twoAnnas.roster(), blocked: [7, 9], directChats: [7: 70, 9: 90])) == .refuse(.blocked(7)),
            "none callable is a block, not an ambiguity")
    }

    @Test("a name only ever reaches an active member")
    func namedCannotReachOutsideTheGate() {
        // A store that still holds a left member's row, with a name to
        // match: the gate filters the roster before any matching.
        let held = directory(roster: [])
        #expect(decide(.named("Bob"), body: "Hi", in: held) == .refuse(.unknownRecipient))
    }

    // MARK: - The picker's ids

    @Test("a member's picker id is their call handle, and comes back the same member")
    func idsAreTheHandles() {
        let id = MessageIntentPlan.RecipientID.member(7)
        #expect(id == CallHandle.value(userID: 7))
        #expect(MessageIntentPlan.RecipientID.recipient(from: id) == .member(userID: 7))
        #expect(
            MessageIntentPlan.RecipientID.recipient(from: "familyconnect://7") == .member(userID: 7),
            "as tolerant as the handle the call side already accepts")
    }

    @Test("the family chat is an id no member can have")
    func familyChatID() {
        let id = MessageIntentPlan.RecipientID.familyChat
        #expect(MessageIntentPlan.RecipientID.recipient(from: id) == .familyChat)
        #expect(MessageIntentPlan.RecipientID.recipient(from: id.uppercased()) == .familyChat)
        #expect(CallHandle.userID(from: id) == nil, "it cannot be read as a member by accident")
    }

    @Test("anything else is no recipient at all")
    func junkIDs() {
        for junk in ["", "7", "familyconnect:", "familyconnect:x", "familyconnect:-3", "sms:1234"] {
            #expect(MessageIntentPlan.RecipientID.recipient(from: junk) == nil, "\(junk)")
        }
    }

    @Test("the picker offers the family chat first, then the roster in order")
    func choices() {
        let roster = [
            CallRequestRouter.Candidate(userID: 7, name: "Anna Smith"),
            CallRequestRouter.Candidate(userID: 8, name: "Bob"),
        ]
        let offered = MessageIntentPlan.choices(familyChatTitle: "The Smiths", roster: roster)
        #expect(offered == [
            MessageIntentPlan.Choice(id: MessageIntentPlan.RecipientID.familyChat, name: "The Smiths"),
            MessageIntentPlan.Choice(id: CallHandle.value(userID: 7), name: "Anna Smith"),
            MessageIntentPlan.Choice(id: CallHandle.value(userID: 8), name: "Bob"),
        ])
    }

    @Test("a family chat with no name is not offered")
    func unnamedFamilyChat() {
        #expect(MessageIntentPlan.choices(familyChatTitle: nil, roster: []).isEmpty)
        #expect(MessageIntentPlan.choices(familyChatTitle: "   ", roster: []).isEmpty)
    }
}
