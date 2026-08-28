//
//  CallIntentsTests.swift
//  FamilyConnectTests
//
//  The two strings the system keeps and hands back, pinned on the
//  simulator: what CallKit is told as the handle of an outgoing call
//  (a linked contact's number, an e-mail, or the display name), and how
//  the person the system names — customIdentifier, handle type, contact
//  identifier — turns into a CallRequest. iPhone only, like the code.
//

#if os(iOS)

import CallKit
import Foundation
import Intents
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Call intents")
struct CallIntentsTests {

    @Test("an outgoing handle is the linked contact's number, else e-mail, else the display name")
    func outgoingHandle() {
        let linked = ContactLink(contactIdentifier: "C1", contactName: "Anna Smith",
                                 phoneNumbers: ["+1 (555) 123-4567", "+1 555 000 0000"], emailAddresses: ["anna@example.com"])
        let phone = CallKitController.handle(peerName: "Anna", link: linked)
        #expect(phone.type == .phoneNumber)
        #expect(phone.value == "+1 (555) 123-4567", "the first number, as the card spells it")

        let emailOnly = ContactLink(contactIdentifier: "C2", contactName: "Bob", emailAddresses: ["bob@example.com"])
        let email = CallKitController.handle(peerName: "Bob", link: emailOnly)
        #expect(email.type == .emailAddress)
        #expect(email.value == "bob@example.com")

        let bare = ContactLink(contactIdentifier: "C3", contactName: "Nobody", phoneNumbers: [""], emailAddresses: [""])
        let name = CallKitController.handle(peerName: "Junior", link: bare)
        #expect(name.type == .generic)
        #expect(name.value == "Junior", "empty entries do not count")

        let unlinked = CallKitController.handle(peerName: "Zoë", link: nil)
        #expect(unlinked.type == .generic)
        #expect(unlinked.value == "Zoë")
    }

    @Test("a person Siri resolved (our custom identifier) is our handle, whatever else they carry")
    func siriPerson() {
        let person = INPerson(
            personHandle: INPersonHandle(value: "+15551234567", type: .phoneNumber),
            nameComponents: nil, displayName: "Anna", image: nil,
            contactIdentifier: "C1", customIdentifier: CallHandle.value(userID: 7))
        let request = CallRequest(person: person, video: true)
        #expect(request.handle == .generic("familyconnect:7"))
        #expect(request.contactIdentifier == "C1")
        #expect(request.contactName == "Anna")
        #expect(request.video)
    }

    @Test("a person the Phone app named carries the number or e-mail as such, and the contact")
    func systemPerson() {
        let byPhone = INPerson(
            personHandle: INPersonHandle(value: "+1 (555) 123-4567", type: .phoneNumber),
            nameComponents: nil, displayName: "Anna Smith", image: nil, contactIdentifier: "C1", customIdentifier: nil)
        #expect(CallRequest(person: byPhone, video: false).handle == .phoneNumber("+1 (555) 123-4567"))
        #expect(CallRequest(person: byPhone, video: false).contactIdentifier == "C1")

        let byEmail = INPerson(
            personHandle: INPersonHandle(value: "anna@example.com", type: .emailAddress),
            nameComponents: nil, displayName: "Anna", image: nil, contactIdentifier: nil, customIdentifier: nil)
        #expect(CallRequest(person: byEmail, video: false).handle == .emailAddress("anna@example.com"))

        // A Recents row of an unlinked member: the display name, generic.
        let byName = INPerson(
            personHandle: INPersonHandle(value: "Junior", type: .unknown),
            nameComponents: nil, displayName: "Junior", image: nil, contactIdentifier: nil, customIdentifier: nil)
        #expect(CallRequest(person: byName, video: false).handle == .generic("Junior"))

        // A foreign custom identifier is not ours: the handle decides.
        let foreign = INPerson(
            personHandle: INPersonHandle(value: "+15550000000", type: .phoneNumber),
            nameComponents: nil, displayName: "X", image: nil, contactIdentifier: nil, customIdentifier: "otherapp:9")
        #expect(CallRequest(person: foreign, video: false).handle == .phoneNumber("+15550000000"))

        #expect(CallRequest(person: nil, video: false).handle == nil)
    }

    @Test("the activity types are the three intent class names, in the order the plist lists them")
    func activityTypes() {
        #expect(CallRequest.activityTypes == ["INStartCallIntent", "INStartAudioCallIntent", "INStartVideoCallIntent"])
        let declared = Bundle.main.object(forInfoDictionaryKey: "NSUserActivityTypes") as? [String]
        // The host app's plist when the tests run in it; absent under a bare
        // test host, which is not a failure.
        if let declared {
            #expect(Set(declared).isSuperset(of: CallRequest.activityTypes))
        }
    }
}

#endif
