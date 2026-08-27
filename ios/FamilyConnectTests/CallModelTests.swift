//
//  CallModelTests.swift
//  FamilyConnectTests
//
//  The pure pieces of voice calls: the record's wording, the in-call
//  status line, the socket-hold rule, the VoIP payload parser, the wire
//  shapes of the record and of `calls_enabled`, and the chat-list preview.
//  Plus, at the bottom, the one stateful piece: what a call record looks
//  like after a trip through the store.
//

import Foundation
import SwiftData
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

    @Test("the record's wording on a video call")
    func videoRecordWording() {
        let completed = CallDTO(outcome: "completed", durationSecs: 222, video: true)
        #expect(CallRecordText.label(completed, isMine: true) == "Video call · 3:42")
        #expect(CallRecordText.label(completed, isMine: false) == "Video call · 3:42")
        #expect(CallRecordText.label(CallDTO(outcome: "completed", video: true), isMine: true) == "Video call")

        let missed = CallDTO(outcome: "missed", video: true)
        #expect(CallRecordText.label(missed, isMine: true) == "No answer")
        #expect(CallRecordText.label(missed, isMine: false) == "Missed video call")

        let declined = CallDTO(outcome: "declined", video: true)
        #expect(CallRecordText.label(declined, isMine: true) == "Video call declined")
        #expect(CallRecordText.label(declined, isMine: false) == "Declined video call")

        // Failure wording is kind-neutral on purpose.
        #expect(CallRecordText.label(CallDTO(outcome: "failed", video: true), isMine: true) == "Call failed")
        // The render floor keeps the kind.
        #expect(CallRecordText.label(CallDTO(outcome: "hologram", video: true), isMine: false) == "Video call")
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

    @Test("the status line's video variants, and the camera-denied note")
    func videoStatusLine() {
        #expect(CallStatusText.line(phase: .incoming, direction: .incoming, elapsed: 0, video: true) == "Incoming video call")
        #expect(CallStatusText.line(phase: .incoming, direction: .incoming, elapsed: 0, video: false) == "Incoming call")
        #expect(CallStatusText.ended(.timeout, direction: .incoming, video: true) == "Missed video call")
        // The caller's side stays kind-neutral: nobody answered, that is the news.
        #expect(CallStatusText.ended(.timeout, direction: .outgoing, video: true) == "No answer")
        #expect(CallStatusText.cameraDeniedNote == "Camera access is off — the call is voice-only for you.")
        // The server's video_calls_disabled refusal gets its own words —
        // the generic "Unavailable" reads as a failure and invites
        // retries against a deliberate operator setting.
        #expect(CallEndReason(wire: "video_calls_disabled") == .videoUnavailable)
        #expect(CallStatusText.ended(.videoUnavailable, direction: .outgoing) == "Video calls are off on this server.")
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

        // The video record flows through the same two doors.
        let missedVideo = CallDTO(outcome: "missed", video: true)
        #expect(ChatSyncCoordinator.preview(body: "Missed video call", attachment: nil, call: missedVideo, isMine: false) == "Missed video call")
        #expect(ChatSyncCoordinator.preview(body: "Video call", attachment: nil, call: CallDTO(outcome: "completed", durationSecs: 9, video: true), isMine: true) == "Video call · 0:09")
        #expect(ChatNotifier.body(text: "Missed video call", attachment: nil, call: missedVideo) == "Missed video call")
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

    @Test("the VoIP push's video flag: true as a bool or a string, absent means voice")
    func voipVideoFlag() {
        // The doc's literal: {"kind": "call", …, "video": true}.
        let asBool: [AnyHashable: Any] = [
            "kind": "call", "call_id": "6a1f", "chat_id": NSNumber(value: 42),
            "from_user_id": NSNumber(value: 7), "caller_name": "Anna", "video": NSNumber(value: true),
        ]
        #expect(IncomingCallPush.parse(asBool)?.video == true)
        let asString: [AnyHashable: Any] = [
            "kind": "call", "call_id": "6a1f", "chat_id": "42", "from_user_id": "7", "video": "true",
        ]
        #expect(IncomingCallPush.parse(asString)?.video == true)
        let absent: [AnyHashable: Any] = [
            "kind": "call", "call_id": "6a1f", "chat_id": "42", "from_user_id": "7",
        ]
        #expect(IncomingCallPush.parse(absent)?.video == false)
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

    @Test("the record's video flag decodes, and absent means voice — per the doc's Call object")
    func callRecordVideoFlag() throws {
        // Doc literal: {"outcome": …, "duration_secs": …, "video": true}.
        let video = #"{"id": 1341, "chat_id": 42, "sender_id": 7, "client_msg_id": "7b2e1d4f-0000-4000-8000-000000000001", "body": "Video call", "created_at": "2026-08-19T17:03:12Z", "call": {"outcome": "completed", "duration_secs": 222, "video": true}}"#
        let message = try APICoding.decoder().decode(MessageDTO.self, from: Data(video.utf8))
        #expect(message.call == CallDTO(outcome: "completed", durationSecs: 222, video: true))

        // A voice record has no "video" key at all; the flag defaults off.
        let voice = #"{"id": 1342, "chat_id": 42, "sender_id": 7, "client_msg_id": "7b2e1d4f-0000-4000-8000-000000000002", "body": "Voice call", "created_at": "2026-08-19T17:03:12Z", "call": {"outcome": "completed", "duration_secs": 9}}"#
        let voiceMessage = try APICoding.decoder().decode(MessageDTO.self, from: Data(voice.utf8))
        #expect(voiceMessage.call?.video == false)
    }

    @Test("calls_enabled decodes, and defaults to false on an older server")
    func callsEnabled() throws {
        let user = #"{"id": 7, "username": "anna", "display_name": "Anna", "created_at": "2026-08-19T17:03:12Z", "avatar_version": 0}"#
        let on = try APICoding.decoder().decode(MeResponse.self, from: Data(#"{"user": \#(user), "family": null, "role": null, "pending_join_request": null, "calls_enabled": true}"#.utf8))
        #expect(on.callsEnabled)
        let old = try APICoding.decoder().decode(MeResponse.self, from: Data(#"{"user": \#(user), "family": null, "role": null, "pending_join_request": null}"#.utf8))
        #expect(!old.callsEnabled)
    }

    @Test("video_calls_enabled decodes, and defaults to false on a server that predates video")
    func videoCallsEnabled() throws {
        let user = #"{"id": 7, "username": "anna", "display_name": "Anna", "created_at": "2026-08-19T17:03:12Z", "avatar_version": 0}"#
        let on = try APICoding.decoder().decode(MeResponse.self, from: Data(#"{"user": \#(user), "family": null, "role": null, "pending_join_request": null, "calls_enabled": true, "video_calls_enabled": true}"#.utf8))
        #expect(on.videoCallsEnabled)
        // Voice on, video off — the operator's Raspberry Pi says no.
        let voiceOnly = try APICoding.decoder().decode(MeResponse.self, from: Data(#"{"user": \#(user), "family": null, "role": null, "pending_join_request": null, "calls_enabled": true, "video_calls_enabled": false}"#.utf8))
        #expect(voiceOnly.callsEnabled)
        #expect(!voiceOnly.videoCallsEnabled)
        let old = try APICoding.decoder().decode(MeResponse.self, from: Data(#"{"user": \#(user), "family": null, "role": null, "pending_join_request": null, "calls_enabled": true}"#.utf8))
        #expect(!old.videoCallsEnabled)
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

/// The record's trip through SwiftData. The video flag rode the wire and
/// the wording from the day video shipped, but the ROW never stored it —
/// every cached record came back out as voice and a video call's bubble
/// said "Voice call". These pin all three write sites the flag has.
@MainActor
@Suite("Call record persistence")
struct CallRecordPersistenceTests {

    private static let serverDate = ISO8601DateFormatter().date(from: "2026-08-19T17:05:00Z")!

    private func makeContainer() throws -> ModelContainer {
        try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self,
            configurations: ModelConfiguration(isStoredInMemoryOnly: true))
    }

    private func dto(id: Int64, call: CallDTO?) -> MessageDTO {
        MessageDTO(
            id: id, chatID: 42, senderID: 9, clientMsgID: nil,
            body: call?.video == true ? "Video call" : "Voice call",
            createdAt: Self.serverDate, call: call)
    }

    @Test("a row built from a video record reads back as a video record, a voice one as voice")
    func entityRoundTrip() throws {
        let container = try makeContainer()
        let video = MessageEntity(
            localID: "s:1", serverID: 1, chatID: 42, senderID: 9, body: "Video call",
            createdAt: Self.serverDate, status: .sent,
            call: CallDTO(outcome: "completed", durationSecs: 222, video: true))
        let voice = MessageEntity(
            localID: "s:2", serverID: 2, chatID: 42, senderID: 9, body: "Voice call",
            createdAt: Self.serverDate, status: .sent,
            call: CallDTO(outcome: "completed", durationSecs: 9))
        let plain = MessageEntity(
            localID: "s:3", serverID: 3, chatID: 42, senderID: 9, body: "hi",
            createdAt: Self.serverDate, status: .sent)
        container.mainContext.insert(video)
        container.mainContext.insert(voice)
        container.mainContext.insert(plain)
        try container.mainContext.save()

        // Back out through a fetch, not the instances in hand.
        let rows = try container.mainContext.fetch(FetchDescriptor<MessageEntity>())
        let byID = Dictionary(uniqueKeysWithValues: rows.map { ($0.localID, $0) })
        #expect(byID["s:1"]?.callSnapshot == CallDTO(outcome: "completed", durationSecs: 222, video: true))
        #expect(byID["s:1"]?.callSnapshot?.video == true)
        #expect(byID["s:2"]?.callSnapshot == CallDTO(outcome: "completed", durationSecs: 9))
        #expect(byID["s:2"]?.callSnapshot?.video == false)
        #expect(byID["s:3"]?.callSnapshot == nil)
        // The wording the bubble draws is downstream of exactly this.
        let stored = try #require(byID["s:1"]?.callSnapshot)
        #expect(CallRecordText.label(stored, isMine: false) == "Video call · 3:42")
    }

    @Test("the coordinator stores the flag on first sight and rewrites it on re-delivery; an absent record never wipes")
    func coordinatorWritesTheFlag() throws {
        let host = "call-video.test"
        StubURLProtocol.register(host: host, handler: { _ in .empty(204) })
        defer { StubURLProtocol.unregister(host: host) }
        let container = try makeContainer()
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        container.mainContext.insert(ChatEntity(chatID: 42, kind: "family", pinRank: 0, title: "The Smiths"))
        try container.mainContext.save()

        // First sight, from a history page.
        let missedVideo = CallDTO(outcome: "missed", video: true)
        let row = coordinator.upsert(dto(id: 100, call: missedVideo), bumpUnread: false)
        #expect(row.callVideo)
        #expect(row.callSnapshot == missedVideo)

        // A row cached by the build that had no column: the flag is off
        // although the record is a video one. Re-delivery of the same id
        // (a re-fetched page, a resync overlapping a live frame) goes
        // through applyCall and repairs it.
        row.callVideo = false
        try container.mainContext.save()
        let again = coordinator.upsert(dto(id: 100, call: missedVideo), bumpUnread: false)
        #expect(again.localID == row.localID)
        #expect(row.callVideo)
        #expect(row.callSnapshot?.video == true)

        // The absent-field rule, on the flag as on the rest of the record.
        _ = coordinator.upsert(dto(id: 100, call: nil), bumpUnread: false)
        #expect(row.callSnapshot == missedVideo)

        // A voice record stays voice through the same door.
        let voice = coordinator.upsert(dto(id: 101, call: CallDTO(outcome: "completed", durationSecs: 9)), bumpUnread: false)
        #expect(!voice.callVideo)
        #expect(voice.callSnapshot?.video == false)
    }
}
