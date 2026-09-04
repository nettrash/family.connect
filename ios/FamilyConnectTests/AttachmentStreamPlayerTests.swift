//
//  AttachmentStreamPlayerTests.swift
//  FamilyConnectTests
//
//  The re-entrancy guard in front of a streamed attachment's player.
//
//  What is provable here is exactly the part that broke when the stream
//  URL moved behind an `await`: how many players a burst of starts builds,
//  and what a stop during the suspension leaves behind. Those are counts
//  and nil checks on MainActor state, so they are deterministic — the
//  tests await the loader's own task handle rather than sleeping, which is
//  why none of them can flake under load.
//
//  What is NOT provable here is playback itself: whether AVPlayer's
//  byte-range requests actually carry the Authorization header to a live
//  server, and whether audio comes out. Those need a device against a real
//  box (see the note on issue #46) — so the builder is injected and these
//  tests assert on the (url, headers) pair handed to it, which is the last
//  point the app still controls.
//

import AVFoundation
import Foundation
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Streamed attachment players")
struct AttachmentStreamPlayerTests {

    /// Records every construction the loader asks for. A class because the
    /// closure escapes into the loader and has to write back somewhere the
    /// test can still read.
    @MainActor
    private final class Builder {
        private(set) var built: [(url: URL, headers: [String: String])] = []

        var count: Int { built.count }

        /// An itemless AVPlayer: cheap, opens no connection, and nothing
        /// under test looks at what is inside it.
        func make(_ url: URL, _ headers: [String: String]) -> AVPlayer {
            built.append((url, headers))
            return AVPlayer()
        }
    }

    private func configuredAPI() async -> APIClient {
        let api = APIClient(serverURL: URL(string: "https://stream-guard.test")!)
        await api.setToken("sekret")
        return api
    }

    // MARK: - The bug this guard exists for

    /// The regression. AttachmentViewer asks for a start from `onAppear`
    /// AND from the `isCurrent` change, so two calls in one runloop turn
    /// are the normal case, not a stress test. Before the guard both would
    /// pass `player == nil` — the check is on one side of the await and
    /// the assignment on the other — and the first player would be
    /// overwritten while still playing.
    @Test("A double start builds exactly one player")
    func doubleStartBuildsOnePlayer() async {
        let api = await configuredAPI()
        let builder = Builder()
        let loader = AttachmentStreamPlayer(build: builder.make)

        loader.start(attachment: 7, from: api)
        loader.start(attachment: 7, from: api)
        await loader.loadTask?.value

        #expect(builder.count == 1)
        #expect(loader.player != nil)
    }

    /// Three, four, ten — the slot is held for the whole suspension, so
    /// the count does not depend on how the burst is spaced.
    @Test("A burst of starts builds exactly one player")
    func burstOfStartsBuildsOnePlayer() async {
        let api = await configuredAPI()
        let builder = Builder()
        let loader = AttachmentStreamPlayer(build: builder.make)

        for _ in 0..<10 { loader.start(attachment: 7, from: api) }
        await loader.loadTask?.value

        #expect(builder.count == 1)
    }

    /// The other half: once a player exists, starting again must not
    /// replace it. This is the path a page that reappears takes.
    @Test("Starting again after the load landed changes nothing")
    func startAfterLoadIsANoOp() async {
        let api = await configuredAPI()
        let builder = Builder()
        let loader = AttachmentStreamPlayer(build: builder.make)

        loader.start(attachment: 7, from: api)
        await loader.loadTask?.value
        let first = loader.player

        loader.start(attachment: 7, from: api)
        await loader.loadTask?.value

        #expect(builder.count == 1)
        #expect(loader.player === first)
    }

    // MARK: - Disappearing mid-load

    /// The view went away while the URL was still being fetched: paged
    /// past, window closed, bubble scrolled off. Nothing may be built, and
    /// nothing may be assigned — a player handed to a view that is gone is
    /// a clip you can hear and cannot see.
    @Test("A stop during the load leaves no player and builds none")
    func stopDuringLoadBuildsNothing() async {
        let api = await configuredAPI()
        let builder = Builder()
        let loader = AttachmentStreamPlayer(build: builder.make)

        loader.start(attachment: 7, from: api)
        // Grabbed before the stop, because stop() clears the slot: this is
        // the handle that lets the test await the exact moment the
        // cancelled body resumes, with no sleeping.
        let inFlight = loader.loadTask
        loader.stop()
        await inFlight?.value

        #expect(builder.count == 0)
        #expect(loader.player == nil)
    }

    /// The sequence that separates a real guard from a plausible one:
    /// onAppear starts, the isCurrent change starts again, and then the
    /// page is swiped away — all before the URL has come back.
    ///
    /// This is the test to keep. Weaken the guard to "re-check `player`
    /// after the await" and it fails, because that design spawns a task
    /// per start and only the last one is still on the handle: `stop`
    /// cancels that one and the orphan goes on to assign a player nothing
    /// can reach to pause. The count assertion is the leak, the nil
    /// assertion is the clip you can hear and cannot see.
    @Test("Two starts and a stop leave nothing playing")
    func doubleStartThenStopLeavesNothingPlaying() async {
        let api = await configuredAPI()
        let builder = Builder()
        let loader = AttachmentStreamPlayer(build: builder.make)

        loader.start(attachment: 7, from: api)
        loader.start(attachment: 7, from: api)
        // Awaiting the handle drains the MainActor past every task that
        // was enqueued before it — an orphan would have run and assigned
        // by the time this returns. No sleeping, so no flake.
        let inFlight = loader.loadTask
        loader.stop()
        await inFlight?.value

        #expect(builder.count == 0)
        #expect(loader.player == nil)
    }

    /// And the slot is genuinely freed by that stop — a guard that wedged
    /// would be a silent "this video never plays again".
    @Test("A start after a stop mid-load still works")
    func startAfterStopMidLoadWorks() async {
        let api = await configuredAPI()
        let builder = Builder()
        let loader = AttachmentStreamPlayer(build: builder.make)

        loader.start(attachment: 7, from: api)
        let inFlight = loader.loadTask
        loader.stop()
        await inFlight?.value

        loader.start(attachment: 7, from: api)
        await loader.loadTask?.value

        #expect(builder.count == 1)
        #expect(loader.player != nil)
    }

    /// Paging away and back: stop then start builds a fresh player rather
    /// than resurrecting the old one.
    @Test("Stop then start builds a second player")
    func stopThenStartBuildsAFreshPlayer() async {
        let api = await configuredAPI()
        let builder = Builder()
        let loader = AttachmentStreamPlayer(build: builder.make)

        loader.start(attachment: 7, from: api)
        await loader.loadTask?.value
        let first = loader.player
        loader.stop()
        #expect(loader.player == nil)

        loader.start(attachment: 7, from: api)
        await loader.loadTask?.value

        #expect(builder.count == 2)
        #expect(loader.player !== first)
    }

    // MARK: - Handing the player back

    /// AudioPlayerView installs a periodic time observer on the player and
    /// has to take it off again before letting go, or it fires forever.
    /// `stop` hands the outgoing player over for exactly that, once.
    @Test("Stop hands the outgoing player to its caller exactly once")
    func stopReleasesThePlayerOnce() async {
        let api = await configuredAPI()
        let builder = Builder()
        let loader = AttachmentStreamPlayer(build: builder.make)

        loader.start(attachment: 7, from: api)
        await loader.loadTask?.value
        let created = loader.player

        var released: [AVPlayer] = []
        loader.stop { released.append($0) }
        loader.stop { released.append($0) }

        #expect(released.count == 1)
        #expect(released.first === created)
    }

    /// onReady is where the scrubber's observers go on. It must see the
    /// player the loader kept, and must not run for a load that produced
    /// nothing.
    @Test("onReady runs once, with the player that was kept")
    func onReadyRunsOnceWithTheKeptPlayer() async {
        let api = await configuredAPI()
        let builder = Builder()
        let loader = AttachmentStreamPlayer(build: builder.make)

        var ready: [AVPlayer] = []
        loader.start(attachment: 7, from: api) { ready.append($0) }
        loader.start(attachment: 7, from: api) { ready.append($0) }
        await loader.loadTask?.value

        #expect(ready.count == 1)
        #expect(ready.first === loader.player)
    }

    @Test("onReady does not run when the load was cancelled")
    func onReadySkippedOnCancel() async {
        let api = await configuredAPI()
        let builder = Builder()
        let loader = AttachmentStreamPlayer(build: builder.make)

        var ready = 0
        loader.start(attachment: 7, from: api) { _ in ready += 1 }
        let inFlight = loader.loadTask
        loader.stop()
        await inFlight?.value

        #expect(ready == 0)
    }

    // MARK: - What reaches the player

    /// The token is the reason `attachmentStreamURL` is on an actor at
    /// all. This is the last point the app controls before AVFoundation
    /// takes over, so it is where the header is worth asserting.
    @Test("The builder is handed the endpoint and the bearer token")
    func builderGetsTheURLAndToken() async {
        let api = await configuredAPI()
        let builder = Builder()
        let loader = AttachmentStreamPlayer(build: builder.make)

        loader.start(attachment: 34, from: api)
        await loader.loadTask?.value

        // Not `#require`: the recorded pair is a tuple, which is not
        // Equatable, and the macro cannot check one for nil.
        #expect(builder.count == 1)
        #expect(builder.built.first?.url.absoluteString
            == "https://stream-guard.test/api/v1/attachments/34")
        #expect(builder.built.first?.headers["Authorization"] == "Bearer sekret")
    }

    /// No server yet (the app before setup, or after a sign-out) means no
    /// URL. Nothing is built, and — the part that matters — the slot is
    /// free again, so the same view works the moment a server arrives.
    @Test("An unconfigured client builds nothing and leaves the slot free")
    func unconfiguredClientBuildsNothing() async {
        let api = APIClient(serverURL: nil)
        let builder = Builder()
        let loader = AttachmentStreamPlayer(build: builder.make)

        loader.start(attachment: 7, from: api)
        await loader.loadTask?.value

        #expect(builder.count == 0)
        #expect(loader.player == nil)

        await api.configure(serverURL: URL(string: "https://late.test")!, token: "t")
        loader.start(attachment: 7, from: api)
        await loader.loadTask?.value

        #expect(builder.count == 1)
        #expect(loader.player != nil)
    }

    /// The default builder is the one the app uses; it must produce a
    /// player with an item pointed at the stream. (Whether that item ever
    /// loads is a device question — nothing here connects.)
    @Test("The shipped builder makes a player pointed at the stream URL")
    func shippedBuilderPointsAtTheStream() {
        let url = URL(string: "https://stream-guard.test/api/v1/attachments/9")!
        let player = AttachmentStreamPlayer.streamingPlayer(
            url: url, headers: ["Authorization": "Bearer sekret"])
        let asset = player.currentItem?.asset as? AVURLAsset
        #expect(asset?.url == url)
    }
}
