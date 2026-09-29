//
//  MediaPrep.swift
//  FamilyConnect
//
//  Turning what the picker hands over into something worth uploading: a
//  downscaled JPEG for a photo, an MP4 brought to the protocol's profile for
//  a video that is not already within it, an M4A for a sound file the audio
//  rules say to re-encode, and for pictures a small preview the bubble can
//  draw before the full file has been fetched. A FILE is not touched at
//  all — it is copied where we own it and sent as it is.
//
//  What a video or a sound file becomes is DECIDED by `MediaPlan` (the port
//  of docs/protocol.md's "Preparing media before upload" that every client
//  shares), from what `MediaProbe` reads, and DONE by `MediaTranscoder`.
//  This file only chooses which bytes go up — including, when anything goes
//  wrong, exactly the bytes 1.1 would have sent (rule C).
//
//  Everything here produces a FILE ON DISK, never a Data in memory. A
//  100 MB video read into a Data is 100 MB of resident memory on a phone
//  that also has to render a chat — and URLSession can stream an upload
//  straight from a file.
//
//  The server never decodes an image or a video (docs/protocol.md), which
//  is exactly why the preview is made here.
//
//  Android counterpart: data/repo/MediaPrep.kt
//

import AVFoundation
import CoreTransferable
import ImageIO
import UniformTypeIdentifiers
import os

nonisolated enum MediaPrep {

    /// The server's own magic-number check, on this side of the wire.
    ///
    /// A CLAIM ABOUT BYTES IS CHECKED BEFORE IT IS MADE. The server verifies that a declared type
    /// matches what the bytes ARE for every photo, video and audio upload — "a type that
    /// contradicts the kind, or bytes that do not match the type declared, is
    /// `invalid_attachment`" (docs/protocol.md) — and refuses the upload when it does not. A
    /// client that guesses the type from a file EXTENSION therefore has two ways to be wrong
    /// about the same file, and both end as a failed send the sender cannot act on:
    ///
    ///   * a `.aac` file is usually raw ADTS, not ISO base media, so calling it `audio/mp4` (as
    ///     the extension table did) is a 400 for every such file a family ever picks;
    ///   * a `.mkv` conforms to `public.movie`, so it went down the video path and was uploaded
    ///     UNCHANGED under a flat `video/mp4` — Matroska bytes claiming an MP4 container.
    ///     AVFoundation cannot re-encode it either, so there is nothing to convert it into.
    ///
    /// The protocol already says what to do with something a client cannot honestly type:
    /// "a recording that a client cannot encode into a checkable container should be sent as
    /// `kind=file` instead, where nothing is verified." So this decides, and the callers below
    /// fall back to the file path — where the family still gets the thing they picked.
    ///
    /// Mirrors `server/src/handlers_attachment.rs::matches_magic` and
    /// `fc_text::media::matches_magic` (the web's copy) table for table.
    enum Magic {
        /// How many bytes are enough to judge any of the types below.
        static let head = 12

        static func matches(mime: String, head bytes: Data) -> Bool {
            let head = [UInt8](bytes.prefix(Self.head))
            switch mime {
            case "image/jpeg":
                return head.starts(with: [0xFF, 0xD8, 0xFF])
            case "image/png":
                return head.starts(with: [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
            // HEIC/HEIF, MP4/MOV and m4a are all ISO base media: "ftyp" at offset 4, with the
            // brand that follows telling them apart.
            case "image/heic", "image/heif", "video/mp4", "video/quicktime",
                 "audio/mp4", "audio/m4a":
                return head.count >= 12 && Array(head[4..<8]) == Array("ftyp".utf8)
            // An MP3 is either an ID3 tag or a raw frame sync (11 set bits).
            case "audio/mpeg":
                return head.starts(with: Array("ID3".utf8))
                    || (head.count >= 2 && head[0] == 0xFF && (head[1] & 0xE0) == 0xE0)
            case "audio/wav":
                return head.count >= 12
                    && head.starts(with: Array("RIFF".utf8))
                    && Array(head[8..<12]) == Array("WAVE".utf8)
            case "audio/ogg":
                return head.starts(with: Array("OggS".utf8))
            default:
                return false
            }
        }

        /// The first bytes of a file, or nil when it cannot be read at all.
        static func head(of url: URL) -> Data? {
            guard let handle = try? FileHandle(forReadingFrom: url) else { return nil }
            defer { try? handle.close() }
            return try? handle.read(upToCount: Self.head)
        }

        /// Whether this file may honestly be uploaded as `mime`. A file we cannot read is judged
        /// UNSAFE: the send would fail anyway, and the file path is the answer either way.
        static func honest(url: URL, mime: String) -> Bool {
            guard let bytes = head(of: url) else { return false }
            return matches(mime: mime, head: bytes)
        }
    }

    /// What the picker gave us, prepared for upload.
    struct Prepared {
        /// The file to upload. Lives in a temp directory; delete after.
        let fileURL: URL
        let mime: String
        /// "photo" | "video" | "file".
        let kind: String
        let width: Int?
        let height: Int?
        let durationMS: Int?
        /// Small JPEG for the bubble: the downscaled photo, or a video's
        /// poster frame. Files have none — there is nothing to draw.
        let previewJPEG: Data?
        /// Files only: the name the sender picked it by.
        let name: String?

        init(
            fileURL: URL,
            mime: String,
            kind: String,
            width: Int? = nil,
            height: Int? = nil,
            durationMS: Int? = nil,
            previewJPEG: Data? = nil,
            name: String? = nil
        ) {
            self.fileURL = fileURL
            self.mime = mime
            self.kind = kind
            self.width = width
            self.height = height
            self.durationMS = durationMS
            self.previewJPEG = previewJPEG
            self.name = name
        }
    }

    /// Throw away an item that will not be sent: taken off the strip,
    /// refused past the ten-item cap, or a board pin that is over.
    ///
    /// The file goes ONLY if MediaPrep wrote it. `prepareVideo` hands back
    /// the ORIGINAL url when a clip goes as it is (rule A, C or D), and a
    /// picked or dropped file is the person's own — deleting `fileURL` without
    /// asking deleted their video from wherever they kept it. The same
    /// ownership test `PendingMediaStaging.adopt` uses to copy instead of
    /// move.
    static func discard(_ prepared: Prepared) {
        guard PendingMediaStaging.isOurs(prepared.fileURL) else { return }
        try? FileManager.default.removeItem(at: prepared.fileURL)
    }

    enum PrepError: Error {
        /// The item could not be read or decoded at all.
        case unreadable
        /// A video that is still over the ceiling after re-encoding. The
        /// only case where the user has to do something themselves.
        case tooLargeAfterCompression(bytes: Int)
    }

    /// The protocol's default ceiling (docs/protocol.md, "Photos, videos
    /// and files"). A self-hosted server may be configured lower and does not
    /// advertise its limit, so this is what the client PREPARES to; a
    /// stricter server still answers `attachment_too_large` and the send
    /// fails with a message rather than silently.
    static let sizeLimit = 100 * 1024 * 1024

    /// Longest edge of an uploaded photo. Generous enough to look right
    /// full-screen on any device, small enough that a family album does not
    /// fill a home server.
    static let photoEdge = 2048
    /// Longest edge of the preview drawn in a bubble.
    static let previewEdge = 600
    static let photoQuality: CGFloat = 0.85
    static let previewQuality: CGFloat = 0.7

    // MARK: - Photos

    /// `async` so it leaves the main actor. MediaPrep is nonisolated, so
    /// an async call from the UI hops to the cooperative pool — where
    /// decoding and re-encoding a 12-megapixel photo belongs. Called
    /// synchronously it ran ON the main actor and froze the thread.
    static func preparePhoto(from data: Data, limit: Int) async throws -> Prepared {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else {
            throw PrepError.unreadable
        }
        guard let full = downsample(source: source, maxPixels: photoEdge),
              let jpeg = PlatformImage.jpegData(from: full, quality: photoQuality)
        else {
            throw PrepError.unreadable
        }
        // A downscaled photo is far under any sane ceiling, but a
        // pathological one (a huge PNG of noise) could still exceed it —
        // and the message is better refused here than by the server.
        if jpeg.count > limit {
            throw PrepError.tooLargeAfterCompression(bytes: jpeg.count)
        }

        let url = temporaryURL(extension: "jpg")
        try jpeg.write(to: url, options: .atomic)

        let preview = downsample(source: source, maxPixels: previewEdge)
            .flatMap { PlatformImage.jpegData(from: $0, quality: previewQuality) }

        return Prepared(
            fileURL: url,
            mime: "image/jpeg",
            kind: "photo",
            width: full.width,
            height: full.height,
            durationMS: nil,
            previewJPEG: preview)
    }

    /// Decode no larger than `maxPixels` on the longest edge, honouring
    /// EXIF orientation so a portrait photo does not arrive on its side.
    private static func downsample(source: CGImageSource, maxPixels: Int) -> CGImage? {
        CGImageSourceCreateThumbnailAtIndex(source, 0, [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixels,
        ] as CFDictionary)
    }

    // MARK: - Videos

    /// How a planned transcode is carried out. The app passes nothing and
    /// gets AVFoundation (`MediaTranscoder`); a test passes one that fails,
    /// or one whose result is bigger than its source — the only way to reach
    /// rules C and D on demand, since a working encoder does neither on cue.
    /// The same seam idea as `APIClient`'s injected `URLSession`.
    nonisolated struct Transcoder: Sendable {
        var video: @Sendable (_ source: URL, _ target: MediaPlan.VideoTarget, _ output: URL) async throws -> Void
        var audio: @Sendable (_ source: URL, _ bitrate: Int, _ output: URL) async throws -> Void

        static let avFoundation = Transcoder(
            video: { try await MediaTranscoder.transcodeVideo(from: $0, to: $1, writingTo: $2) },
            audio: { try await MediaTranscoder.transcodeAudio(from: $0, bitrate: $1, writingTo: $2) })
    }

    /// Bring a video to the protocol's profile — or leave it alone, or send
    /// it the way 1.1 did (docs/protocol.md, "Preparing media before upload").
    ///
    /// THIS REVERSES A DECISION, and says so. 1.1 re-encoded only a clip
    /// over the ceiling — nettrash's choice: keep what the sender shot when
    /// it fits, compress when it does not. Issue #74 reverses it, because the
    /// server keeps whatever it is given for good and so the size of a
    /// family's history is decided here: a minute of an iPhone's 1080p HEVC
    /// is about 60 MB, the same minute at the profile's 720p H.264 about 16,
    /// and HEVC in a QuickTime file does not even play in Firefox. What
    /// survives of the old choice is rule A — a clip ALREADY within the
    /// profile goes exactly as shot, because re-encoding it would only cost
    /// it quality.
    ///
    /// In the protocol's order:
    ///
    ///   * rule A — within the profile: the original, untouched;
    ///   * otherwise a transcode to the planner's target (`MediaTranscoder`);
    ///   * rule D — a result bigger than a sendable source is thrown away;
    ///   * rule C — a transcode that fails, or a source with nothing to
    ///     transcode, goes exactly as it went in 1.1 (`videoAfterFailure`).
    ///
    /// `@concurrent`, because this target builds with
    /// NonisolatedNonsendingByDefault: a plain `nonisolated async` function
    /// runs on its CALLER's actor, and every caller is a composer on the main
    /// actor. Seconds of probing, copying and waiting on an encoder belong
    /// on the cooperative pool, where the old comment on `preparePhoto`
    /// assumed every function in this type already ran.
    @concurrent
    static func prepareVideo(
        from sourceURL: URL, limit: Int, transcoder: Transcoder = .avFoundation
    ) async throws -> Prepared {
        let originalSize = fileSize(of: sourceURL)
        // The type is read off the bytes, as the server will read it. A file
        // with no `ftyp` at all (Matroska, AVI, an old QuickTime movie) is
        // not sendable as a video just as it is — but AVFoundation may still
        // read it, and then it can be MADE into one.
        let container = MediaProbe.videoContainer(of: sourceURL)
        let sourceSendable = MediaPlan.sendable(
            kind: "video", container: container ?? "", honest: container != nil,
            sizeBytes: originalSize, ceilingBytes: limit)
        let source = await MediaProbe.video(
            at: sourceURL, container: container ?? mimeType(for: sourceURL), sizeBytes: originalSize)

        switch MediaPlan.planVideo(source) {
        case .keep:
            // Rule A names no ceiling, but "untouched" is the one thing the
            // server refuses for a clip over it — so the plan cannot be
            // carried out, and that is rule C: the way 1.1 sent it.
            guard sourceSendable else {
                return try await videoAfterFailure(from: sourceURL, sourceSendable: false, limit: limit)
            }
            return await preparedVideo(at: sourceURL)

        case .fallback:
            return try await videoAfterFailure(
                from: sourceURL, sourceSendable: sourceSendable, limit: limit)

        case .transcode(let target):
            let output = temporaryURL(extension: "mp4")
            do {
                try await transcoder.video(sourceURL, target, output)
            } catch {
                try? FileManager.default.removeItem(at: output)
                // Somebody took it off the strip, or left: not a failure to
                // recover from, and nothing to send.
                if error is CancellationError || Task.isCancelled { throw CancellationError() }
                AppLog.sync.error(
                    "A video transcode failed (\(String(describing: error), privacy: .public)); sending it the way 1.1 did")
                return try await videoAfterFailure(
                    from: sourceURL, sourceSendable: sourceSendable, limit: limit)
            }
            let resultSize = fileSize(of: output)
            switch MediaPlan.keepSmaller(
                sourceBytes: originalSize, sourceSendable: sourceSendable, resultBytes: resultSize) {
            case .source:
                try? FileManager.default.removeItem(at: output)
                return await preparedVideo(at: sourceURL)
            case .result:
                // A result the server would still refuse — over the ceiling
                // for a very long clip — has not produced anything sendable,
                // which is a failure like any other.
                guard MediaPlan.sendable(
                    kind: "video", container: "video/mp4",
                    honest: Magic.honest(url: output, mime: "video/mp4"),
                    sizeBytes: resultSize, ceilingBytes: limit)
                else {
                    try? FileManager.default.removeItem(at: output)
                    return try await videoAfterFailure(
                        from: sourceURL, sourceSendable: sourceSendable, limit: limit)
                }
                return await preparedVideo(at: output)
            }
        }
    }

    /// Rule C — a plan that could not be carried out sends what 1.1 would
    /// have sent: the original when it can go as it is, and otherwise
    /// 1.1's own path for it (`videoAsBefore`).
    private static func videoAfterFailure(
        from sourceURL: URL, sourceSendable: Bool, limit: Int
    ) async throws -> Prepared {
        switch MediaPlan.onFailure(sourceSendable: sourceSendable) {
        case .original:
            return await preparedVideo(at: sourceURL)
        case .todaysPath:
            return try await videoAsBefore(from: sourceURL, limit: limit)
        }
    }

    /// EXACTLY what 1.1 did with a video, kept whole as rule C's "what would
    /// have been sent without this section": untouched when it is an honest
    /// MP4 or QuickTime file within the ceiling, sent as a FILE when it is
    /// not one of those, and over the ceiling squeezed through the 1080p
    /// export preset — refused only if even that does not fit. Preparing
    /// media is an optimisation; a clip that went up in 1.1 must still go up.
    private static func videoAsBefore(from sourceURL: URL, limit: Int) async throws -> Prepared {
        guard fileSize(of: sourceURL) > limit else {
            // UNCHANGED, which means the bytes have to be what this path is about to call them.
            // A container the server does not take as video — Matroska, AVI, WebM, all of which
            // conform to `public.movie` and so arrive here — was uploaded as `video/mp4` and
            // refused; AVFoundation cannot re-encode those either, so a FILE is what the family
            // can actually be given (see `Magic`).
            guard Magic.honest(url: sourceURL, mime: "video/mp4")
                || Magic.honest(url: sourceURL, mime: "video/quicktime")
            else {
                return try await prepareFile(from: sourceURL, name: nil, limit: limit)
            }
            return await preparedVideo(at: sourceURL)
        }
        let exported = try await export(asset: AVURLAsset(url: sourceURL))
        let compressed = fileSize(of: exported)
        if compressed > limit {
            try? FileManager.default.removeItem(at: exported)
            throw PrepError.tooLargeAfterCompression(bytes: compressed)
        }
        return await preparedVideo(at: exported)
    }

    /// The upload for a video file that has been chosen: its TURNED size,
    /// its duration and a poster frame, read off the file that will go.
    private static func preparedVideo(at uploadURL: URL) async -> Prepared {
        let asset = AVURLAsset(url: uploadURL)
        let duration = (try? await asset.load(.duration)).map { CMTimeGetSeconds($0) } ?? 0
        let track = try? await asset.loadTracks(withMediaType: .video).first
        var width: Int?
        var height: Int?
        if let track, let size = try? await track.load(.naturalSize) {
            // naturalSize ignores the track's rotation; a portrait video
            // would otherwise report itself as landscape and the bubble
            // would lay out the wrong shape. (A transcoded clip has its
            // rotation baked in and an identity transform, so this is a
            // no-op for it — and still right.)
            let transform = (try? await track.load(.preferredTransform)) ?? .identity
            let oriented = size.applying(transform)
            width = Int(abs(oriented.width))
            height = Int(abs(oriented.height))
        }

        return Prepared(
            fileURL: uploadURL,
            mime: "video/mp4",
            kind: "video",
            width: width,
            height: height,
            durationMS: Int(duration * 1000),
            previewJPEG: await posterFrame(of: asset))
    }

    private static func export(asset: AVURLAsset) async throws -> URL {
        // 1.1's re-encode, kept only as rule C's fallback: 1080p rather than
        // "highest", because the point was to fit, and a family chat on a
        // home server does not need a 4K master.
        let preset = AVAssetExportPreset1920x1080
        guard let session = AVAssetExportSession(asset: asset, presetName: preset) else {
            throw PrepError.unreadable
        }
        let url = temporaryURL(extension: "mp4")
        do {
            try await session.export(to: url, as: .mp4)
        } catch {
            // A half-written export is nobody's to upload, and nothing else
            // would ever sweep it out of tmp.
            try? FileManager.default.removeItem(at: url)
            throw error
        }
        return url
    }

    /// The first frame that is not black-ish — a video whose opening frame
    /// is a fade-in would otherwise get an empty poster.
    ///
    /// Nil is a real outcome, not just a failure to try: some codecs will
    /// not yield a still at all. It is worth SAYING so (issue #54) —
    /// without a line here, a video that never had a poster and one whose
    /// poster upload failed produce the same grey tile with nothing in the
    /// log to tell them apart, and they need different fixes. Nothing
    /// retries this: three seek points have already been tried, and a
    /// fourth pass over the same file would fail the same way.
    private static func posterFrame(of asset: AVURLAsset) async -> Data? {
        let generator = AVAssetImageGenerator(asset: asset)
        generator.appliesPreferredTrackTransform = true
        generator.maximumSize = CGSize(width: previewEdge, height: previewEdge)
        let times = [0.5, 0.0, 2.0].map { CMTime(seconds: $0, preferredTimescale: 600) }
        for time in times {
            if let image = try? await generator.image(at: time).image {
                return PlatformImage.jpegData(from: image, quality: previewQuality)
            }
        }
        AppLog.sync.error(
            "No poster frame could be read from a video after \(times.count, privacy: .public) seek points; it will be sent without one")
        return nil
    }

    // MARK: - Files

    /// Copy a picked document somewhere we own, and read its name and size.
    ///
    /// The copy is not incidental: the picker hands back a URL into
    /// another process's storage, guarded by a security scope that ends
    /// the moment this function returns — uploading straight from it would
    /// work in the simulator and fail on a device, or on iCloud Drive.
    /// Nothing is re-encoded and nothing is inspected; a file is whatever
    /// the sender picked (docs/protocol.md, "Files").
    /// `async` for the same reason as `preparePhoto`: copying a file the
    /// picker handed over can block for seconds when it is an iCloud Drive
    /// item that has to be downloaded first.
    static func prepareFile(
        from sourceURL: URL, name: String? = nil, limit: Int
    ) async throws -> Prepared {
        let scoped = sourceURL.startAccessingSecurityScopedResource()
        defer { if scoped { sourceURL.stopAccessingSecurityScopedResource() } }

        // `name` overrides the file's own, and the clipboard is why it
        // exists: a pasted item is written into a scratch file called
        // `fc-upload-<UUID>.pdf`, and that must never be what the recipient
        // sees. Cleaned either way — a name travels in a header the server
        // parses and ends up on somebody else's disk.
        let name = sanitizedName(name ?? sourceURL.lastPathComponent) ?? "file"
        let destination = temporaryURL(extension: sourceURL.pathExtension)
        do {
            try FileManager.default.copyItem(at: sourceURL, to: destination)
        } catch {
            throw PrepError.unreadable
        }

        let size = fileSize(of: destination)
        if size > limit {
            // A document cannot be made smaller the way a video can, so
            // this one really is the end of the road.
            try? FileManager.default.removeItem(at: destination)
            throw PrepError.tooLargeAfterCompression(bytes: size)
        }

        return Prepared(
            fileURL: destination,
            mime: mimeType(for: sourceURL),
            kind: "file",
            name: name)
    }

    /// Prepare a piece of audio — a voice note, or a track off a disk.
    ///
    /// A VOICE NOTE is never touched: it is recorded straight into the
    /// profile's own format (AAC-LC, mono, 64 000 bit/s, in MP4 —
    /// `AudioRecorder`), and the protocol's audio rules are for PICKED files
    /// only. The recorder's callers say which it is.
    ///
    /// A picked file follows those rules (docs/protocol.md, "Preparing media
    /// before upload"), and they REVERSE 1.1 IN PART. 1.1 never re-encoded
    /// audio, on nettrash's reasoning that re-encoding someone's music to
    /// save a few megabytes would be a worse trade than refusing it — and for
    /// music that is already compressed that still stands: MP3 and AAC at or
    /// below 192 000 bit/s go untouched, because a second lossy generation
    /// costs more than the megabytes it saves. What changed is audio with no
    /// first generation to lose: a WAV (and anything else uncompressed or
    /// lossless) becomes an M4A of AAC-LC at 128 000 bit/s, 64 000 mono,
    /// about a tenth of the size — and so does lossy audio above 192 000.
    /// Rules C and D apply as they do to a video: a failure sends the file
    /// exactly as 1.1 did, and a result bigger than its source is dropped.
    ///
    /// There is no preview: audio has nothing to look at, so a bubble draws a
    /// play control, the duration and a scrubber (docs/protocol.md, "Audio").
    /// `@concurrent` for `prepareVideo`'s reason.
    @concurrent
    static func prepareAudio(
        from sourceURL: URL,
        name: String? = nil,
        limit: Int,
        isVoiceNote: Bool = false,
        transcoder: Transcoder = .avFoundation
    ) async throws -> Prepared {
        let scoped = sourceURL.startAccessingSecurityScopedResource()
        defer { if scoped { sourceURL.stopAccessingSecurityScopedResource() } }

        if isVoiceNote {
            return try await audioAsBefore(from: sourceURL, name: name, limit: limit)
        }
        return try await preparePickedAudio(
            from: sourceURL, name: name, limit: limit, transcoder: transcoder,
            todaysPath: { try await audioAsBefore(from: sourceURL, name: name, limit: limit) })
    }

    /// Audio the server will NOT take as it is — an AIFF, a FLAC, a CAF, a
    /// raw ADTS `.aac` — which 1.1 sent as a `file`. The audio rules re-encode
    /// the uncompressed and lossless ones, and those AVFoundation can read
    /// come out as an M4A a bubble can play; everything else, and every
    /// failure, is still the file 1.1 sent.
    @concurrent
    private static func prepareUnacceptedAudio(
        from sourceURL: URL, name: String?, limit: Int, transcoder: Transcoder
    ) async throws -> Prepared {
        let scoped = sourceURL.startAccessingSecurityScopedResource()
        defer { if scoped { sourceURL.stopAccessingSecurityScopedResource() } }

        return try await preparePickedAudio(
            from: sourceURL, name: name, limit: limit, transcoder: transcoder,
            todaysPath: { try await prepareFile(from: sourceURL, name: name, limit: limit) })
    }

    /// The audio rules for a picked file. `todaysPath` is what 1.1 did with
    /// it at the door it came through — as audio, or as a file — which is
    /// rule C's fallback for anything that cannot go up as it is.
    private static func preparePickedAudio(
        from sourceURL: URL,
        name: String?,
        limit: Int,
        transcoder: Transcoder,
        todaysPath: () async throws -> Prepared
    ) async throws -> Prepared {
        let mime = audioMIME(for: sourceURL)
        let size = fileSize(of: sourceURL)
        let sourceSendable = MediaPlan.sendable(
            kind: "audio", container: mime, honest: Magic.honest(url: sourceURL, mime: mime),
            sizeBytes: size, ceilingBytes: limit)
        let source = await MediaProbe.audio(at: sourceURL, container: mime, sizeBytes: size)

        switch MediaPlan.planAudio(source) {
        case .keep:
            // Untouched — as audio when it can go that way, and otherwise the
            // way it went before (rule C).
            switch MediaPlan.onFailure(sourceSendable: sourceSendable) {
            case .original: return try await audioAsBefore(from: sourceURL, name: name, limit: limit)
            case .todaysPath: return try await todaysPath()
            }

        case .transcode(let bitrate):
            let output = temporaryURL(extension: "m4a")
            do {
                try await transcoder.audio(sourceURL, bitrate, output)
            } catch {
                try? FileManager.default.removeItem(at: output)
                if error is CancellationError || Task.isCancelled { throw CancellationError() }
                // AVFoundation has no Ogg reader, so every Ogg file lands
                // here — the protocol's "wherever the platform can decode it".
                AppLog.sync.error(
                    "An audio transcode failed (\(String(describing: error), privacy: .public)); sending it the way 1.1 did")
                switch MediaPlan.onFailure(sourceSendable: sourceSendable) {
                case .original: return try await audioAsBefore(from: sourceURL, name: name, limit: limit)
                case .todaysPath: return try await todaysPath()
                }
            }
            let resultSize = fileSize(of: output)
            let resultSendable = MediaPlan.sendable(
                kind: "audio", container: "audio/mp4",
                honest: Magic.honest(url: output, mime: "audio/mp4"),
                sizeBytes: resultSize, ceilingBytes: limit)
            let upload = MediaPlan.keepSmaller(
                sourceBytes: size, sourceSendable: sourceSendable, resultBytes: resultSize)
            guard upload == .result, resultSendable else {
                try? FileManager.default.removeItem(at: output)
                switch MediaPlan.onFailure(sourceSendable: sourceSendable) {
                case .original: return try await audioAsBefore(from: sourceURL, name: name, limit: limit)
                case .todaysPath: return try await todaysPath()
                }
            }
            // The name follows the bytes: a recipient saving "Take 3.wav"
            // would get AAC under a WAV name.
            let renamed = name.map { ($0 as NSString).deletingPathExtension + ".m4a" }
            return await preparedAudio(at: output, name: renamed)
        }
    }

    /// EXACTLY what 1.1 did with a piece of audio: copy it somewhere we own,
    /// refuse it over the ceiling, and read its duration. Nothing re-encoded.
    private static func audioAsBefore(
        from sourceURL: URL, name: String?, limit: Int
    ) async throws -> Prepared {
        let destination = temporaryURL(extension: sourceURL.pathExtension)
        do {
            try FileManager.default.copyItem(at: sourceURL, to: destination)
        } catch {
            throw PrepError.unreadable
        }

        let size = fileSize(of: destination)
        if size > limit {
            try? FileManager.default.removeItem(at: destination)
            throw PrepError.tooLargeAfterCompression(bytes: size)
        }
        return await preparedAudio(at: destination, name: name)
    }

    /// The upload for an audio file that has been chosen.
    private static func preparedAudio(at uploadURL: URL, name: String?) async -> Prepared {
        let asset = AVURLAsset(url: uploadURL)
        let duration = (try? await asset.load(.duration)).map {
            Int(CMTimeGetSeconds($0) * 1000)
        }

        // A name only when there is one worth showing: a recording's
        // identity is its length, a track's is its title. The server takes
        // `name` as optional for audio, unlike a file.
        //
        // Passed IN rather than read off the URL, and that is the fix for a
        // real one: a voice note is recorded into `fc-voice-<UUID>.m4a`, so
        // reading the file name here put a scratch file name on the wire
        // and in the composer's chip. A recorder passes nothing.
        return Prepared(
            fileURL: uploadURL,
            mime: audioMIME(for: uploadURL),
            kind: "audio",
            durationMS: duration,
            name: sanitizedName(name))
    }

    /// The type the SERVER will accept, which is narrower than what the
    /// system might name. Anything unrecognised is left to the file path,
    /// where nothing is verified.
    static func audioMIME(for url: URL) -> String {
        switch url.pathExtension.lowercased() {
        case "m4a", "mp4", "aac": "audio/mp4"
        case "mp3": "audio/mpeg"
        case "wav", "wave": "audio/wav"
        case "ogg", "oga": "audio/ogg"
        default: mimeType(for: url)
        }
    }

    /// Whether the server will take this as audio at all. When it will not,
    /// the caller sends it as a file — where the type is metadata and no
    /// magic number is checked — rather than getting a 400.
    static func isSupportedAudio(_ url: URL) -> Bool {
        guard ["m4a", "mp4", "aac", "mp3", "wav", "wave", "ogg", "oga"]
            .contains(url.pathExtension.lowercased())
        else {
            return false
        }
        // AND the bytes have to be what the extension says. A `.aac` is usually raw ADTS rather
        // than an ISO container, so the type this would claim for it (`audio/mp4`) is a 400 on
        // every one — and an `.m4a` somebody renamed is the same story one extension over. The
        // file path takes it instead, where nothing is verified and the family still gets it.
        return Magic.honest(url: url, mime: audioMIME(for: url))
    }

    /// The system's type for this extension, or the generic one. The server
    /// stores it without checking — it is metadata, not a claim.
    static func mimeType(for url: URL) -> String {
        UTType(filenameExtension: url.pathExtension)?.preferredMIMEType
            ?? "application/octet-stream"
    }

    // MARK: - Anything at all

    /// Prepare whatever is at `url`, deciding the kind from the file.
    ///
    /// The Mac has one picker rather than the phone's two, so the KIND is
    /// read from the file's type rather than from which button was
    /// pressed: an image goes through the photo path (downscaled, with a
    /// preview), a movie through the video path (brought to the profile
    /// unless it is already within it), audio through the audio rules, and
    /// everything else is a file, sent as it is.
    static func prepare(
        fileAt url: URL,
        type explicitType: UTType? = nil,
        name: String? = nil,
        limit: Int,
        transcoder: Transcoder = .avFoundation
    ) async throws -> Prepared {
        // The clipboard KNOWS the type; a picked file only has an
        // extension to go on. Trust the caller when it has something
        // better.
        let type = explicitType ?? UTType(filenameExtension: url.pathExtension)
        if type?.conforms(to: .image) == true {
            // An animated image is not a photo, whatever it conforms to.
            if sendsAsFile(imageType: type) {
                return try await prepareFile(from: url, name: name, limit: limit)
            }
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            guard let data = try? Data(contentsOf: url) else { throw PrepError.unreadable }
            return try await preparePhoto(from: data, limit: limit)
        }
        if type?.conforms(to: .movie) == true {
            return try await prepareVideo(from: url, limit: limit, transcoder: transcoder)
        }
        // Audio the server will actually accept goes as audio, so it gets a
        // player instead of a document row. Anything else claiming to be
        // audio (a type the magic-number check does not know) is re-encoded
        // to one it does when the audio rules say to and AVFoundation can —
        // an AIFF, a FLAC — and otherwise falls through to the file path
        // rather than earning a 400, exactly as it did in 1.1.
        if type?.conforms(to: .audio) == true {
            if isSupportedAudio(url) {
                return try await prepareAudio(
                    from: url, name: name ?? url.lastPathComponent, limit: limit,
                    transcoder: transcoder)
            }
            return try await prepareUnacceptedAudio(
                from: url, name: name ?? url.lastPathComponent, limit: limit,
                transcoder: transcoder)
        }
        return try await prepareFile(from: url, name: name, limit: limit)
    }

    /// Prepare bytes we already hold, deciding the kind from `type`.
    ///
    /// The clipboard hands over DATA, not a file, and everything below here
    /// works on files — so this writes one and then goes down the ordinary
    /// path. Deliberately NOT a second preparation path: a pasted photo is
    /// still downscaled by `preparePhoto`, a pasted clip still gets its
    /// poster frame from `prepareVideo`.
    static func prepare(
        data: Data, type: UTType, name: String? = nil, limit: Int
    ) async throws -> Prepared {
        // Never "dat" for something we could name: the extension is what
        // `prepareFile` turns into the MIME type the recipient's app opens.
        let scratch = temporaryURL(extension: type.preferredFilenameExtension ?? "dat")
        do {
            try data.write(to: scratch, options: .atomic)
        } catch {
            throw PrepError.unreadable
        }
        do {
            let prepared = try await prepare(
                fileAt: scratch, type: type, name: name, limit: limit)
            // Every path but one copies or re-encodes; `prepareVideo` hands
            // back the source itself when it goes as it is (rules A, C and
            // D), so only delete the scratch file when it is not the thing
            // being uploaded.
            if prepared.fileURL != scratch {
                try? FileManager.default.removeItem(at: scratch)
            }
            return prepared
        } catch {
            try? FileManager.default.removeItem(at: scratch)
            throw error
        }
    }

    /// Image types that must go as `kind=file` rather than as a photo.
    ///
    /// An ANIMATED GIF loses its animation the instant it goes through
    /// `preparePhoto`: CGImageSource hands back frame zero and JPEG has
    /// nowhere to put the rest — the picture still arrives, and it is the
    /// wrong picture. The server will not take `image/gif` as a photo
    /// either (docs/protocol.md, "Photos, videos and files"). WebP animates
    /// for the same reason, and BMP is here because neither is one of the
    /// four types the photo endpoint magic-checks. All three go as files,
    /// where the ORIGINAL BYTES travel and nothing is verified or thrown
    /// away.
    ///
    /// This is the rule the Mac's own picker was missing: dropping a GIF on
    /// `prepare(fileAt:)` quietly posted a still of it.
    static func sendsAsFile(imageType type: UTType?) -> Bool {
        guard let type else { return false }
        return [UTType.gif, .webP, .bmp].contains { type.conforms(to: $0) }
    }

    /// A name the server will take and a recipient will recognise.
    ///
    /// `kind=file` REQUIRES 1–255 characters (docs/protocol.md, "Photos,
    /// videos and files"), and the name also lands on somebody else's disk,
    /// so it is cleaned on the way out the way an incoming one is cleaned
    /// on the way in: no path separators, no control characters, nothing
    /// that is only whitespace. Nil in, nil out — audio is allowed to have
    /// no name at all.
    static func sanitizedName(_ raw: String?) -> String? {
        guard let raw else { return nil }
        let stripped = String(raw.map { (character: Character) -> Character in
            if character == "/" || character == ":" { return "_" }
            if character.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) {
                return "_"
            }
            return character
        })
        let name = stripped.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { return nil }
        guard name.count > maxNameLength else { return name }
        // Truncate the STEM, so ".pdf" survives and the recipient's system
        // still knows what it is holding.
        let url = URL(fileURLWithPath: name)
        let ext = url.pathExtension
        let stem = url.deletingPathExtension().lastPathComponent
        guard !ext.isEmpty, ext.count + 1 < maxNameLength else {
            return String(name.prefix(maxNameLength))
        }
        return String(stem.prefix(maxNameLength - ext.count - 1)) + "." + ext
    }

    /// The protocol's ceiling for an attachment name.
    static let maxNameLength = 255

    // MARK: - Shared

    static func fileSize(of url: URL) -> Int {
        // One cast, not two: `[.size]` is `Any?`, so the inner `as? Int`
        // already yields `Int?` and `try?` flattens rather than nesting it.
        // The second `as? Int` was therefore casting `Int?` to `Int?` — a
        // no-op that read as if it were doing the unwrapping the `?? 0` does.
        (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? Int) ?? 0
    }

    static func temporaryURL(extension ext: String) -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("fc-upload-\(UUID().uuidString)")
            .appendingPathExtension(ext)
    }
}

/// A video off the picker, as a file we own.
///
/// PhotosPicker hands a movie over as a file whose lifetime ends when the
/// import closure returns, so it is copied out rather than referenced. The
/// copy is the caller's to delete — `MediaPrep.prepareVideo` may return it
/// unchanged as the upload file when the original goes as it is.
nonisolated struct PickedMovie: Transferable {
    let url: URL

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(contentType: .movie) { movie in
            SentTransferredFile(movie.url)
        } importing: { received in
            let ext = received.file.pathExtension.isEmpty ? "mov" : received.file.pathExtension
            let copy = MediaPrep.temporaryURL(extension: ext)
            try FileManager.default.copyItem(at: received.file, to: copy)
            return PickedMovie(url: copy)
        }
    }
}
