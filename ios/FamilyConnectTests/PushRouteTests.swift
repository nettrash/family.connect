//
//  PushRouteTests.swift
//  FamilyConnectTests
//
//  Golden tests for the payload → route table against the exact payload
//  shapes documented in docs/protocol.md §Push notifications, plus the
//  tolerance cases the parser promises: chat_id as NSNumber (how APNs
//  surfaces our server's JSON numbers) or String, and unknown kinds
//  degrading to the chat list instead of dropping the tap.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("PushRoute parser")
struct PushRouteTests {

    /// The documented message payload, verbatim from protocol.md.
    private static let messagePayload: [AnyHashable: Any] = [
        "aps": [
            "alert": ["title": "Anna", "body": "Dinner at 7?"],
            "sound": "default",
            "badge": 3,
            "thread-id": "chat-42",
        ] as [String: Any],
        "chat_id": 42,
        "message_id": 1338,
        "kind": "message",
    ]

    @Test("kind message → chat(chat_id)")
    func message() {
        #expect(PushRoute.parse(userInfo: Self.messagePayload) == .chat(42))
    }

    @Test("chat_id tolerates Int64, NSNumber and String forms")
    func chatIDTolerance() {
        let forms: [Any] = [Int64(42), NSNumber(value: 42), "42"]
        for value in forms {
            var payload = Self.messagePayload
            payload["chat_id"] = value
            #expect(PushRoute.parse(userInfo: payload) == .chat(42))
        }
    }

    @Test("kind join_request → joinRequests (family_id ignored)")
    func joinRequest() {
        let payload: [AnyHashable: Any] = [
            "aps": [
                "alert": ["title": "The Smiths", "body": "junior asked to join"],
                "sound": "default",
            ] as [String: Any],
            "kind": "join_request",
            "family_id": 3,
        ]
        #expect(PushRoute.parse(userInfo: payload) == .joinRequests)
    }

    @Test("kind joined → chatList")
    func joined() {
        let payload: [AnyHashable: Any] = ["kind": "joined", "family_id": 3]
        #expect(PushRoute.parse(userInfo: payload) == .chatList)
    }

    @Test("unknown and missing kinds → chatList, never a dropped tap")
    func unknownKind() {
        #expect(PushRoute.parse(userInfo: ["kind": "call_invite"]) == .chatList)
        #expect(PushRoute.parse(userInfo: ["aps": ["alert": "hi"]]) == .chatList)
        #expect(PushRoute.parse(userInfo: [:]) == .chatList)
    }

    @Test("message without a parseable chat_id degrades to chatList")
    func malformedChatID() {
        var payload = Self.messagePayload
        payload["chat_id"] = nil
        #expect(PushRoute.parse(userInfo: payload) == .chatList)
        payload["chat_id"] = "not-a-number"
        #expect(PushRoute.parse(userInfo: payload) == .chatList)
    }
}
