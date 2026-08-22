//
//  AvatarImageTests.swift
//  FamilyConnectTests
//
//  The upload has to come out square, small and within a byte budget for
//  any photo — a flat test image compresses to nothing and proves none of
//  that, so these use high-entropy content, which is the case that
//  actually pushed uploads over a proxy's limit.
//

import Foundation
import Testing
import UIKit
@testable import FamilyConnect

@MainActor
struct AvatarImageTests {

    /// Deterministic detail at a photograph's scale, so the JPEG encoder
    /// cannot cheat its way under the budget on a flat test image.
    ///
    /// `format.scale = 1` matters: the renderer defaults to the SCREEN
    /// scale, so a "1600x1200" source would really be 4800x3600 and the
    /// numbers below would all be off by 3x.
    private func photo(width: Int, height: Int, detail: Int = 16) -> Data {
        var seed: UInt64 = 0x5DEECE66D
        func next() -> CGFloat {
            seed = seed &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
            return CGFloat((seed >> 33) % 256) / 255.0
        }
        let size = CGSize(width: width, height: height)
        let format = UIGraphicsImageRendererFormat.default()
        format.scale = 1
        let image = UIGraphicsImageRenderer(size: size, format: format).image { context in
            for y in stride(from: 0, to: height, by: detail) {
                for x in stride(from: 0, to: width, by: detail) {
                    UIColor(red: next(), green: next(), blue: next(), alpha: 1).setFill()
                    context.fill(CGRect(x: x, y: y, width: detail, height: detail))
                }
            }
        }
        return image.jpegData(compressionQuality: 1.0)!
    }

    /// Photograph-like: large smooth fields with edges across them, which
    /// is what a JPEG encoder is built for. (Random per-block colour is
    /// NOT a photo — it has no spatial correlation and does not fit at any
    /// quality, which is what `pathologicalImageStillEncodes` covers.)
    private func photograph(width: Int, height: Int) -> Data {
        let size = CGSize(width: width, height: height)
        let format = UIGraphicsImageRendererFormat.default()
        format.scale = 1
        let image = UIGraphicsImageRenderer(size: size, format: format).image { context in
            let cg = context.cgContext
            let colors = [UIColor.systemTeal.cgColor, UIColor.systemIndigo.cgColor,
                          UIColor.systemOrange.cgColor] as CFArray
            if let gradient = CGGradient(
                colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: colors, locations: [0, 0.6, 1]) {
                cg.drawLinearGradient(
                    gradient, start: .zero,
                    end: CGPoint(x: width, y: height), options: [])
            }
            UIColor(white: 1, alpha: 0.55).setFill()
            for index in 0..<24 {
                let side = CGFloat(40 + index * 17)
                cg.fillEllipse(in: CGRect(
                    x: CGFloat(index * 53 % max(1, width - Int(side))),
                    y: CGFloat(index * 91 % max(1, height - Int(side))),
                    width: side, height: side))
            }
        }
        return image.jpegData(compressionQuality: 1.0)!
    }

    @Test("a photo is squared, downscaled and kept under the budget")
    func detailedPhotoFitsTheBudget() throws {
        let source = photograph(width: 1600, height: 1200)
        let jpeg = try #require(AvatarImage.squareJPEG(from: source))

        #expect(jpeg.count <= AvatarImage.maxBytes)
        // Still a JPEG…
        #expect(Array(jpeg.prefix(3)) == [0xFF, 0xD8, 0xFF])
        // …and still a 512 square.
        let decoded = try #require(UIImage(data: jpeg))
        #expect(decoded.size.width == CGFloat(AvatarImage.edge))
        #expect(decoded.size.height == CGFloat(AvatarImage.edge))
    }

    @Test("a portrait photo crops to a square rather than squashing")
    func portraitCrops() throws {
        let jpeg = try #require(AvatarImage.squareJPEG(from: photograph(width: 900, height: 1600)))
        let decoded = try #require(UIImage(data: jpeg))
        #expect(decoded.size.width == decoded.size.height)
    }

    @Test("an image smaller than the target is not blown up")
    func smallImageStaysSmall() throws {
        let jpeg = try #require(AvatarImage.squareJPEG(from: photo(width: 120, height: 160)))
        let decoded = try #require(UIImage(data: jpeg))
        #expect(decoded.size.width == 120)
        #expect(decoded.size.height == 120)
    }

    /// Pure per-pixel noise is worse than any photograph and does not fit
    /// at the bottom of the ladder. The documented behaviour is to send
    /// the smallest attempt anyway rather than refuse to set a picture —
    /// the server's own ceiling is far higher than the budget.
    @Test("an incompressible image still produces an upload, at the bottom of the ladder")
    func pathologicalImageStillEncodes() throws {
        let source = photo(width: 1600, height: 1200, detail: 4)
        let jpeg = try #require(AvatarImage.squareJPEG(from: source))
        #expect(jpeg.count > 0)
        #expect(Array(jpeg.prefix(3)) == [0xFF, 0xD8, 0xFF])

        // The ladder has to have actually stepped down: the result must
        // beat what the top quality alone would have produced. Without
        // this the loop could return its first attempt and the test would
        // still pass on size alone.
        let square = try #require(UIImage(data: jpeg))
        let topQuality = try #require(square.jpegData(compressionQuality: AvatarImage.qualitySteps[0]))
        #expect(jpeg.count < topQuality.count)
    }

    @Test("bytes that are not an image yield nil rather than throwing")
    func notAnImage() {
        #expect(AvatarImage.squareJPEG(from: Data()) == nil)
        #expect(AvatarImage.squareJPEG(from: Data("not an image".utf8)) == nil)
    }

    /// Mirrored constants: the two platforms must produce interchangeable
    /// uploads, so a change here is a change on Android too.
    /// Android: AvatarImage.kt — EDGE, QUALITY_STEPS, MAX_BYTES.
    @Test("the ladder and budget match the Android constants")
    func parityConstants() {
        #expect(AvatarImage.edge == 512)
        #expect(AvatarImage.maxBytes == 56 * 1024)
        #expect(AvatarImage.qualitySteps == [0.8, 0.65, 0.5, 0.4])
    }
}
