//
//  StickerFixtures.swift
//  FamilyConnectTests
//
//  Real sticker bytes for the sticker-pack tests.
//
//  The WebP ones are FIXTURES rather than something drawn here, for the
//  reason a PNG is accepted as a sticker at all: an Apple device decodes a
//  WebP and cannot write one (docs/protocol.md, "What a sticker is made
//  of"). Both were made with Pillow — 8 × 8, lossless, the left half
//  opaque and the right half transparent — and the animated one is two
//  40 ms frames, red then blue.
//
//  The PNGs are drawn, because those this platform can write.
//

import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers
@testable import FamilyConnect

enum StickerFixtures {

    /// A still 8 × 8 WebP with transparency — 38 bytes.
    static let stillWebP = Data(base64Encoded:
        "UklGRh4AAABXRUJQVlA4TBEAAAAvB8ABEA8Q8x/zH4w+RPQ/AAA=")!

    /// An ANIMATED 8 × 8 WebP, two frames — 140 bytes.
    static let animatedWebP = Data(base64Encoded:
        "UklGRoQAAABXRUJQVlA4WAoAAAACAAAABwAABwAAQU5JTQYAAAAAAAAAAABBTk1GKAAAAAAAAAAAAAMAAAcAACgAAAJWUDhMDwAAAC8DwAEABxD9j/4HIqL/AQBBTk1GKAAAAAAAAAAAAAMAAAcAACgAAABWUDhMDwAAAC8DwAEABxDR//4HIqL/AQA=")!

    /// A PNG whose left half is opaque red and whose right half is fully
    /// transparent — the cut-out shape a sticker has.
    static func png(width: Int, height: Int) -> Data {
        guard let context = CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return Data() }
        context.clear(CGRect(x: 0, y: 0, width: width, height: height))
        context.setFillColor(red: 1, green: 0, blue: 0, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: width / 2, height: height))
        guard let image = context.makeImage() else { return Data() }
        return encode(image, as: .png)
    }

    /// Noise as a PNG: incompressible, so its byte count is predictably
    /// large — what "over the ceiling" needs.
    static func noisyPNG(width: Int, height: Int) -> Data {
        var seed: UInt64 = 0x9E3779B97F4A7C15
        var pixels = [UInt8](repeating: 255, count: width * height * 4)
        for index in stride(from: 0, to: pixels.count, by: 4) {
            seed = seed &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
            pixels[index] = UInt8(truncatingIfNeeded: seed >> 33)
            pixels[index + 1] = UInt8(truncatingIfNeeded: seed >> 41)
            pixels[index + 2] = UInt8(truncatingIfNeeded: seed >> 49)
        }
        guard let provider = CGDataProvider(data: Data(pixels) as CFData),
              let image = CGImage(
                  width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32,
                  bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
                  bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue),
                  provider: provider, decode: nil, shouldInterpolate: false,
                  intent: .defaultIntent)
        else { return Data() }
        return encode(image, as: .png)
    }

    /// An ANIMATED GIF, two 40 ms frames, red then blue — a picture that
    /// moves and is not a WebP. Written here: a GIF this platform can write.
    static let animatedGIF = animation(
        as: .gif,
        frame: [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFDelayTime: 0.04]],
        file: [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFLoopCount: 0]])

    /// An ANIMATED PNG, the same two frames. Its first eight bytes are a
    /// PNG's, so by its magic number alone it is a sticker type.
    static let animatedPNG = animation(
        as: .png,
        frame: [kCGImagePropertyPNGDictionary: [kCGImagePropertyAPNGDelayTime: 0.04]],
        file: [kCGImagePropertyPNGDictionary: [kCGImagePropertyAPNGLoopCount: 0]])

    /// A TIFF of two PAGES: several images in one file, and nothing moves.
    static let twoPageTIFF = animation(as: .tiff, frame: [:], file: [:])

    private static func animation(
        as type: UTType, frame: [CFString: Any], file: [CFString: Any]
    ) -> Data {
        let output = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(
            output, type.identifier as CFString, 2, nil)
        else { return Data() }
        CGImageDestinationSetProperties(destination, file as CFDictionary)
        for red in [CGFloat(1), 0] {
            guard let context = CGContext(
                data: nil, width: 8, height: 8, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            else { return Data() }
            context.setFillColor(red: red, green: 0, blue: 1 - red, alpha: 1)
            context.fill(CGRect(x: 0, y: 0, width: 8, height: 8))
            guard let image = context.makeImage() else { return Data() }
            CGImageDestinationAddImage(destination, image, frame as CFDictionary)
        }
        guard CGImageDestinationFinalize(destination) else { return Data() }
        return output as Data
    }

    private static func encode(_ image: CGImage, as type: UTType) -> Data {
        let output = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(
            output, type.identifier as CFString, 1, nil)
        else { return Data() }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else { return Data() }
        return output as Data
    }

    /// The alpha of one pixel of an encoded picture, 0…255 — how a test
    /// tells a cut-out from a picture flattened onto white.
    static func alpha(of data: Data, x: Int, y: Int) -> UInt8? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let image = CGImageSourceCreateImageAtIndex(source, 0, nil),
              let context = CGContext(
                  data: nil, width: image.width, height: image.height,
                  bitsPerComponent: 8, bytesPerRow: image.width * 4,
                  space: CGColorSpaceCreateDeviceRGB(),
                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue),
              x < image.width, y < image.height
        else { return nil }
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        guard let base = context.data else { return nil }
        // CGContext's origin is bottom-left; row 0 of the buffer is the top.
        let offset = (y * image.width + x) * 4 + 3
        return base.load(fromByteOffset: offset, as: UInt8.self)
    }
}
