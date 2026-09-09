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

    /// Has it already happened? A past event is drawn quieter rather than
    /// removed: the wall is the family's, and clearing it is their call.
    static func isPast(_ starts: Date, ends: Date?, now: Date = Date()) -> Bool {
        (ends ?? starts) < now
    }
}

/// The block an event note draws above its title.
struct NoteEventBlock: View {
    let starts: Date
    let ends: Date?
    let place: String?
    let going: Int
    let maybe: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(EventFormat.when(starts: starts, ends: ends))
                .font(.caption2.weight(.semibold))
                .foregroundStyle(.black.opacity(EventFormat.isPast(starts, ends: ends) ? 0.4 : 0.75))
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
