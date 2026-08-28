//
//  CallHandle.swift
//  FamilyConnect
//
//  A member's identity as a string the system can hand back: the USER
//  ID under the app's URL scheme. Never reused (docs/protocol.md,
//  "Deleting an account") — the username is released for somebody else
//  to register, and a display name is no identifier at all.
//
//  Where it is used: as the person Siri resolves (CallIntentHandler puts
//  it in the INPerson's custom identifier, and it comes back verbatim
//  when the system continues the intent in the app) and in the donations
//  that feed Siri's suggestions (CallDonation). It is NOT what CallKit
//  shows: a `.generic` handle's raw value is what a Recents row displays,
//  so CallKitController reports a linked contact's number or, failing
//  that, the display name — see `handle(peerName:link:)` there — and
//  CallRequestRouter turns either back into a member.
//

import Foundation

nonisolated enum CallHandle {

    /// The app's URL scheme (Info.plist), reused as the handle namespace.
    static let scheme = "familyconnect"

    /// `familyconnect:<user id>`.
    static func value(userID: Int64) -> String {
        "\(scheme):\(userID)"
    }

    /// The member a handle names, or nil for anything that is not one of
    /// ours: a phone number the system passed along, an old row from
    /// before handles were ids, noise. Tolerant of `familyconnect://7`
    /// and case, strict about the number.
    static func userID(from value: String) -> Int64? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard trimmed.hasPrefix(scheme + ":") else { return nil }
        var rest = trimmed.dropFirst(scheme.count + 1)
        while rest.hasPrefix("/") { rest = rest.dropFirst() }
        guard let id = Int64(rest), id > 0 else { return nil }
        return id
    }
}
