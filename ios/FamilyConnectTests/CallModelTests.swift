//
//  CallModelTests.swift
//  FamilyConnectTests
//
//  The pure pieces of voice calls: the record's wording, the in-call
//  status line, the socket-hold rule, the VoIP payload parser, the wire
//  shapes of the record and of `calls_enabled`, and the chat-list preview.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Call records and wording")
struct CallModelTests {

    // MARK: - Wording

    @Test("the record's wording by outcome and side")
    func recordWording() {
        let completed = CallDTO(outcome: "completed", durationSecs: 222)
        #expect(CallRecordText.label(completed, isMine: true) == "Voice call · 3:42")
        #expect(CallRecordText.label(completed, isMine: false) == "Voice call · 3:42")

        let missed = CallDTO(outcome: "missed")
        #expect(CallRecordText.label(missed, isMine: true) == "No answer")
        #expect(CallRecordText.label(missed, isMine: false) == "Missed voice call")

        let declined = CallDTO(outcome: "declined")
        #expect(CallRecordText.label(declined, isMine: true) == "Voice call declined")
        #expect(CallRecordText.label(declined, isMine: false) == "Declined voice call")

        #expect(CallRecordText.label(CallDTO(outcome: "failed"), isMine: true) == "Call failed")
        #expect(CallRecordText.label(CallDTO(outcome: "failed", durationSecs: 61), isMine: false) == "Call failed · 1:01")
        // The render floor: an outcome this build does not know is still a call.
        #expect(CallRecordText.label(CallDTO(outcome: "hologram"), isMine: false) == "Voice call")
    }

    @Test("durations count in m:ss, and h:mm:ss past an hour")
    func durations() {
        #expect(CallRecordText.duration(0) == "0:00")
        #expect(CallRecordText.duration(7) == "0:07")
        #expect(CallRecordText.duration(222) == "3:42")
        #expect(CallRecordText.duration(3661) == "1:01:01")
        #expect(CallRecordText.duration(-5) == "0:00")
    }

    @Test("the status line follows the phase, and the end reason the direction")
    func statusLine() {
        #expect(CallStatusText.line(phase: .outgoing(ringing: false), direction: .outgoing, elapsed: 0) == "Calling…")
        #expect(CallStatusText.line(phase: .outgoing(ringing: true), direction: .outgoing, elapsed: 0) == "Ringing…")
        #expect(CallStatusText.line(phase: .incoming, direction: .incoming, elapsed: 0) == "Incoming call")
        #expect(CallStatusText.line(phase: .connecting, direction: .incoming, elapsed: 0) == "Connecting…")
        #expect(CallStatusText.line(phase: .active(since: Date()), direction: .outgoing, elapsed: 65) == "1:05")
        #expect(CallStatusText.ended(.decline, direction: .outgoing) == "Declined")
        #expect(CallStatusText.ended(.decline, direction: .incoming) == "Call ended")
        #expect(CallStatusText.ended(.timeout, direction: .outgoing) == "No answer")
        #expect(CallStatusText.ended(.timeout, direction: .incoming) == "Missed voice call")
        #expect(CallStatusText.ended(.answeredElsewhere, direction: .incoming) == "Answered on another device")
        #expect(CallStatusText.ended(.busy, direction: .outgoing) == "Busy")
        #expect(CallStatusText.ended(.unreachable, direction: .outgoing) == "Unavailable")
        #expect(CallStatusText.ended(.microphoneDenied, direction: .outgoing) == "Microphone access is needed for calls.")
    }

    @Test("the chat-list preview draws the record, never the placeholder body")
    func preview() {
        let missed = CallDTO(outcome: "missed")
        #expect(ChatSyncCoordinator.preview(body: "Missed voice call", attachment: nil, call: missed, isMine: false) == "Missed voice call")
        #expect(ChatSyncCoordinator.preview(body: "Missed voice call", attachment: nil, call: missed, isMine: true) == "No answer")
        #expect(ChatSyncCoordinator.preview(body: "Voice call", attachment: nil, call: CallDTO(outcome: "completed", durationSecs: 9), isMine: true) == "Voice call · 0:09")
        // Without a record the old rule is untouched.
        #expect(ChatSyncCoordinator.preview(body: "hello", attachment: nil) == "hello")
        #expect(ChatNotifier.body(text: "Missed voice call", attachment: nil, call: missed) == "Missed voice call")
    }

    // MARK: - Socket hold

    @Test("backgrounding suspends the socket unless a call is in progress")
    func socketHold() {
        #expect(SocketHold.decide(isInBackground: true, isCallInProgress: false) == .suspend)
        #expect(SocketHold.decide(isInBackground: true, isCallInProgress: true) == .keep)
        #expect(SocketHold.decide(isInBackground: false, isCallInProgress: false) == .keep)
        #expect(SocketHold.decide(isInBackground: false, isCallInProgress: true) == .keep)
    }

    // MARK: - The VoIP payload

    @Test("the VoIP push parses with numbers as NSNumber or as strings, and refuses anything else")
    func voipPayload() {
        let numeric: [AnyHashable: Any] = [
            "kind": "call", "call_id": "6a1f", "chat_id": NSNumber(value: 42),
            "from_user_id": NSNumber(value: 7), "caller_name": "Anna",
        ]
        #expect(IncomingCallPush.parse(numeric) == IncomingCallPush(callID: "6a1f", chatID: 42, fromUserID: 7, callerName: "Anna"))
        let stringy: [AnyHashable: Any] = ["kind": "call", "call_id": "6a1f", "chat_id": "42", "from_user_id": "7"]
        #expect(IncomingCallPush.parse(stringy) == IncomingCallPush(callID: "6a1f", chatID: 42, fromUserID: 7, callerName: ""))
        #expect(IncomingCallPush.parse(["kind": "message", "chat_id": 42]) == nil)
        #expect(IncomingCallPush.parse(["kind": "call", "chat_id": 42, "from_user_id": 7]) == nil)
        #expect(IncomingCallPush.parse(["kind": "call", "call_id": "", "chat_id": 42, "from_user_id": 7]) == nil)
    }

    // MARK: - Wire shapes

    @Test("a message carries its call record, absent on an ordinary message")
    func messageCallRecord() throws {
        let json = #"{"id": 1338, "chat_id": 42, "sender_id": 7, "client_msg_id": "6a1f0c3e-0000-4000-8000-000000000001", "body": "Voice call", "created_at": "2026-08-19T17:03:12Z", "call": {"outcome": "completed", "duration_secs": 222}}"#
        let message = try APICoding.decoder().decode(MessageDTO.self, from: Data(json.utf8))
        #expect(message.call == CallDTO(outcome: "completed", durationSecs: 222))

        let missed = #"{"id": 1339, "chat_id": 42, "sender_id": 7, "client_msg_id": "6a1f0c3e-0000-4000-8000-000000000002", "body": "Missed voice call", "created_at": "2026-08-19T17:03:12Z", "call": {"outcome": "missed"}}"#
        let missedMessage = try APICoding.decoder().decode(MessageDTO.self, from: Data(missed.utf8))
        #expect(missedMessage.call == CallDTO(outcome: "missed", durationSecs: nil))

        let plain = #"{"id": 1340, "chat_id": 42, "sender_id": 7, "client_msg_id": "6a1f0c3e-0000-4000-8000-000000000003", "body": "hi", "created_at": "2026-08-19T17:03:12Z"}"#
        #expect(try APICoding.decoder().decode(MessageDTO.self, from: Data(plain.utf8)).call == nil)
    }

    @Test("calls_enabled decodes, and defaults to false on an older server")
    func callsEnabled() throws {
        let user = #"{"id": 7, "username": "anna", "display_name": "Anna", "created_at": "2026-08-19T17:03:12Z", "avatar_version": 0}"#
        let on = try APICoding.decoder().decode(MeResponse.self, from: Data(#"{"user": \#(user), "family": null, "role": null, "pending_join_request": null, "calls_enabled": true}"#.utf8))
        #expect(on.callsEnabled)
        let old = try APICoding.decoder().decode(MeResponse.self, from: Data(#"{"user": \#(user), "family": null, "role": null, "pending_join_request": null}"#.utf8))
        #expect(!old.callsEnabled)
    }

    @Test("the ICE servers reply decodes with optional TURN credentials")
    func iceServers() throws {
        let json = #"{"ice_servers": [{"urls": ["stun:stun.example.com:3478"]}, {"urls": ["turn:turn.example.com:3478?transport=udp"], "username": "1756300000:7", "credential": "abc="}], "ttl_secs": 86400}"#
        let response = try APICoding.decoder().decode(IceServersResponse.self, from: Data(json.utf8))
        #expect(response.ttlSecs == 86400)
        #expect(response.iceServers == [
            IceServerDTO(urls: ["stun:stun.example.com:3478"]),
            IceServerDTO(urls: ["turn:turn.example.com:3478?transport=udp"], username: "1756300000:7", credential: "abc="),
        ])
    }

    // MARK: - Registration

    @Test("the VoIP token is news only when PushKit has spoken and it differs from the stored one")
    func voipRegistrationRule() {
        #expect(!PushRegistrationLogic.needsRegistration(
            token: "a", storedToken: "a", storedDeviceID: 1, voipToken: nil, storedVoIPToken: "v"))
        #expect(PushRegistrationLogic.needsRegistration(
            token: "a", storedToken: "a", storedDeviceID: 1, voipToken: "v", storedVoIPToken: nil))
        #expect(!PushRegistrationLogic.needsRegistration(
            token: "a", storedToken: "a", storedDeviceID: 1, voipToken: "v", storedVoIPToken: "v"))
        #expect(PushRegistrationLogic.needsRegistration(
            token: "b", storedToken: "a", storedDeviceID: 1, voipToken: nil, storedVoIPToken: nil))
    }
}
