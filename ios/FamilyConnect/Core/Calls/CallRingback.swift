//
//  CallRingback.swift
//  FamilyConnect
//
//  Plays the caller's ringback tone (RingbackTone) while an outgoing call
//  rings, and nothing else. CallManager says start and stop from its phase
//  transitions; this decides WHETHER it can actually play right now.
//
//  On the iPhone that question is real: CallKit owns the AVAudioSession's
//  activation (CallKitController's didActivate), and sound started before
//  it — or a session activated by hand — is exactly the "call connects but
//  nobody hears anything" trap WebRTCClient's header describes. So on iOS
//  a start() before activation only ARMS the player, and the activation
//  callback the manager forwards is what starts the sound; a deactivation
//  mid-ring stops it and a re-activation resumes it. On the Mac there is
//  no CallKit and the session is always there.
//
//  AVAudioPlayer rather than a node in WebRTC's graph: it follows the
//  session's route on its own (the receiver for a voice call, the speaker
//  for a video one, Bluetooth when connected), the manager keeps owning
//  that route, and looping a WAV held in memory needs no bundled asset.
//
//  Android counterpart: AndroidCallAudio.startRingback/stopRingback (calls/CallAudio.kt)
//

import AVFoundation
import Foundation
import os

/// What CallManager needs from a ringback: start it, stop it, and be told
/// about the session. Every call is idempotent — the manager says stop on
/// every phase change, most of which have nothing ringing.
@MainActor
protocol CallRingback: AnyObject {
    func start()
    func stop()
    /// The audio session became live (CallKit's didActivate on iOS).
    func audioSessionDidActivate()
    /// The audio session went away (CallKit's didDeactivate on iOS).
    func audioSessionDidDeactivate()
    /// The call is over. Forgets what was learned about the session as
    /// well: CallKit's didDeactivate can arrive late (or never, after a
    /// providerDidReset), and a "live" left over from the last call must
    /// not let the next one's ring play before ITS activation — an
    /// AVAudioPlayer started then would activate the session itself, the
    /// one thing this app must never do under CallKit.
    func callDidEnd()
}

/// The part of AVAudioPlayer the player uses, so RingbackPlayerTests can
/// count plays without a real audio device.
@MainActor
protocol RingbackSink: AnyObject {
    @discardableResult
    func play() -> Bool
    func stop()
}

extension AVAudioPlayer: RingbackSink {}

@MainActor
final class RingbackPlayer: CallRingback {

    /// Whether the session can be assumed live without being told:
    /// true on the Mac, false on the iPhone where CallKit decides.
    nonisolated static var sessionAlwaysLive: Bool {
        #if os(iOS)
        false
        #else
        true
        #endif
    }

    /// Loud enough over a receiver, not painful over a speaker; the tone
    /// itself already sits at about -6 dBFS peak (RingbackTone).
    static let volume: Float = 0.6

    private let region: () -> String?
    private let makeSink: @MainActor (Data) throws -> any RingbackSink
    /// How long an armed ring may wait for the session before it is
    /// worth a log line (and one retry): the activation normally follows
    /// the start action within a fraction of a second.
    private let armedGrace: Duration
    /// What `isSessionLive` goes back to when a call ends: the platform's
    /// standing answer (the Mac's session is always there; the iPhone's
    /// is CallKit's to grant, per call).
    private let sessionLiveByDefault: Bool
    private var sink: (any RingbackSink)?
    private var armedCheck: Task<Void, Never>?
    /// The manager wants sound. Survives a deactivation, so a session
    /// that comes back mid-ring rings again.
    private(set) var isWanted = false
    private(set) var isSessionLive: Bool

    init(
        sessionLive: Bool = RingbackPlayer.sessionAlwaysLive,
        armedGrace: Duration = .seconds(2),
        region: @escaping () -> String? = { Locale.current.region?.identifier },
        makeSink: @escaping @MainActor (Data) throws -> any RingbackSink = RingbackPlayer.systemSink
    ) {
        self.isSessionLive = sessionLive
        self.sessionLiveByDefault = sessionLive
        self.armedGrace = armedGrace
        self.region = region
        self.makeSink = makeSink
    }

    /// True while the tone is actually sounding.
    var isPlaying: Bool { sink != nil }

    func start() {
        isWanted = true
        playIfPossible()
        scheduleArmedCheck()
    }

    func stop() {
        isWanted = false
        armedCheck?.cancel()
        armedCheck = nil
        silence()
    }

    func callDidEnd() {
        stop()
        isSessionLive = sessionLiveByDefault
    }

    func audioSessionDidActivate() {
        isSessionLive = true
        playIfPossible()
    }

    func audioSessionDidDeactivate() {
        isSessionLive = false
        // Wanted stays: the next activation picks the ring back up.
        silence()
    }

    private func playIfPossible() {
        guard isWanted, isSessionLive, sink == nil else { return }
        let cadence = RingbackTone.cadence(region: region())
        do {
            let player = try makeSink(RingbackTone.wav(cadence))
            guard player.play() else {
                AppLog.call.warning("Ringback did not start (\(cadence.rawValue, privacy: .public))")
                return
            }
            sink = player
        } catch {
            AppLog.call.error("Ringback player failed: \(String(describing: error))")
        }
    }

    private func silence() {
        sink?.stop()
        sink = nil
    }

    /// A ring that is still silent after the grace period is the one
    /// failure the state machine cannot see — the session never went
    /// live, or the player refused. Say so in the log (the device test's
    /// first stop), and try once more in case it was the latter.
    private func scheduleArmedCheck() {
        guard sink == nil else { return }
        armedCheck?.cancel()
        let grace = armedGrace
        armedCheck = Task { [weak self] in
            try? await Task.sleep(for: grace)
            guard !Task.isCancelled, let self, self.isWanted, self.sink == nil else { return }
            AppLog.call.warning("Ringback still silent after \(grace); session live: \(self.isSessionLive)")
            self.playIfPossible()
        }
    }

    private static func systemSink(_ data: Data) throws -> any RingbackSink {
        let player = try AVAudioPlayer(data: data, fileTypeHint: AVFileType.wav.rawValue)
        player.numberOfLoops = -1
        player.volume = volume
        player.prepareToPlay()
        return player
    }
}
