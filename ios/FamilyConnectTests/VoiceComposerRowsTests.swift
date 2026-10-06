//
//  VoiceComposerRowsTests.swift
//  FamilyConnectTests
//
//  #79, Phase 1: the rows that take the field's place must not grow the bar
//  (docs/audio-video-messages-2026-10-04.md, S2.3, S2.4, S2.6, S2.7). The
//  composer's height is what the thread re-pins against on every change
//  (ConversationView's header) — so the one fact about their ink that matters
//  is that each is exactly one control tall, whatever it says: the hold row
//  armed or not, the recording row with its two-line "Still recording" hint,
//  the Undo row, and the playing review chip no taller than the file chip it
//  replaced.
//

import CoreGraphics
import Foundation
import SwiftUI
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Voice rows keep the bar's height")
struct VoiceComposerRowsTests {

    /// The composer's control side at the default text size.
    private static let control: CGFloat = 36

    private func height(_ view: some View, width: CGFloat = 300) throws -> Int {
        let renderer = ImageRenderer(content: view.frame(width: width))
        renderer.scale = 1
        let image = try #require(renderer.cgImage, "the view did not render")
        return image.height
    }

    #if os(iOS)
    @Test("the hold row is one control tall, armed or not, warned or not", arguments: [
        (false, false), (true, false), (false, true), (true, true),
    ])
    func holdRow(armed: Bool, warning: Bool) throws {
        let h = try height(VoiceHoldRow(
            elapsed: 272, armed: armed, warning: warning, height: Self.control))
        #expect(h == Int(Self.control), "the hold row is \(h) points tall")
    }

    @Test("the recording row is one control tall, beside words or not, hinting or not", arguments: [
        (false, false, false), (true, false, false), (false, true, false), (false, false, true),
        (true, true, true),
    ])
    func recordingRow(besideDraft: Bool, stillRecording: Bool, warning: Bool) throws {
        let h = try height(VoiceRecordingRow(
            elapsed: 42, litBars: 5, besideDraft: besideDraft, warning: warning,
            stillRecording: stillRecording, control: Self.control,
            onDelete: {}, onStop: {}, onMagicTap: {}))
        #expect(h == Int(Self.control), "the recording row is \(h) points tall")
    }

    @Test("a narrow recording row drops the meter rather than growing")
    func narrowRecordingRow() throws {
        let h = try height(
            VoiceRecordingRow(
                elapsed: 42, litBars: 5, besideDraft: false, warning: false, stillRecording: false,
                control: Self.control, onDelete: {}, onStop: {}, onMagicTap: {}),
            width: 150)
        #expect(h == Int(Self.control))
    }

    @Test("the Undo row fits inside the field it covers")
    func undoRow() throws {
        let h = try height(
            VoiceUndoRow(
                recordedMS: 12_400, untilMS: 15_000, windowMS: 5_000, clock: { 12_500 }, onUndo: {})
                .frame(height: Self.control))
        #expect(h == Int(Self.control))
    }
    #endif

    @Test("the review chip of a voice note is no taller than the file chip it replaced")
    func reviewChip() throws {
        let url = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("chip.m4a")
        let note = StagedAttachment(
            prepared: MediaPrep.Prepared(fileURL: url, mime: "audio/mp4", kind: "audio", durationMS: 42_000),
            isVoiceNote: true)
        let file = StagedAttachment(
            prepared: MediaPrep.Prepared(fileURL: url, mime: "audio/mp4", kind: "audio", durationMS: 42_000))

        let voice = try height(StagedAttachmentChip(item: note, onRemove: {}))
        let plain = try height(StagedAttachmentChip(item: file, onRemove: {}))

        #expect(voice <= plain, "the voice note's chip grew the bar from \(plain) to \(voice) points")
        #expect(voice > 0)
    }
}
