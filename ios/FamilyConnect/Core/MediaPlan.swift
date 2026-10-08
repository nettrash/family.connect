//
//  MediaPlan.swift
//  FamilyConnect
//
//  What a picked video or sound file becomes before it is uploaded
//  (docs/protocol.md, "Preparing media before upload"; the reasoning is
//  docs/media-upload-2026-09-28.md, issue #74).
//
//  The server stores what it is given and never transcodes, so the size of a
//  family's history is decided on the sending device — by four codebases that
//  have to reach the same answer for the same file. This client compressing
//  to 1080p while Android compressed to 720p is what happened when nothing
//  wrote the target down. So this is only the DECISION, as arithmetic: what
//  the source is, as AVFoundation saw it (`MediaProbe`), goes in; what to do
//  with it comes out. Doing it is `MediaTranscoder`'s job, and choosing which
//  bytes to upload is `MediaPrep`'s.
//
//  A PORT, NOT A READING. The reference is the web client's
//  `web/text/src/media_plan.rs`, and this file is held to it by the vectors
//  it prints (FamilyConnectTests/Fixtures/media-plan-vectors.json, the same
//  bytes Android and Windows test against). So the names follow it one for
//  one, in Swift's casing, and so does the arithmetic — including the two
//  places where the order of operations is part of the rule (`targetSize`'s
//  half-up, `profileVideoBitrate`'s single product and division) and would
//  otherwise be the first thing a tidy-minded edit "simplified".
//
//  Integers everywhere the protocol says so, `Double` only where a frame rate
//  enters, and every function is total: a number AVFoundation could not fill
//  in is "unknown", never a trap.
//

import Foundation

nonisolated enum MediaPlan {

    // MARK: - The profile

    /// The target's SHORT side at most — a 720p clip is indistinguishable
    /// from 1080p in a chat bubble, and roughly half the bytes.
    static let maxShortSide = 720

    /// The frame rate a transcode is capped at: 60 fps doubles the bits for
    /// smoothness a family clip rarely needs.
    static let maxFrameRate = 30.0

    /// The frame rate a source may have and still count as "at most 30". Not
    /// 30 itself, because `nominalFrameRate` reports a nominal 30 as 30.0003
    /// or a variable one as 30.2, and re-encoding a 30 fps clip for that would
    /// only cost quality. A rate ABOVE this becomes `maxFrameRate`; one at or
    /// below it is kept exactly as it is, 29.97, 25 and 24 included.
    static let frameRateTolerance = 30.5

    /// The video bitrate's floor and ceiling. 2 000 000 bit/s is the
    /// profile's rate at 1280×720, 30 fps; below 250 000 a small clip turns
    /// to mush for a saving nobody would notice.
    static let minVideoBitrate = 250_000
    static let maxVideoBitrate = 2_000_000

    /// AAC-LC for the audio in a video and for a picked file that is
    /// re-encoded: transparent for speech and ambient sound. Mono gets half —
    /// the same quality per channel.
    static let stereoAudioBitrate = 128_000
    static let monoAudioBitrate = 64_000

    /// A voice note, which is recorded to the profile directly
    /// (`AudioRecorder`) and never passes through `planAudio`: AAC-LC, mono,
    /// 44.1 kHz, this many bits a second.
    static let voiceNoteBitrate = 64_000

    /// MP3 or AAC at or below this is uploaded untouched. A second lossy
    /// generation costs more than the few megabytes it saves — nettrash's 1.1
    /// objection to re-encoding someone's music, kept where it is right.
    static let maxKeptLossyAudioBitrate = 192_000

    // MARK: - What a reader saw

    /// A video about to be sent as `kind=video`, as `MediaProbe` read it.
    ///
    /// Codecs are lowercase names rather than four-character codes, so four
    /// platforms' readers can agree on them: `"h264"` (`avc1`/`avc3`),
    /// `"hevc"` (`hvc1`/`hev1`), `"av1"`, `"vp9"`, and `"unknown"` for
    /// anything else. Only `"h264"` and, for audio, `"aac"` (any AAC profile)
    /// are ever asked about.
    struct VideoSource: Equatable, Sendable {
        /// The size as it is DISPLAYED — after the track's rotation. A
        /// portrait phone clip is a landscape track with a 90° turn; this is
        /// 1080×1920, not 1920×1080. 0 when it could not be read.
        var width: Int
        var height: Int
        /// Frames per second, or nil. A value that is not a finite number
        /// above zero is unknown too (`knownFrameRate`).
        var frameRate: Double?
        /// The type it would go up as — `"video/mp4"` or `"video/quicktime"`,
        /// told apart by the `ftyp` brand, not by the extension.
        var container: String
        var videoCodec: String
        /// Nil when the file has NO audio track. A track whose codec cannot
        /// be named is `"unknown"`, which is not "absent".
        var audioCodec: String?
        /// Only exactly 1 is mono.
        var audioChannels: Int?
        /// The rates the container states, in bit/s, or nil (0 is nil too).
        var videoBitrate: Int?
        var audioBitrate: Int?
        /// The whole file.
        var sizeBytes: Int
        /// Milliseconds, or nil (and 0 is nil).
        var durationMS: Int?
    }

    /// What a video is transcoded TO.
    struct VideoTarget: Equatable, Sendable {
        /// Even, never larger than the source's, in the source's orientation.
        var width: Int
        var height: Int
        var frameRate: Double
        var videoBitrate: Int
        /// Nil when the source has no audio track — a transcode does not
        /// invent silence.
        var audioBitrate: Int?
    }

    /// What happens to a video.
    enum VideoPlan: Equatable, Sendable {
        /// Rule A: it is already within the profile, and the ORIGINAL goes.
        case keep
        /// Rule 5: transcode to this target.
        case transcode(VideoTarget)
        /// Rule C: there is no size to scale to — none could be read, or the
        /// short side is one pixel and an even target would have none — so
        /// this platform "cannot transcode this source at all" and it goes
        /// the way it went before the section existed (`onFailure`).
        case fallback
    }

    /// A sound file the member PICKED — never a voice note.
    ///
    /// `codec` is lowercase: `"pcm"` (WAV and AIFF are both PCM), `"flac"`,
    /// `"alac"`, `"aac"`, `"mp3"`, `"vorbis"`, `"opus"`, or `"unknown"`.
    struct AudioSource: Equatable, Sendable {
        /// The type the file is: one the server accepts as audio
        /// (`"audio/mp4"`, `"audio/mpeg"`, `"audio/wav"`, `"audio/ogg"`) or
        /// the system's own name for one it does not (`"audio/aiff"`,
        /// `"audio/flac"`).
        var container: String
        var codec: String
        var channels: Int?
        /// As the file states it, in bit/s, or nil (0 is nil).
        var bitrate: Int?
        var sizeBytes: Int
        var durationMS: Int?
    }

    /// What happens to a picked sound file.
    enum AudioPlan: Equatable, Sendable {
        /// Uploaded untouched.
        case keep
        /// Re-encoded as M4A (`audio/mp4`), AAC-LC, at this bitrate.
        case transcode(bitrate: Int)
    }

    // MARK: - Unknowns

    /// A count the reader handed over, or nil when it could not. 0 is nil as
    /// well — it is what a container writes for a rate it does not state, and
    /// a file of no duration has no rate to estimate — and so is anything
    /// negative, which the reference's unsigned integers cannot even hold.
    private static func known(_ value: Int?) -> Int? {
        guard let value, value > 0 else { return nil }
        return value
    }

    /// A frame rate worth believing, or nil: absent, zero, negative, infinite
    /// or NaN all mean "unknown" to rule 2.
    static func knownFrameRate(_ frameRate: Double?) -> Double? {
        guard let frameRate, frameRate.isFinite, frameRate > 0 else { return nil }
        return frameRate
    }

    /// `a × b`, pinned at `Int.max` rather than trapping — the reference's
    /// `saturating_mul`. Nonsense in must not be a crash out.
    private static func saturating(_ a: Int, times b: Int) -> Int {
        let (product, overflow) = a.multipliedReportingOverflow(by: b)
        return overflow ? Int.max : product
    }

    // MARK: - The rules

    /// A bitrate the container does not state, estimated as the protocol
    /// says: `size × 8 × 1000 ÷ duration_ms − audio_bitrate`, in integers (the
    /// division truncates). Nil when it "cannot be estimated": no duration, or
    /// nothing left once the audio is taken off.
    ///
    /// For a sound file on its own there is nothing to take off, and
    /// `audioBitrate` is 0. It counts every byte of the file, so an MP3's
    /// cover art reads as bitrate; a stated rate always wins over it.
    static func estimatedBitrate(sizeBytes: Int, durationMS: Int?, audioBitrate: Int) -> Int? {
        guard let duration = known(durationMS) else { return nil }
        let whole = saturating(sizeBytes, times: 8 * 1000) / duration
        guard whole >= audioBitrate else { return nil }
        let video = whole - audioBitrate
        return video > 0 ? video : nil
    }

    /// `V`: the video bitrate as stated, or else as estimated.
    ///
    /// The estimate takes the AUDIO's bitrate off the whole, so it needs that
    /// bitrate: with no audio track it is 0, but a track whose rate is not
    /// known leaves an unknown in the formula — and then `V` "cannot be
    /// estimated either". (Which sends such a file past rule A to a transcode,
    /// and rule D keeps the original if that came out bigger.)
    static func sourceVideoBitrate(_ source: VideoSource) -> Int? {
        if let stated = known(source.videoBitrate) {
            return stated
        }
        let audio: Int
        if source.audioCodec == nil {
            audio = 0
        } else {
            guard let stated = known(source.audioBitrate) else { return nil }
            audio = stated
        }
        return estimatedBitrate(
            sizeBytes: source.sizeBytes, durationMS: source.durationMS, audioBitrate: audio)
    }

    /// Rule 1: the target size for a DISPLAYED `width × height`.
    ///
    /// The short side becomes `min(720, short)` — never upscaled. The long
    /// side follows in proportion, rounded half up in integers:
    /// `(2 × long × ts + short) ÷ (2 × short)`. Then each side drops to even,
    /// because H.264 works in 2×2 chroma blocks — DROPS, not rounds, so the
    /// result can never exceed the source. The orientation is the source's.
    ///
    /// (0, 0) for a source with no size.
    static func targetSize(width: Int, height: Int) -> (width: Int, height: Int) {
        let short = min(width, height)
        let long = max(width, height)
        guard short > 0 else { return (0, 0) }
        let targetShortUneven = min(short, maxShortSide)
        let targetLongUneven = (2 * long * targetShortUneven + short) / (2 * short)
        let targetShort = targetShortUneven - targetShortUneven % 2
        let targetLong = targetLongUneven - targetLongUneven % 2
        return width >= height ? (targetLong, targetShort) : (targetShort, targetLong)
    }

    /// Rule 2: 30 when the source's rate is above `frameRateTolerance` or
    /// unknown, the source's own rate otherwise — never raised, and never
    /// "tidied": 29.97 stays 29.97.
    static func targetFrameRate(_ frameRate: Double?) -> Double {
        if let rate = knownFrameRate(frameRate), rate <= frameRateTolerance {
            return rate
        }
        return maxFrameRate
    }

    /// Rule 3, before the source cap: `2 000 000 × (w × h ÷ 921 600) × (f ÷ 30)`,
    /// clamped to [250 000, 2 000 000], rounded to the nearest 1 000, HALF UP.
    ///
    /// THE ORDER IS PART OF THE RULE. Evaluated left to right as the protocol
    /// writes it, 3618×128 at 25 fps comes out 837 499.9999 and rounds down,
    /// where the exact value is 837 500. `2 000 000 ÷ (921 600 × 30)` is exactly
    /// `1 ÷ 13 824`, so the rate in THOUSANDS is one product of the pixel count
    /// and the frame rate, then one division — for a whole frame rate the
    /// product is exact and the division correctly rounded, so a true tie
    /// lands on exactly .5. And `.toNearestOrAwayFromZero`, spelled out:
    /// half-to-even would make 404×288 at 30 fps (252.5 thousand) 252 000
    /// where every other port says 253 000.
    ///
    /// `Double(width) * Double(height)` is the same double as the reference's
    /// `(w × h) as f64`: both are the one correct rounding of the exact
    /// product, and this way a nonsense size cannot overflow an `Int` first.
    static func profileVideoBitrate(width: Int, height: Int, frameRate: Double) -> Int {
        let thousands = Double(width) * Double(height) * frameRate / 13_824.0
        // Only a NaN frame rate gets here with NaN, and Rust's `NaN as u64` is
        // 0; `Int(Double.nan)` would trap instead.
        guard !thousands.isNaN else { return 0 }
        let clamped = min(
            max(thousands, Double(minVideoBitrate / 1000)), Double(maxVideoBitrate / 1000))
        return Int(clamped.rounded(.toNearestOrAwayFromZero)) * 1000
    }

    /// Rule 3 whole: the profile's bitrate, and then — if `V` is known — no
    /// higher than `V` (rule B). The cap comes AFTER the rounding, so a capped
    /// target is the source's exact rate, not a whole thousand.
    static func targetVideoBitrate(
        width: Int, height: Int, frameRate: Double, sourceBitrate: Int?
    ) -> Int {
        let profile = profileVideoBitrate(width: width, height: height, frameRate: frameRate)
        guard let source = known(sourceBitrate) else { return profile }
        return min(profile, source)
    }

    /// 128 000 for stereo, 64 000 for mono — never above the source's, when
    /// that is known. Only exactly one channel is mono: 5.1 takes the stereo
    /// rate, and so does a count nobody could read, because guessing mono
    /// would halve a stereo track.
    static func targetAudioBitrate(channels: Int?, sourceBitrate: Int?) -> Int {
        let profile = channels == 1 ? monoAudioBitrate : stereoAudioBitrate
        guard let source = known(sourceBitrate) else { return profile }
        return min(profile, source)
    }

    /// Rules 1–3 for a source: the size, frame rate and two bitrates it would
    /// be transcoded to. Nil when it has no size.
    ///
    /// The audio's cap is the rate the container STATES for it: the protocol
    /// gives no way to estimate an audio track's rate inside a video.
    static func videoTarget(_ source: VideoSource) -> VideoTarget? {
        guard source.width != 0, source.height != 0 else { return nil }
        let size = targetSize(width: source.width, height: source.height)
        let frameRate = targetFrameRate(source.frameRate)
        let videoBitrate = targetVideoBitrate(
            width: size.width, height: size.height, frameRate: frameRate,
            sourceBitrate: sourceVideoBitrate(source))
        let audioBitrate = source.audioCodec.map { _ in
            targetAudioBitrate(channels: source.audioChannels, sourceBitrate: source.audioBitrate)
        }
        return VideoTarget(
            width: size.width, height: size.height, frameRate: frameRate,
            videoBitrate: videoBitrate, audioBitrate: audioBitrate)
    }

    /// Rule A — leave it alone. Every condition, exactly as listed: MP4;
    /// H.264; AAC or no audio; short side at most 720; `F` known and at most
    /// 30.5; `V` known and at most 1.25 × step 3's bitrate, which is
    /// `4 × V ≤ 5 × target` so it stays in integers. (`video/quicktime` is
    /// never within the profile, even holding H.264: Firefox will not play the
    /// container — which is every iPhone camera clip.)
    ///
    /// What it does NOT ask, because the protocol's list does not: the
    /// audio's bitrate, the sides being even, or where the `moov` box is. A
    /// kept file is the original, whatever those are.
    static func withinProfile(_ source: VideoSource) -> Bool {
        guard let target = videoTarget(source),
              let rate = knownFrameRate(source.frameRate),
              let bitrate = sourceVideoBitrate(source)
        else {
            return false
        }
        return source.container == "video/mp4"
            && source.videoCodec == "h264"
            && (source.audioCodec == nil || source.audioCodec == "aac")
            && min(source.width, source.height) <= maxShortSide
            && rate <= frameRateTolerance
            && saturating(bitrate, times: 4) <= saturating(target.videoBitrate, times: 5)
    }

    /// Rules A, 5 and C, in that order: kept when within the profile (a
    /// one-pixel-wide clip can be — it is the original that goes); otherwise
    /// transcoded; and with no size to transcode to, the fallback.
    static func planVideo(_ source: VideoSource) -> VideoPlan {
        guard let target = videoTarget(source) else { return .fallback }
        if withinProfile(source) { return .keep }
        if target.width == 0 || target.height == 0 { return .fallback }
        return .transcode(target)
    }

    /// A sound file's bitrate: as stated, or else estimated from its size and
    /// duration with nothing taken off — it is the only stream.
    static func sourceAudioBitrate(_ source: AudioSource) -> Int? {
        known(source.bitrate)
            ?? estimatedBitrate(sizeBytes: source.sizeBytes, durationMS: source.durationMS, audioBitrate: 0)
    }

    /// The audio rules, in order:
    ///
    /// 1. Uncompressed or lossless — `pcm` (WAV, AIFF), `flac`, `alac` — is
    ///    re-encoded: about ten times smaller, and no first generation to lose.
    /// 2. Anything in Ogg is re-encoded, because AVFoundation has no Ogg reader
    ///    — which on THIS platform means the transcode fails and rule C sends
    ///    the original, exactly as the protocol's "wherever the platform can
    ///    decode it" allows. It is still asked, so the answer is the one every
    ///    port gives.
    /// 3. MP3 or AAC above 192 000 bit/s is re-encoded; at or below, or with a
    ///    bitrate that can be neither read nor estimated, it is untouched.
    ///
    /// Anything else — a lossy codec no rule names, or a codec nobody could
    /// name outside Ogg — is left untouched, because no rule says otherwise.
    static func planAudio(_ source: AudioSource) -> AudioPlan {
        let bitrate = sourceAudioBitrate(source)
        let transcode: Bool
        switch source.codec {
        case "pcm", "flac", "alac":
            transcode = true
        case _ where source.container == "audio/ogg":
            transcode = true
        case "mp3", "aac":
            transcode = bitrate.map { $0 > maxKeptLossyAudioBitrate } ?? false
        default:
            transcode = false
        }
        guard transcode else { return .keep }
        return .transcode(bitrate: targetAudioBitrate(channels: source.channels, sourceBitrate: bitrate))
    }

    // MARK: - When it goes wrong, or does not help

    /// The types the server takes as each kind (server `models.rs`,
    /// `Attachment::ACCEPTED`; docs/protocol.md, "Audio" for the audio list).
    private static let acceptedVideo: Set<String> = ["video/mp4", "video/quicktime"]
    private static let acceptedAudio: Set<String> = [
        "audio/mp4", "audio/m4a", "audio/mpeg", "audio/wav", "audio/ogg",
    ]

    /// Whether a file could go up as `kind` (`"video"` or `"audio"`) just as
    /// it is — the protocol's "sendable as that kind": an accepted type,
    /// honest bytes (`MediaPrep.Magic`), and within the ceiling. Within means
    /// at most: the server refuses only a body LARGER than its limit.
    static func sendable(
        kind: String, container: String, honest: Bool, sizeBytes: Int, ceilingBytes: Int
    ) -> Bool {
        let accepted: Bool
        switch kind {
        case "video": accepted = acceptedVideo.contains(container)
        case "audio": accepted = acceptedAudio.contains(container)
        default: accepted = false
        }
        return accepted && honest && sizeBytes <= ceilingBytes
    }

    /// What rule C sends when a transcode fails, or cannot be attempted.
    enum OnFailure: Equatable, Sendable {
        /// The original, untouched, as its kind.
        case original
        /// Whatever this client did before the section existed — for a video
        /// over the ceiling, 1.1's 1080p export; for anything the server would
        /// not take as it is, a `file`. Not a new rule: the old one, unchanged.
        case todaysPath
    }

    /// Rule C — a failure sends what would have been sent without this
    /// section. Preparing media is an optimisation, and it may never turn a
    /// send that would have worked into one that does not.
    static func onFailure(sourceSendable: Bool) -> OnFailure {
        sourceSendable ? .original : .todaysPath
    }

    /// Which bytes rule D uploads.
    enum Upload: Equatable, Sendable {
        case source
        case result
    }

    /// Rule D — a result BIGGER than its source is thrown away and the source
    /// uploaded instead, provided the source is sendable as that kind;
    /// otherwise the result is the only thing that can be sent. Exactly the
    /// source's size is not bigger: the result is used, since it is in the
    /// profile and the source may not be.
    static func keepSmaller(sourceBytes: Int, sourceSendable: Bool, resultBytes: Int) -> Upload {
        resultBytes > sourceBytes && sourceSendable ? .source : .result
    }
}
