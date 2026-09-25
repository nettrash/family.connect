//
//  NoteFontTests.swift
//  FamilyConnectTests
//
//  The four hands a note can be written in (docs/protocol.md, "Board").
//
//  A font is an INTENT resolved to a SYSTEM design: the wire never carries a
//  family name, and this app bundles no fonts. The vocabulary, the fallback
//  and the patch rule are pinned here against the Android counterpart,
//  NoteFontsTest.
//

import SwiftUI
import Testing
@testable import FamilyConnect

@Suite("Note fonts")
struct NoteFontTests {

    @Test("the vocabulary is the protocol's, in picker order")
    func vocabulary() {
        #expect(NoteFont.allCases == [.plain, .serif, .mono, .casual])
        #expect(NoteFont.allCases.map(\.name) == ["plain", "serif", "mono", "casual"])
        for face in NoteFont.allCases {
            #expect(NoteFont(name: face.name) == face)
        }
    }

    /// A name from a NEWER server must draw as something rather than fail,
    /// and plain is what every note was written in before fonts existed —
    /// the same forgiveness `color` and `size` get.
    @Test("an unknown hand falls back to plain")
    func unknownFallsBack() {
        #expect(NoteFont(name: "comic-sans") == .plain)
        #expect(NoteFont(name: nil) == .plain)
        #expect(NoteFont(name: "") == .plain)
    }

    /// The rule that keeps a fifth face from a newer server alive: it DRAWS
    /// as plain, but a text edit must not WRITE it back as plain. Only a
    /// hand the author actually changed is sent.
    @Test("only a changed hand is patched")
    func patchName() {
        #expect(NoteFont.plain.patchName(replacing: "plain") == nil)
        #expect(NoteFont.serif.patchName(replacing: "plain") == "serif")
        #expect(NoteFont.plain.patchName(replacing: "serif") == "plain")
        // The unknown-name case, which is the whole reason this exists.
        #expect(NoteFont.plain.patchName(replacing: "comic-sans") == nil)
        #expect(NoteFont.casual.patchName(replacing: "comic-sans") == "casual")
        #expect(NoteFont.plain.patchName(replacing: nil) == nil)
    }

    /// Each hand is a system DESIGN, and the four differ — nothing bundled,
    /// nothing downloaded.
    @Test("each hand is its own system design")
    func designs() {
        #expect(NoteFont.plain.design == .default)
        #expect(NoteFont.serif.design == .serif)
        #expect(NoteFont.mono.design == .monospaced)
        #expect(NoteFont.casual.design == .rounded)
        #expect(Set(NoteFont.allCases.map(\.design)).count == 4)
    }

    /// A note keeps a SEMANTIC size in every hand, so the wall goes on
    /// tracking Dynamic Type. A point size fixed in the font would freeze
    /// it, and the fitting rule would then quietly hide the regression.
    @Test("a hand carries the size's own text style, not a point size")
    func sizesStaySemantic() {
        for size in NoteSize.allCases {
            for face in NoteFont.allCases {
                #expect(face.font(for: size) == Font.system(size.textStyle, design: face.design))
            }
            #expect(size.font == Font.system(size.textStyle))
        }
    }
}
