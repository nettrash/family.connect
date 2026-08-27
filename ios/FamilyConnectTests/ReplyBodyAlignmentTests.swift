//
//  ReplyBodyAlignmentTests.swift
//  FamilyConnectTests
//
//  The pure half of the own-reply body rule: which edge a body of N lines
//  hugs, and how N is read off two heights. The drawn half — that the
//  balloon actually rags its lines the way the rule says — is
//  BubbleLayoutTests, on the phone.
//

import CoreGraphics
import Testing
@testable import FamilyConnect

@Suite("Own-reply body alignment rule")
struct ReplyBodyAlignmentTests {

    @Test("one and two lines sit trailing; three and more go back to leading")
    func edgeByLineCount() {
        #expect(ReplyBodyAlignment.maxTrailingLines == 2)
        #expect(ReplyBodyAlignment.alignsTrailing(lineCount: 1))
        #expect(ReplyBodyAlignment.alignsTrailing(lineCount: 2))
        #expect(!ReplyBodyAlignment.alignsTrailing(lineCount: 3))
        #expect(!ReplyBodyAlignment.alignsTrailing(lineCount: 4))
        #expect(!ReplyBodyAlignment.alignsTrailing(lineCount: 40))
    }

    @Test("the line count is the rounded height ratio, never below one")
    func lineCountFromHeights() {
        // Exact multiples.
        #expect(ReplyBodyAlignment.lineCount(bodyHeight: 20.3, lineHeight: 20.3) == 1)
        #expect(ReplyBodyAlignment.lineCount(bodyHeight: 40.6, lineHeight: 20.3) == 2)
        #expect(ReplyBodyAlignment.lineCount(bodyHeight: 60.9, lineHeight: 20.3) == 3)
        // A point of leading either way is still the same line count —
        // this is why the ratio rounds rather than floors.
        #expect(ReplyBodyAlignment.lineCount(bodyHeight: 41.5, lineHeight: 20.3) == 2)
        #expect(ReplyBodyAlignment.lineCount(bodyHeight: 39.8, lineHeight: 20.3) == 2)
        #expect(ReplyBodyAlignment.lineCount(bodyHeight: 59.9, lineHeight: 20.3) == 3)
        // The boundary that decides the edge: two-and-a-bit rounds down,
        // two-and-a-half rounds up.
        #expect(ReplyBodyAlignment.lineCount(bodyHeight: 48, lineHeight: 20) == 2)
        #expect(ReplyBodyAlignment.lineCount(bodyHeight: 50, lineHeight: 20) == 3)
    }

    @Test("an unmeasured or impossible height reads as one line — the old rule, the safe answer")
    func degenerateHeights() {
        #expect(ReplyBodyAlignment.lineCount(bodyHeight: 0, lineHeight: 20) == 1)
        #expect(ReplyBodyAlignment.lineCount(bodyHeight: 60, lineHeight: 0) == 1)
        #expect(ReplyBodyAlignment.lineCount(bodyHeight: 60, lineHeight: -1) == 1)
        #expect(ReplyBodyAlignment.lineCount(bodyHeight: -60, lineHeight: 20) == 1)
        #expect(ReplyBodyAlignment.lineCount(bodyHeight: .nan, lineHeight: 20) == 1)
        #expect(ReplyBodyAlignment.lineCount(bodyHeight: .infinity, lineHeight: 20) == 1)
        // A body shorter than a line (a tiny emoji ladder mismatch) is
        // still one line, not zero.
        #expect(ReplyBodyAlignment.lineCount(bodyHeight: 4, lineHeight: 20) == 1)
    }
}
