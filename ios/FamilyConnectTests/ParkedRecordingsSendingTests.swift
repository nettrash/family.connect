//
//  ParkedRecordingsSendingTests.swift
//  FamilyConnectTests
//
//  #79, Phase 1: the parked store's part in the Undo window
//  (docs/audio-video-messages-2026-10-04.md, S2.6). A release that sends is
//  parked marked "sending" before its row shows; while the window runs it is
//  NOT a "not sent" row — it has the Undo row — and a hand-off that does not
//  happen `settle`s it into an ordinary one, never an orphan.
//

import Foundation
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Parked recordings: the sending entry of an Undo window")
struct ParkedRecordingsSendingTests {

    private func store() throws -> (ParkedRecordings, URL) {
        let root = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("parked-sending-\(UUID().uuidString)", isDirectory: true)
        return (ParkedRecordings(root: { root }, account: { "u3-b" }), root)
    }

    private func recordingFile() throws -> URL {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("parked-sending-src-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let url = dir.appendingPathComponent("\(AudioRecorder.filePrefix)\(UUID().uuidString).m4a")
        try Data(count: 4096).write(to: url)
        return url
    }

    @Test("a sending entry is not a not-sent row while its window runs")
    func sendingIsNotWaiting() throws {
        let (parked, _) = try store()
        let sending = try #require(parked.park(
            fileAt: try recordingFile(), duration: 3, chatID: 5, replyTo: nil, caption: nil, sending: true))
        let waiting = try #require(parked.park(
            fileAt: try recordingFile(), duration: 4, chatID: 5, replyTo: nil, caption: nil))

        #expect(parked.entries(for: 5).count == 2)
        #expect(parked.waiting(for: 5) == [waiting])
        #expect(!parked.waiting(for: 5).contains(sending))
    }

    @Test("settle turns a sending entry into an ordinary not-sent row, kept on disk")
    func settleMakesItWait() throws {
        let (parked, root) = try store()
        let entry = try #require(parked.park(
            fileAt: try recordingFile(), duration: 3, chatID: 5, replyTo: nil, caption: nil, sending: true))
        let before = parked.revision

        parked.settle(entry)

        let waiting = parked.waiting(for: 5)
        #expect(waiting.count == 1)
        #expect(waiting.first?.sending == false)
        #expect(waiting.first?.id == entry.id)
        #expect(parked.revision > before, "a settled entry did not redraw the rows")
        #expect(parked.fileURL(for: entry) != nil)
        // And it is what a relaunch reads, not just what the cache says.
        let relaunched = ParkedRecordings(root: { root }, account: { "u3-b" })
        #expect(relaunched.waiting(for: 5).map(\.id) == [entry.id])
    }

    @Test("settling an entry that is already waiting, or gone, changes nothing")
    func settleIsQuietOtherwise() throws {
        let (parked, _) = try store()
        let entry = try #require(parked.park(
            fileAt: try recordingFile(), duration: 3, chatID: 5, replyTo: nil, caption: nil))
        let before = parked.revision
        parked.settle(entry)
        #expect(parked.revision == before)

        parked.remove(entry)
        let afterRemove = parked.revision
        parked.settle(entry)
        #expect(parked.revision == afterRemove)
        #expect(parked.entries(for: 5).isEmpty)
    }
}
