//
//  VoiceNoteReplayAndLargeTextTests.swift
//  FamilyConnectTests
//
//  Three things the Apple polish check found in #79's approved design, held
//  where they can be measured:
//
//    - a voice message that played to its end plays again: the row rests at
//      the start, and `AVPlayer.play()` at the end of its item plays nothing
//      — the row showed Pause over silence, holding the one-thing-plays
//      slot, with no end notification ever coming to give it back;
//    - the "Not sent" row keeps its Send and Delete inside its width at the
//      largest text sizes;
//    - the recorder's captions stop growing where four columns of them
//      still fit a phone;
//    - the speed chip's tap slack leaves the waveform above it alone.
//
//  The player is a real AVPlayer over a short generated file, muted, with
//  no microphone or camera anywhere.
//

import AVFoundation
import Foundation
import SwiftUI
import Testing
@testable import FamilyConnect
#if os(iOS)
import UIKit
#endif

@MainActor
@Suite("Voice notes: replay, and the largest text", .serialized)
struct VoiceNoteReplayAndLargeTextTests {

    // MARK: - Replay after the end

    /// Half a second of a quiet tone, as a CAF file nobody else touches.
    private func shortNote() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("fc-replay-\(UUID().uuidString).caf")
        let format = try #require(AVAudioFormat(standardFormatWithSampleRate: 8000, channels: 1))
        let file = try AVAudioFile(forWriting: url, settings: format.settings)
        let buffer = try #require(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 4000))
        buffer.frameLength = 4000
        for i in 0..<4000 { buffer.floatChannelData![0][i] = sin(Float(i) * 0.3) * 0.2 }
        try file.write(from: buffer)
        return url
    }

    /// A muted player over `url`, its item ready and sitting at its END —
    /// where a note that played through leaves it.
    private func playerAtTheEnd(_ url: URL) async throws -> AVPlayer {
        let player = AVPlayer(url: url)
        player.isMuted = true
        player.actionAtItemEnd = .pause
        let item = try #require(player.currentItem)
        for _ in 0..<60 where item.status != .readyToPlay {
            try await Task.sleep(for: .milliseconds(50))
        }
        try #require(item.status == .readyToPlay, "the generated note never became playable")
        _ = await player.seek(to: item.duration, toleranceBefore: .zero, toleranceAfter: .zero)
        #expect(AudioPlayerView.isAtEnd(player))
        return player
    }

    @Test("Play after the end plays it again, from the start")
    func playAfterTheEndReplays() async throws {
        let url = try shortNote()
        defer { try? FileManager.default.removeItem(at: url) }
        let player = try await playerAtTheEnd(url)
        defer { player.pause() }

        AudioPlayerView.resume(player)
        try await Task.sleep(for: .milliseconds(150))

        #expect(player.rate > 0, "Play at the end set nothing going (rate \(player.rate))")
        #expect(player.currentTime().seconds < 0.4,
                "it did not start again from the start (at \(player.currentTime().seconds) s)")
        #expect(!AudioPlayerView.isAtEnd(player))
    }

    @Test("the end puts the player back at the start, where the row rests")
    func theEndRewinds() async throws {
        let url = try shortNote()
        defer { try? FileManager.default.removeItem(at: url) }
        let player = try await playerAtTheEnd(url)

        AudioPlayerView.rewind(player)
        for _ in 0..<40 where AudioPlayerView.isAtEnd(player) {
            try await Task.sleep(for: .milliseconds(25))
        }

        #expect(!AudioPlayerView.isAtEnd(player))
        #expect(player.currentTime().seconds < 0.05)
    }

    @Test("Play part-way through carries on from there")
    func playMidwayResumes() async throws {
        let url = try shortNote()
        defer { try? FileManager.default.removeItem(at: url) }
        let player = try await playerAtTheEnd(url)
        defer { player.pause() }
        _ = await player.seek(
            to: CMTime(seconds: 0.25, preferredTimescale: 600), toleranceBefore: .zero, toleranceAfter: .zero)
        #expect(!AudioPlayerView.isAtEnd(player))

        AudioPlayerView.resume(player)

        #expect(player.currentTime().seconds >= 0.2, "a pause part-way was sent back to the start")
    }

    // MARK: - The speed chip leaves the waveform its taps

    @Test("the speed chip's tap slack reaches sideways and down, never up into the waveform")
    func chipSlackStaysOffTheWaveform() {
        let chip = CGRect(x: 100, y: 40, width: 30, height: 14)
        let target = VoiceSpeedChipTarget().path(in: chip)
        #expect(target.boundingRect.minY == chip.minY, "the slack reaches up into the waveform")
        #expect(target.contains(CGPoint(x: chip.minX - 9, y: chip.midY)))
        #expect(target.contains(CGPoint(x: chip.maxX + 9, y: chip.midY)))
        #expect(target.contains(CGPoint(x: chip.midX, y: chip.maxY + 9)))
        #expect(!target.contains(CGPoint(x: chip.midX, y: chip.minY - 1)))
    }

    // MARK: - The largest text

    #if os(iOS)
    private static func fitted(_ view: some View, width: CGFloat) -> CGSize {
        let host = UIHostingController(rootView: view)
        return host.sizeThatFits(in: CGSize(width: width, height: .greatestFiniteMagnitude))
    }

    private static let entry = ParkedRecordings.Entry(
        id: "e1", chatID: 5, fileName: "e1.m4a", durationMS: 42_000,
        replyTo: nil, caption: nil, createdAt: Date(timeIntervalSince1970: 0),
        waveform: String(repeating: "39cf", count: 12))

    /// 358 is an iPhone 17's composer; 343 an iPhone SE's.
    @Test("the Not sent row keeps Send and Delete inside the row at every text size",
          arguments: DynamicTypeSize.allCases)
    func notSentRowFits(size: DynamicTypeSize) {
        for width: CGFloat in [358, 343] {
            let row = NotSentVoiceRow(entry: Self.entry, replyAuthor: nil, onSend: {}, onDelete: {})
                .environment(\.dynamicTypeSize, size)
            let natural = Self.fitted(row, width: width)
            // A row that fits fills the width it is offered, to the pixel
            // (a third of a point over, at 3×); one that does not asks for
            // well over — 488 points of 358 before the cap.
            #expect(natural.width < width + 1, "at \(size) the row needs \(natural.width) of \(width) points")
        }
    }

    @Test("the recorder's captions grow up to their largest size and no further")
    func captionsStopGrowing() {
        func size(_ type: DynamicTypeSize) -> CGSize {
            Self.fitted(
                RecorderCaption(text: "Voice message").environment(\.dynamicTypeSize, type),
                width: 1000)
        }
        let largest = size(RecorderCaption.largestType)
        #expect(size(.large).width < largest.width, "the caption does not grow with the text at all")
        #expect(size(.accessibility5) == largest, "past its largest size the caption still grew")
    }
    #endif
}
