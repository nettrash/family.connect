//
//  EventCalendar.swift
//  FamilyConnect
//
//  One board event as an `.ics` file, for the platform's own calendar
//  (docs/protocol.md, "Board").
//
//  NOTHING HERE IS ON THE WIRE. The protocol carries no calendar and no
//  `.ics`: a client that can put an event in the system calendar builds it
//  locally out of the title, the times and the place, and the server
//  neither generates one nor knows whether anybody kept it.
//
//  WHY A FILE AND NOT EVENTKIT. `EKEventStore` with
//  `EKEventEditViewController` is the more native road, and it costs a
//  permission: an `NSCalendarsWriteOnlyAccessUsageDescription` on the phone
//  and a `com.apple.security.personal-information.calendars` entitlement on
//  a sandboxed Mac, both of which are questions App Review asks and a
//  family answers. Handing the system a file asks for nothing at all — the
//  share sheet on the phone, Calendar's own import on the Mac — and it is
//  the same thing the web does with a download. If the native editor is
//  wanted later, only this file's caller changes.
//
//  What it copies is the title, the start, the end and the place. Not who
//  is coming — that is the family's business and not the calendar's — and
//  not the backdrop.
//
//  Web counterpart: `fc_text::calendar::one_event`, whose tests are the
//  same cases as this file's.
//
import Foundation

nonisolated enum EventCalendar {

    /// The `.ics` text for one event, or nil when it has no start — which a
    /// note that is not an event has not.
    static func ics(
        noteID: Int64,
        title: String,
        startsAt: Date?,
        endsAt: Date?,
        place: String?,
        now: Date = Date()
    ) -> String? {
        guard let startsAt else { return nil }
        var lines: [String] = [
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            "PRODID:-//nettrash//Family Connect//EN",
            "CALSCALE:GREGORIAN",
            // PUBLISH, not REQUEST: a copy of something the family already
            // agreed on, not an invitation with attendees to answer.
            "METHOD:PUBLISH",
            "BEGIN:VEVENT",
            "UID:fc-note-\(noteID)@family.connect",
            "DTSTAMP:\(stamp(now))",
            "DTSTART:\(stamp(startsAt))",
        ]
        // An event with no end is its calendar's own default duration: a
        // file with no DTEND says that, where an invented hour would say
        // something the family did not.
        if let endsAt {
            lines.append("DTEND:\(stamp(endsAt))")
        }
        lines.append("SUMMARY:\(escaped(title))")
        if let place, !place.isEmpty {
            lines.append("LOCATION:\(escaped(place))")
        }
        lines.append("END:VEVENT")
        lines.append("END:VCALENDAR")
        // CRLF, as RFC 5545 asks: a file with bare newlines is refused by
        // some calendars and truncated by others.
        return lines.map(folded).joined(separator: "\r\n") + "\r\n"
    }

    /// The file to hand the system, written where a share sheet can reach
    /// it. The name is the event's title, so what somebody sees in the
    /// sheet is the event and not a hex string.
    static func file(named title: String, ics: String) -> URL? {
        let stem = fileStem(title)
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent(stem)
            .appendingPathExtension("ics")
        do {
            try Data(ics.utf8).write(to: url, options: .atomic)
            return url
        } catch {
            return nil
        }
    }

    /// `YYYYMMDDTHHMMSSZ`, in UTC — the one form every calendar reads the
    /// same way, and the reason this does not use a localised formatter.
    static func stamp(_ date: Date) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(identifier: "UTC")
        formatter.dateFormat = "yyyyMMdd'T'HHmmss'Z'"
        return formatter.string(from: date)
    }

    /// A TEXT value's own characters, kept from being read as the format's:
    /// a comma left alone would end the value and make the rest of the
    /// title a second property.
    static func escaped(_ value: String) -> String {
        var out = ""
        out.reserveCapacity(value.count)
        for character in value {
            switch character {
            case "\\": out += "\\\\"
            case ";": out += "\\;"
            case ",": out += "\\,"
            case "\n": out += "\\n"
            // Half a line ending; the newline beside it carries the meaning.
            case "\r": break
            default: out.append(character)
            }
        }
        return out
    }

    /// A content line broken at 75 OCTETS and continued with a leading
    /// space, never splitting a UTF-8 sequence: a family writing in
    /// Cyrillic reaches the limit at half the characters an English one
    /// does, and a fold inside a code point produces a file nothing reads.
    static func folded(_ line: String) -> String {
        let limit = 75
        guard line.utf8.count > limit else { return line }
        var out = ""
        var room = limit
        for character in line {
            let width = String(character).utf8.count
            if width > room {
                out += "\r\n "
                // The continuation's leading space is an octet of the line.
                room = limit - 1
            }
            out.append(character)
            room -= width
        }
        return out
    }

    /// A title as a file name: the words it has, and nothing a file system
    /// would refuse.
    static func fileStem(_ title: String) -> String {
        let kept = title.map { character -> Character in
            character.isLetter || character.isNumber || character == " " || character == "-"
                ? character
                : " "
        }
        let words = String(kept).split(separator: " ").map(String.init)
        return words.isEmpty ? "event" : words.joined(separator: " ")
    }
}
