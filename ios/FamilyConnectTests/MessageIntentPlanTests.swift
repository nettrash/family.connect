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
import UniformTypeIdentifiers
@testable import FamilyConnect

@Suite("Shortcuts message plan")
struct MessageIntentPlanTests {

    /// The store the plan is told about: the family chats listed (by
    /// default one, id 1), direct chats only for the ids listed, and a
    /// roster.
    private func directory(
        roster: [CallRequestRouter.Candidate] = [
            CallRequestRouter.Candidate(userID: 7, name: "Anna Smith"),
            CallRequestRouter.Candidate(userID: 8, name: "Bob"),
        ],
        blocked: [Int64] = [],
        directChats: [Int64: Int64] = [7: 70],
        familyChats: [Int64] = [1]
    ) -> MessageIntentPlan.Directory {
        MessageIntentPlan.Directory(
            isActiveMember: { id in roster.contains { $0.userID == id } },
            isBlocked: { blocked.contains($0) },
            roster: { roster },
            directChatID: { directChats[$0] },
            isFamilyChat: { familyChats.contains($0) })
    }

    private let smiths = MessageIntentPlan.Recipient.familyChat(chatID: 1)

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
            smiths, body: nil,
            attachments: ShareHandoff.maxAttachmentsPerMessage + 1,
            signedIn: false, in: directory())
        #expect(refusal == .refuse(.signedOut))
    }

    @Test("an empty message is refused, but an empty message with a file is sent")
    func emptyNeedsAnAttachmentToCount() {
        let held = directory()
        #expect(decide(smiths, body: nil, in: held) == .refuse(.nothingToSend))
        #expect(decide(smiths, body: "", in: held) == .refuse(.nothingToSend))
        #expect(
            decide(smiths, body: " \n\t ", in: held) == .refuse(.nothingToSend),
            "whitespace is not a message")
        #expect(
            decide(smiths, body: "   ", attachments: 1, in: held) == .send(chatID: 1, peerUserID: nil),
            "a voice note with no caption is still something to send")
        #expect(decide(smiths, body: "Hi", in: held) == .send(chatID: 1, peerUserID: nil))
    }

    @Test("the cap is the composer's, and it is refused rather than truncated")
    func attachmentCap() {
        let cap = ShareHandoff.maxAttachmentsPerMessage
        let held = directory()
        #expect(
            decide(smiths, body: nil, attachments: cap, in: held) == .send(chatID: 1, peerUserID: nil),
            "the cap itself is allowed")
        #expect(
            decide(smiths, body: nil, attachments: cap + 1, in: held) == .refuse(.tooManyAttachments),
            "half a Shortcut's files arriving silently is the bad answer")
    }

    // MARK: - Who is meant

    @Test("the family chat goes to the chat this device holds")
    func familyChat() {
        #expect(decide(smiths, body: "Dinner at eight", in: directory()) == .send(chatID: 1, peerUserID: nil))
        #expect(
            decide(smiths, body: "Dinner at eight", in: directory(familyChats: [])) == .refuse(.noFamilyChat),
            "a device in no family has nothing to post to")
    }

    @Test("with several families, each family chat is its own recipient")
    func severalFamilyChats() {
        // The Smiths (1) and the Joneses (2) on one account: a Shortcut aimed
        // at the Joneses goes there whichever family is on screen, and one
        // aimed at a family this account has left is refused, not rerouted.
        let both = directory(familyChats: [1, 2])
        #expect(decide(.familyChat(chatID: 2), body: "Hi", in: both) == .send(chatID: 2, peerUserID: nil))
        #expect(decide(.familyChat(chatID: 1), body: "Hi", in: both) == .send(chatID: 1, peerUserID: nil))
        #expect(
            decide(.familyChat(chatID: 3), body: "Hi", in: both) == .refuse(.noFamilyChat),
            "a family this device no longer holds")
        #expect(
            decide(.familyChat(chatID: 70), body: "Hi", in: both) == .refuse(.noFamilyChat),
            "a direct chat's id is not a family chat")
    }

    @Test("a member is reachable while ANY family this device holds lists them")
    func heldRoster() {
        // Rows from before several families: only the projection says.
        #expect(MessageIntentPlan.isOnAHeldRoster(familyRolesText: "", hasLeft: false))
        #expect(!MessageIntentPlan.isOnAHeldRoster(familyRolesText: "", hasLeft: true))
        // On the other family's roster: `hasLeft` is the on-screen family's
        // projection, and a Shortcut has no screen.
        #expect(
            MessageIntentPlan.isOnAHeldRoster(familyRolesText: "7=member", hasLeft: true),
            "a member of the family that is not on screen")
        #expect(MessageIntentPlan.isOnAHeldRoster(familyRolesText: "3=owner;7=member", hasLeft: false))
        // On no roster here: left every shared family, or a family this
        // account left took its roles with it (`dropFamily`).
        #expect(!MessageIntentPlan.isOnAHeldRoster(familyRolesText: FamilyIDList.noFamily, hasLeft: true))
        #expect(!MessageIntentPlan.isOnAHeldRoster(familyRolesText: FamilyIDList.noFamily, hasLeft: false))
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

    @Test("a family chat's id names its chat, and is an id no member can have")
    func familyChatID() {
        let id = MessageIntentPlan.RecipientID.familyChat(12)
        #expect(id == "familyconnect:family:12")
        #expect(MessageIntentPlan.RecipientID.recipient(from: id) == .familyChat(chatID: 12))
        #expect(MessageIntentPlan.RecipientID.recipient(from: id.uppercased()) == .familyChat(chatID: 12))
        #expect(CallHandle.userID(from: id) == nil, "it cannot be read as a member by accident")
    }

    @Test("anything else is no recipient at all")
    func junkIDs() {
        for junk in [
            "", "7", "familyconnect:", "familyconnect:x", "familyconnect:-3", "sms:1234",
            "familyconnect:family", "familyconnect:family:", "familyconnect:family:x",
            "familyconnect:family:0", "familyconnect:family:-1",
        ] {
            #expect(MessageIntentPlan.RecipientID.recipient(from: junk) == nil, "\(junk)")
        }
    }

    @Test("the picker offers every family chat first, in order, then the roster")
    func choices() {
        let roster = [
            CallRequestRouter.Candidate(userID: 7, name: "Anna Smith"),
            CallRequestRouter.Candidate(userID: 8, name: "Bob"),
        ]
        let offered = MessageIntentPlan.choices(
            familyChats: [
                MessageIntentPlan.FamilyChat(chatID: 1, title: "The Smiths"),
                MessageIntentPlan.FamilyChat(chatID: 2, title: "The Joneses"),
            ],
            roster: roster)
        #expect(offered == [
            MessageIntentPlan.Choice(id: "familyconnect:family:1", name: "The Smiths"),
            MessageIntentPlan.Choice(id: "familyconnect:family:2", name: "The Joneses"),
            MessageIntentPlan.Choice(id: CallHandle.value(userID: 7), name: "Anna Smith"),
            MessageIntentPlan.Choice(id: CallHandle.value(userID: 8), name: "Bob"),
        ])
    }

    @Test("a family chat with no name is not offered")
    func unnamedFamilyChat() {
        #expect(MessageIntentPlan.choices(
            familyChats: [MessageIntentPlan.FamilyChat(chatID: 1, title: "   ")], roster: []).isEmpty)
        #expect(MessageIntentPlan.choices(familyChats: [], roster: []).isEmpty)
    }
}

/// The runner's half: what is touched before a refusal, and what the
/// Shortcut is told about a message that is only queued (issue #118). The
/// store and the outbox are closures here, as they are in the app.
@MainActor
@Suite("Shortcuts message runner")
struct MessageIntentRunnerTests {

    private struct Unreadable: Error {}

    /// Signed in, two families' chats, Anna with a direct chat (70) and
    /// Bob without one; every send queues as "c:1" and settles as `states`
    /// says, one read per poll, the last one repeated.
    private func runner(
        signedIn: Bool = true,
        states: [MessageStatus?] = [.sent],
        knownNames: [Int64: String] = [7: "Anna Smith", 8: "Bob"]
    ) -> MessageIntentRunner {
        let runner = MessageIntentRunner()
        let roster = [
            CallRequestRouter.Candidate(userID: 7, name: "Anna Smith"),
            CallRequestRouter.Candidate(userID: 8, name: "Bob"),
        ]
        runner.roster = { roster }
        runner.isActiveMember = { id in roster.contains { $0.userID == id } }
        runner.directChatID = { $0 == 7 ? 70 : nil }
        runner.familyChats = {
            [
                MessageIntentPlan.FamilyChat(chatID: 1, title: "The Smiths"),
                MessageIntentPlan.FamilyChat(chatID: 2, title: "The Joneses"),
            ]
        }
        runner.memberName = { knownNames[$0] }
        runner.isSignedIn = { signedIn }
        runner.sendText = { _, _ in "c:1" }
        var remaining = states
        runner.deliveryState = { _ in
            remaining.count > 1 ? remaining.removeFirst() : remaining.first ?? nil
        }
        runner.sleep = { _ in }
        runner.deliveryWait = .seconds(1)
        runner.pollInterval = .milliseconds(250)
        return runner
    }

    private func failure(
        _ body: () async throws -> MessageIntentRunner.Outcome
    ) async -> MessageIntentRunner.Failure? {
        do {
            _ = try await body()
            return nil
        } catch let failure as MessageIntentRunner.Failure {
            return failure
        } catch {
            Issue.record("unexpected error \(error)")
            return nil
        }
    }

    @Test("a refusal is answered before any file is prepared")
    func refusalPreparesNothing() async {
        let runner = runner(signedIn: false)
        var prepared = 0
        runner.prepareFile = { _ in
            prepared += 1
            throw Unreadable()
        }
        let files = Array(
            repeating: ShortcutFile(fileURL: nil, data: Data([1]), type: .data, name: nil), count: 3)
        let refused = await failure {
            try await runner.send(recipient: .familyChat(chatID: 1), body: "Hi", files: files)
        }
        guard case .refusal(.signedOut) = refused else {
            Issue.record("expected signed out, got \(String(describing: refused))")
            return
        }
        #expect(prepared == 0, "nothing converted only to be thrown away")
    }

    @Test("\"Sent\" only once the server has it")
    func sentOnlyWhenDelivered() async throws {
        let outcome = try await runner(states: [.pending, .pending, .sent])
            .send(recipient: .member(userID: 7), body: "Hi", files: [])
        #expect(outcome == MessageIntentRunner.Outcome(delivered: true, name: "Anna Smith"))
    }

    @Test("still in the outbox when the wait ends is queued, not sent and not an error")
    func stillPendingIsQueued() async throws {
        let outcome = try await runner(states: [.pending])
            .send(recipient: .familyChat(chatID: 2), body: "Good morning", files: [])
        #expect(outcome == MessageIntentRunner.Outcome(delivered: false, name: "The Joneses"))
    }

    @Test("a message the server refused is the action's error")
    func refusedByTheServer() async {
        let runner = runner(states: [.pending, .failed])
        let failed = await failure {
            try await runner.send(recipient: .member(userID: 7), body: "Hi", files: [])
        }
        guard case .notDelivered = failed else {
            Issue.record("expected not delivered, got \(String(describing: failed))")
            return
        }
    }

    @Test("a direct chat is named by its person or not at all, never by a family")
    func namesNeverCross() async throws {
        let unnamed = try await runner(knownNames: [:])
            .send(recipient: .member(userID: 7), body: "Hi", files: [])
        #expect(unnamed == MessageIntentRunner.Outcome(delivered: true, name: nil))
        let family = try await runner().send(recipient: .familyChat(chatID: 1), body: "Hi", files: [])
        #expect(family == MessageIntentRunner.Outcome(delivered: true, name: "The Smiths"))
    }

    @Test("a member with no direct chat yet gets one opened, then the message")
    func opensTheDirectChat() async throws {
        let runner = runner()
        var opened: [Int64] = []
        var sentInto: [Int64] = []
        runner.openDirectChat = { userID in
            opened.append(userID)
            return 80
        }
        runner.sendText = { _, chatID in
            sentInto.append(chatID)
            return "c:2"
        }
        let outcome = try await runner.send(recipient: .member(userID: 8), body: "Hi", files: [])
        #expect(opened == [8])
        #expect(sentInto == [80])
        #expect(outcome == MessageIntentRunner.Outcome(delivered: true, name: "Bob"))
    }
}
