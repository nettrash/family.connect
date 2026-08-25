//
//  FamilyLanguage.swift
//  FamilyConnect
//
//  The nine languages a family may declare, spelled exactly as the
//  protocol spells them (protocol.md, "The family's language").
//
//  A fixed list rather than any well-formed BCP 47 tag, because the only
//  thing that reads this is the assistant: a family that typed something
//  the server merely accepted would get answers they could not explain.
//  Two of the nine name a SCRIPT — `sr` and `sr-Latn` are one language in
//  two alphabets, and a family that reads Cyrillic cannot read the answer
//  that comes back in the other one — so nothing here may "simplify" a tag
//  down to its language.
//
//  Each is named in ITS OWN language, the way every OS language picker
//  does it: somebody looking for their language scans for the word they
//  recognise, not for its English name. That is also why these names are
//  NOT catalogue strings — "Русский" is "Русский" in all nine locales, and
//  putting it through the catalogue would ask eight translators to retype
//  it.
//
//  Android counterpart: ui/settings/FamilyLanguage.kt
//

import Foundation

nonisolated struct FamilyLanguage: Identifiable, Equatable, Sendable {
    /// The canonical tag, which is what the server always sends back.
    let tag: String
    /// The language's name in itself.
    let name: String

    var id: String { tag }

    /// The nine, in the order a picker shows them.
    static let all: [FamilyLanguage] = [
        FamilyLanguage(tag: "en", name: "English"),
        FamilyLanguage(tag: "de", name: "Deutsch"),
        FamilyLanguage(tag: "es", name: "Español"),
        FamilyLanguage(tag: "fr", name: "Français"),
        FamilyLanguage(tag: "ja", name: "日本語"),
        FamilyLanguage(tag: "ru", name: "Русский"),
        FamilyLanguage(tag: "sr", name: "Српски"),
        FamilyLanguage(tag: "sr-Latn", name: "Srpski (latinica)"),
        FamilyLanguage(tag: "zh-Hans", name: "简体中文"),
    ]

    /// The one a tag names, if any. Case-insensitive because casing is not
    /// significant going IN — `sr-latn` is the same choice as `sr-Latn` —
    /// and a family set from another client may have been stored before the
    /// server canonicalised it.
    static func named(_ tag: String?) -> FamilyLanguage? {
        guard let tag else { return nil }
        return all.first { $0.tag.caseInsensitiveCompare(tag) == .orderedSame }
    }
}
