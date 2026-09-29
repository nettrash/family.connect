//
//  MediaProbe.swift
//  FamilyConnect
//
//  What a picked video or sound file IS, as AVFoundation reads it: the
//  planner's input (`MediaPlan.VideoSource` / `AudioSource`), and nothing
//  more. The planner decides; this only reports.
//
//  EVERY FIELD IS ALLOWED TO BE UNKNOWN. A file AVFoundation cannot open
//  still gets a source — no size, no codec — and the planner turns "no size"
//  into rule C's fallback rather than this file turning it into an error.
//  That is what keeps a send that worked in 1.1 working now: an unreadable
//  clip is exactly as sendable as it was.
//
//  THE NAMES ARE THE REFERENCE'S, NOT APPLE'S. Codecs are reported as the
//  lowercase words `web/text/src/media_plan.rs` uses, so that "is this
//  H.264?" has one answer on four platforms rather than four spellings of
//  a four-character code. AAC is AAC in every profile (HE, HE v2, LD, ELD),
//  as the protocol's rule A means it.
//
//  Android counterpart: data/repo/MediaPrep.kt.
//

import AVFoundation
import CoreMedia
import Foundation

nonisolated enum MediaProbe {

    // MARK: - Video

    /// The type a video would go up as, read off its BYTES: an ISO base media
    /// file whose `ftyp` major brand is `qt  ` is a QuickTime movie, any other
    /// `ftyp` is an MP4, and a file with no `ftyp` at 4 is neither — nothing
    /// this client may honestly claim as a video (`MediaPrep.Magic`).
    ///
    /// Not the extension: a camera clip is a `.mov` and a screen recording an
    /// `.mp4`, and rule A turns on the difference, because Firefox will not
    /// play the QuickTime container whatever it holds.
    static func videoContainer(of url: URL) -> String? {
        guard let head = MediaPrep.Magic.head(of: url),
              MediaPrep.Magic.matches(mime: "video/mp4", head: head)
        else {
            return nil
        }
        let bytes = [UInt8](head)
        return Array(bytes[8..<12]) == Array("qt  ".utf8) ? "video/quicktime" : "video/mp4"
    }

    /// A video as the planner needs it. `container` is `videoContainer`'s
    /// answer, or the system's own name for a file that has none.
    static func video(at url: URL, container: String, sizeBytes: Int) async -> MediaPlan.VideoSource {
        let asset = AVURLAsset(url: url)
        var source = MediaPlan.VideoSource(
            width: 0, height: 0, frameRate: nil, container: container,
            videoCodec: "unknown", audioCodec: nil, audioChannels: nil,
            videoBitrate: nil, audioBitrate: nil, sizeBytes: sizeBytes, durationMS: nil)
        source.durationMS = await durationMS(of: asset)

        if let track = try? await asset.loadTracks(withMediaType: .video).first,
           let loaded = try? await track.load(
               .naturalSize, .preferredTransform, .nominalFrameRate, .estimatedDataRate,
               .formatDescriptions) {
            let (natural, transform, rate, dataRate, formats) = loaded
            // The DISPLAYED size: `naturalSize` is the track as stored, and a
            // portrait phone clip is stored on its side with a 90° turn.
            let shown = CGRect(origin: .zero, size: natural).applying(transform)
            source.width = Int(abs(shown.width).rounded())
            source.height = Int(abs(shown.height).rounded())
            source.frameRate = rate > 0 ? Double(rate) : nil
            source.videoBitrate = bitrate(dataRate)
            source.videoCodec = formats.first.map { videoCodec(CMFormatDescriptionGetMediaSubType($0)) } ?? "unknown"
        }

        // A track that is there but cannot be described is "unknown", not
        // absent: only NO audio track may count as "no audio" in rule A.
        if let track = try? await asset.loadTracks(withMediaType: .audio).first {
            let (dataRate, formats) = (try? await track.load(.estimatedDataRate, .formatDescriptions))
                ?? (Float(0), [])
            let format = formats.first
            source.audioCodec = format.map { audioCodec(CMFormatDescriptionGetMediaSubType($0)) } ?? "unknown"
            source.audioChannels = format.flatMap(channels(of:))
            source.audioBitrate = bitrate(dataRate)
        }
        return source
    }

    // MARK: - Audio alone

    /// A picked sound file as the planner needs it. `container` is the type
    /// it would go up as (`MediaPrep.audioMIME`), or the system's own name
    /// for one the server does not take.
    static func audio(at url: URL, container: String, sizeBytes: Int) async -> MediaPlan.AudioSource {
        let asset = AVURLAsset(url: url)
        var source = MediaPlan.AudioSource(
            container: container, codec: "unknown", channels: nil, bitrate: nil,
            sizeBytes: sizeBytes, durationMS: nil)
        source.durationMS = await durationMS(of: asset)
        if let track = try? await asset.loadTracks(withMediaType: .audio).first,
           let loaded = try? await track.load(.estimatedDataRate, .formatDescriptions) {
            let (dataRate, formats) = loaded
            let format = formats.first
            source.codec = format.map { audioCodec(CMFormatDescriptionGetMediaSubType($0)) } ?? "unknown"
            source.channels = format.flatMap(channels(of:))
            source.bitrate = bitrate(dataRate)
        }
        return source
    }

    // MARK: - Names

    /// A four-character code from its spelling — for the codes Core Media
    /// has no constant for on every deployment target (`avc3`, `hev1`,
    /// `av01`, `vp09`).
    static func fourCC(_ code: String) -> FourCharCode {
        code.utf8.reduce(0) { ($0 << 8) | FourCharCode($1) }
    }

    static func videoCodec(_ subtype: FourCharCode) -> String {
        switch subtype {
        case kCMVideoCodecType_H264, fourCC("avc3"): "h264"
        case kCMVideoCodecType_HEVC, fourCC("hev1"): "hevc"
        case fourCC("av01"): "av1"
        case fourCC("vp09"): "vp9"
        default: "unknown"
        }
    }

    static func audioCodec(_ subtype: FourCharCode) -> String {
        switch subtype {
        // Every AAC object type Core Audio names. xHE-AAC (`usac`) is left
        // out on purpose: it is a different codec family that several
        // browsers do not decode, so treating it as "AAC" in rule A would
        // keep a file some members cannot play.
        case kAudioFormatMPEG4AAC, kAudioFormatMPEG4AAC_HE, kAudioFormatMPEG4AAC_HE_V2,
             kAudioFormatMPEG4AAC_LD, kAudioFormatMPEG4AAC_ELD, kAudioFormatMPEG4AAC_ELD_SBR,
             kAudioFormatMPEG4AAC_ELD_V2:
            "aac"
        case kAudioFormatMPEGLayer3: "mp3"
        case kAudioFormatLinearPCM: "pcm"
        case kAudioFormatAppleLossless: "alac"
        case kAudioFormatFLAC: "flac"
        case kAudioFormatOpus: "opus"
        default: "unknown"
        }
    }

    // MARK: - Numbers

    /// Channels per frame, or nil when the description does not say.
    static func channels(of format: CMFormatDescription) -> Int? {
        guard let description = CMAudioFormatDescriptionGetStreamBasicDescription(format)?.pointee,
              description.mChannelsPerFrame > 0
        else {
            return nil
        }
        return Int(description.mChannelsPerFrame)
    }

    /// `estimatedDataRate` in whole bits a second — AVFoundation's own
    /// figure for the track's samples, which is what the container states
    /// about them. 0 (not known) is nil, as the planner reads it.
    private static func bitrate(_ dataRate: Float) -> Int? {
        guard dataRate.isFinite, dataRate > 0 else { return nil }
        return Int(dataRate.rounded())
    }

    /// Milliseconds, truncated the way `Prepared.durationMS` always has been,
    /// or nil for an asset that has no duration worth the name.
    private static func durationMS(of asset: AVURLAsset) async -> Int? {
        guard let duration = try? await asset.load(.duration), duration.isNumeric else { return nil }
        let milliseconds = Int(duration.seconds * 1000)
        return milliseconds > 0 ? milliseconds : nil
    }
}
