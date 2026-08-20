//
//  SyncPlanTests.swift
//  FamilyConnectTests
//
//  The resync fetch planner: a step per chat where the server holds
//  something newer than the local cursor, nothing otherwise.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("SyncPlan")
struct SyncPlanTests {

    @Test("a chat with newer server messages yields a step from the local cursor")
    func plansBehindChat() {
        let steps = SyncPlan.make(
            chats: [.init(chatID: 42, serverLatestMessageID: 1338)],
            localCursors: [42: 1330])
        #expect(steps == [.init(chatID: 42, afterID: 1330)])
    }

    @Test("an up-to-date chat yields no step")
    func skipsCurrentChat() {
        let steps = SyncPlan.make(
            chats: [.init(chatID: 42, serverLatestMessageID: 1338)],
            localCursors: [42: 1338])
        #expect(steps.isEmpty)
    }

    @Test("an empty chat (no last message) yields no step")
    func skipsEmptyChat() {
        let steps = SyncPlan.make(
            chats: [.init(chatID: 7, serverLatestMessageID: nil)],
            localCursors: [:])
        #expect(steps.isEmpty)
    }

    @Test("a brand-new local chat starts from cursor 0")
    func newChatStartsFromZero() {
        let steps = SyncPlan.make(
            chats: [.init(chatID: 9, serverLatestMessageID: 500)],
            localCursors: [:])
        #expect(steps == [.init(chatID: 9, afterID: 0)])
    }

    @Test("local ahead of server (clock skew / dropped chat echo) yields no step")
    func localAheadYieldsNothing() {
        let steps = SyncPlan.make(
            chats: [.init(chatID: 42, serverLatestMessageID: 100)],
            localCursors: [42: 250])
        #expect(steps.isEmpty)
    }

    @Test("mixed chat list plans exactly the behind ones")
    func mixedList() {
        let steps = SyncPlan.make(
            chats: [
                .init(chatID: 1, serverLatestMessageID: 900),   // behind
                .init(chatID: 2, serverLatestMessageID: nil),   // empty
                .init(chatID: 3, serverLatestMessageID: 40),    // current
                .init(chatID: 4, serverLatestMessageID: 1200),  // new local
            ],
            localCursors: [1: 850, 3: 40])
        #expect(steps == [
            .init(chatID: 1, afterID: 850),
            .init(chatID: 4, afterID: 0),
        ])
    }

    // MARK: - Reaction catch-up planning

    @Test("a chat whose server max_reaction_seq is ahead yields a step from the stored cursor")
    func plansBehindReactionCursor() {
        let steps = SyncPlan.makeReactionSteps(
            chats: [.init(chatID: 42, serverLatestMessageID: 1338, serverMaxReactionSeq: 120)],
            localCursors: [42: 100])
        #expect(steps == [.init(chatID: 42, afterSeq: 100)])
    }

    @Test("an up-to-date reaction cursor yields no step")
    func skipsCurrentReactionCursor() {
        let steps = SyncPlan.makeReactionSteps(
            chats: [.init(chatID: 42, serverLatestMessageID: 1338, serverMaxReactionSeq: 120)],
            localCursors: [42: 120])
        #expect(steps.isEmpty)
    }

    @Test("a never-reacted chat (server omits max_reaction_seq → 0) yields no step")
    func skipsNeverReactedChat() {
        let steps = SyncPlan.makeReactionSteps(
            chats: [.init(chatID: 42, serverLatestMessageID: 1338)],
            localCursors: [:])
        #expect(steps.isEmpty)
    }

    @Test("a brand-new local chat with reaction history starts from cursor 0")
    func newChatReactionsStartFromZero() {
        let steps = SyncPlan.makeReactionSteps(
            chats: [.init(chatID: 9, serverLatestMessageID: 500, serverMaxReactionSeq: 7)],
            localCursors: [:])
        #expect(steps == [.init(chatID: 9, afterSeq: 0)])
    }

    @Test("mixed chat list plans exactly the reaction-behind ones")
    func mixedReactionList() {
        let steps = SyncPlan.makeReactionSteps(
            chats: [
                .init(chatID: 1, serverLatestMessageID: 900, serverMaxReactionSeq: 50),  // behind
                .init(chatID: 2, serverLatestMessageID: nil),                            // never reacted
                .init(chatID: 3, serverLatestMessageID: 40, serverMaxReactionSeq: 8),    // current
            ],
            localCursors: [1: 45, 3: 8])
        #expect(steps == [.init(chatID: 1, afterSeq: 45)])
    }
}
