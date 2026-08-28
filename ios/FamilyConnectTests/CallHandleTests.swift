//
//  CallHandleTests.swift
//  FamilyConnectTests
//
//  The system's handle for a member: their id in the app's namespace,
//  parsed back tolerantly and never from a name, a number or noise.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Call handle")
struct CallHandleTests {

    @Test("a member's handle is their id under the app's scheme")
    func value() {
        #expect(CallHandle.value(userID: 7) == "familyconnect:7")
        #expect(CallHandle.value(userID: 123_456_789) == "familyconnect:123456789")
    }

    @Test("round trip, with tolerance for case, whitespace and a URL-style spelling")
    func parse() {
        #expect(CallHandle.userID(from: "familyconnect:7") == 7)
        #expect(CallHandle.userID(from: CallHandle.value(userID: 42)) == 42)
        #expect(CallHandle.userID(from: " FamilyConnect:7 ") == 7)
        #expect(CallHandle.userID(from: "familyconnect://7") == 7)
    }

    @Test("anything that is not one of ours parses as nobody")
    func rejects() {
        #expect(CallHandle.userID(from: "Anna") == nil, "the old display-name handle")
        #expect(CallHandle.userID(from: "+15551234567") == nil)
        #expect(CallHandle.userID(from: "anna@example.com") == nil)
        #expect(CallHandle.userID(from: "familyconnect:") == nil)
        #expect(CallHandle.userID(from: "familyconnect:unknown") == nil)
        #expect(CallHandle.userID(from: "familyconnect:0") == nil)
        #expect(CallHandle.userID(from: "familyconnect:-3") == nil)
        #expect(CallHandle.userID(from: "familyconnect:7x") == nil)
        #expect(CallHandle.userID(from: "otherapp:7") == nil)
        #expect(CallHandle.userID(from: "") == nil)
    }
}
