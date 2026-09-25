//
//  AssistantConsentTests.swift
//  FamilyConnectTests
//
//  Nothing a member writes reaches the model before that member has said
//  yes (docs/protocol.md, "Consenting to the assistant").
//
//  Two halves are pinned here. The GATE — which drafts, in which chats,
//  have to be asked about — is the mirror of `model_surface` in
//  `server/src/handlers_chat.rs`, and a disagreement between them is
//  either a message refused after it was typed or one sent having asked
//  nothing. And the DISCLOSURE, because what that screen promises depends
//  on the family's own switches: with history off it must not claim the
//  last 30 days travel, and with pictures off it must not mention photos
//  at all.
//

import Foundation
import Testing

@testable import FamilyConnect

@Suite("Consenting to the assistant")
struct AssistantConsentTests {

    static let processor = "Microsoft — Azure OpenAI"

    // MARK: - Which messages reach the model

    @Test("The assistant's own chat always does, whatever the words are")
    func privateChatAlwaysReaches() {
        for body in ["hello", "", "/draw a cat", "no mention here"] {
            #expect(AssistantConsent.reachesTheModel(chatKind: "ai", body: body))
        }
    }

    @Test("The family chat does only when the message says @ai")
    func familyChatReachesOnAMention() {
        #expect(AssistantConsent.reachesTheModel(chatKind: "family", body: "@ai when is dinner?"))
        #expect(AssistantConsent.reachesTheModel(chatKind: "family", body: "hey @AI"))
        // `/draw` needs no case of its own: in the family chat a picture
        // request IS a mention, and a bare one asks nobody.
        #expect(AssistantConsent.reachesTheModel(chatKind: "family", body: "@ai /draw a cat"))
        #expect(!AssistantConsent.reachesTheModel(chatKind: "family", body: "/draw a cat"))
        #expect(!AssistantConsent.reachesTheModel(chatKind: "family", body: "dinner at 7?"))
        // The grammar's own boundaries, so the gate cannot be looser than
        // the highlight or the server.
        #expect(!AssistantConsent.reachesTheModel(chatKind: "family", body: "write to anna@ai.example"))
        #expect(!AssistantConsent.reachesTheModel(chatKind: "family", body: "@aiden said so"))
    }

    @Test("Nowhere else does — a direct chat is two people")
    func otherChatsNeverReach() {
        for kind in ["direct", "unknown", nil] as [String?] {
            #expect(!AssistantConsent.reachesTheModel(chatKind: kind, body: "@ai hello"))
        }
    }

    // MARK: - When the question has to be asked

    @Test("Asked once, and not again once it is answered")
    func askedUntilAnswered() {
        let asked = AssistantConsent.isRequired(
            chatKind: "ai", body: "hello", processor: Self.processor, agreedAt: nil)
        #expect(asked)
        let answered = AssistantConsent.isRequired(
            chatKind: "ai", body: "hello", processor: Self.processor,
            agreedAt: Date(timeIntervalSince1970: 1_700_000_000))
        #expect(!answered)
    }

    @Test("Never asked where nothing would be sent")
    func notAskedForAnOrdinaryMessage() {
        #expect(
            !AssistantConsent.isRequired(
                chatKind: "family", body: "dinner at 7?", processor: Self.processor, agreedAt: nil))
        #expect(
            !AssistantConsent.isRequired(
                chatKind: "direct", body: "@ai hello", processor: Self.processor, agreedAt: nil))
    }

    /// A server that names nobody has an assistant this client must not
    /// offer: a consent screen with a hole where the recipient goes is not
    /// consent, so there is no question to ask and no affordance to show.
    @Test("A server that names no processor offers no assistant")
    func noProcessorNoAssistant() {
        #expect(!AssistantConsent.isAvailable(processor: nil))
        #expect(!AssistantConsent.isAvailable(processor: ""))
        #expect(!AssistantConsent.isAvailable(processor: "   "))
        #expect(AssistantConsent.isAvailable(processor: Self.processor))
        #expect(
            !AssistantConsent.isRequired(
                chatKind: "ai", body: "hello", processor: nil, agreedAt: nil),
            "nothing to ask about, so nothing is asked")
    }

    /// A server that HAS an assistant but names nobody: consent cannot be
    /// asked for, so the message is held back rather than sent. The
    /// regression this guards is the one that was rejected — words going
    /// to a third party the person was never told about.
    @Test("An assistant the server will not name is not used at all")
    func unnamedAssistantWithholdsTheMessage() {
        #expect(
            AssistantConsent.isWithheldFromAnUnnamedAssistant(
                chatKind: "ai", body: "hello", hasAssistant: true, processor: nil))
        #expect(
            AssistantConsent.isWithheldFromAnUnnamedAssistant(
                chatKind: "family", body: "@ai hello", hasAssistant: true, processor: ""))
        #expect(
            !AssistantConsent.isWithheldFromAnUnnamedAssistant(
                chatKind: "ai", body: "hello", hasAssistant: true, processor: Self.processor),
            "a named one is asked about instead")
    }

    /// And the case that must NOT be swallowed: on a server with no
    /// assistant at all, `@ai` is three characters of ordinary text that
    /// reach nobody, and refusing to send them would break a conversation
    /// to protect nothing.
    @Test("Where there is no assistant, @ai is just words")
    func noAssistantMeansNoWithholding() {
        #expect(
            !AssistantConsent.isWithheldFromAnUnnamedAssistant(
                chatKind: "family", body: "@ai hello", hasAssistant: false, processor: nil))
        #expect(
            !AssistantConsent.isRequired(
                chatKind: "family", body: "@ai hello", processor: nil, agreedAt: nil))
    }

    // MARK: - What the screen says

    @Test("It names who receives the words, verbatim")
    func disclosureNamesTheProcessor() {
        let lines = AssistantConsent.disclosure(
            processor: Self.processor, familyHistory: true, familyVision: true)
        #expect(lines.contains { $0.contains(Self.processor) })
    }

    @Test("With family history on it says what travels with a mention")
    func disclosureWithHistory() {
        let lines = AssistantConsent.disclosure(
            processor: Self.processor, familyHistory: true, familyVision: false)
        #expect(lines.contains { $0.contains("30") && $0.contains("200") })
    }

    /// The half that would be a lie: with `ai_history` off a mention takes
    /// nothing but itself, and a screen promising otherwise would be
    /// asking permission for something that does not happen — and hiding
    /// what does.
    @Test("With it off it promises the opposite, and says neither number")
    func disclosureWithoutHistory() {
        let lines = AssistantConsent.disclosure(
            processor: Self.processor, familyHistory: false, familyVision: false)
        #expect(!lines.contains { $0.contains("30") || $0.contains("200") })
        #expect(lines.contains { $0.contains(AssistantMention.token) })
    }

    @Test("Photos are mentioned only where a photo could go")
    func disclosureMentionsPicturesOnlyWhenTheyCanTravel() {
        let withPictures = AssistantConsent.disclosure(
            processor: Self.processor, familyHistory: false, familyVision: true)
        let without = AssistantConsent.disclosure(
            processor: Self.processor, familyHistory: false, familyVision: false)
        #expect(withPictures.count == without.count + 1)
    }

    @Test("It always says where the answer lands, and how to stop")
    func disclosureAlwaysSaysTheRest() {
        for history in [true, false] {
            for vision in [true, false] {
                let lines = AssistantConsent.disclosure(
                    processor: Self.processor, familyHistory: history, familyVision: vision)
                #expect(lines.count >= 4)
                #expect(lines.allSatisfy { !$0.isEmpty })
            }
        }
    }

    // MARK: - The wire

    /// `assistant_consent_at` is a date on `GET /me`, and its absence —
    /// on a server that predates the question, or for somebody who has
    /// not answered — is the same nil.
    @Test("`/me` carries the stamp, or nothing")
    func meDecodesTheStamp() throws {
        let withStamp = #"""
            {"user":{"id":1,"username":"olive","display_name":"Olive"},
             "family":null,"role":null,"pending_join_request":null,
             "assistant_consent_at":"2026-09-19T19:34:43.792761Z"}
            """#
        let me = try APICoding.decoder().decode(MeResponse.self, from: Data(withStamp.utf8))
        #expect(me.assistantConsentAt != nil)

        let without = #"""
            {"user":{"id":1,"username":"olive","display_name":"Olive"},
             "family":null,"role":null,"pending_join_request":null}
            """#
        let older = try APICoding.decoder().decode(MeResponse.self, from: Data(without.utf8))
        #expect(older.assistantConsentAt == nil)

        let nulled = #"""
            {"user":{"id":1,"username":"olive","display_name":"Olive"},
             "family":null,"role":null,"pending_join_request":null,
             "assistant_consent_at":null}
            """#
        let nobody = try APICoding.decoder().decode(MeResponse.self, from: Data(nulled.utf8))
        #expect(nobody.assistantConsentAt == nil)
    }

    @Test("The family names who answers, and an older server names nobody")
    func assistantDecodesTheProcessor() throws {
        let named = #"""
            {"user_id":9,"display_name":"Assistant","mention":"@ai",
             "processor":"Microsoft — Azure OpenAI"}
            """#
        let assistant = try APICoding.decoder().decode(AssistantDTO.self, from: Data(named.utf8))
        #expect(assistant.processor == Self.processor)
        #expect(AssistantConsent.isAvailable(processor: assistant.processor))

        let older = #"{"user_id":9,"display_name":"Assistant","mention":"@ai"}"#
        let legacy = try APICoding.decoder().decode(AssistantDTO.self, from: Data(older.utf8))
        #expect(legacy.processor == nil)
        #expect(!AssistantConsent.isAvailable(processor: legacy.processor))
    }

    /// The refusal is TERMINAL: the server read the send and refused it,
    /// so the bubble goes red and waits for a person rather than being
    /// retried into a refusal that cannot change by itself.
    @Test("`assistant_consent_required` is a terminal send failure")
    func theRefusalIsTerminal() {
        #expect(
            ChatSyncCoordinator.terminalSendCodes.contains("assistant_consent_required"))
    }
}
