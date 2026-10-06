//
//  AudioPlayerView.swift
//  FamilyConnect
//
//  A piece of audio inside a bubble (#79, the approved design "Voice and
//  Video Messages"): a round play/pause button in the tint, the recording's
//  WAVEFORM — the played bars taking the tint as it plays, a tap or a drag
//  on it seeking — the elapsed time while it plays and the length at rest in
//  tabular digits, an unplayed dot, and the speed chip (1× → 1.5× → 2×)
//  while it plays.
//
//  The waveform is the SENDER's (`AttachmentDTO.waveform`, docs/protocol.md,
//  "A voice note's waveform"), drawn before a byte of the recording is
//  downloaded; a picked sound file, an old message or an old server's echo
//  has none and draws the flat placeholder (`Waveform.levelsOrPlaceholder`).
//
//  Streamed rather than downloaded, like video: `AVURLAsset` with the
//  session's Authorization header, because `AVPlayer(url:)` sends no
//  headers and every byte-range request needs one. That construction lives
//  in AttachmentStreamPlayer, shared with both video pages, because the URL
//  it needs comes from an actor and the await in front of it has to be
//  guarded rather than merely awaited. The duration comes from the
//  attachment, so the waveform seeks right before a single byte arrives.
//
//  Platform-free — the same row on iOS and macOS.
//

import AVFoundation
import SwiftUI

struct AudioPlayerView: View {
    let attachment: AttachmentDTO
    /// Which balloon this sits in, for contrast — an own balloon is filled
    /// with the tint, so the button is a white disc and the played bars are
    /// white there. It is also whose message this is: your own recording
    /// never carries the unplayed dot.
    var isMine: Bool = false

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.colorSchemeContrast) private var contrast

    /// The player, and the fetch of the URL it is built from. The stream
    /// URL comes from the API actor, so it can only be had with an
    /// `await` — which puts a suspension inside what used to be a
    /// straight-line "no player yet? build one and play it" on a button
    /// tap. Two taps inside that window would each have built a player,
    /// AND each installed a periodic time observer; the loser is
    /// overwritten by `player = created` and its observer is then never
    /// removed, so it keeps firing and writing `elapsed` forever. The
    /// loader holds the slot for the whole await, so the second tap is a
    /// no-op instead. See Core/AttachmentStreamPlayer.swift.
    @State private var stream = AttachmentStreamPlayer()
    @State private var isPlaying = false
    @State private var elapsed: TimeInterval = 0
    /// True while a finger or the pointer drags along the waveform, so the
    /// periodic observer does not fight it for the position.
    @State private var scrubbing = false
    /// The drag's direction, decided once: only a mostly sideways drag
    /// scrubs, so a vertical one is left to the thread's scrolling.
    @State private var dragIsSideways: Bool?
    @State private var waveWidth: CGFloat = 0
    @State private var observer: Any?
    /// This row's name to the app's one-thing-plays rule (#79, NowPlaying):
    /// another note starting, or a recording starting, pauses this one.
    @State private var playToken = UUID()
    /// "You can play this after recording." — said for a tap that came
    /// while a recording runs (S1.7).
    @State private var saysAfterRecording = false
    /// Whether this row took the audio session for playback and still owes
    /// it back — so a pause, the end and leaving each give it back exactly
    /// once (S5.3).
    @State private var holdsSession = false

    private var speed: VoicePlaybackSpeed { .shared }
    private var plays: RoundVideoPlays { .voiceNotes }

    /// A voice recording runs somewhere in the app: no app sound plays, and
    /// the play control is dimmed (S1.7).
    private var recordingElsewhere: Bool { VoiceRecordingArbiter.shared.isRecording }

    private var total: TimeInterval {
        max(0.1, Double(attachment.durationMS ?? 0) / 1000)
    }

    /// The 48 levels the bars are drawn from.
    private var levels: [UInt8] { Waveform.levelsOrPlaceholder(attachment.waveform) }

    /// Somebody else's note this device has not played yet. Never your own,
    /// and never one the server has not named.
    private var showsUnplayedDot: Bool {
        !isMine && attachment.id > 0 && !plays.isPlayed(attachment.id)
    }

    /// Started and not back at the start: the elapsed time and the speed
    /// chip show.
    private var isUnderway: Bool { isPlaying || (elapsed > 0.05 && elapsed < total - 0.2) }

    /// The tint on a received bubble; white on the tint-filled own one.
    /// Under Increase Contrast a received bubble's played bars take the
    /// strongest ink.
    private var ink: Color { isMine ? .white : .accentColor }
    private var playedInk: Color {
        isMine ? .white : (contrast == .increased ? .primary : .accentColor)
    }
    private var unplayedInk: Color {
        isMine ? Color.white.opacity(0.42) : Color.secondary.opacity(contrast == .increased ? 0.7 : 0.45)
    }

    /// What the screen reader calls it: a voice note by its length; a picked
    /// sound file, which has a name, as audio.
    private var label: String {
        let length = AudioRecorder.timeLabel(total)
        if let name = attachment.name, !name.isEmpty {
            return String(localized: "Audio, \(length)")
        }
        return String(localized: "Voice message, \(length)")
    }

    var body: some View {
        HStack(alignment: .center, spacing: 10) {
            playButton

            VStack(alignment: .leading, spacing: 3) {
                waveform
                if saysAfterRecording {
                    Text("You can play this after recording.")
                        .font(.caption2)
                        .lineLimit(1)
                        .minimumScaleFactor(0.8)
                        .opacity(0.75)
                } else {
                    meta
                }
            }
            // White words on the tint-filled own bubble, the system's ink on
            // a received one — in both appearances.
            .foregroundStyle(isMine ? AnyShapeStyle(Color.white) : AnyShapeStyle(.primary))
        }
        .padding(.vertical, 4)
        .padding(.leading, 2)
        .padding(.trailing, 4)
        .frame(minWidth: 200, maxWidth: 260)
        .onDisappear(perform: teardown)
        .onChange(of: recordingElsewhere) { _, now in
            if !now { saysAfterRecording = false }
        }
        .onChange(of: speed.rate) { _, rate in
            guard let player = stream.player else { return }
            player.defaultRate = Float(rate)
            if isPlaying { player.rate = Float(rate) }
        }
        // One element: the button and the bars are one control to a screen
        // reader — activate plays or pauses, swipe up or down seeks.
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: label))
        .accessibilityValue(Text(verbatim: accessibilityValue))
        .accessibilityHint(isPlaying ? Text("Pause") : Text("Play"))
        .accessibilityAddTraits(.isButton)
        .accessibilityAddTraits(.startsMediaSession)
        .accessibilityAction { toggle() }
        .accessibilityAdjustableAction { direction in
            let step = max(1, (total / 10).rounded())
            switch direction {
            case .increment: seekTo(min(total, elapsed + step))
            case .decrement: seekTo(max(0, elapsed - step))
            @unknown default: break
            }
        }
        .accessibilityAction(named: Text(String(localized: "Playback speed, \(speed.label)"))) {
            speed.cycle()
        }
    }

    /// "0:12" while it is underway, then "Played" or "Not played" on
    /// somebody else's note — the dot, in words (S6).
    private var accessibilityValue: String {
        var parts: [String] = []
        if isUnderway { parts.append(AudioRecorder.timeLabel(elapsed)) }
        if saysAfterRecording {
            parts.append(String(localized: "You can play this after recording."))
        }
        if !isMine, attachment.id > 0 {
            parts.append(showsUnplayedDot ? String(localized: "Not played") : String(localized: "Played"))
        }
        return parts.joined(separator: ", ")
    }

    // MARK: - Pieces

    /// A 40-point disc in the tint (white on an own bubble), in a 44-point
    /// target.
    private var playButton: some View {
        Button {
            toggle()
        } label: {
            ZStack {
                Circle()
                    .fill(isMine ? Color.white : Color.accentColor)
                Image(systemName: isPlaying ? "pause.fill" : "play.fill")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(isMine ? Color.accentColor : Color.white)
                    // The play triangle's weight sits left of its box.
                    .offset(x: isPlaying ? 0 : 1.5)
            }
            .frame(width: 40, height: 40)
            .opacity(recordingElsewhere ? 0.35 : 1)
            .frame(width: 44, height: 44)
            .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .help(isPlaying ? Text("Pause") : Text("Play"))
    }

    private var waveform: some View {
        VoiceWaveformBars(
            levels: levels,
            playedFraction: isUnderway
                ? (UInt64(max(0, elapsed) * 1000), UInt64(total * 1000)) : nil,
            played: playedInk,
            unplayed: unplayedInk)
            .frame(height: 28)
            .frame(maxWidth: .infinity)
            .contentShape(Rectangle())
            .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { waveWidth = $0 }
            // A tap seeks there; a sideways drag scrubs. Simultaneous, so the
            // bubble's long press and the thread's scrolling still work.
            .simultaneousGesture(
                SpatialTapGesture().onEnded { tap in seek(atX: tap.location.x) })
            .simultaneousGesture(scrub)
    }

    private var scrub: some Gesture {
        DragGesture(minimumDistance: 8)
            .onChanged { drag in
                if dragIsSideways == nil {
                    dragIsSideways = abs(drag.translation.width) > abs(drag.translation.height)
                }
                guard dragIsSideways == true else { return }
                scrubbing = true
                elapsed = position(atX: drag.location.x)
            }
            .onEnded { drag in
                defer {
                    dragIsSideways = nil
                    scrubbing = false
                }
                guard dragIsSideways == true else { return }
                seekTo(position(atX: drag.location.x))
            }
    }

    private var meta: some View {
        HStack(spacing: 8) {
            Text(verbatim: AudioRecorder.timeLabel(isUnderway ? elapsed : total))
                .font(.caption.monospacedDigit())
                .opacity(0.75)
            if showsUnplayedDot {
                Circle()
                    .fill(isMine ? Color.white : (contrast == .increased ? Color.primary : Color.accentColor))
                    .frame(width: 7, height: 7)
            }
            Spacer(minLength: 4)
            if isUnderway {
                VoiceSpeedChip(speed: speed, ink: ink)
            }
        }
        // The chip is the one control here: the rest is said by the row.
        .lineLimit(1)
    }

    // MARK: - Playing

    private func position(atX x: CGFloat) -> TimeInterval {
        guard waveWidth > 0 else { return 0 }
        return total * Double(min(max(0, x / waveWidth), 1))
    }

    private func seek(atX x: CGFloat) {
        seekTo(position(atX: x))
    }

    /// Move the position there — the player too once there is one; before
    /// that, the first Play starts from it.
    private func seekTo(_ seconds: TimeInterval) {
        elapsed = seconds
        seek(to: seconds)
    }

    private func toggle() {
        if isPlaying {
            pause()
            return
        }
        // Nothing of the app's plays under a recording (S1.7).
        guard !recordingElsewhere else {
            saysAfterRecording = true
            AccessibilityNotification.Announcement(String(localized: "You can play this after recording.")).post()
            return
        }
        saysAfterRecording = false
        guard let player = stream.player else {
            // First tap. The player cannot exist yet in this turn — its
            // URL is behind an actor — so playback and the pause glyph
            // both arrive one hop later, from `beginPlayback`'s onReady.
            // Deliberately NOT set optimistically here: a stream that
            // cannot be built (no server configured) would otherwise
            // leave the row showing a pause button over nothing. A second
            // tap inside that hop finds the slot taken and does nothing,
            // which is the whole point.
            beginPlayback()
            return
        }
        // Replaying after it ran to the end: without this the play button
        // does nothing, because the item is already at its duration. Two
        // ways to be there: the position was dragged to the end (`elapsed`
        // says so), or the note PLAYED to its end — the row then rests at
        // the start while the player still sits at the end, so the player's
        // own position is what `resume` asks.
        if elapsed >= total - 0.2 { seekTo(0) }
        claimPlayback()
        player.defaultRate = Float(speed.rate)
        Self.resume(player)
        isPlaying = true
        markPlayed()
    }

    private func beginPlayback() {
        let startAt = elapsed < total - 0.2 ? elapsed : 0
        stream.start(attachment: attachment.id, from: coordinator.api) { created in
            // Runs on the MainActor with the new player, before it starts,
            // so the bars are live from the first tick — and only for a
            // load that was neither cancelled nor unresolvable, so there is
            // no path that installs these on a player nobody will ever see.
            observer = created.addPeriodicTimeObserver(
                forInterval: CMTime(seconds: 0.1, preferredTimescale: 600),
                queue: .main
            ) { time in
                guard !scrubbing else { return }
                elapsed = CMTimeGetSeconds(time)
            }
            // Speech at 1.5× and 2× keeps its pitch.
            created.currentItem?.audioTimePitchAlgorithm = .timeDomain
            created.defaultRate = Float(speed.rate)
            // A seek made before the first play: start from there.
            if startAt > 0.05 {
                created.seek(
                    to: CMTime(seconds: startAt, preferredTimescale: 600),
                    toleranceBefore: .zero, toleranceAfter: .zero)
            }
            // Stop at the end rather than sitting there looking paused-at-zero.
            if let item = created.currentItem {
                NotificationCenter.default.addObserver(
                    forName: .AVPlayerItemDidPlayToEndTime,
                    object: item,
                    queue: .main
                ) { _ in
                    Task { @MainActor in
                        isPlaying = false
                        elapsed = 0
                        // The row rests at the start, so the player goes
                        // there too: left at its end, the next Play would
                        // be a `play()` that plays nothing.
                        if let player = stream.player { Self.rewind(player) }
                        NowPlaying.shared.release(playToken)
                        giveSession()
                    }
                }
            }
            claimPlayback()
            isPlaying = true
            markPlayed()
        }
    }

    /// Play from where the player stands — from the start when it has run
    /// to its end. `AVPlayer.play()` at the end of its item sets nothing
    /// going (rate 0, the position unmoved): the row would show Pause over
    /// silence, holding the app's one-thing-plays slot, with no end
    /// notification ever coming to give it back.
    static func resume(_ player: AVPlayer) {
        if isAtEnd(player) { rewind(player) }
        player.play()
    }

    /// Back to the start, exactly.
    static func rewind(_ player: AVPlayer) {
        player.seek(to: .zero, toleranceBefore: .zero, toleranceAfter: .zero)
    }

    /// The player's item is at (or within a frame or two of) its end.
    static func isAtEnd(_ player: AVPlayer) -> Bool {
        guard let item = player.currentItem else { return false }
        let duration = item.duration
        guard duration.isNumeric, duration.seconds > 0 else { return false }
        return item.currentTime().seconds >= duration.seconds - 0.05
    }

    /// Somebody else's note loses its dot the moment it starts.
    private func markPlayed() {
        guard !isMine else { return }
        plays.markPlayed(attachment.id)
    }

    /// Pause, and let the app know nothing of this row plays any more.
    private func pause() {
        stream.player?.pause()
        isPlaying = false
        NowPlaying.shared.release(playToken)
        giveSession()
    }

    /// One thing plays at a time: whoever played before is paused, and a
    /// recording starting pauses this one (#79, S1.7, S5.3).
    ///
    /// Then `.playback`, heard with the silent switch on and never while a
    /// call holds the session (S5.3, Decision 23) — AFTER the claim, so the
    /// outgoing player's giving back cannot land after this taking.
    private func claimPlayback() {
        NowPlaying.shared.claim(playToken, kind: .voice) {
            stream.player?.pause()
            isPlaying = false
            giveSession()
        }
        if !holdsSession {
            holdsSession = true
            PlaybackSessionControl.system.begin()
        }
    }

    private func giveSession() {
        guard holdsSession else { return }
        holdsSession = false
        PlaybackSessionControl.system.end()
    }

    private func seek(to seconds: TimeInterval) {
        stream.player?.seek(
            to: CMTime(seconds: seconds, preferredTimescale: 600),
            toleranceBefore: .zero,
            toleranceAfter: .zero)
    }

    private func teardown() {
        // The time observer has to come off the player it was added to,
        // before that player is let go — `stop` hands it over for exactly
        // this. It also cancels a load still in flight, so a bubble
        // scrolled off screen mid-tap never gets a player at all.
        stream.stop { player in
            if let observer { player.removeTimeObserver(observer) }
        }
        observer = nil
        isPlaying = false
        NowPlaying.shared.release(playToken)
        giveSession()
    }
}
