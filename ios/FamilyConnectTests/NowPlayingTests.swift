//
//  NowPlayingTests.swift
//  FamilyConnectTests
//
//  #79, Phase 1: one thing plays at a time, a recording pauses it, and a
//  voice note in review or in the "not sent" row plays from the file on this
//  device through `.playback` — giving the session back when it stops
//  (docs/audio-video-messages-2026-10-04.md, S1.7, S2.7, S5.3).
//

import Foundation
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Now playing: one at a time, a recording pauses it, the review chip plays")
struct NowPlayingTests {

    // MARK: - The owner

    @Test("whoever starts playing pauses whoever was playing")
    func claimPausesThePrevious() {
        let owner = NowPlaying()
        var first = 0
        var second = 0
        let a = UUID()
        let b = UUID()

        owner.claim(a) { first += 1 }
        owner.claim(b) { second += 1 }

        #expect(first == 1)
        #expect(second == 0)
        #expect(owner.current == b)
    }

    @Test("claiming again for the same player pauses nothing")
    func reclaimIsQuiet() {
        let owner = NowPlaying()
        var paused = 0
        let a = UUID()
        owner.claim(a) { paused += 1 }
        owner.claim(a) { paused += 1 }
        #expect(paused == 0)
    }

    @Test("a release by somebody who is not playing changes nothing")
    func foreignReleaseIsIgnored() {
        let owner = NowPlaying()
        let a = UUID()
        owner.claim(a) {}
        owner.release(UUID())
        #expect(owner.current == a)
        owner.release(a)
        #expect(owner.current == nil)
        #expect(!owner.isPlaying)
    }

    @Test("pauseAll pauses what plays and says whether anything did")
    func pauseAll() {
        let owner = NowPlaying()
        #expect(owner.pauseAll() == false)
        var paused = 0
        owner.claim(UUID()) { paused += 1 }
        #expect(owner.pauseAll() == true)
        #expect(paused == 1)
        #expect(owner.current == nil)
        #expect(owner.pauseAll() == false)
    }

    // MARK: - The local player

    @MainActor
    final class Engine: LocalAudioEngine {
        var currentTime: TimeInterval = 0
        var duration: TimeInterval = 42
        var isPlaying = false
        var onFinish: (() -> Void)?
        var plays = 0
        var refusesToPlay = false

        func play() -> Bool {
            guard !refusesToPlay else { return false }
            plays += 1
            isPlaying = true
            return true
        }
        func pause() { isPlaying = false }
        func stop() { isPlaying = false }
        func finishes() {
            isPlaying = false
            currentTime = duration
            onFinish?()
        }
    }

    @MainActor
    final class World {
        let owner = NowPlaying()
        let player = LocalVoicePlayer()
        var engines: [Engine] = []
        var begins = 0
        var ends = 0
        let url = URL(fileURLWithPath: "/tmp/voice-note.m4a")

        init() {
            player.owner = owner
            player.session = PlaybackSessionControl(
                begin: { [unowned self] in self.begins += 1 },
                end: { [unowned self] in self.ends += 1 })
            player.makeEngine = { [unowned self] _ in
                let engine = Engine()
                self.engines.append(engine)
                return engine
            }
        }
    }

    @Test("play takes the session for playback and claims the app's one player")
    func playClaims() {
        let world = World()
        world.player.play(world.url)

        #expect(world.player.isPlaying)
        #expect(world.owner.current == world.player.id)
        #expect(world.begins == 1)
        #expect(world.engines.first?.plays == 1)
    }

    @Test("pause gives the session back and lets the owner go")
    func pauseReleases() {
        let world = World()
        world.player.play(world.url)
        world.player.pause()

        #expect(!world.player.isPlaying)
        #expect(world.owner.current == nil)
        #expect(world.ends == 1)
    }

    @Test("a recording starting pauses it through the owner")
    func recordingPausesIt() {
        let world = World()
        world.player.play(world.url)

        world.owner.pauseAll()

        #expect(!world.player.isPlaying)
        #expect(world.engines.first?.isPlaying == false)
        #expect(world.ends == 1)
    }

    @Test("playing to the end releases everything, and the next play starts from the top")
    func endThenReplay() throws {
        let world = World()
        world.player.play(world.url)
        let engine = try #require(world.engines.first)
        engine.finishes()

        #expect(!world.player.isPlaying)
        #expect(world.owner.current == nil)
        #expect(world.ends == 1)

        world.player.toggle(world.url)
        #expect(engine.currentTime == 0, "a replay did not start from the top")
        #expect(world.player.isPlaying)
        #expect(world.engines.count == 1, "a replay built a second engine")
    }

    @Test("a note paused at its very end plays again from the top, finish callback or not")
    func pausedAtTheEndRestarts() throws {
        let world = World()
        world.player.play(world.url)
        let engine = try #require(world.engines.first)
        // The last 50 ms: AVAudioPlayer's own end, before (or without) its
        // delegate saying so.
        engine.currentTime = engine.duration - 0.01
        world.player.pause()

        world.player.play(world.url)

        #expect(engine.currentTime == 0, "▶ at the end did nothing but claim the player")
        #expect(world.player.isPlaying)
    }

    @Test("toggle pauses what plays and plays what is paused")
    func togglePlaysAndPauses() {
        let world = World()
        world.player.toggle(world.url)
        #expect(world.player.isPlaying)
        world.player.toggle(world.url)
        #expect(!world.player.isPlaying)
    }

    @Test("an engine that will not play leaves nothing claimed and the session given back")
    func refusalLeavesNothing() {
        let world = World()
        world.player.makeEngine = { _ in
            let engine = Engine()
            engine.refusesToPlay = true
            return engine
        }
        world.player.play(world.url)

        #expect(!world.player.isPlaying)
        #expect(world.owner.current == nil)
        #expect(world.begins == world.ends)
    }

    @Test("stop lets the file go, so the chip can delete it")
    func stopLetsGo() {
        let world = World()
        world.player.play(world.url)
        world.player.stop()

        #expect(!world.player.isPlaying)
        #expect(world.owner.current == nil)
        #expect(world.player.elapsed == 0)
        world.player.play(world.url)
        #expect(world.engines.count == 2, "a stopped player reused the engine over the old file")
    }

    @Test("the elapsed time follows the engine while it plays")
    func elapsedFollows() throws {
        let world = World()
        world.player.play(world.url)
        let engine = try #require(world.engines.first)
        engine.currentTime = 12.3
        world.player.tick()
        #expect(world.player.elapsed == 12.3)
    }
}
