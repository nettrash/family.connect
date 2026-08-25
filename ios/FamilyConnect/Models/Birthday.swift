//
//  Birthday.swift
//  FamilyConnect
//
//  What a day-and-month is allowed to be, and how it is written down.
//
//  The wire shape lives with the other DTOs (BirthdayDTO in APIModels);
//  this is the half the screens need: the day-count rule that stops an
//  editor offering 31 April in the first place, and a formatter that
//  draws "14 March", "14 марта" or "3月14日" from the reader's own locale.
//
//  The rule is the SERVER's — three apps must not end up disagreeing about
//  which dates exist (protocol.md, "Birthdays"). This copy exists so a
//  picker cannot be pointed at an impossible date, not so a client can
//  decide the answer: a `validation` error still has to be shown when one
//  comes back.
//
//  Android counterpart: ui/settings/BirthdayScreen.kt
//

import Foundation

extension BirthdayDTO {

    /// How many days that month has, with NO year to ask — which is why
    /// February is 29 here and not 28. A birthday has no year for the
    /// 29th to fail to exist in, so the leap-day child gets their date.
    static func daysIn(month: Int) -> Int {
        switch month {
        case 1, 3, 5, 7, 8, 10, 12: 31
        case 4, 6, 9, 11: 30
        case 2: 29
        default: 0
        }
    }

    /// The same check the server makes, so a picker can refuse to offer an
    /// impossible date rather than spending a round trip to be told.
    var isValid: Bool {
        (1...12).contains(month) && (1...Self.daysIn(month: month)).contains(day)
    }

    /// The days an editor may offer for `month` — never an inverted range.
    ///
    /// `daysIn` answers 0 for a month outside the calendar, and `1...0` is
    /// not an empty range in Swift, it is a TRAP: the birthday sheet built
    /// its day picker as `1...daysIn(month:)`, so a roster row carrying
    /// month 0 or 13 took the whole app down the instant the owner opened
    /// it. A roster row is not the place to find out the server did not
    /// validate one — Android coerces the same input in the same place
    /// (TimeFormat.daysInBirthdayMonth), and a wrong date shown is
    /// enormously better than a crash on both.
    static func dayRange(forMonth month: Int) -> ClosedRange<Int> {
        1...max(1, daysIn(month: month))
    }

    /// This value as an editor can show it: a month in 1…12 and a day that
    /// month has. Identical to `self` for anything `isValid` accepts, which
    /// is every birthday the server has ever written.
    var clamped: BirthdayDTO {
        let month = min(max(self.month, 1), 12)
        return BirthdayDTO(
            month: month, day: min(max(day, 1), Self.daysIn(month: month)))
    }

    /// "14 March", "March 14", "14 марта", "3月14日" — day and month in the
    /// reader's locale, and never a year.
    ///
    /// Built from a TEMPLATE rather than a format string: "MMMM d" is the
    /// English ordering and would put the month first for a Russian reader
    /// too. `dateFormat(fromTemplate:)` asks the locale which way round its
    /// own dates go, and drops nothing else in on the way — there is no
    /// year in the template, so there is no year in the result and no age
    /// anywhere for anyone to work out.
    func formatted(locale: Locale = .autoupdatingCurrent) -> String {
        var components = DateComponents()
        components.month = month
        components.day = day
        // A year the calendar is certain to accept, purely so there is a
        // date to format at all. 2024 is a leap year, so 29 February
        // resolves rather than rolling into March.
        components.year = 2024
        var calendar = Calendar(identifier: .gregorian)
        calendar.locale = locale
        guard let date = calendar.date(from: components) else { return "" }

        let formatter = DateFormatter()
        formatter.calendar = calendar
        formatter.locale = locale
        formatter.dateFormat = DateFormatter.dateFormat(
            fromTemplate: "MMMMd", options: 0, locale: locale) ?? "MMMM d"
        return formatter.string(from: date)
    }

    /// The month names an editor's picker shows, in the reader's language,
    /// straight from the OS. Not catalogue strings: every locale the app
    /// ships in already knows what its months are called, and eight
    /// translations of "March" would be eight chances to disagree with the
    /// calendar the date is drawn with.
    static func monthNames(locale: Locale = .autoupdatingCurrent) -> [String] {
        let formatter = DateFormatter()
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.locale = locale
        return formatter.standaloneMonthSymbols ?? formatter.monthSymbols ?? []
    }
}
