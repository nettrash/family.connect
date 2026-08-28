//
//  RingbackPlayerTests.swift
//  FamilyConnectTests
//
//  The player's arming logic over a fake sink: on the iPhone the sound
//  waits for CallKit's activation and survives a deactivation; on the Mac
//  it starts at once. Nothing here touches an audio device.
//

import Foundation
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Ringback player")
struct RingbackPlayerTests {

    @MainActor
    final class FakeSink: RingbackSink {
        var plays = 0
        var stops = 0
        var refuses = false
        func play() -> Bool {
            if refuses { return false }
            plays += 1
            return true
        }
        func stop() { stops += 1 }
    }

    /// What the player asked for: every sink made and the WAV it was
    /// given, plus a switch to have the next sink refuse to play.
    @MainActor
    final class Recorder {
        var made: [FakeSink] = []
        var data: [Data] = []
        var refuseNext = false
    }

    @MainActor
    final class Harness {
        let recorder = Recorder()
        let player: RingbackPlayer
        init(sessionLive: Bool, region: String? = "RS", grace: Duration = .seconds(30)) {
            let recorder = self.recorder
            player = RingbackPlayer(sessionLive: sessionLive, armedGrace: grace, region: { region }) { data in
                let sink = FakeSink()
                sink.refuses = recorder.refuseNext
                recorder.made.append(sink)
                recorder.data.append(data)
                return sink
            }
        }
        var made: [FakeSink] { recorder.made }
        var data: [Data] { recorder.data }
        func refuse(_ on: Bool) { recorder.refuseNext = on }
    }

    @Test("mac: a live session plays at once, stop silences, start is idempotent")
    func liveSession() {
        let h = Harness(sessionLive: true)
        h.player.start()
        #expect(h.made.count == 1)
        #expect(h.made[0].plays == 1)
        #expect(h.player.isPlaying)
        h.player.start()
        #expect(h.made.count == 1, "a second start makes no second player")
        h.player.stop()
        #expect(h.made[0].stops == 1)
        #expect(!h.player.isPlaying)
        h.player.stop()
        #expect(h.made[0].stops == 1, "stop is idempotent")
        #expect(!h.player.isWanted)
    }

    @Test("iphone: start before activation only arms; activation plays; deactivation silences and re-activation resumes")
    func armedUntilActivation() {
        let h = Harness(sessionLive: false)
        h.player.start()
        #expect(h.made.isEmpty, "nothing plays on a session that is not live")
        #expect(h.player.isWanted)
        h.player.audioSessionDidActivate()
        #expect(h.made.count == 1)
        #expect(h.made[0].plays == 1)
        h.player.audioSessionDidDeactivate()
        #expect(h.made[0].stops == 1)
        #expect(!h.player.isPlaying)
        #expect(h.player.isWanted, "a deactivation is not a stop")
        h.player.audioSessionDidActivate()
        #expect(h.made.count == 2, "resumed on a fresh player")
        h.player.stop()
        #expect(h.made[1].stops == 1)
        // A late activation after the stop plays nothing.
        h.player.audioSessionDidActivate()
        #expect(h.made.count == 2)
    }

    @Test("iphone: activation before the ring means the ring plays immediately")
    func activationFirst() {
        let h = Harness(sessionLive: false)
        h.player.audioSessionDidActivate()
        #expect(h.made.isEmpty)
        h.player.start()
        #expect(h.made.count == 1)
        #expect(h.made[0].plays == 1)
    }

    @Test("the region chooses the WAV; a refused play keeps the player armed")
    func regionAndRefusal() {
        let h = Harness(sessionLive: true, region: "US")
        h.refuse(true)
        h.player.start()
        #expect(h.made.count == 1)
        #expect(h.made[0].plays == 0)
        #expect(!h.player.isPlaying)
        #expect(h.player.isWanted)
        // The WAV handed over was the ANSI cycle (6 s at 8 kHz).
        #expect(h.data[0].count == 44 + 6 * 8000 * 2)
        // Once the session is activated again the play is retried.
        h.refuse(false)
        h.player.audioSessionDidActivate()
        #expect(h.made.count == 2)
        #expect(h.made[1].plays == 1)
    }

    @Test("a refused play is retried once after the grace period, without any activation")
    func refusalRetriedAfterGrace() async {
        let h = Harness(sessionLive: true, grace: .milliseconds(50))
        h.refuse(true)
        h.player.start()
        #expect(h.made.count == 1)
        #expect(!h.player.isPlaying)
        h.refuse(false)
        let deadline = ContinuousClock.now + .seconds(3)
        while !h.player.isPlaying, ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(20))
        }
        #expect(h.made.count == 2)
        #expect(h.player.isPlaying)
        // And a stop cancels the check: no third player appears later.
        h.player.stop()
        try? await Task.sleep(for: .milliseconds(120))
        #expect(h.made.count == 2)
    }

    @Test("iphone: the end of a call forgets a stale 'live' — the next ring waits for its own activation")
    func callEndForgetsTheSession() {
        let h = Harness(sessionLive: false)
        h.player.audioSessionDidActivate()
        h.player.start()
        #expect(h.made.count == 1)
        // The call ends without a didDeactivate (providerDidReset, or one
        // that is still on its way).
        h.player.callDidEnd()
        #expect(!h.player.isPlaying)
        #expect(!h.player.isSessionLive)
        // The next call rings before CallKit activates: nothing may play yet.
        h.player.start()
        #expect(h.made.count == 1, "no player until this call's activation")
        h.player.audioSessionDidActivate()
        #expect(h.made.count == 2)
    }

    @Test("mac: the end of a call leaves the always-live session live")
    func callEndOnMac() {
        let h = Harness(sessionLive: true)
        h.player.start()
        h.player.callDidEnd()
        #expect(h.player.isSessionLive)
        h.player.start()
        #expect(h.made.count == 2)
    }
}
