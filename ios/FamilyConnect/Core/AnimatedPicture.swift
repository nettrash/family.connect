//
//  AnimatedPicture.swift
//  FamilyConnect
//
//  What an animated GIF, WebP or PNG SAYS about itself (issue #96,
//  docs/protocol.md "Animations" › "What becomes one" and "The file"):
//  whether it moves, and how long each of its frames is stated to last.
//
//  WHETHER IT MOVES is `StickerPack.isAnimated` — the test the sticker pack
//  already uses, ImageIO finding more than one frame in a container where
//  more than one frame means motion — narrowed to the three formats the
//  protocol names. An image sequence (HEICS) or an AVIS is not an animation
//  here; it goes the way it always went.
//
//  THE DELAYS are read off the BYTES, not off ImageIO's properties, so the
//  numbers are exactly the ones the reference's vectors are written in:
//  a GIF's Graphic Control Extension in hundredths, a WebP's `ANMF` duration
//  in milliseconds, an APNG's `fcTL` as `num ÷ den`. ImageIO's unclamped
//  delay is a Double of SECONDS — 0.07 comes back as 0.07000000000000001 —
//  and it is used only where the bytes and ImageIO disagree about how many
//  frames there are (a truncated or unusual file), rounded half up to whole
//  milliseconds, which gives the same integer for every delay a GIF or an
//  APNG writes in practice.
//

import CoreGraphics
import Foundation
import ImageIO

nonisolated enum AnimatedPicture {

    /// The three formats an animation may come in.
    enum Format: Equatable, Sendable {
        case gif
        case webp
        case png
    }

    /// The format, from the magic numbers — never from a file extension.
    static func format(of data: Data) -> Format? {
        let head = [UInt8](data.prefix(12))
        if head.starts(with: Array("GIF87a".utf8)) || head.starts(with: Array("GIF89a".utf8)) {
            return .gif
        }
        if head.starts(with: [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
            return .png
        }
        if head.count >= 12,
           Array(head[0..<4]) == Array("RIFF".utf8),
           Array(head[8..<12]) == Array("WEBP".utf8) {
            return .webp
        }
        return nil
    }

    /// Is this an ANIMATION in the protocol's sense — an animated GIF, WebP
    /// or PNG? A still one of those, and anything else, is not.
    static func isAnimation(_ data: Data) -> Bool {
        format(of: data) != nil && StickerPack.isAnimated(data)
    }

    // MARK: - Stated delays

    /// Each frame's length as the picture states it, in milliseconds, in
    /// the order ImageIO numbers the frames — before
    /// `AnimationTiming.frameDurationMS`. nil when the picture is not one of
    /// the three formats, or neither its bytes nor ImageIO can say.
    static func statedDelaysMS(_ data: Data, source: CGImageSource) -> [Int]? {
        guard let format = format(of: data) else { return nil }
        let count = CGImageSourceGetCount(source)
        guard count > 0 else { return nil }
        let parsed: [Int]? = switch format {
        case .gif: gifDelaysMS(data)
        case .webp: webpDelaysMS(data)
        case .png: apngDelaysMS(data)
        }
        if let parsed, parsed.count == count { return parsed }
        return imageIODelaysMS(source: source, format: format, count: count)
    }

    /// The bytes' own delays for a GIF: one per image, the delay of the
    /// Graphic Control Extension that precedes it — 0 for an image with
    /// none, which `frameDurationMS` then shows for 100 ms like every
    /// browser does.
    static func gifDelaysMS(_ data: Data) -> [Int]? {
        let bytes = [UInt8](data)
        guard bytes.count >= 13, format(of: data) == .gif else { return nil }
        var at = 6
        let screenPacked = bytes[at + 4]
        at += 7
        if screenPacked & 0x80 != 0 {
            at += 3 * (1 << (Int(screenPacked & 0x07) + 1))
        }
        var delays: [Int] = []
        var pending: Int?
        /// Skip a run of sub-blocks; false when the data ends inside it.
        func skipSubBlocks() -> Bool {
            while at < bytes.count {
                let size = Int(bytes[at])
                at += 1
                if size == 0 { return true }
                at += size
            }
            return false
        }
        while at < bytes.count {
            switch bytes[at] {
            case 0x21:
                guard at + 1 < bytes.count else { return delays.isEmpty ? nil : delays }
                let label = bytes[at + 1]
                at += 2
                if label == 0xF9, at + 4 < bytes.count, bytes[at] >= 4 {
                    let centiseconds = UInt16(bytes[at + 2]) | UInt16(bytes[at + 3]) << 8
                    pending = AnimationTiming.gifDelayMS(centiseconds: centiseconds)
                }
                guard skipSubBlocks() else { return delays.isEmpty ? nil : delays }
            case 0x2C:
                guard at + 10 <= bytes.count else { return delays.isEmpty ? nil : delays }
                let packed = bytes[at + 9]
                at += 10
                if packed & 0x80 != 0 {
                    at += 3 * (1 << (Int(packed & 0x07) + 1))
                }
                // The LZW minimum code size, then the image's sub-blocks.
                at += 1
                delays.append(pending ?? 0)
                pending = nil
                guard skipSubBlocks() else { return delays }
            case 0x3B:
                return delays
            default:
                // Not a block any GIF has: whatever was read so far is all
                // there is to read.
                return delays.isEmpty ? nil : delays
            }
        }
        return delays.isEmpty ? nil : delays
    }

    /// The bytes' own delays for an animated WebP: one per `ANMF` chunk, its
    /// 24-bit duration in milliseconds.
    static func webpDelaysMS(_ data: Data) -> [Int]? {
        let bytes = [UInt8](data)
        guard bytes.count >= 12, format(of: data) == .webp else { return nil }
        var delays: [Int] = []
        var at = 12
        while at + 8 <= bytes.count {
            let fourCC = String(decoding: bytes[at..<at + 4], as: UTF8.self)
            let size = Int(bytes[at + 4]) | Int(bytes[at + 5]) << 8
                | Int(bytes[at + 6]) << 16 | Int(bytes[at + 7]) << 24
            let payload = at + 8
            if fourCC == "ANMF" {
                guard payload + 15 <= bytes.count else { break }
                // X, Y, width − 1, height − 1 (three bytes each), then the
                // duration, three bytes little-endian.
                let duration = Int(bytes[payload + 12]) | Int(bytes[payload + 13]) << 8
                    | Int(bytes[payload + 14]) << 16
                delays.append(duration)
            }
            // A chunk is padded to an even length.
            at = payload + size + (size & 1)
        }
        return delays.isEmpty ? nil : delays
    }

    /// The bytes' own delays for an APNG: one per `fcTL` chunk, in order.
    static func apngDelaysMS(_ data: Data) -> [Int]? {
        let bytes = [UInt8](data)
        guard bytes.count >= 8, format(of: data) == .png else { return nil }
        var delays: [Int] = []
        var at = 8
        while at + 8 <= bytes.count {
            let length = Int(bytes[at]) << 24 | Int(bytes[at + 1]) << 16
                | Int(bytes[at + 2]) << 8 | Int(bytes[at + 3])
            let type = String(decoding: bytes[at + 4..<at + 8], as: UTF8.self)
            let payload = at + 8
            if type == "fcTL" {
                guard payload + 26 <= bytes.count else { break }
                let num = UInt16(bytes[payload + 20]) << 8 | UInt16(bytes[payload + 21])
                let den = UInt16(bytes[payload + 22]) << 8 | UInt16(bytes[payload + 23])
                delays.append(AnimationTiming.apngDelayMS(num: num, den: den))
            }
            if type == "IEND" { break }
            // The chunk's data, then its CRC.
            at = payload + length + 4
        }
        return delays.isEmpty ? nil : delays
    }

    /// ImageIO's own reading, for a file whose bytes and ImageIO disagree
    /// about the number of frames: the UNCLAMPED delay (ImageIO's clamped
    /// one already turns a GIF's 0 into 0.1), in seconds, to whole
    /// milliseconds half up.
    private static func imageIODelaysMS(source: CGImageSource, format: Format, count: Int) -> [Int]? {
        let (dictionaryKey, unclampedKey, clampedKey): (CFString, CFString, CFString) = switch format {
        case .gif: (kCGImagePropertyGIFDictionary, kCGImagePropertyGIFUnclampedDelayTime,
                    kCGImagePropertyGIFDelayTime)
        case .webp: (kCGImagePropertyWebPDictionary, kCGImagePropertyWebPUnclampedDelayTime,
                     kCGImagePropertyWebPDelayTime)
        case .png: (kCGImagePropertyPNGDictionary, kCGImagePropertyAPNGUnclampedDelayTime,
                    kCGImagePropertyAPNGDelayTime)
        }
        var delays: [Int] = []
        for index in 0..<count {
            let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil) as? [CFString: Any]
            let dictionary = properties?[dictionaryKey] as? [CFString: Any]
            let seconds = (dictionary?[unclampedKey] as? NSNumber)?.doubleValue
                ?? (dictionary?[clampedKey] as? NSNumber)?.doubleValue
            guard let seconds, seconds.isFinite, seconds >= 0 else { return nil }
            delays.append(Int((seconds * 1000).rounded(.toNearestOrAwayFromZero)))
        }
        return delays
    }
}
