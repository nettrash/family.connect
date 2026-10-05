//
//  RoundVideoPlaybackTests.swift
//  FamilyConnectTests
//
//  Playing a circle (#79, docs/audio-video-messages-2026-10-04.md, S5.3, S4's
//  last column, S1.7): the tap rules, loading and failure, one thing at a
//  time, the session, the end and the unplayed dot — and the app-wide owner's
//  Phase 2 half: what pauses everything, and keeping the screen awake.
//
//  The engine is a fake: nothing here needs a network, a decoder or an
//  audio session, and every rule is about what is asked of them.
//

import AVFoundation
import Foundation
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Round video playback")
struct RoundVideoPlaybackTests {

    @MainActor
    final class Engine: RoundVideoEngine {
        var player: AVPlayer? { nil }
        var events: ((RoundVideoEvent) -> Void)?
        var loads = 0
        var plays = 0
        var pauses = 0
        var rewinds = 0
        var stops = 0

        func load(events: @escaping @MainActor (RoundVideoEvent) -> Void) {
            loads += 1
            self.events = events
        }
        func play() { plays += 1 }
        func pause() { pauses += 1 }
        func rewind() { rewinds += 1 }
        func stop() { stops += 1 }
        func send(_ event: RoundVideoEvent) { events?(event) }
    }

    @MainActor
    final class World {
        let owner = NowPlaying()
        let playback = RoundVideoPlayback()
        var engines: [Engine] = []
        var begins = 0
        var ends = 0
        var recording = false
        var finished = 0

        init(canBuild: Bool = true) {
            playback.owner = owner
            playback.session = PlaybackSessionControl(
                begin: { [unowned self] in self.begins += 1 },
                end: { [unowned self] in self.ends += 1 })
            playback.isRecording = { [unowned self] in self.recording }
            playback.onFinished = { [unowned self] in self.finished += 1 }
            playback.makeEngine = canBuild
                ? { [unowned self] in
                    let engine = Engine()
                    self.engines.append(engine)
                    return engine
                }
                : { nil }
        }

        var engine: Engine { engines.last! }
    }

    // MARK: - A tap

    @Test("a tap loads it, claims the app's one player and takes the session")
    func aTapLoads() {
        let world = World()
        #expect(world.playback.tap() == .started)
        #expect(world.playback.phase == .loading)
        #expect(world.engine.loads == 1)
        #expect(world.owner.current == world.playback.id)
        #expect(world.owner.currentKind == .circle)
        #expect(world.begins == 1)

        world.engine.send(.playing)
        #expect(world.playback.phase == .playing)
        #expect(world.playback.hasFrames)
    }

    @Test("a tap while it plays pauses it, lets the owner go and gives the session back")
    func aTapPauses() {
        let world = World()
        world.playback.tap()
        world.engine.send(.playing)

        #expect(world.playback.tap() == .paused)
        #expect(world.playback.phase == .paused)
        #expect(world.engine.pauses == 1)
        #expect(world.owner.current == nil)
        #expect(world.ends == 1)

        // And another plays on from there, with the same engine.
        #expect(world.playback.tap() == .started)
        #expect(world.playback.phase == .playing)
        #expect(world.engine.plays == 1)
        #expect(world.engines.count == 1)
        #expect(world.begins == 2)
    }

    @Test("a second tap while it loads gives up")
    func aTapWhileLoadingGivesUp() {
        let world = World()
        world.playback.tap()
        #expect(world.playback.tap() == .gaveUp)
        #expect(world.playback.phase == .idle)
        #expect(world.engine.stops == 1)
        #expect(world.owner.current == nil)
        #expect(world.ends == 1)
    }

    @Test("nothing plays under a recording")
    func refusedWhileRecording() {
        let world = World()
        world.recording = true
        #expect(world.playback.tap() == .refusedWhileRecording)
        #expect(world.playback.phase == .idle)
        #expect(world.engines.isEmpty)
        #expect(world.owner.current == nil)
        #expect(world.begins == 0)
    }

    // MARK: - Loading, stalls and failure

    @Test("a stall shows the loading ring again, and frames moving take it away")
    func aStall() {
        let world = World()
        world.playback.tap()
        world.engine.send(.playing)
        world.engine.send(.waiting)
        #expect(world.playback.phase == .loading)
        world.engine.send(.playing)
        #expect(world.playback.phase == .playing)
    }

    @Test("a failure leaves the poster and the sentence; a tap tries again with a new engine")
    func failureAndRetry() {
        let world = World()
        world.playback.tap()
        world.engine.send(.failed)
        #expect(world.playback.phase == .failed)
        #expect(world.engines[0].stops == 1)
        #expect(world.owner.current == nil)
        #expect(world.ends == 1)

        #expect(world.playback.tap() == .started)
        #expect(world.playback.phase == .loading)
        #expect(world.engines.count == 2)
        #expect(world.engine.loads == 1)
    }

    @Test("nothing to stream from is a failure, not an endless ring")
    func noEngineFails() {
        let world = World(canBuild: false)
        world.playback.tap()
        #expect(world.playback.phase == .failed)
        #expect(world.owner.current == nil)
        #expect(world.begins == world.ends)
    }

    // MARK: - Progress and the end

    @Test("progress is clamped to the ring")
    func progressIsClamped() {
        let world = World()
        world.playback.tap()
        world.engine.send(.playing)
        world.engine.send(.progress(0.5))
        #expect(world.playback.progress == 0.5)
        world.engine.send(.progress(1.7))
        #expect(world.playback.progress == 1)
        world.engine.send(.progress(-1))
        #expect(world.playback.progress == 0)
    }

    @Test("at the end it returns to the poster, rewound, and loses its dot")
    func theEnd() {
        let world = World()
        world.playback.tap()
        world.engine.send(.playing)
        world.engine.send(.progress(0.9))
        world.engine.send(.finished)

        #expect(world.playback.phase == .idle)
        #expect(world.playback.progress == 0)
        #expect(!world.playback.hasFrames)
        #expect(world.engine.rewinds == 1)
        #expect(world.finished == 1)
        #expect(world.owner.current == nil)
        #expect(world.ends == 1)

        // A tap plays it again from the bytes it has.
        world.playback.tap()
        #expect(world.engines.count == 1)
        #expect(world.engine.plays == 1)
    }

    @Test("a late report after it stopped changes nothing")
    func lateReportsAreIgnored() {
        let world = World()
        world.playback.tap()
        let engine = world.engine
        world.playback.stop()
        engine.send(.finished)
        engine.send(.progress(0.5))
        #expect(world.finished == 0)
        #expect(world.playback.progress == 0)
        #expect(world.playback.phase == .idle)
    }

    // MARK: - One thing at a time

    @Test("another player starting pauses the circle; a recording starting does too")
    func theOwnerPausesIt() {
        let world = World()
        world.playback.tap()
        world.engine.send(.playing)

        world.owner.claim(UUID()) {}
        #expect(world.playback.phase == .paused)
        #expect(world.engine.pauses == 1)
        #expect(world.ends == 1)

        let again = World()
        again.playback.tap()
        again.engine.send(.playing)
        again.owner.pauseAll()
        #expect(again.playback.phase == .paused)
        #expect(again.ends == 1)
    }

    @Test("paused by the owner while it is still loading, it lets the load go")
    func theOwnerStopsALoad() {
        let world = World()
        world.playback.tap()
        world.owner.pauseAll()
        #expect(world.playback.phase == .idle)
        #expect(world.engine.stops == 1)
        #expect(world.ends == 1)
    }

    @Test("scrolled away or closed, it stops and gives everything back once")
    func stopReleases() {
        let world = World()
        world.playback.tap()
        world.engine.send(.playing)
        world.playback.stop()
        world.playback.stop()
        #expect(world.playback.phase == .idle)
        #expect(world.engine.stops == 1)
        #expect(world.owner.current == nil)
        #expect(world.begins == 1)
        #expect(world.ends == 1)
    }
}

// MARK: - The owner's Phase 2 half

@MainActor
@Suite("Now playing: what pauses it, and the screen")
struct NowPlayingSystemTests {

    @MainActor
    final class Screen {
        var calls: [Bool] = []
        var heldElsewhere = false
    }

    private func owner(_ screen: Screen) -> NowPlaying {
        NowPlaying(
            keepAwake: { screen.calls.append($0) },
            screenHeldElsewhere: { screen.heldElsewhere })
    }

    @Test("the screen stays awake from the first claim until nothing plays")
    func keepsTheScreenAwake() {
        let screen = Screen()
        let owner = owner(screen)
        let a = UUID()
        owner.claim(a) {}
        owner.claim(UUID()) {}
        #expect(screen.calls == [true], "a hand-over is not a gap")
        owner.pauseAll()
        #expect(screen.calls == [true, false])

        owner.claim(a) {}
        owner.release(a)
        #expect(screen.calls == [true, false, true, false])
    }

    @Test("a recording or a call holding the switch keeps it on when playback stops")
    func neverSwitchesItOffUnderOthers() {
        let screen = Screen()
        let owner = owner(screen)
        owner.claim(UUID()) {}
        screen.heldElsewhere = true
        owner.pauseAll()
        #expect(screen.calls == [true])
    }

    @Test("a minimised window or a hidden app pauses a circle and lets a voice note play")
    func circlesOnly() {
        let screen = Screen()
        let owner = owner(screen)
        var voicePaused = 0
        owner.claim(UUID(), kind: .voice) { voicePaused += 1 }
        #expect(!owner.pauseCircles())
        #expect(voicePaused == 0)
        #expect(owner.isPlaying)

        var circlePaused = 0
        owner.claim(UUID(), kind: .circle) { circlePaused += 1 }
        #expect(owner.pauseCircles())
        #expect(circlePaused == 1)
        #expect(!owner.isPlaying)
    }

    @Test("another app's sound pauses whatever plays")
    func interruption() {
        let owner = owner(Screen())
        var paused = 0
        owner.claim(UUID()) { paused += 1 }
        owner.interruptionBegan()
        #expect(paused == 1)
    }

    #if os(iOS)
    @Test("headphones going pause it; headphones coming play on")
    func headphones() {
        let owner = owner(Screen())
        var paused = 0
        owner.claim(UUID()) { paused += 1 }
        owner.routeChanged(.newDeviceAvailable)
        owner.routeChanged(.categoryChange)
        #expect(paused == 0)
        owner.routeChanged(.oldDeviceUnavailable)
        #expect(paused == 1)
    }
    #endif
}

// MARK: - The unplayed dot's memory

@MainActor
@Suite("Round video plays: per account, capped, wiped")
struct RoundVideoPlaysTests {

    private func defaults() -> UserDefaults {
        let name = "round-plays-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return defaults
    }

    @Test("a played video is remembered for its account, and only there")
    func perAccount() {
        let store = defaults()
        var account: String? = "u7-a"
        let plays = RoundVideoPlays(defaults: store, account: { account })
        #expect(!plays.isPlayed(91))
        plays.markPlayed(91)
        #expect(plays.isPlayed(91))

        account = "u8-a"
        #expect(!plays.isPlayed(91), "the next account inherited this one's dots")
        account = nil
        #expect(!plays.isPlayed(91))
        plays.markPlayed(92)
        account = "u7-a"
        #expect(!plays.isPlayed(92))

        // And it survives a relaunch: a new store over the same defaults.
        let again = RoundVideoPlays(defaults: store, account: { "u7-a" })
        #expect(again.isPlayed(91))
    }

    @Test("the newest videos are the ones kept")
    func capKeepsTheNewest() {
        let plays = RoundVideoPlays(defaults: defaults(), account: { "u7-a" }, cap: 3)
        for id in [10, 40, 20, 30] as [Int64] { plays.markPlayed(id) }
        #expect(!plays.isPlayed(10))
        #expect(plays.isPlayed(20) && plays.isPlayed(30) && plays.isPlayed(40))
        #expect(RoundVideoPlays.cap == 5_000)
    }

    @Test("a provisional id is never kept")
    func provisionalIgnored() {
        let plays = RoundVideoPlays(defaults: defaults(), account: { "u7-a" })
        plays.markPlayed(-4)
        #expect(!plays.isPlayed(-4))
        #expect(plays.revision == 0)
    }

    @Test("sign-out wipes every account's record")
    func wipe() {
        let store = defaults()
        var account = "u7-a"
        let plays = RoundVideoPlays(defaults: store, account: { account })
        plays.markPlayed(91)
        account = "u8-b"
        plays.markPlayed(92)
        plays.removeAll()
        #expect(!plays.isPlayed(92))
        account = "u7-a"
        #expect(!plays.isPlayed(91))
        #expect(!store.dictionaryRepresentation().keys.contains { $0.hasPrefix(RoundVideoPlays.keyPrefix) })
    }
}
