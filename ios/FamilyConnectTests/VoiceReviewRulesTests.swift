//
//  VoiceReviewRulesTests.swift
//  FamilyConnectTests
//
//  #79, Phase 1: two small rules of the voice flow that live outside the
//  reducer (docs/audio-video-messages-2026-10-04.md, S2.7, S2.9).
//
//  - A voice note IN REVIEW asks "Delete this recording?" from ten seconds,
//    exactly as the recording row and the not-sent row do (S1.1,
//    `DELETE_ASKS_FROM_MS`) — the chip's ✕ reads `StagedAttachment.deleteAsks`.
//  - The reducer names a haptic; the iPhone plays S2.9's `.sensoryFeedback`
//    for it, and nothing when there is no cue. (An iPad plays none — the
//    simulator these run on is an iPhone, so that half is the code's word.)
//

import Foundation
import SwiftUI
import Testing
#if os(iOS)
import UIKit
#endif
@testable import FamilyConnect

@MainActor
@Suite("Voice review: the chip's delete question, the iPhone's haptics")
struct VoiceReviewRulesTests {

    private func note(durationMS: Int?) -> StagedAttachment {
        let url = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("rules.m4a")
        return StagedAttachment(
            prepared: MediaPrep.Prepared(fileURL: url, mime: "audio/mp4", kind: "audio", durationMS: durationMS),
            isVoiceNote: true)
    }

    @Test("the review chip's ✕ asks from ten seconds, and not a millisecond sooner")
    func chipDeleteAsksFromTen() {
        #expect(!note(durationMS: nil).deleteAsks, "a note with no length asked")
        #expect(!note(durationMS: 9_999).deleteAsks, "9.999 s asked before deleting")
        #expect(note(durationMS: 10_000).deleteAsks, "10 s deleted without asking")
        #expect(note(durationMS: 42_000).deleteAsks)
        #expect(note(durationMS: 10_000).duration == 10)
    }

    #if os(iOS)
    @Test("each haptic the reducer names is S2.9's feedback on an iPhone; no cue, no feedback")
    func iPhoneHaptics() throws {
        try #require(UIDevice.current.userInterfaceIdiom == .phone, "these run on an iPhone simulator")
        let expected: [(RecordGesture.Haptic, SensoryFeedback)] = [
            (.light, .impact(weight: .light, intensity: 0.6)),
            (.success, .success),
            (.warning, .warning),
        ]
        for (index, pair) in expected.enumerated() {
            let cue = VoiceComposer.HapticCue(haptic: pair.0, serial: index)
            #expect(VoiceHaptics.feedback(for: cue) == pair.1, "\(pair.0) played the wrong feedback")
        }
        #expect(VoiceHaptics.feedback(for: nil) == nil)
    }
    #endif
}
