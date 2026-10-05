//
//  VoiceTips.swift
//  FamilyConnect
//
//  The one coach mark #79 allows (docs/audio-video-messages-2026-10-04.md,
//  S7.2): on a touch device, once per device, after the first hands-free
//  voice message sent from a touch screen — "You can also hold the
//  microphone while you talk." — anchored above the microphone, gone at any
//  tap, and never shown while a screen reader runs. Nothing else teaches:
//  no tour, no explainer screen, no mode bubble.
//
//  TipKit keeps the count on the device, which is exactly "once per device";
//  `Tips.configure()` runs once, at launch (FamilyConnectApp).
//

#if os(iOS)

import SwiftUI
import TipKit

struct HoldToTalkTip: Tip {
    /// A hands-free voice message went out from a touch screen.
    static let handsFreeSentByTouch = Tips.Event(id: "voice.handsFreeSentByTouch")

    /// VoiceOver runs: never shown then (S7.2).
    @Parameter
    static var screenReaderRuns: Bool = false

    var title: Text {
        Text("You can also hold the microphone while you talk.")
    }

    var rules: [Rule] {
        #Rule(Self.handsFreeSentByTouch) { $0.donations.count >= 1 }
        #Rule(Self.$screenReaderRuns) { $0 == false }
    }

    var options: [any TipOption] {
        [Tips.MaxDisplayCount(1)]
    }

    /// The moment it may appear.
    static func handsFreeSent() {
        Task { await handsFreeSentByTouch.donate() }
    }
}

#endif
