//
//  TestImages.swift
//  FamilyConnectTests
//
//  Building test images without UIKit, so the suite runs on macOS as well
//  as iOS.
//
//  These used to be UIGraphicsImageRenderer, which pinned the whole bundle
//  to iOS — and the Mac app then had no automated coverage at all despite
//  sharing the entire Core. CGContext draws the same pixels on both
//  platforms, and PlatformImage (which the app itself now uses) encodes
//  them.
//
//  The `scale = 1` note that used to live here still applies in spirit:
//  a CGContext is in PIXELS, which is what these tests want — a renderer
//  defaulting to screen scale would have made a "1600x1200" source really
//  4800x3600 and every number below wrong by 3x.
//

import CoreGraphics
import Foundation
@testable import FamilyConnect

enum TestImages {

    /// A bitmap context in pixels, opaque, ready to draw into.
    private static func context(width: Int, height: Int) -> CGContext? {
        CGContext(
            data: nil,
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)
    }

    /// Random blocks: deliberately NOT photograph-like. It has no spatial
    /// correlation, so JPEG cannot fit it at any quality — which is what
    /// the "pathological image still encodes" case needs.
    static func noise(width: Int, height: Int, detail: Int = 16) -> Data {
        guard let cg = context(width: width, height: height) else { return Data() }
        var seed: UInt64 = 0x5DEECE66D
        func next() -> CGFloat {
            seed = seed &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
            return CGFloat((seed >> 33) % 256) / 255.0
        }
        for y in stride(from: 0, to: height, by: detail) {
            for x in stride(from: 0, to: width, by: detail) {
                cg.setFillColor(red: next(), green: next(), blue: next(), alpha: 1)
                cg.fill(CGRect(x: x, y: y, width: detail, height: detail))
            }
        }
        guard let image = cg.makeImage() else { return Data() }
        return PlatformImage.jpegData(from: image, quality: 1.0) ?? Data()
    }

    /// Photograph-like: smooth gradient fields with edges across them,
    /// which is what a JPEG encoder is built for.
    static func photograph(width: Int, height: Int) -> Data {
        guard let cg = context(width: width, height: height) else { return Data() }
        let colors = [
            CGColor(red: 0.19, green: 0.69, blue: 0.78, alpha: 1),
            CGColor(red: 0.35, green: 0.34, blue: 0.84, alpha: 1),
            CGColor(red: 1.00, green: 0.58, blue: 0.00, alpha: 1),
        ] as CFArray
        if let gradient = CGGradient(
            colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: colors, locations: [0, 0.6, 1]) {
            cg.drawLinearGradient(
                gradient,
                start: .zero,
                end: CGPoint(x: width, y: height),
                options: [])
        }
        cg.setFillColor(red: 1, green: 1, blue: 1, alpha: 0.55)
        for index in 0..<24 {
            let side = CGFloat(40 + index * 17)
            cg.fillEllipse(in: CGRect(
                x: CGFloat(index * 53 % max(1, width - Int(side))),
                y: CGFloat(index * 91 % max(1, height - Int(side))),
                width: side, height: side))
        }
        guard let image = cg.makeImage() else { return Data() }
        return PlatformImage.jpegData(from: image, quality: 1.0) ?? Data()
    }

    /// A flat colour with one contrasting band — compresses well, so
    /// "the downscaled file is smaller" is a real assertion rather than
    /// luck.
    static func solid(width: Int, height: Int) -> Data {
        guard let cg = context(width: width, height: height) else { return Data() }
        cg.setFillColor(red: 0.19, green: 0.69, blue: 0.78, alpha: 1)
        cg.fill(CGRect(x: 0, y: 0, width: width, height: height))
        cg.setFillColor(red: 1.00, green: 0.58, blue: 0.00, alpha: 1)
        cg.fill(CGRect(x: 0, y: 0, width: width / 3, height: height))
        guard let image = cg.makeImage() else { return Data() }
        return PlatformImage.jpegData(from: image, quality: 1.0) ?? Data()
    }

    /// Pixel dimensions of encoded bytes, for the assertions that care
    /// about shape rather than content.
    static func size(of data: Data) -> (width: Int, height: Int)? {
        guard let image = PlatformImage.decode(data, maxPixels: 100_000) else { return nil }
        return (image.width, image.height)
    }
}
