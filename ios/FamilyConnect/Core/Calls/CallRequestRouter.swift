//
//  CallRequestRouter.swift
//  FamilyConnect
//
//  A call the SYSTEM asked for — a Recents row tapped, "Family video call"
//  on a contact card or in the Phone app's Favorites, Siri told to call
//  somebody on Family — reduced to what the app can act on. The request
//  arrives as an intent (CallIntents.swift turns it into a CallRequest);
//  this decides who is meant, from four kinds of evidence, in order:
//
//    1. Our own handle (`familyconnect:<id>`, CallHandle) — a person Siri
//       already resolved. Exact.
//    2. The device contact the system named, or the number / e-mail it
//       passed along (a Favorites entry, a contact-card button — the
//       identifier is not promised, the number is) — looked up in the
//       per-device link table (ContactLinks).
//    3. A name: what the handle of an UNLINKED member is (the Recents row
//       shows the handle's raw value, so it is the name), matched against
//       the roster the way Siri's spoken name is. One hit calls; two
//       "Anna"s ask.
//    4. Otherwise: a device contact nobody is linked to yet is the moment
//       to ask "which family member is this?" (and keep the answer); a
//       bare number with no contact behind it is honestly unknown —
//       Family has no phone numbers on the wire to match it against.
//
//  Plus the name matching Siri needs: a spoken name against the roster,
//  where two "Anna"s are Siri's disambiguation to run and not a guess.
//
//  Pure, so every branch is pinned on the simulator (CallRequestRouterTests)
//  — the intent framework itself is not testable there.
//

import Foundation

/// What the system told us about the call it wants placed.
nonisolated struct CallRequest: Equatable, Sendable {
    enum Handle: Equatable, Sendable {
        case generic(String)
        case phoneNumber(String)
        case emailAddress(String)
    }

    var handle: Handle?
    /// `CNContact.identifier` of the device contact the request came from.
    var contactIdentifier: String?
    /// What the system called them — for the "who is this?" prompt.
    var contactName: String?
    var video: Bool
}

nonisolated enum CallRequestRouter {

    enum Resolution: Equatable, Sendable {
        case member(Int64)
        /// The request names somebody the caller has BLOCKED. Not
        /// `.unknown`: the app knows exactly who this is, and saying it
        /// does not would send the blocker to re-link a contact that is
        /// already linked (docs/protocol.md, "Calls"). The refusal reaches
        /// nobody but the blocker, so it can be said plainly.
        case blocked(Int64)
        /// The person has to say who is meant. With a contact identifier
        /// the answer is kept as a link (ContactLinks); without one — a
        /// name two members share — it is just this call's choice.
        case needsChoice(contactIdentifier: String?, name: String?)
        case unknown
    }

    /// What the router consults: the roster's gate and the link table.
    /// `isActiveMember` is the member pickers' rule — not me, not left,
    /// not deleted, never the assistant — so an OS handle that outlived a
    /// membership cannot ring somebody who is gone.
    struct Directory {
        var isActiveMember: (Int64) -> Bool
        /// Separate from `isActiveMember` on purpose. A blocked member is
        /// still active — still on the roster, still nameable, still the
        /// author of messages that need a name — so folding the two would
        /// make every other caller of the roster gate wrong.
        var isBlocked: (Int64) -> Bool
        var roster: () -> [Candidate]
        var linkedMember: (_ contactIdentifier: String) -> Int64?
        var memberByPhone: (String) -> Int64?
        var memberByEmail: (String) -> Int64?
    }

    static func resolve(_ request: CallRequest, in directory: Directory) -> Resolution {
        let active = { (id: Int64?) -> Int64? in id.flatMap { directory.isActiveMember($0) ? $0 : nil } }
        // Every road out of here goes through this, so a member identified
        // by ANY of the four kinds of evidence gets the same answer.
        let reached = { (id: Int64) -> Resolution in
            directory.isBlocked(id) ? .blocked(id) : .member(id)
        }
        // 1. Ours.
        if case .generic(let value)? = request.handle, let id = CallHandle.userID(from: value) {
            return active(id).map(reached) ?? .unknown
        }
        // 2. The contact, or its number / e-mail, through the links.
        if let contact = request.contactIdentifier, !contact.isEmpty, let id = active(directory.linkedMember(contact)) {
            return reached(id)
        }
        switch request.handle {
        case .phoneNumber(let number)?:
            if let id = active(directory.memberByPhone(number)) { return reached(id) }
        case .emailAddress(let email)?:
            if let id = active(directory.memberByEmail(email)) { return reached(id) }
        case .generic(let name)?:
            // 3. A name — the handle of an unlinked member.
            let roster = directory.roster().filter { directory.isActiveMember($0.userID) }
            switch match(name: name, in: roster) {
            case .one(let member): return reached(member.userID)
            case .several(let members):
                // "Anna" with two Annas is Siri's question to ask — unless
                // blocking has already answered it. One callable Anna left
                // is not ambiguous, and none left is a block rather than a
                // choice (docs/protocol.md, "Calls").
                let callable = members.filter { !directory.isBlocked($0.userID) }
                if callable.count == 1 { return .member(callable[0].userID) }
                if let onlyBlocked = callable.isEmpty ? members.first : nil {
                    return .blocked(onlyBlocked.userID)
                }
                return .needsChoice(contactIdentifier: request.contactIdentifier, name: name)
            case .none: break
            }
        case nil:
            break
        }
        // 4. Ask, when there is a contact to remember the answer for.
        if let contact = request.contactIdentifier, !contact.isEmpty {
            return .needsChoice(contactIdentifier: contact, name: request.contactName)
        }
        return .unknown
    }

    // MARK: - Names (Siri)

    struct Candidate: Equatable, Sendable {
        let userID: Int64
        let name: String
    }

    enum NameMatch: Equatable, Sendable {
        case none
        case one(Candidate)
        case several([Candidate])
    }

    /// A spoken or typed name against the roster. Whole-name equality
    /// wins outright; otherwise a name that starts a member's name or one
    /// of its words ("Anna" for "Anna Smith", "Smith" for the same) — so
    /// that "Anna" with two Annas is `.several`, which Siri turns into a
    /// question rather than a wrong call. Diacritics and case are folded
    /// (Siri hears "Zoe" for "Zoë").
    static func match(name: String, in roster: [Candidate]) -> NameMatch {
        let wanted = fold(name)
        guard !wanted.isEmpty else { return .none }
        let exact = roster.filter { fold($0.name) == wanted }
        let hits = exact.isEmpty ? roster.filter { startsAWord(of: fold($0.name), with: wanted) } : exact
        switch hits.count {
        case 0: return .none
        case 1: return .one(hits[0])
        default: return .several(hits)
        }
    }

    private static func fold(_ text: String) -> String {
        text.folding(options: [.diacriticInsensitive, .caseInsensitive, .widthInsensitive], locale: nil)
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func startsAWord(of name: String, with wanted: String) -> Bool {
        if name.hasPrefix(wanted) { return true }
        return name.split(whereSeparator: { $0.isWhitespace || $0 == "-" })
            .contains { $0.hasPrefix(wanted) }
    }
}
