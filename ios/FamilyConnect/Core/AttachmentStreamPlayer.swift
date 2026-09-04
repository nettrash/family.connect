//
//  AttachmentStreamPlayer.swift
//  FamilyConnect
//
//  One streamed attachment, playing: the AVPlayer a view shows, plus the
//  in-flight fetch of the URL it was built from.
//
//  Why this exists at all. The stream URL comes from `APIClient`, which is
//  an actor precisely so the mutable (serverURL, token) pair is race-free
//  between the MainActor UI, the sync coordinator and the socket's resync
//  calls (APIClient's own header). Reading it therefore costs an `await`,
//  and that `await` lands in the worst possible place: between "is there
//  already a player?" and "here is the player". Three views used to do
//  that check-then-assign synchronously and got away with it; the moment
//  it suspends, two callers can both pass the check and build two
//  AVPlayers, the first of which is then dropped on the floor still
//  playing. AttachmentViewer asks for a start from BOTH `onAppear` and an
//  `isCurrent` change, so that is not a theoretical burst.
//
//  The guard is a stored `Task` handle, not an `isStarting` flag and not a
//  re-check of `player` after the await. All three stop the double
//  ASSIGNMENT; only the task handle also does the two things the call
//  sites actually need:
//
//    - It CANCELS. A view can vanish (paged away, window closed, bubble
//      scrolled out) while the load is suspended, and the load has to be
//      abandoned rather than hand a playing player to a view that is
//      gone. A Bool cannot be cancelled, and a bare `Task {}` nobody kept
//      a handle to outlives the view that started it.
//    - It keeps there being exactly ONE task to cancel. This is the part
//      that is easy to get wrong, and it is why re-checking `player`
//      after the await is not enough on its own. Under that design every
//      start spawns a task and the last one to start overwrites the
//      handle, orphaning the earlier ones — so the stop on disappear
//      cancels one task and leaves the others running, and an orphan then
//      assigns a player nothing will ever stop. Two starts and a stop is
//      not a contrived order: it is onAppear, the isCurrent change, and a
//      fast swipe away. AttachmentStreamPlayerTests has that sequence as
//      `doubleStartThenStopLeavesNothingPlaying`, and it is the test that
//      fails if this guard is ever weakened to the post-await re-check.
//
//  The post-await re-check is kept anyway, below, as belt and braces.
//
//  Both halves of the guard are read before the first suspension point,
//  and this type is MainActor-isolated, so nothing can interleave between
//  them: whoever gets in first owns the slot until it either assigns a
//  player or is cancelled. Everything after the await re-tests the world
//  (cancellation, and `player` again) because by then the slot's owner may
//  have been told to stop.
//
//  Considered and rejected: driving this from SwiftUI's `.task(id:)`,
//  which cancels on disappear for free. It is not testable outside a view
//  (this file is), it does not promise that the outgoing body has finished
//  before the incoming one starts, and it would still have needed the
//  post-suspension re-check. It also has nothing to say about the audio
//  path, which starts from a button rather than from appearing.
//
//  This carries no wire change — the stream endpoint and its Authorization
//  header are exactly as docs/protocol.md already describes them.
//

import AVFoundation
import Foundation

@MainActor
@Observable
final class AttachmentStreamPlayer {

    /// The player a view should show, or nil while there is none. Views
    /// read this; only this type writes it.
    private(set) var player: AVPlayer?

    /// The load in flight, or nil. It is `private(set)` rather than
    /// `private` for exactly one reason: the double-start and
    /// stop-mid-load tests await this handle, which is the only way to
    /// observe the post-suspension re-check without sleeping on a clock.
    /// Nothing in the app reads it.
    @ObservationIgnored private(set) var loadTask: Task<Void, Never>?

    /// How a player is built from a resolved stream. Injected so a test
    /// can count constructions — the assertion "a double start produced
    /// ONE player" is a count, not a nil check. The app passes nothing.
    @ObservationIgnored private let build: (URL, [String: String]) -> AVPlayer

    init(build: @escaping (URL, [String: String]) -> AVPlayer = AttachmentStreamPlayer.streamingPlayer) {
        self.build = build
    }

    /// AVURLAsset is the only way to attach an Authorization header:
    /// `AVPlayer(url:)` sends none, and the attachment endpoint wants one
    /// on every byte-range request the player makes.
    nonisolated static func streamingPlayer(url: URL, headers: [String: String]) -> AVPlayer {
        let asset = AVURLAsset(
            url: url,
            options: ["AVURLAssetHTTPHeaderFieldsKey": headers])
        return AVPlayer(playerItem: AVPlayerItem(asset: asset))
    }

    /// Resolve the stream URL and begin playing it. A no-op if a player
    /// already exists or a load is already in flight — call it from as
    /// many places as the view has reasons to start.
    ///
    /// `onReady` runs on the MainActor with the new player, before
    /// playback begins, for a caller that has to hang observers off it
    /// (AudioPlayerView's scrubber). It does NOT run when the load was
    /// cancelled or the URL could not be built.
    func start(
        attachment id: Int64,
        from api: APIClient,
        onReady: @escaping (AVPlayer) -> Void = { _ in }
    ) {
        // Read synchronously, before any suspension: this is the whole
        // re-entrancy guard.
        guard player == nil, loadTask == nil else { return }
        // The task inherits this type's MainActor isolation, so its body
        // cannot begin until `start` has returned — the assignment below
        // is always in place before anything can clear it.
        loadTask = Task { [weak self] in
            let stream = await api.attachmentStreamURL(id: id)
            // `self` gone means the view that owned it is gone; a
            // cancelled task means stop() was called across the await.
            // Neither may leave a player running.
            guard let self, !Task.isCancelled else { return }
            // Free the slot first: a nil stream (no server configured yet)
            // must leave the next start free to try again, and a caller's
            // onReady must be able to see a settled object.
            self.loadTask = nil
            // Belt and braces. Nothing can currently reach here with a
            // player already assigned — the slot was held for the whole
            // await — but if a future caller ever hands the player over
            // some other way, silently building a second one is the bug
            // this file exists to prevent.
            guard let stream, self.player == nil else { return }
            let created = self.build(stream.url, stream.headers)
            self.player = created
            onReady(created)
            created.play()
        }
    }

    /// Stop and release. Abandons a load in flight, so a start that was
    /// suspended when the view disappeared assigns nothing.
    ///
    /// `release` runs with the outgoing player, before it is paused, for a
    /// caller that has to take observers back off it — a periodic time
    /// observer outlives the player otherwise.
    func stop(release: (AVPlayer) -> Void = { _ in }) {
        loadTask?.cancel()
        loadTask = nil
        if let player {
            release(player)
            player.pause()
        }
        player = nil
    }
}
