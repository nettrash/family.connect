//
//  ContactLinksTests.swift
//  FamilyConnectTests
//
//  The per-device member ↔ contact table over a throwaway defaults suite:
//  link, look up both ways, a contact moves when linked again, unlink,
//  and the wipe that goes with a logout.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Contact links")
struct ContactLinksTests {

    private func fresh() -> (ContactLinks, UserDefaults) {
        let name = "ContactLinksTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return (ContactLinks(defaults: defaults), defaults)
    }

    @Test("link, read back both ways, unlink")
    func roundTrip() {
        let (links, _) = fresh()
        #expect(links.all.isEmpty)
        #expect(links.link(for: 7) == nil)
        links.link(userID: 7, to: ContactLink(contactIdentifier: "C1", contactName: "Anna Smith"))
        #expect(links.link(for: 7) == ContactLink(contactIdentifier: "C1", contactName: "Anna Smith"))
        #expect(links.userID(linkedTo: "C1") == 7)
        #expect(links.userID(linkedTo: "C2") == nil)
        links.unlink(userID: 7)
        #expect(links.link(for: 7) == nil)
        #expect(links.all.isEmpty)
        // Unlinking nobody is fine.
        links.unlink(userID: 7)
    }

    @Test("one contact is one member: linking it again moves the link")
    func contactMoves() {
        let (links, _) = fresh()
        links.link(userID: 7, to: ContactLink(contactIdentifier: "C1", contactName: "Anna"))
        links.link(userID: 9, to: ContactLink(contactIdentifier: "C1", contactName: "Anna"))
        #expect(links.link(for: 7) == nil)
        #expect(links.userID(linkedTo: "C1") == 9)
        // And a member relinked to another contact drops the old one.
        links.link(userID: 9, to: ContactLink(contactIdentifier: "C2", contactName: "Anna S."))
        #expect(links.userID(linkedTo: "C1") == nil)
        #expect(links.link(for: 9)?.contactIdentifier == "C2")
    }

    @Test("the table survives a re-read and is wiped whole")
    func persistenceAndWipe() {
        let (links, defaults) = fresh()
        links.link(userID: 7, to: ContactLink(contactIdentifier: "C1", contactName: "Anna"))
        links.link(userID: 9, to: ContactLink(contactIdentifier: "C2", contactName: "Bob"))
        let again = ContactLinks(defaults: defaults)
        #expect(again.all.count == 2)
        #expect(again.link(for: 9)?.contactName == "Bob")
        again.removeAll()
        #expect(links.all.isEmpty)
        #expect(defaults.data(forKey: ContactLinks.key) == nil)
    }

    @Test("numbers match by digits and national tail; e-mails by case-folded equality")
    func matching() {
        let (links, _) = fresh()
        links.link(userID: 7, to: ContactLink(
            contactIdentifier: "C1", contactName: "Anna",
            phoneNumbers: ["+1 (555) 123-4567", "+381 64 123 456"],
            emailAddresses: ["Anna@Example.com"]))
        #expect(links.userID(matchingPhone: "+15551234567") == 7)
        #expect(links.userID(matchingPhone: "555-123-4567") == 7, "the national spelling ends the international one")
        #expect(links.userID(matchingPhone: "064123456") == 7)
        #expect(links.userID(matchingPhone: "+15551234568") == nil)
        #expect(links.userID(matchingPhone: "4567") == nil, "too short to mean anything")
        #expect(links.userID(matchingEmail: "anna@example.com") == 7)
        #expect(links.userID(matchingEmail: " ANNA@EXAMPLE.COM ") == 7)
        #expect(links.userID(matchingEmail: "ann@example.com") == nil)
        #expect(ContactLink.phonesMatch("123", "123"))
        #expect(!ContactLink.phonesMatch("", ""))
    }

    @Test("a number two linked contacts share is nobody's answer — the router asks instead")
    func sharedNumber() {
        let (links, _) = fresh()
        links.link(userID: 7, to: ContactLink(contactIdentifier: "C1", contactName: "Mum", phoneNumbers: ["+1 555 100 0000", "+1 555 777 7777"], emailAddresses: ["home@example.com"]))
        links.link(userID: 9, to: ContactLink(contactIdentifier: "C2", contactName: "Dad", phoneNumbers: ["+1 555 100 0000"], emailAddresses: ["home@example.com"]))
        #expect(links.userID(matchingPhone: "+15551000000") == nil, "the landline is both of them")
        #expect(links.userID(matchingEmail: "home@example.com") == nil)
        #expect(links.userID(matchingPhone: "+15557777777") == 7, "Mum's own number is still hers")
    }

    @Test("a link written before numbers existed still reads")
    func decodesOldShape() throws {
        let (links, defaults) = fresh()
        let old = Data(#"{"7":{"contactIdentifier":"C1","contactName":"Anna"}}"#.utf8)
        defaults.set(old, forKey: ContactLinks.key)
        #expect(links.link(for: 7) == ContactLink(contactIdentifier: "C1", contactName: "Anna"))
        #expect(links.link(for: 7)?.phoneNumbers.isEmpty == true)
    }

    @Test("garbage under the key reads as no links")
    func garbage() {
        let (links, defaults) = fresh()
        defaults.set(Data("not json".utf8), forKey: ContactLinks.key)
        #expect(links.all.isEmpty)
        defaults.set("a string", forKey: ContactLinks.key)
        #expect(links.all.isEmpty)
    }
}
