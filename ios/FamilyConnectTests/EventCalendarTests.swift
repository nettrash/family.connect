//
//  EventCalendarTests.swift
//  FamilyConnectTests
//
//  One board event as an `.ics` file (docs/protocol.md, "Board"): nothing
//  here is on the wire, and all of it is what a calendar refuses when it is
//  wrong — CRLF line endings, the format's own characters escaped, and
//  folding counted in OCTETS.
//
//  The cases are the same ones `fc_text::calendar`'s tests pin for the web,
//  because the two build the same file for the same event.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("An event as a calendar file")
struct EventCalendarTests {

    private let starts = Date(timeIntervalSince1970: 1_798_128_000)
    private let ends = Date(timeIntervalSince1970: 1_798_142_400)

    @Test("The file has the shape a calendar reads")
    func shape() throws {
        let ics = try #require(EventCalendar.ics(
            noteID: 12,
            title: "Christmas dinner",
            startsAt: starts,
            endsAt: ends,
            place: "Gran's house",
            now: Date(timeIntervalSince1970: 1_757_600_000)))

        #expect(ics.hasPrefix("BEGIN:VCALENDAR\r\n"))
        #expect(ics.hasSuffix("END:VCALENDAR\r\n"))
        #expect(ics.contains("UID:fc-note-12@family.connect\r\n"))
        #expect(ics.contains("SUMMARY:Christmas dinner\r\n"))
        #expect(ics.contains("LOCATION:Gran's house\r\n"))
        #expect(ics.contains("DTSTART:\(EventCalendar.stamp(starts))\r\n"))
        #expect(ics.contains("DTEND:\(EventCalendar.stamp(ends))\r\n"))
        // Every line ends CRLF: a bare newline is refused by some
        // calendars and silently truncates the file in others.
        for line in ics.components(separatedBy: "\r\n") where !line.isEmpty {
            #expect(!line.contains("\n"), "a bare newline in \(line)")
        }
    }

    /// UTC, in the one form every calendar reads the same way — and
    /// through a POSIX formatter, so a device set to a non-Gregorian
    /// calendar does not write a year nothing can parse.
    @Test("The stamp is UTC, in iCalendar's own shape")
    func stamps() {
        #expect(EventCalendar.stamp(Date(timeIntervalSince1970: 0)) == "19700101T000000Z")
        let stamp = EventCalendar.stamp(starts)
        #expect(stamp.count == 16)
        #expect(stamp.hasSuffix("Z"))
        #expect(stamp.contains("T"))
    }

    @Test("An event with no end and no place says neither")
    func sparse() throws {
        let ics = try #require(EventCalendar.ics(
            noteID: 1, title: "Picnic", startsAt: starts, endsAt: nil, place: nil))
        #expect(!ics.contains("DTEND"))
        #expect(!ics.contains("LOCATION"))
    }

    /// A note that is not an event has no start, and there is no file to
    /// hand anybody.
    @Test("No start, no file")
    func noStart() {
        #expect(EventCalendar.ics(
            noteID: 1, title: "Milk", startsAt: nil, endsAt: nil, place: nil) == nil)
    }

    @Test("The format's own characters are escaped")
    func escaping() throws {
        let ics = try #require(EventCalendar.ics(
            noteID: 1,
            title: "Dinner, drinks; then\nfireworks \\ home",
            startsAt: starts,
            endsAt: nil,
            place: "Gran's, upstairs"))
        // A comma left alone would end the value and make the rest of the
        // title a second property.
        #expect(ics.contains("SUMMARY:Dinner\\, drinks\\; then\\nfireworks \\\\ home\r\n"))
        #expect(ics.contains("LOCATION:Gran's\\, upstairs\r\n"))
        #expect(!ics.contains("then\r\nfireworks"))
    }

    @Test("A long line folds at 75 octets and never mid-character")
    func folding() throws {
        let long = String(repeating: "Дед Мороз", count: 20)
        let ics = try #require(EventCalendar.ics(
            noteID: 1, title: long, startsAt: starts, endsAt: nil, place: nil))
        for line in ics.components(separatedBy: "\r\n") {
            #expect(line.utf8.count <= 75, "\(line.utf8.count) octets")
        }
        // The folds are all that was added: take them out and the title is
        // back, character for character.
        let unfolded = ics.replacingOccurrences(of: "\r\n ", with: "")
        #expect(unfolded.contains("SUMMARY:\(long)"))
    }

    /// What the share sheet shows: the event, not a hex string.
    @Test("The file is named after the event")
    func names() {
        #expect(EventCalendar.fileStem("Christmas dinner") == "Christmas dinner")
        #expect(EventCalendar.fileStem("Dinner: 7pm / Gran's") == "Dinner 7pm Gran s")
        #expect(EventCalendar.fileStem("///") == "event")
    }
}
