//
//  RecordVectorTests.swift
//  FamilyConnectTests
//
//  AN ORACLE, NOT FOUR READINGS — #79's shared rules (the Send slot, the
//  video button, the round helpers and the voice recording's reducer) are
//  written once, in
//  `web/text/src/record.rs`, and printed by `win/tools/board-oracle`
//  (`cargo run -- record`) into Fixtures/record-vectors.json: the same bytes
//  Android and Windows check, and CI compares the three copies. Every case
//  here is a case every port gets right, or none of them does.
//
//  A `hold_step` case is ONE step — a state, an event and the constants in;
//  the next state, the effects and what the slot is told out — taken from a
//  named scenario, so each transition is met in a state it can really be in.
//  The expected values are compared as the reference wrote them: this suite
//  turns the Swift answer back into the oracle's JSON shape and compares the
//  two as parsed values, so a float written `20` and a Double 20.0 agree and
//  an effect list in the wrong order does not.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Record vectors")
struct RecordVectorTests {

    // MARK: - Loading

    struct Case {
        let name: String
        let function: String
        let input: [String: Any]
        let expected: [String: Any]
    }

    /// Bundle(for:) needs a class; a Swift Testing suite is a struct.
    private final class Anchor {}

    /// The synchronized group flattens Fixtures/ into the bundle's root,
    /// with the source tree as the fallback MediaPlanVectorTests uses.
    private static func vectorsURL() -> URL {
        if let bundled = Bundle(for: Anchor.self).url(forResource: "record-vectors", withExtension: "json") {
            return bundled
        }
        return URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .appending(path: "Fixtures")
            .appending(path: "record-vectors.json")
    }

    private static func load() throws -> [Case] {
        let data = try Data(contentsOf: vectorsURL())
        let array = try #require(try JSONSerialization.jsonObject(with: data) as? [[String: Any]])
        return try array.map { raw in
            Case(
                name: try #require(raw["name"] as? String),
                function: try #require(raw["function"] as? String),
                input: try #require(raw["input"] as? [String: Any]),
                expected: try #require(raw["expected"] as? [String: Any]))
        }
    }

    /// The cases for one function — never none of them: a renamed function
    /// in the generator would otherwise pass by asserting nothing.
    private static func cases(for function: String) throws -> [Case] {
        let selected = try load().filter { $0.function == function }
        #expect(!selected.isEmpty, "no vectors for \(function)")
        return selected
    }

    // MARK: - JSON in

    private static func u64(_ value: Any?) throws -> UInt64 {
        let number = try #require(value as? NSNumber, "not a number: \(String(describing: value))")
        return number.uint64Value
    }

    private static func double(_ value: Any?) throws -> Double {
        let number = try #require(value as? NSNumber, "not a number: \(String(describing: value))")
        return number.doubleValue
    }

    private static func bool(_ value: Any?) throws -> Bool {
        let number = try #require(value as? NSNumber, "not a bool: \(String(describing: value))")
        return number.boolValue
    }

    private static func string(_ value: Any?) throws -> String {
        try #require(value as? String, "not a string: \(String(describing: value))")
    }

    private static func dictionary(_ value: Any?) throws -> [String: Any] {
        try #require(value as? [String: Any], "not an object: \(String(describing: value))")
    }

    static func dimmed(_ value: Any?) throws -> ComposerSlot.Dimmed? {
        if value == nil || value is NSNull { return nil }
        switch try string(value) {
        case "call": return .call
        case "busy": return .busy
        case "not_sent": return .notSent
        case let other: throw VectorError.unknown("dimmed \(other)")
        }
    }

    static func recording(_ value: Any?) throws -> ComposerSlot.Recording {
        switch try string(value) {
        case "none": return .none
        case "hands_free": return .handsFree
        case "hands_free_beside_draft": return .handsFreeBesideDraft
        case let other: throw VectorError.unknown("recording \(other)")
        }
    }

    static func slotInputs(_ value: Any?) throws -> ComposerSlot.Inputs {
        let raw = try dictionary(value)
        return ComposerSlot.Inputs(
            recorderOpen: try bool(raw["recorder_open"]),
            recording: try recording(raw["recording"]),
            editing: try bool(raw["editing"]),
            draftBlank: try bool(raw["draft_blank"]),
            staged: try bool(raw["staged"]),
            assistantChat: try bool(raw["assistant_chat"]),
            canRecord: try bool(raw["can_record"]),
            call: try bool(raw["call"]),
            busy: try bool(raw["busy"]),
            notSent: try bool(raw["not_sent"]))
    }

    static func doorInputs(_ value: Any?) throws -> VideoDoor.Inputs {
        let raw = try dictionary(value)
        return VideoDoor.Inputs(
            slot: try slotInputs(raw["slot"]),
            familyOrDirectChat: try bool(raw["family_or_direct_chat"]),
            serverOffersRound: try bool(raw["server_offers_round"]),
            hasCamera: try bool(raw["has_camera"]),
            encoderProbePasses: try bool(raw["encoder_probe_passes"]),
            recordsRoundVideo: try bool(raw["records_round_video"]))
    }

    static func constants(_ value: Any?) throws -> RecordGesture.HoldConstants {
        let raw = try dictionary(value)
        #expect(Set(raw.keys) == ["shortest_recording_ms", "activation_guard_ms", "delete_asks_from_ms"],
                "the constants carry a key this port does not read: \(raw.keys.sorted())")
        return RecordGesture.HoldConstants(
            shortestRecordingMS: try u64(raw["shortest_recording_ms"]),
            activationGuardMS: try u64(raw["activation_guard_ms"]),
            deleteAsksFromMS: try u64(raw["delete_asks_from_ms"]))
    }

    static func situation(_ value: Any?) throws -> RecordGesture.Situation {
        let raw = try dictionary(value)
        let permission: RecordGesture.Permission
        switch try string(raw["permission"]) {
        case "granted": permission = .granted
        case "not_asked": permission = .notAsked
        case "denied": permission = .denied
        case let other: throw VectorError.unknown("permission \(other)")
        }
        #expect(Set(raw.keys) == ["permission", "blocked"],
                "the situation carries a key this port does not read: \(raw.keys.sorted())")
        return RecordGesture.Situation(permission: permission, blocked: try dimmed(raw["blocked"]))
    }

    static func source(_ value: Any?) throws -> RecordGesture.Source {
        switch try string(value) {
        case "tap": return .tap
        case "menu": return .menu
        case let other: throw VectorError.unknown("source \(other)")
        }
    }

    static func state(_ value: Any?) throws -> RecordGesture.HoldState {
        let raw = try dictionary(value)
        let phase: RecordGesture.Phase
        switch try string(raw["phase"]) {
        case "idle":
            phase = .idle
        case "hands_free":
            phase = .handsFree(besideDraft: try bool(raw["beside_draft"]))
        case "asking_delete":
            phase = .askingDelete(recordedMS: try u64(raw["recorded_ms"]))
        case "awaiting_permission":
            phase = .awaitingPermission(
                source: try source(raw["source"]), besideDraft: try bool(raw["beside_draft"]))
        case let other:
            throw VectorError.unknown("phase \(other)")
        }
        return RecordGesture.HoldState(phase: phase, guardUntilMS: try u64(raw["guard_until_ms"]))
    }

    static func event(_ value: Any?) throws -> RecordGesture.HoldEvent {
        let raw = try dictionary(value)
        let at = try u64(raw["at_ms"])
        switch try string(raw["event"]) {
        case "cap":
            return .cap(atMS: at)
        case "interruption":
            return .interruption(atMS: at, recordedMS: try u64(raw["recorded_ms"]))
        case "activate":
            return .activate(
                atMS: at, situation: try situation(raw["situation"]), recordedMS: try u64(raw["recorded_ms"]))
        case "record":
            return .record(
                atMS: at, besideDraft: try bool(raw["beside_draft"]),
                situation: try situation(raw["situation"]), recordedMS: try u64(raw["recorded_ms"]))
        case "stop":
            return .stop(atMS: at, recordedMS: try u64(raw["recorded_ms"]))
        case "delete":
            return .delete(atMS: at, recordedMS: try u64(raw["recorded_ms"]))
        case "answer":
            return .answer(atMS: at, delete: try bool(raw["delete"]))
        case "permission_answer":
            return .permissionAnswer(atMS: at, granted: try bool(raw["granted"]))
        case "other_action":
            return .otherAction(atMS: at)
        case "emptied":
            return .emptied(atMS: at)
        case let other:
            throw VectorError.unknown("event \(other)")
        }
    }

    enum VectorError: Error, CustomStringConvertible {
        case unknown(String)
        var description: String {
            switch self { case .unknown(let what): "unknown \(what)" }
        }
    }

    // MARK: - JSON out, in the oracle's shape

    private static func json(_ value: String?) -> Any { value.map { $0 as Any } ?? NSNull() }

    static func name(_ reason: ComposerSlot.Dimmed) -> String {
        switch reason {
        case .call: "call"
        case .busy: "busy"
        case .notSent: "not_sent"
        }
    }

    static func name(_ recording: ComposerSlot.Recording) -> String {
        switch recording {
        case .none: "none"
        case .handsFree: "hands_free"
        case .handsFreeBesideDraft: "hands_free_beside_draft"
        }
    }

    static func encode(_ slot: ComposerSlot) -> [String: Any] {
        let name: String
        var enabled: Any = NSNull()
        var reason: Any = NSNull()
        switch slot {
        case .recorder: name = "recorder"
        case .sendVoice: name = "send_voice"
        case .stopRecording: name = "stop_recording"
        case .save(let on): name = "save"; enabled = on
        case .send: name = "send"
        case .sendDisabled: name = "send_disabled"
        case .dimmed(let why): name = "dimmed"; reason = Self.name(why)
        case .microphone: name = "microphone"
        }
        return [
            "row": slot.row, "slot": name, "enabled": enabled, "reason": reason,
            "label": json(slot.labelKey), "notice": json(slot.noticeKey),
        ]
    }

    static func encode(_ door: VideoDoor) -> [String: Any] {
        let name: String
        var reason: Any = NSNull()
        switch door {
        case .hidden: name = "hidden"
        case .dimmed(let why): name = "dimmed"; reason = Self.name(why)
        case .shown: name = "shown"
        }
        return ["door": name, "reason": reason, "label": json(door.labelKey), "notice": json(door.noticeKey)]
    }

    static func encode(_ state: RecordGesture.HoldState) -> [String: Any] {
        var value: [String: Any]
        switch state.phase {
        case .idle:
            value = ["phase": "idle"]
        case let .handsFree(besideDraft):
            value = ["phase": "hands_free", "beside_draft": besideDraft]
        case let .askingDelete(recordedMS):
            value = ["phase": "asking_delete", "recorded_ms": recordedMS]
        case let .awaitingPermission(source, besideDraft):
            let name: String
            switch source {
            case .tap: name = "tap"
            case .menu: name = "menu"
            }
            value = ["phase": "awaiting_permission", "source": name, "beside_draft": besideDraft]
        }
        value["guard_until_ms"] = state.guardUntilMS
        return value
    }

    static func encode(_ effect: RecordGesture.HoldEffect) -> [String: Any] {
        switch effect {
        case .start: return ["effect": "start"]
        case .delete: return ["effect": "delete"]
        case .send: return ["effect": "send"]
        case .review: return ["effect": "review"]
        case .park: return ["effect": "park"]
        case .askDelete: return ["effect": "ask_delete"]
        case .askPermission: return ["effect": "ask_permission"]
        case .denied: return ["effect": "denied"]
        case .explain(let reason):
            return ["effect": "explain", "reason": name(reason), "text": reason.noticeKey]
        case .hint(let hint):
            let name: String
            switch hint {
            case .stoppedAtFiveMinutes: name = "stopped_at_five_minutes"
            case .tooShort: name = "too_short"
            }
            return ["effect": "hint", "hint": name, "text": hint.key]
        case .announce(let announcement):
            let name: String
            switch announcement {
            case .recording: name = "recording"
            case .recordingDeleted: name = "recording_deleted"
            case .voiceMessageSent: name = "voice_message_sent"
            case .readyToReview: name = "ready_to_review"
            case .tooShort: name = "too_short"
            case .stoppedAtFiveMinutes: name = "stopped_at_five_minutes"
            }
            var value: [String: Any] = ["effect": "announce", "announcement": name, "text": announcement.key]
            if case .readyToReview(let recordedMS) = announcement {
                value["recorded_ms"] = recordedMS
            }
            return value
        case .haptic(let haptic):
            let name: String
            switch haptic {
            case .light: name = "light"
            case .success: name = "success"
            case .warning: name = "warning"
            }
            return ["effect": "haptic", "haptic": name]
        }
    }

    /// Parsed-value equality, the way the reference's JSON reads: numbers by
    /// value, null as null, arrays in order.
    static func same(_ swift: Any, _ oracle: Any) -> Bool {
        (swift as AnyObject).isEqual(oracle)
    }

    static func describe(_ value: Any) -> String {
        guard JSONSerialization.isValidJSONObject(value),
              let data = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]),
              let text = String(data: data, encoding: .utf8)
        else { return String(describing: value) }
        return text
    }

    // MARK: - The file itself

    @Test("every vector names a function this port checks, and none is missing")
    func everyFunctionIsCovered() throws {
        let all = try Self.load()
        let known: Set<String> = [
            "constants", "composer_slot", "video_door", "round_cap_ms",
            "round_warning_ms", "round_diameter", "is_round", "hold_step",
        ]
        #expect(all.count >= 218, "the reference printed 218 cases; \(all.count) were read")
        for vector in all {
            #expect(known.contains(vector.function), "\(vector.name): unknown function \(vector.function)")
        }
        #expect(Set(all.map(\.function)) == known, "a function this port checks has no vectors")
    }

    // MARK: - The constants

    @Test("S1.1's constants and S5.2's diameters are the reference's")
    func constantsMatch() throws {
        let vector = try #require(try Self.cases(for: "constants").first)
        let e = vector.expected
        #expect(RecordRules.activationGuardMS == (try Self.u64(e["activation_guard_ms"])))
        #expect(RecordRules.shortestRecordingMS == (try Self.u64(e["shortest_recording_ms"])))
        #expect(RecordRules.voiceCapMS == (try Self.u64(e["voice_cap_ms"])))
        #expect(RecordRules.voiceWarningMS == (try Self.u64(e["voice_warning_ms"])))
        #expect(RecordRules.defaultMaxRoundVideoMS == (try Self.u64(e["default_max_round_video_ms"])))
        #expect(RecordRules.roundCapMarginMS == (try Self.u64(e["round_cap_margin_ms"])))
        #expect(RecordRules.roundWarningLeadMS == (try Self.u64(e["round_warning_lead_ms"])))
        #expect(RecordRules.silencePeakDBFS == (try Self.double(e["silence_peak_dbfs"])))
        #expect(UInt64(RecordRules.silenceMaxAmplitude) == (try Self.u64(e["silence_max_amplitude"])))
        #expect(RecordRules.silenceSampleMagnitude == (try Self.double(e["silence_sample_magnitude"])))
        #expect(RecordRules.silenceWarningAfterMS == (try Self.u64(e["silence_warning_after_ms"])))
        #expect(RecordRules.deleteAsksFromMS == (try Self.u64(e["delete_asks_from_ms"])))
        #expect(RecordRules.previewIdleCloseMS == (try Self.u64(e["preview_idle_close_ms"])))
        #expect(RecordRules.slotCrossfadeMS == (try Self.u64(e["slot_crossfade_ms"])))
        #expect(RecordRules.recorderFadeMS == (try Self.u64(e["recorder_fade_ms"])))
        #expect(UInt64(RecordRules.minTargetApplePT) == (try Self.u64(e["min_target_apple_pt"])))
        #expect(UInt64(RecordRules.minTargetAndroidDP) == (try Self.u64(e["min_target_android_dp"])))
        #expect(UInt64(RecordRules.minTargetWindowsEPX) == (try Self.u64(e["min_target_windows_epx"])))
        #expect(UInt64(RecordRules.minTargetWebPX) == (try Self.u64(e["min_target_web_px"])))
        #expect(UInt64(RecordRules.roundDiameterCompact) == (try Self.u64(e["round_diameter_compact"])))
        #expect(UInt64(RecordRules.roundDiameterRegular) == (try Self.u64(e["round_diameter_regular"])))
        #expect(VideoDoor.labelKey == (try Self.string(e["video_door_label"])))
        #expect(VideoDoor.tooltipKey == (try Self.string(e["video_door_tooltip"])))
        #expect(RecordGesture.HoldConstants.standard == (try Self.constants(e["default_hold_constants"])))
    }

    @MainActor
    @Test("the constants the recorder and the store already used are the shared ones")
    func phaseZeroConstantsAgree() {
        #expect(AudioRecorder.shortestKept * 1000 == Double(RecordRules.shortestRecordingMS))
        #expect(Double(AudioRecorder.silenceLine) == RecordRules.silencePeakDBFS)
        #expect(UInt64(ParkedRecordings.deleteAsksFromMS) == RecordRules.deleteAsksFromMS)
        #expect(AudioRecorder.maxDuration * 1000 == Double(RecordRules.voiceCapMS))
    }

    // MARK: - The slot and the door

    @Test("the trailing slot is the reference's, every row")
    func composerSlot() throws {
        for vector in try Self.cases(for: "composer_slot") {
            let slot = ComposerSlot.of(try Self.slotInputs(vector.input))
            let mine = Self.encode(slot)
            #expect(Self.same(mine, vector.expected),
                    "\(vector.name): \(Self.describe(mine)) != \(Self.describe(vector.expected))")
            // The reference's `is_microphone` is rows 7 to 10.
            #expect(slot.isMicrophone == (7...10).contains(slot.row), "\(vector.name)")
        }
    }

    @Test("the video button is the reference's, and hidden in every build that does not record")
    func videoDoor() throws {
        for vector in try Self.cases(for: "video_door") {
            let inputs = try Self.doorInputs(vector.input)
            let door = VideoDoor.of(inputs)
            let mine = Self.encode(door)
            #expect(Self.same(mine, vector.expected),
                    "\(vector.name): \(Self.describe(mine)) != \(Self.describe(vector.expected))")
            // Decision 40: a build that does not record shows no door…
            var receivesOnly = inputs
            receivesOnly.recordsRoundVideo = false
            #expect(VideoDoor.of(receivesOnly) == .hidden, "\(vector.name): a door in a build that records no video")
            // …and this one (Phase 3a) records, so it shows what the
            // reference says a recording build shows.
            var thisBuild = inputs
            thisBuild.recordsRoundVideo = VideoDoor.thisBuildRecords
            var recording = inputs
            recording.recordsRoundVideo = true
            #expect(VideoDoor.of(thisBuild) == VideoDoor.of(recording), "\(vector.name)")
        }
    }

    // MARK: - The round video's arithmetic

    @Test("the round cap and warning are the reference's, saturating at zero")
    func roundTimes() throws {
        for vector in try Self.cases(for: "round_cap_ms") {
            let max = try Self.u64(vector.input["max_round_video_ms"])
            #expect(RoundVideo.capMS(maxRoundVideoMS: max) == (try Self.u64(vector.expected["cap_ms"])), "\(vector.name)")
        }
        for vector in try Self.cases(for: "round_warning_ms") {
            let max = try Self.u64(vector.input["max_round_video_ms"])
            #expect(RoundVideo.warningMS(maxRoundVideoMS: max) == (try Self.u64(vector.expected["warning_ms"])), "\(vector.name)")
        }
    }

    @Test("a circle is 200 compact and 240 regular")
    func roundDiameter() throws {
        for vector in try Self.cases(for: "round_diameter") {
            let width: RoundVideo.WidthClass = try Self.string(vector.input["width_class"]) == "compact" ? .compact : .regular
            #expect(UInt64(RoundVideo.diameter(width)) == (try Self.u64(vector.expected["diameter"])), "\(vector.name)")
        }
    }

    @Test("the drawing test is the reference's: one video, flagged, no body")
    func isRound() throws {
        for vector in try Self.cases(for: "is_round") {
            let body = try Self.string(vector.input["body"])
            let raw = try #require(vector.input["attachments"] as? [[String: Any]])
            let attachments = try raw.map {
                RoundVideo.AttachmentFlags(kind: try Self.string($0["kind"]), round: try Self.bool($0["round"]))
            }
            let expected = try Self.bool(vector.expected["round"])
            #expect(RoundVideo.isRound(body: body, attachments: attachments) == expected, "\(vector.name)")
        }
    }

    // MARK: - The voice recording's reducer

    @Test("every step of the voice recording is the reference's: state, effects and what the slot is told")
    func holdStep() throws {
        for vector in try Self.cases(for: "hold_step") {
            let state = try Self.state(vector.input["state"])
            let event = try Self.event(vector.input["event"])
            let constants = try Self.constants(vector.input["constants"])
            let (next, effects) = RecordGesture.step(state, event, constants)

            let expectedState = try Self.dictionary(vector.expected["state"])
            let mineState = Self.encode(next)
            #expect(Self.same(mineState, expectedState),
                    "\(vector.name): state \(Self.describe(mineState)) != \(Self.describe(expectedState))")

            let expectedEffects = try #require(vector.expected["effects"] as? [Any])
            let mineEffects = effects.map(Self.encode)
            #expect(Self.same(mineEffects, expectedEffects),
                    "\(vector.name): effects \(Self.describe(mineEffects)) != \(Self.describe(expectedEffects))")

            #expect(Self.name(next.recording) == (try Self.string(vector.expected["recording"])), "\(vector.name): recording")
        }
    }

    /// The reverse direction: a vector state read and written back is the
    /// same JSON. Without it, a reader that dropped a field would agree with
    /// an encoder that dropped the same field.
    @Test("a state read from the vectors writes back exactly")
    func stateRoundTrip() throws {
        for vector in try Self.cases(for: "hold_step") {
            for key in ["state"] {
                let raw = try Self.dictionary(vector.input[key])
                let back = Self.encode(try Self.state(raw))
                #expect(Self.same(back, raw), "\(vector.name): \(Self.describe(back)) != \(Self.describe(raw))")
            }
        }
    }
}
