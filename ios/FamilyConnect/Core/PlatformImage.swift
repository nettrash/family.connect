//
//  PlatformImage.swift
//  FamilyConnect
//
//  The one place the app touches an image framework.
//
//  Everything above this file works in CGImage and Data, which are the
//  same on every Apple platform — so the Core layer (stores, loaders,
//  media preparation) compiles for iOS and macOS with no conditionals at
//  all. Only this file knows there is a difference, and mostly there is
//  not: ImageIO decodes and encodes on both, and SwiftUI's `Image` takes a
//  CGImage directly.
//
//  Written when the macOS target arrived. The alternative — a UIImage /
//  NSImage typealias — spreads two APIs that only LOOK alike through every
//  call site: NSImage carries representations rather than pixels, its size
//  is in points not pixels, and `jpegData(compressionQuality:)` does not
//  exist on it. Going through CGImage avoids all of that.
//

import CoreGraphics
import Foundation
import ImageIO
import SwiftUI
import UniformTypeIdentifiers

nonisolated enum PlatformImage {

    /// Decode, never larger than `maxPixels` on the longest edge.
    ///
    /// Bounded on purpose: the bytes can come from a link preview on
    /// somebody else's website, and a 20-megapixel PNG of noise should
    /// cost a thumbnail's memory, not a full decode. EXIF orientation is
    /// applied, so a portrait photo does not arrive on its side.
    static func decode(_ data: Data, maxPixels: Int) -> CGImage? {
        guard !data.isEmpty,
              let source = CGImageSourceCreateWithData(data as CFData, nil)
        else { return nil }
        return CGImageSourceCreateThumbnailAtIndex(source, 0, [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixels,
        ] as CFDictionary)
    }

    /// JPEG-encode, ignoring any alpha channel.
    ///
    /// JPEG cannot store alpha, and ImageIO complains (rightly) when asked
    /// to encode an image that has one — the channel is dead weight that
    /// doubles the bitmap while it is being written. Drawing into an
    /// opaque context first is what avoids that; white, not black, behind
    /// anything actually transparent, so a cut-out PNG reads as a picture
    /// on paper rather than a hole.
    static func jpegData(from image: CGImage, quality: CGFloat) -> Data? {
        guard let opaque = flattened(image) else { return nil }
        let output = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(
            output, UTType.jpeg.identifier as CFString, 1, nil)
        else { return nil }
        CGImageDestinationAddImage(destination, opaque, [
            kCGImageDestinationLossyCompressionQuality: quality,
        ] as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { return nil }
        return output as Data
    }

    /// Redraw onto opaque white. Returns the original when it already has
    /// no alpha — the common case, and worth not copying for.
    private static func flattened(_ image: CGImage) -> CGImage? {
        switch image.alphaInfo {
        case .none, .noneSkipFirst, .noneSkipLast:
            return image
        default:
            break
        }
        guard let context = CGContext(
            data: nil,
            width: image.width,
            height: image.height,
            bitsPerComponent: 8,
            bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)
        else { return nil }
        let rect = CGRect(x: 0, y: 0, width: image.width, height: image.height)
        context.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
        context.fill(rect)
        context.draw(image, in: rect)
        return context.makeImage()
    }

    /// Scale so the longest edge is at most `maxEdge`; the original when it
    /// already fits.
    static func scaled(_ image: CGImage, maxEdge: Int) -> CGImage? {
        let longest = max(image.width, image.height)
        guard longest > maxEdge, longest > 0 else { return image }
        let factor = CGFloat(maxEdge) / CGFloat(longest)
        let width = max(Int(CGFloat(image.width) * factor), 1)
        let height = max(Int(CGFloat(image.height) * factor), 1)
        guard let context = CGContext(
            data: nil,
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return nil }
        context.interpolationQuality = .high
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        return context.makeImage()
    }

    /// The largest centred square — what a circular avatar shows anyway.
    static func centreCroppedSquare(_ image: CGImage) -> CGImage? {
        let side = min(image.width, image.height)
        if image.width == image.height { return image }
        let x = (image.width - side) / 2
        let y = (image.height - side) / 2
        return image.cropping(to: CGRect(x: x, y: y, width: side, height: side))
    }

    /// A SwiftUI image. `Image(_ cgImage:)` exists on every platform, which
    /// is the whole reason the layers above deal in CGImage.
    static func view(_ image: CGImage, label: Text = Text(verbatim: "")) -> Image {
        Image(image, scale: 1, label: label)
    }
}
