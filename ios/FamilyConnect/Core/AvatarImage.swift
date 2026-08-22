//
//  AvatarImage.swift
//  FamilyConnect
//
//  Turning whatever the photo library hands over into something worth
//  uploading: a centre-cropped square JPEG with its longest edge at
//  `edge` pixels.
//
//  This is the client's job, not the server's. The server stores bytes
//  and never transcodes (see handlers_avatar.rs), and its 256 KiB
//  ceiling is a backstop rather than a resizer — a modern phone photo is
//  several megabytes and would simply be refused.
//
//  Decoding goes through ImageIO's thumbnail path so a 12-megapixel
//  original never becomes a 48 MB bitmap on the way to a 512 px square.
//
//  Android counterpart: AvatarImage.kt (BitmapFactory inSampleSize).
//

import ImageIO
import CoreGraphics
import Foundation

nonisolated enum AvatarImage {

    /// Longest edge of the uploaded square, in pixels. 512 is sharp on
    /// every avatar this app draws (the largest is 56 pt) and encodes to
    /// well under the server's limit.
    static let edge = 512

    /// Quality ladder. 0.8 is the usual sweet spot — no visible artefacts
    /// at avatar size — but a detailed photograph can still exceed the
    /// byte budget there, so encoding steps down until it fits.
    static let qualitySteps: [CGFloat] = [0.8, 0.65, 0.5, 0.4]

    /// Byte budget for the upload. The server's own ceiling is 256 KiB,
    /// but a self-hosted family server sits behind nginx, whose stock
    /// `client_max_body_size` is small (this project's own config sets
    /// 64k globally, with a larger allowance only for the avatar route) —
    /// and a proxy rejects an oversize body with a bare 413 that carries
    /// none of the protocol's explanation. Staying well under that means
    /// a picture uploads whatever is in front of the server.
    ///
    /// It also removes a parity trap: the two platforms' JPEG encoders
    /// disagree by tens of kilobytes at the same nominal quality, so a
    /// fixed quality had the same photo landing under the limit on one
    /// platform and over it on the other.
    static let maxBytes = 56 * 1024

    /// Square JPEG for upload, or nil when the data is not a decodable
    /// image.
    static func squareJPEG(from data: Data, edge: Int = AvatarImage.edge) -> Data? {
        // Decode at ~2x the target so the downscale still has detail to
        // work with, then crop, then resize.
        guard let decoded = PlatformImage.decode(data, maxPixels: edge * 2),
              let squared = PlatformImage.centreCroppedSquare(decoded),
              let resized = PlatformImage.scaled(squared, maxEdge: edge)
        else { return nil }
        return encode(resized)
    }

    /// First quality whose output fits the budget; the last attempt when
    /// none do (better a large upload the server may still accept than
    /// refusing to set a picture at all).
    private static func encode(_ image: CGImage) -> Data? {
        var smallest: Data?
        for quality in qualitySteps {
            guard let data = PlatformImage.jpegData(from: image, quality: quality) else {
                continue
            }
            if data.count <= maxBytes { return data }
            smallest = data
        }
        return smallest
    }
}
