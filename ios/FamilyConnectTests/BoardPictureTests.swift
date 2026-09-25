//
//  BoardPictureTests.swift
//  FamilyConnectTests
//
//  A PHOTO IS DRAWN WHOLE (docs/protocol.md, "Board"): fitted in both
//  dimensions and never cropped to fill its box. Issue #71 — the board
//  drew a portrait photograph cut off at the top and the bottom, because
//  the picture filled a strip of fixed height instead of fitting the room
//  it had.
//
//  Two rules, both pinned here: the size a picture takes inside a space
//  (also the size of a BARE photo's card, which IS the picture), and how
//  much of a card the picture gets — the whole of it with no caption, and
//  the strip above the words with one.
//
//  The same numbers the other clients pin: `fc_text::board::fitted_picture`
//  and `BoardPicture.fitted` on Android.
//

import CoreGraphics
import Foundation
import SwiftUI
import Testing
@testable import FamilyConnect

@Suite("Board pictures")
struct BoardPictureTests {

    /// The word #71 was: a picture that FILLS its box is a picture cropped
    /// to the box's shape, and on a board of landscape cards that is the
    /// middle third of every portrait photograph.
    @Test("a pinned picture is fitted, never filled")
    func fittedNotFilled() {
        #expect(NotePicture.contentMode == .fit)
    }

    @Test("a portrait is fitted narrow and a wide one short")
    func fittedBothWays() {
        // The 600x1200 photograph #71 was reported with, on the Mac's
        // medium card: narrow, and every pixel of its height.
        let portrait = BoardPicture.fitted(
            space: CGSize(width: 150, height: 110), picture: CGSize(width: 600, height: 1200))
        #expect(abs(portrait.width - 55) < 0.01)
        #expect(abs(portrait.height - 110) < 0.01)
        // Whole: the picture's own shape, not the card's.
        #expect(abs(portrait.width / portrait.height - 0.5) < 0.001)

        let landscape = BoardPicture.fitted(
            space: CGSize(width: 150, height: 110), picture: CGSize(width: 1600, height: 900))
        #expect(abs(landscape.width - 150) < 0.01)
        #expect(abs(landscape.height - 84.375) < 0.01)
    }

    @Test("a picture never grows past its space")
    func neverBigger() {
        let space = CGSize(width: 132, height: 132)
        #expect(BoardPicture.fitted(space: space, picture: CGSize(width: 300, height: 300)) == space)
        let small = BoardPicture.fitted(space: space, picture: CGSize(width: 30, height: 20))
        #expect(small.width <= space.width)
        #expect(small.height <= space.height)
        // And a panorama still leaves something to tap.
        let panorama = BoardPicture.fitted(
            space: space, picture: CGSize(width: 20_000, height: 10))
        #expect(panorama.height >= 1)
    }

    /// A margin at worst, and never a crop: the picture is still fitted
    /// inside whatever space it is handed.
    @Test("dimensions the server never gave take the whole space")
    func unknownShapeTakesTheSpace() {
        let space = CGSize(width: 132, height: 132)
        #expect(BoardPicture.fitted(space: space, picture: .zero) == space)
        #expect(BoardPicture.fitted(space: space, picture: CGSize(width: 600, height: 0)) == space)
        #expect(BoardPicture.fitted(space: space, picture: CGSize(width: -4, height: 8)) == space)
    }

    /// A BARE photo is the picture: there is no caption to leave a line for
    /// and no author line under it, so the strip's arithmetic has nothing
    /// to take room for. An 84pt strip inside a 132pt square was the other
    /// half of #71 — a third of the photograph, and the rest of the card
    /// empty.
    @Test("a photo with no caption gets the whole card")
    func barePictureTakesTheCard() {
        #expect(NotePicture.height(cardHeight: 132, hasCaption: false) == 132)
        #expect(NotePicture.height(cardHeight: 220, hasCaption: false) == 220)
        // The Mac's small card too, where the old arithmetic left 52.
        #expect(NotePicture.height(cardHeight: 88, hasCaption: false) == 88)
    }

    /// A caption brings the card back, and then the picture takes what is
    /// left after the padding, the author line and the caption's own line —
    /// never more than 84, never less than a strip that still reads as a
    /// picture.
    @Test("a captioned photo shares the card with its words")
    func captionedPictureKeepsItsStrip() {
        #expect(NotePicture.height(cardHeight: 220, hasCaption: true) == 84)
        #expect(NotePicture.height(cardHeight: 110, hasCaption: true) == 52)
        #expect(NotePicture.height(cardHeight: 88, hasCaption: true) == 30)
        // A card with no room left still shows a strip rather than nothing.
        #expect(NotePicture.height(cardHeight: 40, hasCaption: true) == 24)
    }
}
