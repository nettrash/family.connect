#!/usr/bin/env swift
//
//  make-macos-appicon.swift
//  FamilyConnect
//
//  Generates the macOS half of Assets.xcassets/AppIcon.appiconset from the
//  1024×1024 iOS icon.
//
//  WHY THIS EXISTS. The appiconset carried a single image tagged
//  `"platform" : "ios"`, so a macOS build compiled NO app icon at all — no
//  AppIcon.icns in the bundle, no CFBundleIconName in Info.plist — and App
//  Store Connect rejected it with "does not have an icon in ICNS format
//  containing a 512pt x 512pt @2x image". macOS wants the whole ladder
//  (16/32/128/256/512 pt at 1x and 2x), not one 1024 image.
//
//  WHY THE ARTWORK IS RESHAPED. macOS does not mask app icons the way iOS
//  does: whatever is supplied is drawn as-is, so the full-bleed square that is
//  correct on iOS would sit in the Dock as a hard-edged square beside every
//  other app. Measuring shipping Mac App Store icons (Speedtest, Notes, Xcode)
//  on macOS 26 gives the shape exactly:
//
//      canvas 1024×1024, body 824×824 centred (100px margins all round),
//      corners fully transparent, continuous ("squircle") corner curve.
//
//  The radius below was calibrated against those icons rather than guessed: a
//  SwiftUI `.continuous` corner at ~180–185 reproduces their corner profile to
//  the pixel. A plain CGPath rounded rect is a CIRCULAR arc and visibly cuts
//  the corner harder, which is why this renders through SwiftUI.
//
//  Alpha is REQUIRED in these PNGs (the margin is transparent) — the opposite
//  of the iOS rule, which is why the iOS image is left completely untouched.
//
//  Usage, from `ios/`:
//      swift tools/make-macos-appicon.swift
//
//  Rewrites the macOS PNGs and Contents.json in place.
//

import AppKit
import CoreGraphics
import Foundation
import ImageIO
import SwiftUI
import UniformTypeIdentifiers

// MARK: - Geometry, measured from shipping icons

let canvasSide: CGFloat = 1024
let bodySide: CGFloat = 824
let cornerRadius: CGFloat = 185.4

/// pt size → scales. 512@2x (1024px) is the slot the App Store names.
let ladder: [(pt: Int, scales: [Int])] = [
    (16, [1, 2]), (32, [1, 2]), (128, [1, 2]), (256, [1, 2]), (512, [1, 2]),
]

// MARK: - Paths

let cwd = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
let iconset = cwd.appendingPathComponent("FamilyConnect/Assets.xcassets/AppIcon.appiconset")
let source = iconset.appendingPathComponent("AppIcon.png")

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("error: \(message)\n".utf8))
    exit(1)
}

guard FileManager.default.fileExists(atPath: source.path) else {
    fail("cannot find \(source.path) — run this from the ios/ directory")
}
guard let src = CGImageSourceCreateWithURL(source as CFURL, nil),
      let artwork = CGImageSourceCreateImageAtIndex(src, 0, nil)
else { fail("could not decode \(source.lastPathComponent)") }

// MARK: - Render

@MainActor func macIcon(side: CGFloat) -> CGImage? {
    let view = Image(decorative: artwork, scale: 1)
        .resizable()
        .interpolation(.high)
        .frame(width: bodySide, height: bodySide)
        .clipShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
        .frame(width: canvasSide, height: canvasSide)

    let renderer = ImageRenderer(content: view)
    // Render the shape at the target size rather than downscaling one big
    // bitmap, so the corner stays crisp at 16 and 32 px.
    renderer.scale = side / canvasSide
    renderer.isOpaque = false
    return renderer.cgImage
}

func write(_ image: CGImage, to url: URL) -> Bool {
    guard let dest = CGImageDestinationCreateWithURL(
        url as CFURL, UTType.png.identifier as CFString, 1, nil)
    else { return false }
    CGImageDestinationAddImage(dest, image, nil)
    return CGImageDestinationFinalize(dest)
}

MainActor.assumeIsolated {
    // The iOS image is preserved verbatim: full-bleed and opaque, which is
    // what iOS requires and macOS must not copy.
    var entries = ["""
        {
          "filename" : "AppIcon.png",
          "idiom" : "universal",
          "platform" : "ios",
          "size" : "1024x1024"
        }
    """]

    for (pt, scales) in ladder {
        for scale in scales {
            let pixels = pt * scale
            let name = "AppIcon-macOS-\(pt)x\(pt)@\(scale)x.png"
            guard let image = macIcon(side: CGFloat(pixels)) else {
                fail("could not render \(name)")
            }
            guard image.width == pixels, image.height == pixels else {
                fail("\(name) rendered \(image.width)×\(image.height), expected \(pixels)²")
            }
            guard write(image, to: iconset.appendingPathComponent(name)) else {
                fail("could not write \(name)")
            }
            entries.append("""
                {
                  "filename" : "\(name)",
                  "idiom" : "mac",
                  "scale" : "\(scale)x",
                  "size" : "\(pt)x\(pt)"
                }
            """)
            print("  \(name)  \(pixels)×\(pixels)")
        }
    }

    let body = entries
        .map { $0.split(separator: "\n").map { $0.dropFirst(4) }.joined(separator: "\n") }
        .joined(separator: ",\n")
    let contents = """
    {
      "images" : [
    \(body)
      ],
      "info" : {
        "author" : "xcode",
        "version" : 1
      }
    }

    """
    do {
        try contents.write(
            to: iconset.appendingPathComponent("Contents.json"),
            atomically: true, encoding: .utf8)
    } catch {
        fail("could not write Contents.json: \(error)")
    }
    print("  Contents.json  \(entries.count) entries (1 iOS + \(entries.count - 1) macOS)")
}
