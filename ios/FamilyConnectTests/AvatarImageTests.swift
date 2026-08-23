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
import CoreGraphics
@testable import FamilyConnect

@MainActor
struct AvatarImageTests {

    @Test("a photo is squared, downscaled and kept under the budget")
    func detailedPhotoFitsTheBudget() throws {
        let source = TestImages.photograph(width: 1600, height: 1200)
        let jpeg = try #require(AvatarImage.squareJPEG(from: source))

        #expect(jpeg.count <= AvatarImage.maxBytes)
        // Still a JPEG…
        #expect(Array(jpeg.prefix(3)) == [0xFF, 0xD8, 0xFF])
        // …and still a 512 square.
        let decoded = try #require(TestImages.size(of: jpeg))
        #expect(decoded.width == AvatarImage.edge)
        #expect(decoded.height == AvatarImage.edge)
    }

    @Test("a portrait photo crops to a square rather than squashing")
    func portraitCrops() throws {
        let jpeg = try #require(AvatarImage.squareJPEG(from: TestImages.photograph(width: 900, height: 1600)))
        let decoded = try #require(TestImages.size(of: jpeg))
        #expect(decoded.width == decoded.height)
    }

    @Test("an image smaller than the target is not blown up")
    func smallImageStaysSmall() throws {
        let jpeg = try #require(AvatarImage.squareJPEG(from: TestImages.noise(width: 120, height: 160)))
        let decoded = try #require(TestImages.size(of: jpeg))
        #expect(decoded.width == 120)
        #expect(decoded.height == 120)
    }

    /// Pure per-pixel noise is worse than any photograph and does not fit
    /// at the bottom of the ladder. The documented behaviour is to send
    /// the smallest attempt anyway rather than refuse to set a picture —
    /// the server's own ceiling is far higher than the budget.
    @Test("an incompressible image still produces an upload, at the bottom of the ladder")
    func pathologicalImageStillEncodes() throws {
        let source = TestImages.noise(width: 1600, height: 1200, detail: 4)
        let jpeg = try #require(AvatarImage.squareJPEG(from: source))
        #expect(jpeg.count > 0)
        #expect(Array(jpeg.prefix(3)) == [0xFF, 0xD8, 0xFF])

        // The ladder has to have actually stepped down: the result must
        // beat what the top quality alone would have produced. Without
        // this the loop could return its first attempt and the test would
        // still pass on size alone.
        let square = try #require(PlatformImage.decode(jpeg, maxPixels: AvatarImage.edge))
        let topQuality = try #require(
            PlatformImage.jpegData(from: square, quality: AvatarImage.qualitySteps[0]))
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
