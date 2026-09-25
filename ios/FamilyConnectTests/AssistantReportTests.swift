//
//  AssistantReportTests.swift
//  FamilyConnectTests
//
//  WHICH SAFETY ROW A BUBBLE GETS, and what goes on the wire when somebody
//  reports the assistant (docs/protocol.md, "Reporting the assistant").
//
//  The two reports are exclusive, and getting that wrong is not cosmetic: a
//  member report names somebody in your family and the OWNER reads it, while
//  an assistant report names a reply from an account that belongs to no family
//  and the OPERATOR reads it. Each endpoint refuses the other's subject —
//  `not_same_family` one way, `message_not_found` the other — so a menu that
//  offered the wrong row would fail visibly on a safety screen, which is the
//  one screen that must not.
//

import Foundation
import Testing

@testable import FamilyConnect

struct AssistantReportTests {
    private let me: Int64 = 7
    private let anna: Int64 = 11
    private let assistant: Int64 = 1

    @Test("a member's acked message gets the member report, and only that")
    func memberRow() {
        #expect(SafetyRules.canReportMember(
            senderID: anna, currentUserID: me, isAssistant: false, hasServerID: true))
        #expect(!SafetyRules.canReportAssistant(
            senderID: anna, currentUserID: me, isAssistant: false, hasServerID: true))
    }

    @Test("an assistant reply gets the assistant report, and only that")
    func assistantRow() {
        #expect(SafetyRules.canReportAssistant(
            senderID: assistant, currentUserID: me, isAssistant: true, hasServerID: true))
        #expect(!SafetyRules.canReportMember(
            senderID: assistant, currentUserID: me, isAssistant: true, hasServerID: true))
    }

    /// Nothing is reportable before the server has numbered it — the id IS
    /// what a report names — and nothing of the reader's own ever is.
    @Test("a pending row and your own words are reportable by neither path")
    func neitherPath() {
        for isAssistant in [true, false] {
            #expect(!SafetyRules.canReportMember(
                senderID: anna, currentUserID: me, isAssistant: isAssistant, hasServerID: false))
            #expect(!SafetyRules.canReportAssistant(
                senderID: assistant, currentUserID: me, isAssistant: isAssistant, hasServerID: false))
            #expect(!SafetyRules.canReportMember(
                senderID: me, currentUserID: me, isAssistant: isAssistant, hasServerID: true))
            #expect(!SafetyRules.canReportAssistant(
                senderID: me, currentUserID: me, isAssistant: isAssistant, hasServerID: true))
        }
    }

    /// The exclusion, asserted over every bubble a chat can hold rather than
    /// trusted to the two rules being read side by side.
    @Test("no bubble is ever both reports")
    func exclusive() {
        for sender in [me, anna, assistant] {
            for isAssistant in [true, false] {
                for hasServerID in [true, false] {
                    let member = SafetyRules.canReportMember(
                        senderID: sender, currentUserID: me,
                        isAssistant: isAssistant, hasServerID: hasServerID)
                    let model = SafetyRules.canReportAssistant(
                        senderID: sender, currentUserID: me,
                        isAssistant: isAssistant, hasServerID: hasServerID)
                    #expect(!(member && model))
                }
            }
        }
    }

    /// The shape on the wire. `note` is optional and an empty one must not be
    /// sent as `""` — the server would take it as a note that says nothing.
    @Test("the body carries the message, the raw reason and a note only when there is one")
    func wire() throws {
        struct Body: Encodable {
            let messageID: Int64
            let reason: String
            let note: String?
            enum CodingKeys: String, CodingKey {
                case messageID = "message_id"
                case reason
                case note
            }
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = .sortedKeys

        let withNote = try encoder.encode(
            Body(messageID: 34, reason: ReportReason.inappropriate.rawValue, note: "It invented a person."))
        #expect(String(decoding: withNote, as: UTF8.self)
            == #"{"message_id":34,"note":"It invented a person.","reason":"inappropriate"}"#)

        let without = try encoder.encode(
            Body(messageID: 34, reason: ReportReason.other.rawValue, note: nil))
        #expect(String(decoding: without, as: UTF8.self)
            == #"{"message_id":34,"reason":"other"}"#)
    }

    /// The reasons are the product's four, and the RAW value is what travels:
    /// a translated label on the wire is a `validation` refusal in whichever
    /// language the reporter happens to use.
    @Test("the four reasons are the wire's four words")
    func reasons() {
        #expect(ReportReason.allCases.map(\.rawValue)
            == ["spam", "harassment", "inappropriate", "other"])
    }

    /// What the server answers, decoded the way the app decodes it.
    @Test("the answer decodes, and says which surface the reply came from")
    func decode() throws {
        let json = #"""
            {"report": {"id": 3, "message_id": 34,
                        "message_excerpt": "Your grandmother was born in 1812.",
                        "chat_kind": "ai", "reason": "other", "note": "It invented a person."}}
            """#
        let answer = try APICoding.decoder().decode(
            AssistantReportResponse.self, from: Data(json.utf8))
        #expect(answer.report.id == 3)
        #expect(answer.report.messageID == 34)
        #expect(answer.report.chatKind == "ai")
        #expect(answer.report.messageExcerpt == "Your grandmother was born in 1812.")
        #expect(answer.report.note == "It invented a person.")
    }

    /// Retention takes the reply; the row still means something, so the app
    /// must decode one whose `message_id` has gone.
    @Test("an answer whose reply has been swept still decodes")
    func decodeSwept() throws {
        let json = #"""
            {"report": {"id": 4, "message_excerpt": "Swept later.", "chat_kind": "family",
                        "reason": "spam"}}
            """#
        let answer = try APICoding.decoder().decode(
            AssistantReportResponse.self, from: Data(json.utf8))
        #expect(answer.report.messageID == nil)
        #expect(answer.report.note == nil)
        #expect(answer.report.messageExcerpt == "Swept later.")
    }
}
