//
//  NoteEventTests.swift
//  FamilyConnectTests
//
//  An event's when-line, its past rule and its answers (docs/protocol.md,
//  "Board").
//
//  The wire carries an INSTANT, and each reader sees it in their own zone —
//  which is the whole reason the protocol stores a timestamp rather than a
//  local time. What is pinned here is the SHAPE of the answer and the past
//  rule, not one locale's punctuation: a test asserting a formatted string
//  would be asserting Foundation's locale data, not this code.
//
//  Android counterpart: EventFormatTest, RsvpAnswersTest.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Board events")
struct NoteEventTests {

    private let starts = Date(timeIntervalSince1970: 1_798_736_400) // 2026-12-24T17:00Z
    private var sameDayEnd: Date { starts.addingTimeInterval(4 * 3600) }
    private var nextDayEnd: Date { starts.addingTimeInterval(8 * 3600) }

    @Test("a start alone reads as one moment, and an end adds a second")
    func whenLine() {
        let alone = EventFormat.when(starts: starts, ends: nil)
        let ranged = EventFormat.when(starts: starts, ends: sameDayEnd)
        #expect(!alone.isEmpty)
        #expect(ranged.contains("–"))
        #expect(ranged.count > alone.count)
    }

    /// An end on ANOTHER day carries its own date; an end the same day is a
    /// time alone. A range reading "24 Dec, 17:00 – 01:00" would look like
    /// eight hours backwards.
    @Test("an end on another day says which day")
    func acrossMidnight() {
        let sameDay = EventFormat.when(starts: starts, ends: sameDayEnd)
        let nextDay = EventFormat.when(starts: starts, ends: nextDayEnd)
        #expect(nextDay.count > sameDay.count)
    }

    /// The CALENDAR BLOCK and the line beside it (docs/protocol.md,
    /// "Board"): the block carries the date, so the line carries the time
    /// and says the date again only when the event ends on another day.
    @Test("the block is the day and its short month, and the line is the time")
    func calendarBlock() {
        let block = EventFormat.block(starts: starts)
        // The day, as a number and nothing else.
        #expect(Int(block.day) != nil)
        // Short, not the whole word and not a number: a block that said
        // "December" would not be a block.
        #expect(!block.month.isEmpty)
        #expect(block.month.count <= 5)
        #expect(Int(block.month) == nil)

        // Same day: two clock times, and the date is left to the block.
        let sameDay = EventFormat.clock(starts: starts, ends: sameDayEnd)
        #expect(sameDay.contains("–"))
        #expect(!sameDay.contains(block.month))
        // No end: one time.
        #expect(!EventFormat.clock(starts: starts, ends: nil).contains("–"))
        // Another day: the end carries its own date, or "17:00 – 01:00"
        // would read as an event that went backwards.
        let across = EventFormat.clock(starts: starts, ends: nextDayEnd)
        #expect(across.count > sameDay.count)
    }

    @Test("past is decided by the end when there is one, and the start when there is not")
    func pastRule() {
        #expect(!EventFormat.isPast(starts, ends: sameDayEnd, now: starts.addingTimeInterval(60)))
        #expect(EventFormat.isPast(starts, ends: sameDayEnd, now: sameDayEnd.addingTimeInterval(60)))
        #expect(EventFormat.isPast(starts, ends: nil, now: starts.addingTimeInterval(60)))
        #expect(!EventFormat.isPast(starts, ends: nil, now: starts.addingTimeInterval(-60)))
    }

    @Test("the answers are the protocol's, and an unknown one is nobody's")
    func answers() {
        #expect(RsvpAnswer.allCases.map(\.name) == ["going", "maybe", "no"])
        #expect(RsvpAnswer(name: "going") == .going)
        // An answer from a NEWER server is not drawn as one of these: the
        // picker simply highlights nothing, which is better than claiming
        // somebody said something they did not.
        #expect(RsvpAnswer(name: "perhaps") == nil)
        #expect(RsvpAnswer(name: nil) == nil)
    }

    /// The picker opens on something somebody might have meant.
    @Test("a new event opens on the next round hour")
    func roundHour() {
        let calendar = Calendar.current
        for offset in [0, 61, 1_000, 30_000] {
            let now = Date(timeIntervalSince1970: 1_798_736_400 + Double(offset))
            let opening = now.nextRoundHour
            #expect(opening > now)
            #expect(calendar.component(.minute, from: opening) == 0)
            #expect(calendar.component(.second, from: opening) == 0)
        }
    }
}
