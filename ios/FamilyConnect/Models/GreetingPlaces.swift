//
//  GreetingPlaces.swift
//  FamilyConnect
//
//  The owner's places for the daily greeting's weather (docs/protocol.md,
//  "Today's weather, for places the owner chose"): when the editor is
//  offered, and what the list will look like once the server has kept it.
//
//  The server is the judge — `validate_greeting_places` in
//  server/src/handlers_family.rs — and its answer is what the screen shows.
//  This is its mirror, so the editor can say no before a round trip and
//  never sends a list the server would refuse:
//
//  - each name is trimmed and every inner run of whitespace folded to one
//    space — Unicode `White_Space`, the property Rust's `split_whitespace`
//    uses, read off the scalars rather than `Character.isWhitespace`;
//  - a control character (general category Cc) is refused, but only AFTER
//    the fold, so a tab or a line break is folded away rather than refused;
//  - a name may be at most 80 characters, counted as Rust counts `chars()`
//    — Unicode scalars, not grapheme clusters (a flag emoji is two);
//  - a repeat of an earlier name, compared lower-cased, is dropped in
//    silence and the first spelling kept. Compared SCALAR BY SCALAR, as
//    the server compares bytes — never with Swift's `==` or `Set<String>`,
//    which call canonically equivalent spellings one ("Café" with é and
//    with e + U+0301 are two names to the server);
//  - at most 3, counted after repeats are dropped.
//
//  One deliberate difference: a field left EMPTY is dropped here rather than
//  refused. The server refuses an empty name because nobody sends one on
//  purpose; on this screen an empty field is a place the owner added and did
//  not fill, and saving the others is what they meant.
//

import Foundation

nonisolated enum GreetingPlaces {

    /// The most places a family may name — the server's
    /// `MAX_GREETING_PLACES`, and the "3" in "Up to 3 places."
    static let maxPlaces = 3
    /// The longest name, in Unicode scalars, once whitespace is folded — the
    /// server's `MAX_GREETING_PLACE_CHARS`.
    static let maxCharacters = 80

    // MARK: - Is the editor offered?

    /// The place list is offered only to the OWNER — the server refuses
    /// anybody else (`not_family_owner`), and like every other owner-only
    /// assistant setting a member is shown nothing — and only on a server
    /// that says `assistant.greeting_weather`, which is false whenever it
    /// posts no greetings at all. ABSENT otherwise, not disabled: the
    /// footnote promises a forecast, and a list the server would keep and
    /// never use would promise something it cannot do.
    ///
    /// NOT bound to the family's own greeting switch. With that off the
    /// list is still shown and still editable — the server keeps it either
    /// way, the footnote already says it belongs to the daily greeting, and
    /// an owner may reasonably choose the places before turning it on.
    static func isOffered(isOwner: Bool, serverGreetingWeather: Bool) -> Bool {
        isOwner && serverGreetingWeather
    }

    /// Whether another place can be added — the editor shows "Add place"
    /// while this is true and "Up to 3 places." once it is not.
    static func canAdd(count: Int) -> Bool {
        count < maxPlaces
    }

    // MARK: - One name

    /// The name as the server will keep it: trimmed, every inner run of
    /// whitespace folded to one space.
    static func normalize(_ raw: String) -> String {
        var words: [String] = []
        var word = String.UnicodeScalarView()
        for scalar in raw.unicodeScalars {
            if scalar.properties.isWhitespace {
                if !word.isEmpty {
                    words.append(String(word))
                    word = String.UnicodeScalarView()
                }
            } else {
                word.append(scalar)
            }
        }
        if !word.isEmpty { words.append(String(word)) }
        return words.joined(separator: " ")
    }

    /// The length the server counts: Unicode scalars.
    static func length(_ name: String) -> Int {
        name.unicodeScalars.count
    }

    private static func isControl(_ scalar: Unicode.Scalar) -> Bool {
        scalar.properties.generalCategory == .control
    }

    /// The text a field may hold while it is being typed in.
    ///
    /// Control characters that are NOT whitespace are dropped — nobody types
    /// one, and a paste that carries one would otherwise come back as a
    /// refusal the owner cannot see the cause of. Whitespace is left alone,
    /// trailing space included, or the space between two words could never
    /// be typed; the fold happens on save.
    ///
    /// Then the text stops at the 80th character the server will count once
    /// it has folded the whitespace, so a pasted name with a run of spaces in
    /// it is not cut shorter than it needs to be.
    static func limitInput(_ raw: String) -> String {
        var kept = String.UnicodeScalarView()
        var count = 0
        var pendingSpace = false
        for scalar in raw.unicodeScalars {
            if scalar.properties.isWhitespace {
                if count > 0 { pendingSpace = true }
                kept.append(scalar)
                continue
            }
            if isControl(scalar) { continue }
            let needed = pendingSpace ? 2 : 1
            if count + needed > maxCharacters { break }
            count += needed
            pendingSpace = false
            kept.append(scalar)
        }
        return String(kept)
    }

    // MARK: - The list

    /// Why a list would be refused, by POSITION (1-based, among the fields
    /// as drawn) and never by name — the server's own messages do the same.
    enum Problem: Error, Equatable, Sendable {
        case controlCharacter(position: Int)
        case tooLong(position: Int)
        case tooMany
    }

    /// The list as the server will keep it, or why it would be refused.
    ///
    /// Empty fields are dropped (see the file comment); everything else is
    /// the server's rule, in the server's order.
    static func prepare(_ drafts: [String]) -> Result<[String], Problem> {
        var kept: [String] = []
        // Scalars, not Strings: a Set<String> would merge canonically
        // equivalent spellings the server keeps apart.
        var seen: Set<[Unicode.Scalar]> = []
        for (index, raw) in drafts.enumerated() {
            let position = index + 1
            let name = normalize(raw)
            if name.isEmpty { continue }
            if name.unicodeScalars.contains(where: isControl) {
                return .failure(.controlCharacter(position: position))
            }
            if length(name) > maxCharacters {
                return .failure(.tooLong(position: position))
            }
            let folded = Array(lowercased(name).unicodeScalars)
            if seen.contains(folded) { continue }
            seen.insert(folded)
            kept.append(name)
        }
        if kept.count > maxPlaces {
            return .failure(.tooMany)
        }
        return .success(kept)
    }

    /// Lower-cased scalar by scalar with each scalar's full Unicode mapping —
    /// what Rust's `char::to_lowercase` does. Rust's `str::to_lowercase`
    /// also applies the Greek final-sigma rule, which this does not; the two
    /// can disagree only on whether two Greek spellings are one place, and
    /// the server's answer, which the editor shows, settles that.
    static func lowercased(_ name: String) -> String {
        var out = ""
        for scalar in name.unicodeScalars {
            out += scalar.properties.lowercaseMapping
        }
        return out
    }

    /// Whether two lists hold the same names, scalar by scalar — the
    /// server's comparison, never Swift's canonical `==`, so a stored name
    /// retyped in another encoding is a change to send.
    static func same(_ a: [String], _ b: [String]) -> Bool {
        a.count == b.count && zip(a, b).allSatisfy { $0.unicodeScalars.elementsEqual($1.unicodeScalars) }
    }

    /// Optional form of `same`: no list is the same only as no list.
    static func same(_ a: [String], _ b: [String]?) -> Bool {
        guard let b else { return false }
        return same(a, b)
    }

    /// Does saving these drafts change anything? A list equal to what the
    /// server holds — the same names after the fold, repeats and empty
    /// fields aside — needs no request.
    static func needsSave(drafts: [String], stored: [String]) -> Bool {
        switch prepare(drafts) {
        case .success(let kept):
            return !same(kept, stored)
        case .failure:
            return true
        }
    }
}
