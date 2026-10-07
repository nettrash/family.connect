//
//  NotSentVoiceRowTests.swift
//  FamilyConnectTests
//
//  The "Voice message not sent" row is drawn ABOVE the composer's field, and
//  the composer's height is something the thread re-pins against on every
//  change (ConversationView's header). So the one fact about its ink that
//  matters is that it stays ONE ROW, whatever its reply and caption say:
//  each line is cut to one, and the row never grows into a block that
//  shoves the newest messages up the screen.
//

import CoreGraphics
import Foundation
import SwiftUI
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Not-sent voice row")
struct NotSentVoiceRowTests {

    private func entry(replyTo: ReplyToDTO?, caption: String?) -> ParkedRecordings.Entry {
        ParkedRecordings.Entry(
            id: "e1", chatID: 5, fileName: "e1.m4a", durationMS: 42_000,
            replyTo: replyTo, caption: caption,
            createdAt: Date(timeIntervalSince1970: 0))
    }

    private func height(of row: NotSentVoiceRow) throws -> Int {
        let renderer = ImageRenderer(content: row.frame(width: 360))
        renderer.scale = 1
        let image = try #require(renderer.cgImage, "the row did not render")
        return image.height
    }

    @Test("it stays one row tall, however long its reply and caption are")
    func staysOneRow() throws {
        let long = String(repeating: "a very long sentence that would wrap many times ", count: 12)
        let bare = try height(of: NotSentVoiceRow(
            entry: entry(replyTo: nil, caption: nil), replyAuthor: nil, onSend: {}, onDelete: {}))
        let full = try height(of: NotSentVoiceRow(
            entry: entry(
                replyTo: ReplyToDTO(messageID: 41, senderID: 7, excerpt: long),
                caption: long),
            replyAuthor: "Ana", onSend: {}, onDelete: {}))

        #expect(bare > 0)
        #expect(full <= 64, "a long caption grew the row to \(full) points")
    }
}
