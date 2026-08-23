//
//  MediaPrep.swift
//  FamilyConnect
//
//  Turning what the picker hands over into something worth uploading: a
//  downscaled JPEG for a photo, a re-encoded MP4 for a video that would
//  otherwise be too big, and in both cases a small preview the bubble can
//  draw before the full file has been fetched. A FILE is not touched at
//  all — it is copied where we own it and sent as it is.
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

nonisolated enum MediaPrep {

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

    /// Re-encode only when the original is over the ceiling — nettrash's
    /// choice: keep what the sender shot when it fits, compress when it
    /// does not, and refuse only if compression was not enough.
    static func prepareVideo(from sourceURL: URL, limit: Int) async throws -> Prepared {
        let asset = AVURLAsset(url: sourceURL)
        let originalSize = fileSize(of: sourceURL)

        let uploadURL: URL
        if originalSize <= limit {
            uploadURL = sourceURL
        } else {
            uploadURL = try await export(asset: asset)
            let compressed = fileSize(of: uploadURL)
            if compressed > limit {
                try? FileManager.default.removeItem(at: uploadURL)
                throw PrepError.tooLargeAfterCompression(bytes: compressed)
            }
        }

        let exported = AVURLAsset(url: uploadURL)
        let duration = (try? await exported.load(.duration)).map { CMTimeGetSeconds($0) } ?? 0
        let track = try? await exported.loadTracks(withMediaType: .video).first
        var width: Int?
        var height: Int?
        if let track, let size = try? await track.load(.naturalSize) {
            // naturalSize ignores the track's rotation; a portrait video
            // would otherwise report itself as landscape and the bubble
            // would lay out the wrong shape.
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
            previewJPEG: await posterFrame(of: exported))
    }

    private static func export(asset: AVURLAsset) async throws -> URL {
        // 1080p rather than "highest": the point is to fit, and a family
        // chat on a home server does not need a 4K master.
        let preset = AVAssetExportPreset1920x1080
        guard let session = AVAssetExportSession(asset: asset, presetName: preset) else {
            throw PrepError.unreadable
        }
        let url = temporaryURL(extension: "mp4")
        try await session.export(to: url, as: .mp4)
        return url
    }

    /// The first frame that is not black-ish — a video whose opening frame
    /// is a fade-in would otherwise get an empty poster.
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
    static func prepareFile(from sourceURL: URL, limit: Int) async throws -> Prepared {
        let scoped = sourceURL.startAccessingSecurityScopedResource()
        defer { if scoped { sourceURL.stopAccessingSecurityScopedResource() } }

        let name = sourceURL.lastPathComponent
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
            name: name.isEmpty ? "file" : name)
    }

    /// Prepare a piece of audio — a recording, or a track off a disk.
    ///
    /// Nothing is re-encoded. A voice note is already recorded straight
    /// into the container the server checks (AAC in MP4), and re-encoding
    /// someone's music to save a few megabytes would be a worse trade than
    /// refusing it. There is no preview: audio has nothing to look at, so a
    /// bubble draws a play control, the duration and a scrubber
    /// (docs/protocol.md, "Audio").
    static func prepareAudio(from sourceURL: URL, limit: Int) async throws -> Prepared {
        let scoped = sourceURL.startAccessingSecurityScopedResource()
        defer { if scoped { sourceURL.stopAccessingSecurityScopedResource() } }

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

        let asset = AVURLAsset(url: destination)
        let duration = (try? await asset.load(.duration)).map {
            Int(CMTimeGetSeconds($0) * 1000)
        }

        // A name only when there is one worth showing: a recording's
        // identity is its length, a track's is its title. The server takes
        // `name` as optional for audio, unlike a file.
        let name = sourceURL.lastPathComponent
        return Prepared(
            fileURL: destination,
            mime: audioMIME(for: destination),
            kind: "audio",
            durationMS: duration,
            name: name.isEmpty ? nil : name)
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
        ["m4a", "mp4", "aac", "mp3", "wav", "wave", "ogg", "oga"]
            .contains(url.pathExtension.lowercased())
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
    /// preview), a movie through the video path (re-encoded only if it has
    /// to be), and everything else is a file, sent as it is.
    static func prepare(fileAt url: URL, limit: Int) async throws -> Prepared {
        let type = UTType(filenameExtension: url.pathExtension)
        if type?.conforms(to: .image) == true {
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            guard let data = try? Data(contentsOf: url) else { throw PrepError.unreadable }
            return try await preparePhoto(from: data, limit: limit)
        }
        if type?.conforms(to: .movie) == true {
            return try await prepareVideo(from: url, limit: limit)
        }
        // Audio the server will actually accept goes as audio, so it gets a
        // player instead of a document row. Anything else claiming to be
        // audio (a codec the magic-number check does not know) falls
        // through to the file path rather than earning a 400.
        if type?.conforms(to: .audio) == true, isSupportedAudio(url) {
            return try await prepareAudio(from: url, limit: limit)
        }
        return try await prepareFile(from: url, limit: limit)
    }

    // MARK: - Shared

    static func fileSize(of url: URL) -> Int {
        (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? Int) as? Int ?? 0
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
/// unchanged as the upload file when the original already fits.
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
