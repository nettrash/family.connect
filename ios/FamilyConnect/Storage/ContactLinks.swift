//
//  ContactLinks.swift
//  FamilyConnect
//
//  Which device contact a family member IS, on this device: the link the
//  person makes from the roster ("Link to a Contact…") so the Phone app's
//  Favorites and a contact card's call buttons can reach them through
//  Family. Family members have no phone numbers on the wire — by design —
//  so this is the only bridge between an address-book entry and a member,
//  and it lives entirely here: nothing about a contact ever leaves the
//  phone, and the app needs no Contacts permission to make the link (the
//  system picker hands over just the one contact the person chose).
//
//  Per device on purpose. A CNContact identifier is only meaningful on
//  the device that issued it, so there is nothing that COULD be synced;
//  a second phone links again. Account-scoped like the board cursor:
//  user ids belong to one server, so a logout clears the table.
//
//  Kept as JSON in UserDefaults beside AppSettings — a handful of entries,
//  read on every call the system starts and on every roster draw.
//

import Foundation

nonisolated struct ContactLink: Codable, Equatable, Sendable {
    /// `CNContact.identifier`.
    let contactIdentifier: String
    /// The contact's name at the time of linking, for the roster caption.
    let contactName: String
    /// The contact's phone numbers and e-mails as the card spells them,
    /// first one first. Two jobs: the system hands the app a NUMBER when a
    /// contact-card or Favorites button is tapped (the contact identifier
    /// is not promised), so a call request is matched against these; and
    /// the first number is what the app reports as the call's handle, so
    /// the Recents row carries the contact's name instead of a raw value.
    var phoneNumbers: [String] = []
    var emailAddresses: [String] = []

    init(contactIdentifier: String, contactName: String, phoneNumbers: [String] = [], emailAddresses: [String] = []) {
        self.contactIdentifier = contactIdentifier
        self.contactName = contactName
        self.phoneNumbers = phoneNumbers
        self.emailAddresses = emailAddresses
    }

    /// The two lists are optional on disk, so a link written before they
    /// existed still reads.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        contactIdentifier = try container.decode(String.self, forKey: .contactIdentifier)
        contactName = try container.decodeIfPresent(String.self, forKey: .contactName) ?? ""
        phoneNumbers = try container.decodeIfPresent([String].self, forKey: .phoneNumbers) ?? []
        emailAddresses = try container.decodeIfPresent([String].self, forKey: .emailAddresses) ?? []
    }

    /// Digits only, so "+1 (555) 123-4567" and "5551234567" agree. A short
    /// number never matches a long one's tail by accident: both sides
    /// have to be at least seven digits, and the shorter must end the
    /// longer (a national number against its international spelling).
    static func phonesMatch(_ a: String, _ b: String) -> Bool {
        let da = digits(a), db = digits(b)
        guard da.count >= 7, db.count >= 7 else { return da == db && !da.isEmpty }
        return da.hasSuffix(db) || db.hasSuffix(da)
    }

    static func digits(_ number: String) -> String {
        // Leading zeros dropped: a trunk 0 ("064 123 456") is never part
        // of the international number ("+381 64 123 456").
        String(number.filter(\.isNumber).drop(while: { $0 == "0" }))
    }

    static func emailsMatch(_ a: String, _ b: String) -> Bool {
        let fa = a.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let fb = b.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return !fa.isEmpty && fa == fb
    }
}

nonisolated struct ContactLinks {

    static let key = "v1.contactLinks"
    static let shared = ContactLinks(defaults: .standard)

    private let defaults: UserDefaults

    init(defaults: UserDefaults) {
        self.defaults = defaults
    }

    /// Every link, keyed by member user id.
    var all: [Int64: ContactLink] {
        guard let data = defaults.data(forKey: Self.key),
              let decoded = try? JSONDecoder().decode([String: ContactLink].self, from: data)
        else { return [:] }
        var links: [Int64: ContactLink] = [:]
        for (key, link) in decoded {
            if let id = Int64(key) { links[id] = link }
        }
        return links
    }

    func link(for userID: Int64) -> ContactLink? {
        all[userID]
    }

    /// The member a contact is linked to, if any. One contact can only be
    /// one member — linking a contact to a second member moves the link.
    func userID(linkedTo contactIdentifier: String) -> Int64? {
        unique { $0.contactIdentifier == contactIdentifier }
    }

    /// The member whose linked contact carries this number — and ONLY
    /// that member. Two parents on one landline are two links carrying
    /// the same number; a number that fits both is nobody's answer here
    /// (the router asks instead), never whichever the table happened to
    /// list first.
    func userID(matchingPhone number: String) -> Int64? {
        unique { $0.phoneNumbers.contains { ContactLink.phonesMatch($0, number) } }
    }

    /// The member whose linked contact carries this e-mail; unique, as above.
    func userID(matchingEmail email: String) -> Int64? {
        unique { $0.emailAddresses.contains { ContactLink.emailsMatch($0, email) } }
    }

    private func unique(_ matches: (ContactLink) -> Bool) -> Int64? {
        let hits = all.filter { matches($0.value) }
        return hits.count == 1 ? hits.keys.first : nil
    }

    func link(userID: Int64, to link: ContactLink) {
        var links = all
        for (id, existing) in links where existing.contactIdentifier == link.contactIdentifier {
            links.removeValue(forKey: id)
        }
        links[userID] = link
        write(links)
    }

    func unlink(userID: Int64) {
        var links = all
        guard links.removeValue(forKey: userID) != nil else { return }
        write(links)
    }

    func removeAll() {
        defaults.removeObject(forKey: Self.key)
    }

    private func write(_ links: [Int64: ContactLink]) {
        if links.isEmpty {
            defaults.removeObject(forKey: Self.key)
            return
        }
        let encoded = Dictionary(uniqueKeysWithValues: links.map { (String($0.key), $0.value) })
        if let data = try? JSONEncoder().encode(encoded) {
            defaults.set(data, forKey: Self.key)
        }
    }
}
