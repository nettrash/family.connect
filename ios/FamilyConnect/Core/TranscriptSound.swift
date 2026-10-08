//
//  TranscriptSound.swift
//  FamilyConnect
//
//  The sound THIS device sends with a transcript request, for the
//  recordings the server cannot send itself: a video, an Ogg file, an
//  audio type outside the provider's list, or a recording over
//  `transcribe_max_bytes` (docs/protocol.md, "Transcripts on request" —
//  the multipart form).
//
//  WHERE THE FILE COMES FROM. Voice notes, audio and video are STREAMED
//  to their players, never cached on this device (`AttachmentStore`'s
//  header), so the file is fetched here — once, through the same
//  `GET /attachments/{id}` the players read, straight to a file in a
//  scratch folder (never into memory: a video may be 100 MB) — and thrown
//  away with the folder once the sound is out. Keeping a second copy of a
//  video for a text the device keeps anyway would only cost the disk.
//
//  WHAT IS SENT. The sound track as an M4A of AAC (`MediaTranscoder.
//  extractSound`, the #74 reader, writer and pumps; no picture decoded):
//
//    * an AAC track (mono or stereo) is PASSED THROUGH untouched, when its
//      own rate fits the ceiling — no second generation, no work;
//    * anything else — MP3, PCM, Opus in Ogg, ALAC, FLAC, an AAC track too
//      big to pass — is RE-ENCODED to 64 kbit/s mono AAC-LC, about fifty
//      minutes in 25 MiB;
//    * and the result is measured: a pass-through that came out over the
//      ceiling is re-encoded, and a re-encode still over it is "too long".
//
//  WHAT CANNOT BE DONE IS SAID, NEVER HIDDEN (`TranscriptDoor`'s header):
//  `Failure.tooLong` and `Failure.unreadable` become the two sentences the
//  row draws. A recording whose LENGTH alone proves it cannot fit at 64
//  kbit/s is told so before anything is downloaded (`knownTooLong`).
//
//  Nothing here is logged but the outcome's name — never the sound, the
//  text or the file's name.
//
//  Android counterpart: none in this change (#62 phase 3 is per client).
//

import AVFoundation
import CoreMedia
import Foundation

nonisolated enum TranscriptSound {

    /// What this device could not do. Both are terminal for the file.
    nonisolated enum Failure: Error, Equatable {
        /// Over `transcribe_max_bytes` even at 64 kbit/s mono.
        case tooLong
        /// No sound track, or a file this OS cannot open.
        case unreadable
    }

    /// What becomes of the sound track.
    nonisolated enum Treatment: Equatable, Sendable {
        /// The AAC track's own samples, copied into an M4A.
        case passThrough
        /// Decoded and encoded again: `bitrate`, mono, AAC-LC.
        case reencode
    }

    /// The rate a re-encoded track is written at (the design's number).
    static let bitrate = 64_000

    /// The sound track as `MediaProbe` reads it — the facts the treatment
    /// turns on, and nothing else.
    nonisolated struct Track: Equatable, Sendable {
        /// The reference's lowercase name (`MediaProbe.audioCodec`): "aac"
        /// for AAC in any profile.
        var codec: String
        var channels: Int?
        /// Bits a second as the container states them, nil when unknown.
        var bitrate: Int?
    }

    // MARK: - Deciding

    /// Bytes an M4A of `durationMS` at 64 kbit/s comes to: the samples, a
    /// per-millisecond allowance for the sample table (about 43 AAC
    /// packets a second, a few bytes each) and a fixed allowance for the
    /// rest of the container. An over-estimate by design: it is what says
    /// "too long" without a download, and that must never be wrong.
    static func reencodedBytes(durationMS: Int) -> Int64 {
        Int64(durationMS) * Int64(bitrate) / 8_000 + Int64(durationMS) / 4 + 65_536
    }

    /// The same estimate for a track passed through at its own rate.
    static func passedBytes(durationMS: Int, bitrate: Int) -> Int64 {
        Int64(durationMS) * Int64(bitrate) / 8_000 + Int64(durationMS) / 4 + 65_536
    }

    /// Does the recording's LENGTH alone prove that even a 64 kbit/s mono
    /// re-encode cannot fit? Then "too long" is said at once, with nothing
    /// downloaded. Unknown length proves nothing.
    ///
    /// Not quite the same as "nothing can fit": an AAC track below 64
    /// kbit/s inside a very long VIDEO could pass through under the
    /// ceiling. A video long enough to matter (fifty minutes and more)
    /// does not fit in an attachment's 100 MB at any picture worth
    /// watching, so the case is accepted rather than paid for with a
    /// 100 MB download that would usually end in the same answer.
    static func knownTooLong(_ attachment: AttachmentDTO, maxBytes: Int64?) -> Bool {
        guard let duration = attachment.durationMS, duration > 0 else { return false }
        return reencodedBytes(durationMS: duration) > TranscriptDoor.ceiling(maxBytes: maxBytes)
    }

    /// Pass through, re-encode, or give up — from the sound track as read
    /// and the recording's length.
    ///
    /// - No track: `unreadable` (a video with no sound, or a file this OS
    ///   cannot open — both read as no audio track).
    /// - AAC, mono or stereo, whose own rate fits (or is not stated — the
    ///   result is measured either way): pass it through.
    /// - Otherwise a re-encode, when its estimate fits or the length is
    ///   unknown; `tooLong` when it does not.
    static func treatment(
        track: Track?, durationMS: Int?, maxBytes: Int64?
    ) -> Result<Treatment, Failure> {
        guard let track else { return .failure(.unreadable) }
        let ceiling = TranscriptDoor.ceiling(maxBytes: maxBytes)
        let duration = durationMS.flatMap { $0 > 0 ? $0 : nil }
        if track.codec == "aac", track.channels == 1 || track.channels == 2 {
            guard let duration, let rate = track.bitrate else { return .success(.passThrough) }
            if passedBytes(durationMS: duration, bitrate: rate) <= ceiling {
                return .success(.passThrough)
            }
        }
        if let duration, reencodedBytes(durationMS: duration) > ceiling {
            return .failure(.tooLong)
        }
        return .success(.reencode)
    }

    /// Whether a part of `bytes` may be sent: non-empty and within the
    /// ceiling, which is the server's own check of the part.
    static func fits(bytes: Int64, maxBytes: Int64?) -> Bool {
        bytes > 0 && bytes <= TranscriptDoor.ceiling(maxBytes: maxBytes)
    }

    // MARK: - Reading the file

    /// The file's first sound track and its length, as AVFoundation reads
    /// them; nil track when there is none it can open.
    static func probe(_ url: URL) async -> (track: Track?, durationMS: Int?) {
        let asset = AVURLAsset(url: url)
        var durationMS: Int?
        if let duration = try? await asset.load(.duration), duration.isNumeric, duration.seconds > 0 {
            durationMS = Int(duration.seconds * 1000)
        }
        guard let audio = try? await asset.loadTracks(withMediaType: .audio).first else {
            return (nil, durationMS)
        }
        let (dataRate, formats) = (try? await audio.load(.estimatedDataRate, .formatDescriptions))
            ?? (Float(0), [])
        let format = formats.first
        let track = Track(
            codec: format.map { MediaProbe.audioCodec(CMFormatDescriptionGetMediaSubType($0)) }
                ?? "unknown",
            channels: format.flatMap(MediaProbe.channels(of:)),
            bitrate: MediaProbe.bitrate(dataRate))
        return (track, durationMS)
    }

    /// The extension a downloaded file is saved under, from its stored
    /// type. AVFoundation picks its reader by the file's extension: the
    /// same Ogg bytes open as `.ogg` and are "Cannot Open" with none.
    static func fileExtension(for mime: String) -> String {
        switch mime {
        case "audio/mp4", "audio/m4a", "audio/aac", "audio/x-m4a": "m4a"
        case "audio/mpeg", "audio/mp3": "mp3"
        case "audio/wav", "audio/wave", "audio/x-wav": "wav"
        case "audio/ogg", "audio/opus": "ogg"
        case "audio/webm", "video/webm": "webm"
        case "audio/flac", "audio/x-flac": "flac"
        case "audio/aiff", "audio/x-aiff": "aiff"
        case "video/quicktime": "mov"
        case "video/mp4": "mp4"
        default:
            mime.hasPrefix("video/") ? "mp4" : "m4a"
        }
    }

    // MARK: - Making the part

    /// Take the sound out of `source` (a file already here) into
    /// `folder`/sound.m4a and return it — measured, within the ceiling.
    /// Throws `Failure` for what cannot be done, `CancellationError` when
    /// the asker went away.
    @concurrent
    static func extract(
        from source: URL, durationMS knownDuration: Int?, maxBytes: Int64?, into folder: URL
    ) async throws -> URL {
        let probed = await probe(source)
        let duration = probed.durationMS ?? knownDuration
        let first: Treatment
        switch treatment(track: probed.track, durationMS: duration, maxBytes: maxBytes) {
        case .success(let decided): first = decided
        case .failure(let failure): throw failure
        }
        let output = folder.appendingPathComponent("sound").appendingPathExtension("m4a")
        var attempt = first
        while true {
            do {
                try await MediaTranscoder.extractSound(
                    from: source, treatment: attempt, writingTo: output)
            } catch is CancellationError {
                throw CancellationError()
            } catch {
                // A track AVFoundation listed but could not read, or would
                // not take the settings for: this file's sound is not to be
                // had on this device. A failed pass-through still gets its
                // re-encode first.
                if attempt == .passThrough {
                    attempt = .reencode
                    continue
                }
                throw Failure.unreadable
            }
            let size = (try? output.resourceValues(forKeys: [.fileSizeKey]).fileSize)
                .map(Int64.init) ?? 0
            if fits(bytes: size, maxBytes: maxBytes) { return output }
            guard attempt == .passThrough, size > 0 else {
                throw size > 0 ? Failure.tooLong : Failure.unreadable
            }
            attempt = .reencode
        }
    }

    /// The whole of it for one attachment: said "too long" at once when
    /// its length proves it, otherwise downloaded into `folder`, its sound
    /// taken out, and the part returned. The caller owns `folder` and
    /// removes it.
    @concurrent
    static func prepare(
        _ attachment: AttachmentDTO, maxBytes: Int64?, api: APIClient, into folder: URL
    ) async throws -> URL {
        if knownTooLong(attachment, maxBytes: maxBytes) { throw Failure.tooLong }
        let source = folder.appendingPathComponent("source")
            .appendingPathExtension(fileExtension(for: attachment.mime))
        try await api.downloadAttachment(id: attachment.id, to: source)
        return try await extract(
            from: source, durationMS: attachment.durationMS, maxBytes: maxBytes, into: folder)
    }
}
