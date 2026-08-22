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
import UIKit

nonisolated enum AvatarImage {

    /// Longest edge of the uploaded square, in pixels. 512 is sharp on
    /// every avatar this app draws (the largest is 56 pt) and encodes to
    /// well under the server's limit.
    static let edge = 512

    /// JPEG quality. 0.8 is the usual sweet spot: no visible artefacts at
    /// avatar size, a fraction of the bytes of 1.0.
    static let quality: CGFloat = 0.8

    /// Square JPEG for upload, or nil when the data is not a decodable
    /// image.
    static func squareJPEG(from data: Data, edge: Int = AvatarImage.edge) -> Data? {
        guard let downsampled = downsample(data, maxPixels: edge * 2) else { return nil }
        let squared = centreCropSquare(downsampled)
        let resized = resize(squared, edge: CGFloat(edge))
        return resized.jpegData(compressionQuality: quality)
    }

    /// Decode no larger than `maxPixels` on the longest edge.
    private static func downsample(_ data: Data, maxPixels: Int) -> UIImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, [
            kCGImageSourceShouldCache: false,
        ] as CFDictionary) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            // Honours EXIF orientation, so a portrait photo does not
            // arrive on its side.
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixels,
        ]
        guard let cgImage = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else {
            return nil
        }
        return UIImage(cgImage: cgImage)
    }

    /// The largest centred square of an image — what a circular avatar
    /// shows anyway, so cropping here means the bytes are not spent on
    /// pixels nobody will see.
    private static func centreCropSquare(_ image: UIImage) -> UIImage {
        guard let cgImage = image.cgImage else { return image }
        let width = CGFloat(cgImage.width)
        let height = CGFloat(cgImage.height)
        let side = min(width, height)
        guard width != height else { return image }
        let rect = CGRect(
            x: ((width - side) / 2).rounded(.down),
            y: ((height - side) / 2).rounded(.down),
            width: side,
            height: side)
        guard let cropped = cgImage.cropping(to: rect) else { return image }
        return UIImage(cgImage: cropped)
    }

    private static func resize(_ image: UIImage, edge: CGFloat) -> UIImage {
        let size = CGSize(width: edge, height: edge)
        guard image.size.width > edge || image.size.height > edge else { return image }
        let format = UIGraphicsImageRendererFormat.default()
        // Points, not screen pixels: the destination is a byte payload,
        // not a view, so a 3x renderer would silently triple the upload.
        format.scale = 1
        return UIGraphicsImageRenderer(size: size, format: format).image { _ in
            image.draw(in: CGRect(origin: .zero, size: size))
        }
    }
}
