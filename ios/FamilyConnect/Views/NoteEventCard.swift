//
//  NoteEventCard.swift
//  FamilyConnect
//
//  What an event note shows on the wall: when, where, and who is coming
//  (docs/protocol.md, "Board").
//
//  Everything here is drawn from the note's own fields — there is no
//  calendar on the wire and no `.ics`. A client that can put the event in
//  the system calendar builds one locally, which is what `calendarURL`
//  below is for; the server neither generates one nor knows whether
//  anybody kept it.
//
//  Android counterpart: NoteEventBlock in ui/board/BoardScreen.kt.
//

import SwiftUI

/// The three answers, in the order a picker offers them.
nonisolated enum RsvpAnswer: String, CaseIterable, Identifiable, Sendable {
    case going
    case maybe
    case no

    var name: String { rawValue }
    var id: String { rawValue }

    /// An unknown answer from a newer server is not drawn as one of these —
    /// the caller keeps the raw string and simply does not highlight a
    /// button, which is better than claiming somebody said something else.
    init?(name: String?) {
        guard let name, let answer = RsvpAnswer(rawValue: name) else { return nil }
        self = answer
    }

    var title: LocalizedStringKey {
        switch self {
        case .going: "Going"
        case .maybe: "Maybe"
        case .no: "Can't"
        }
    }

    /// The same three words as plain text, for a place that cannot take a
    /// `LocalizedStringKey` — a menu row built by interpolation.
    var plainTitle: String {
        switch self {
        case .going: String(localized: "Going")
        case .maybe: String(localized: "Maybe")
        case .no: String(localized: "Can't")
        }
    }

    var symbol: String {
        switch self {
        case .going: "checkmark.circle.fill"
        case .maybe: "questionmark.circle.fill"
        case .no: "xmark.circle.fill"
        }
    }
}

/// When and where, formatted for the sticker.
///
/// The date is written in the READER's locale and time zone, deliberately:
/// the wire carries an instant, and a family spread across two countries
/// each sees the moment in their own — which is the whole reason the
/// protocol stores a timestamp rather than a local time.
enum EventFormat {
    static func when(starts: Date, ends: Date?) -> String {
        let day = starts.formatted(.dateTime.weekday(.abbreviated).day().month(.abbreviated))
        let from = starts.formatted(date: .omitted, time: .shortened)
        guard let ends else { return "\(day), \(from)" }
        let sameDay = Calendar.current.isDate(ends, inSameDayAs: starts)
        let to = sameDay
            ? ends.formatted(date: .omitted, time: .shortened)
            : ends.formatted(.dateTime.day().month(.abbreviated)) + " "
                + ends.formatted(date: .omitted, time: .shortened)
        return "\(day), \(from) – \(to)"
    }

    /// The date as a CALENDAR BLOCK: the day's number and its short month,
    /// both in the reader's own language (docs/protocol.md, "Board").
    ///
    /// Two strings rather than one, because they are drawn one over the
    /// other — which is the point of the block — and because a joined
    /// "24 Dec" would put them in an order some languages do not use.
    static func block(starts: Date) -> (day: String, month: String) {
        (
            starts.formatted(.dateTime.day()),
            starts.formatted(.dateTime.month(.abbreviated))
        )
    }

    /// The TIME, beside the block that already says the date: "16:00",
    /// "16:00 – 20:00", or "16:00 – 25 Dec 02:00" when it ends on another
    /// day.
    static func clock(starts: Date, ends: Date?) -> String {
        let from = starts.formatted(date: .omitted, time: .shortened)
        guard let ends else { return from }
        let to = ends.formatted(date: .omitted, time: .shortened)
        if Calendar.current.isDate(ends, inSameDayAs: starts) {
            return "\(from) – \(to)"
        }
        let day = ends.formatted(.dateTime.day().month(.abbreviated))
        return "\(from) – \(day) \(to)"
    }

    /// Has it already happened? A past event is drawn quieter rather than
    /// removed: the wall is the family's, and clearing it is their call.
    static func isPast(_ starts: Date, ends: Date?, now: Date = Date()) -> Bool {
        (ends ?? starts) < now
    }
}

/// The block an event note draws above its title — a CALENDAR ENTRY: the
/// date in a block of its own, the time beside it, the place under that
/// (docs/protocol.md, "Board").
///
/// The shape is the same on all four clients. A wall where one device shows
/// a calendar page and another a paragraph of small print is not the same
/// wall, which is the same argument the bare photo rests on.
struct NoteEventBlock: View {
    let starts: Date
    let ends: Date?
    let place: String?
    let going: Int
    let maybe: Int

    private var isPast: Bool { EventFormat.isPast(starts, ends: ends) }

    var body: some View {
        HStack(alignment: .top, spacing: 6) {
            NoteDateBlock(starts: starts, past: isPast)
            lines
        }
    }

    @ViewBuilder
    private var lines: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(EventFormat.clock(starts: starts, ends: ends))
                .font(.caption2.weight(.semibold))
                .foregroundStyle(.black.opacity(isPast ? 0.4 : 0.75))
                .lineLimit(2)
                .minimumScaleFactor(0.7)
            if let place, !place.isEmpty {
                Text(place)
                    .font(.caption2)
                    .foregroundStyle(.black.opacity(0.55))
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
            }
            if going > 0 || maybe > 0 {
                // The count, not the names: a sticker has room for the news
                // and the card that opens has room for the people.
                Text(goingLine)
                    .font(.caption2)
                    .foregroundStyle(.black.opacity(0.55))
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
            }
        }
    }

    private var goingLine: String {
        if maybe == 0 { return String(localized: "\(going) going") }
        if going == 0 { return String(localized: "\(maybe) maybe") }
        return String(localized: "\(going) going, \(maybe) maybe")
    }
}

/// The date, as a torn calendar page: the day's number over its short
/// month, on paper of its own so it reads as a date and not as another line
/// of small print.
///
/// Not read out: the sticker's own label already says when the event is,
/// and a screen reader hearing "24 Dec" twice is worse than once.
struct NoteDateBlock: View {
    let starts: Date
    var past: Bool = false

    var body: some View {
        let block = EventFormat.block(starts: starts)
        VStack(spacing: 0) {
            Text(block.day)
                .font(.headline.weight(.bold))
                .foregroundStyle(.black.opacity(0.78))
            Text(block.month.uppercased())
                .font(.system(size: 9, weight: .semibold))
                .kerning(0.4)
                .foregroundStyle(Color(red: 0.698, green: 0.149, blue: 0.118).opacity(0.85))
        }
        .padding(.horizontal, 5)
        .padding(.vertical, 3)
        .background(.white.opacity(0.55), in: RoundedRectangle(cornerRadius: 5))
        .opacity(past ? 0.55 : 1)
        .accessibilityHidden(true)
    }
}

extension Date {
    /// The next round hour. A family event is PLANNED, so the picker opens
    /// on something somebody might actually have meant, rather than on the
    /// second the button was tapped.
    var nextRoundHour: Date {
        let calendar = Calendar.current
        let next = calendar.date(byAdding: .hour, value: 1, to: self) ?? self
        return calendar.date(
            bySettingHour: calendar.component(.hour, from: next), minute: 0, second: 0, of: next)
            ?? next
    }
}
