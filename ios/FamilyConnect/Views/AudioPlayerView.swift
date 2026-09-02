//
//  AudioPlayerView.swift
//  FamilyConnect
//
//  A piece of audio inside a bubble: play, elapsed/total, and a scrubber.
//
//  Deliberately NOT a waveform. Drawing one means a second artefact the
//  sender has to generate, upload and version — for something the ear does
//  not need and the protocol therefore does not carry (docs/protocol.md,
//  "Audio").
//
//  Streamed rather than downloaded, like video: `AVURLAsset` with the
//  session's Authorization header, because `AVPlayer(url:)` sends no
//  headers and every byte-range request needs one. That construction now
//  lives in AttachmentStreamPlayer, shared with both video pages, because
//  the URL it needs comes from an actor and the await in front of it has
//  to be guarded rather than merely awaited. The duration comes from the
//  attachment, so the scrubber is right before a single byte arrives.
//
//  Platform-free — the same row on iOS and macOS.
//

import AVFoundation
import SwiftUI

struct AudioPlayerView: View {
    let attachment: AttachmentDTO
    /// Which balloon this sits in, for contrast — an own balloon is filled
    /// with the tint, so nothing here may be drawn in the accent colour.
    var isMine: Bool = false

    @Environment(ChatSyncCoordinator.self) private var coordinator

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
    /// True while a drag is in progress, so the periodic observer does not
    /// fight the thumb for the slider's position.
    @State private var scrubbing = false
    @State private var observer: Any?

    private var total: TimeInterval {
        max(0.1, Double(attachment.durationMS ?? 0) / 1000)
    }

    private var ink: Color { isMine ? .white : .accentColor }

    var body: some View {
        HStack(spacing: 10) {
            Button {
                toggle()
            } label: {
                Image(systemName: isPlaying ? "pause.circle.fill" : "play.circle.fill")
                    .font(.system(size: 32))
                    .foregroundStyle(ink)
                    // The glyph is 32; the target is the platform's 44.
                    .frame(width: 44, height: 44)
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(isPlaying ? "Pause" : "Play")

            VStack(alignment: .leading, spacing: 2) {
                Slider(
                    value: Binding(
                        get: { min(elapsed, total) },
                        set: { elapsed = $0 }),
                    in: 0...total,
                    onEditingChanged: { editing in
                        scrubbing = editing
                        if !editing { seek(to: elapsed) }
                    })
                .tint(ink)
                .controlSize(.mini)

                HStack {
                    Text(verbatim: AudioRecorder.timeLabel(elapsed))
                    Spacer(minLength: 8)
                    Text(verbatim: AudioRecorder.timeLabel(total))
                }
                .font(.caption2.monospacedDigit())
                .opacity(0.75)
            }
        }
        .padding(.vertical, 6)
        .padding(.horizontal, 8)
        .frame(maxWidth: 260)
        .background(
            (isMine ? Color.white.opacity(0.14) : Color.primary.opacity(0.05)),
            in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(isMine ? Color.white.opacity(0.16) : Color.primary.opacity(0.06)))
        .onDisappear(perform: teardown)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Audio, \(AudioRecorder.timeLabel(total))")
    }

    private func toggle() {
        if isPlaying {
            stream.player?.pause()
            isPlaying = false
            return
        }
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
        // does nothing, because the item is already at its duration.
        if elapsed >= total - 0.2 { seek(to: 0) }
        player.play()
        isPlaying = true
    }

    private func beginPlayback() {
        stream.start(attachment: attachment.id, from: coordinator.api) { created in
            // Runs on the MainActor with the new player, before it starts,
            // so the scrubber is live from the first tick — and only for
            // a load that was neither cancelled nor unresolvable, so
            // there is no path that installs these on a player nobody
            // will ever see.
            observer = created.addPeriodicTimeObserver(
                forInterval: CMTime(seconds: 0.2, preferredTimescale: 600),
                queue: .main
            ) { time in
                guard !scrubbing else { return }
                elapsed = CMTimeGetSeconds(time)
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
                        elapsed = total
                    }
                }
            }
            isPlaying = true
        }
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
    }
}
