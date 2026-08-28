//
//  CallRequestRouterTests.swift
//  FamilyConnectTests
//
//  Who a system call request means: our handle wins, a linked contact
//  (by identifier, number or e-mail) resolves, a name matches the roster
//  the way Siri's does, an unlinked contact asks, a bare number is
//  unknown — and every answer passes through the roster gate.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Call request router")
struct CallRequestRouterTests {

    private let roster = [
        CallRequestRouter.Candidate(userID: 7, name: "Anna Smith"),
        CallRequestRouter.Candidate(userID: 9, name: "Anna-Maria Jones"),
        CallRequestRouter.Candidate(userID: 11, name: "Zoë"),
        CallRequestRouter.Candidate(userID: 12, name: "Junior"),
        // Left the family: in the roster table, not active.
        CallRequestRouter.Candidate(userID: 3, name: "Old Uncle"),
    ]
    private let active: Set<Int64> = [7, 9, 11, 12]

    private var directory: CallRequestRouter.Directory {
        CallRequestRouter.Directory(
            isActiveMember: { self.active.contains($0) },
            roster: { self.roster },
            linkedMember: { ["contact-anna": 7, "contact-gone": 3][$0] },
            memberByPhone: { number in ContactLink.phonesMatch(number, "+1 555 123 4567") ? 7 : (ContactLink.phonesMatch(number, "+381 64 000 0000") ? 3 : nil) },
            memberByEmail: { $0.lowercased() == "anna@example.com" ? 7 : nil })
    }

    private func request(_ handle: CallRequest.Handle?, contact: String? = nil, name: String? = nil, video: Bool = false) -> CallRequest {
        CallRequest(handle: handle, contactIdentifier: contact, contactName: name, video: video)
    }

    @Test("our own handle resolves the member directly")
    func ownHandle() {
        #expect(CallRequestRouter.resolve(request(.generic("familyconnect:7")), in: directory) == .member(7))
    }

    @Test("our handle for somebody who left is unknown, not a call — and not a question")
    func ownHandleGone() {
        #expect(CallRequestRouter.resolve(request(.generic("familyconnect:3"), contact: "contact-gone"), in: directory) == .unknown)
    }

    @Test("a linked contact resolves by identifier, by number and by e-mail")
    func linkedContact() {
        #expect(CallRequestRouter.resolve(request(.phoneNumber("+15551234567"), contact: "contact-anna", name: "Anna Smith", video: true), in: directory) == .member(7))
        #expect(CallRequestRouter.resolve(request(.phoneNumber("(555) 123-4567")), in: directory) == .member(7), "the number alone, no identifier")
        #expect(CallRequestRouter.resolve(request(.emailAddress("Anna@Example.com")), in: directory) == .member(7))
    }

    @Test("a linked contact whose member is gone asks again rather than ringing them")
    func linkedToGone() {
        #expect(CallRequestRouter.resolve(request(.phoneNumber("+381640000000"), contact: "contact-gone", name: "Old"), in: directory)
                == .needsChoice(contactIdentifier: "contact-gone", name: "Old"))
    }

    @Test("an unlinked contact asks who they are — with the name the system gave")
    func unlinkedContact() {
        #expect(CallRequestRouter.resolve(request(.phoneNumber("+15550000000"), contact: "contact-new", name: "Bob"), in: directory)
                == .needsChoice(contactIdentifier: "contact-new", name: "Bob"))
    }

    @Test("a name handle — an unlinked member's Recents row — matches the roster")
    func nameHandle() {
        #expect(CallRequestRouter.resolve(request(.generic("Junior")), in: directory) == .member(12))
        #expect(CallRequestRouter.resolve(request(.generic("Anna Smith")), in: directory) == .member(7))
        #expect(CallRequestRouter.resolve(request(.generic("zoe")), in: directory) == .member(11))
        // Two Annas: the person chooses for this call; nothing to remember it for.
        #expect(CallRequestRouter.resolve(request(.generic("Anna")), in: directory) == .needsChoice(contactIdentifier: nil, name: "Anna"))
        // A name that left the family is nobody.
        #expect(CallRequestRouter.resolve(request(.generic("Old Uncle")), in: directory) == .unknown)
    }

    @Test("a bare number or e-mail with no contact and no link is unknown; so is nothing at all")
    func bare() {
        #expect(CallRequestRouter.resolve(request(.phoneNumber("+15550000000")), in: directory) == .unknown)
        #expect(CallRequestRouter.resolve(request(.emailAddress("nobody@example.com")), in: directory) == .unknown)
        #expect(CallRequestRouter.resolve(request(nil), in: directory) == .unknown)
        #expect(CallRequestRouter.resolve(request(nil, contact: ""), in: directory) == .unknown)
    }

    // MARK: - Names (Siri)

    @Test("a whole name matches one member; a first name shared by two asks")
    func names() {
        #expect(CallRequestRouter.match(name: "Anna Smith", in: roster) == .one(roster[0]))
        #expect(CallRequestRouter.match(name: "anna smith", in: roster) == .one(roster[0]))
        #expect(CallRequestRouter.match(name: "Anna", in: roster) == .several([roster[0], roster[1]]))
        #expect(CallRequestRouter.match(name: "Junior", in: roster) == .one(roster[3]))
        #expect(CallRequestRouter.match(name: "Jun", in: roster) == .one(roster[3]))
        #expect(CallRequestRouter.match(name: "Smith", in: roster) == .one(roster[0]), "a surname is a word too")
        #expect(CallRequestRouter.match(name: "Maria", in: roster) == .one(roster[1]), "a hyphenated part is a word")
    }

    @Test("diacritics and case are folded; nothing matches nobody")
    func folding() {
        #expect(CallRequestRouter.match(name: "zoe", in: roster) == .one(roster[2]))
        #expect(CallRequestRouter.match(name: "ZOË", in: roster) == .one(roster[2]))
        #expect(CallRequestRouter.match(name: "Bob", in: roster) == .none)
        #expect(CallRequestRouter.match(name: "   ", in: roster) == .none)
        #expect(CallRequestRouter.match(name: "Anna", in: []) == .none)
    }
}
