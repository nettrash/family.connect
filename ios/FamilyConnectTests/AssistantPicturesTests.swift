//
//  AssistantPicturesTests.swift
//  FamilyConnectTests
//
//  The two picture features and, mostly, the LOCKS on the first of them
//  (docs/protocol.md, "Pictures").
//
//  What is pinned here is not "does the photo upload" — a photo has always
//  uploaded, and an `ai` chat takes one exactly like any other chat does.
//  What is pinned is which SURFACES a client offers, because that is the
//  whole of this feature on this side: the server decides what leaves, and
//  the client decides what a member is invited to do and what they are told
//  about it before they do it.
//
//  The three rules, in the order they bind:
//
//  1. the operator configured a deployment that can SEE — `assistant.vision`
//     on `GET /families/mine`, and a server that did not is a server where
//     the surface is ABSENT, not disabled;
//  2. the family's owner turned `ai_vision` on — false by default, for
//     every family, including every family that existed before it;
//  3. the member attached the photo to THIS question. That one is not a
//     setting, has no storage, and must never gain any.
//
//  Nothing here can prove a pixel did not leave — only the server can — so
//  what these assert is the client half: the flags decode the way the
//  protocol says, the defaults fall the safe way round, and the composer's
//  own rule reads all three.
//

import Foundation
import Testing

@testable import FamilyConnect

@Suite("Assistant pictures")
struct AssistantPicturesTests {

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try APICoding.decoder().decode(T.self, from: Data(json.utf8))
    }

    // MARK: - What the server says it can do

    @Test("the assistant object carries what this server can do")
    func assistantCapabilities() throws {
        let assistant = try decode(
            AssistantDTO.self,
            """
            {"user_id": 1, "display_name": "Assistant", "mention": "@ai",
             "draw": "/draw", "vision": true, "images": true}
            """)
        #expect(assistant.vision)
        #expect(assistant.images)
        #expect(assistant.draw == "/draw")
        // And the token the server named is the one this build's grammar
        // is written against, which is the whole reason the field exists.
        #expect(assistant.draw == AssistantMention.drawToken)
    }

    /// A server that predates pictures sends none of the three keys. It
    /// must decode — the protocol's compatibility rule — and it must decode
    /// to "cannot", which is also exactly what a current server with
    /// neither deployment configured reports. The two are indistinguishable
    /// on purpose: both offer nothing.
    @Test("a server that predates pictures decodes, and can do neither")
    func assistantWithoutPictureKeys() throws {
        let assistant = try decode(
            AssistantDTO.self,
            """
            {"user_id": 1, "display_name": "Assistant", "mention": "@ai"}
            """)
        #expect(!assistant.vision)
        #expect(!assistant.images)
        #expect(assistant.draw == nil)
    }

    @Test("a server with only one deployment says so")
    func oneDeploymentOnly() throws {
        let seesOnly = try decode(
            AssistantDTO.self,
            """
            {"user_id": 1, "display_name": "Assistant", "mention": "@ai",
             "draw": "/draw", "vision": true, "images": false}
            """)
        #expect(seesOnly.vision)
        #expect(!seesOnly.images)
    }

    // MARK: - The family's own switch

    /// FALSE by default, and this is the assertion that matters most in
    /// this file. `ai_history` defaults to true and `ai_vision` defaults to
    /// the other one; a client that copied its neighbour's fallback would
    /// show every family a picture surface their owner never turned on.
    @Test("ai_vision is false when the server does not send it")
    func aiVisionDefaultsOff() throws {
        let family = try decode(
            FamilyDTO.self,
            """
            {"id": 3, "name": "The Smiths", "join_policy": "open",
             "created_at": "2026-08-19T17:03:12Z"}
            """)
        #expect(!family.aiVision)
        // Its neighbour still defaults the other way.
        #expect(family.aiHistory)
    }

    @Test("ai_vision is read when the server does send it")
    func aiVisionDecodes() throws {
        let on = try decode(
            FamilyDTO.self,
            """
            {"id": 3, "name": "The Smiths", "join_policy": "open",
             "created_at": "2026-08-19T17:03:12Z",
             "ai_history": false, "ai_vision": true}
            """)
        #expect(on.aiVision)
        #expect(!on.aiHistory)

        let off = try decode(
            FamilyDTO.self,
            """
            {"id": 3, "name": "The Smiths", "join_policy": "open",
             "created_at": "2026-08-19T17:03:12Z",
             "ai_history": true, "ai_vision": false}
            """)
        #expect(!off.aiVision)
    }

    // MARK: - The third switch (protocol.md, "Recent photos from the family chat")

    /// FALSE when absent — for a family that predates it and for a server
    /// that predates it, which are the same answer: no photo nobody pointed
    /// at ever leaves such a server.
    @Test("ai_history_photos is false when the server does not send it")
    func historyPhotosDefaultsOff() throws {
        let family = try decode(
            FamilyDTO.self,
            """
            {"id": 3, "name": "The Smiths", "join_policy": "open",
             "created_at": "2026-08-19T17:03:12Z", "ai_vision": true}
            """)
        #expect(!family.aiHistoryPhotos)
        #expect(family.aiVision)
    }

    @Test("ai_history_photos is read when the server does send it")
    func historyPhotosDecodes() throws {
        let on = try decode(
            FamilyDTO.self,
            """
            {"id": 3, "name": "The Smiths", "join_policy": "open",
             "created_at": "2026-08-19T17:03:12Z",
             "ai_history": true, "ai_vision": true, "ai_history_photos": true}
            """)
        #expect(on.aiHistoryPhotos)
        let off = try decode(
            FamilyDTO.self,
            """
            {"id": 3, "name": "The Smiths", "join_policy": "open",
             "created_at": "2026-08-19T17:03:12Z",
             "ai_history": true, "ai_vision": true, "ai_history_photos": false}
            """)
        #expect(!off.aiHistoryPhotos)
    }

    /// The switch on the owner's screen: offered with both locks open,
    /// otherwise DISABLED with the reason — unlike `ai_vision`, which is
    /// absent on a server that cannot see. The deployment is asked first,
    /// so an owner on such a server is told that rather than "turn
    /// pictures on first", which they could do to no effect.
    @Test("the Recent photos switch is offered, or withheld with its reason")
    func historyPhotosSwitchStates() {
        #expect(AssistantSurfaces.historyPhotosSwitch(serverCanSee: true, familyAllowsPhotos: true) == .offered)
        #expect(AssistantSurfaces.historyPhotosSwitch(serverCanSee: true, familyAllowsPhotos: false) == .withheldVisionOff)
        #expect(AssistantSurfaces.historyPhotosSwitch(serverCanSee: false, familyAllowsPhotos: true) == .withheldNoVisionDeployment)
        #expect(AssistantSurfaces.historyPhotosSwitch(serverCanSee: false, familyAllowsPhotos: false) == .withheldNoVisionDeployment)
        #expect(AssistantSurfaces.HistoryPhotosSwitch.offered.isEnabled)
        #expect(!AssistantSurfaces.HistoryPhotosSwitch.withheldVisionOff.isEnabled)
        #expect(!AssistantSurfaces.HistoryPhotosSwitch.withheldNoVisionDeployment.isEnabled)
    }

    /// The whole `GET /families/mine` shape, because that is the one read
    /// where the family switch and the server capability arrive together —
    /// and a composer needs BOTH before it offers anything.
    @Test("both halves of the vision gate arrive on one read")
    func familyMineCarriesBothHalves() throws {
        let mine = try decode(
            FamilyMineResponse.self,
            """
            {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                        "created_at": "2026-08-19T17:03:12Z",
                        "ai_history": true, "ai_vision": true},
             "members": [],
             "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai",
                           "draw": "/draw", "vision": true, "images": true},
             "blocked_user_ids": []}
            """)
        #expect(mine.family.aiVision)
        #expect(mine.assistant?.vision == true)
        #expect(mine.assistant?.images == true)
    }

    // MARK: - The bounds

    /// A video, a file, an audio note and a location never reach a model
    /// anywhere in this protocol, at any size, under any setting. Kind
    /// first, because it is the rule that actually turns people away.
    @Test("only JPEG and PNG photographs are shown to the model")
    func onlyPhotographsAreShown() {
        #expect(AssistantPictureLimits.isShownToModel(kind: "photo", mime: "image/jpeg"))
        #expect(AssistantPictureLimits.isShownToModel(kind: "photo", mime: "image/png"))
        // The case a client cannot rule out: another client's HEIC original.
        #expect(!AssistantPictureLimits.isShownToModel(kind: "photo", mime: "image/heic"))
        #expect(!AssistantPictureLimits.isShownToModel(kind: "photo", mime: "image/gif"))
        #expect(!AssistantPictureLimits.isShownToModel(kind: "video", mime: "video/mp4"))
        #expect(!AssistantPictureLimits.isShownToModel(kind: "audio", mime: "audio/mp4"))
        #expect(!AssistantPictureLimits.isShownToModel(kind: "file", mime: "application/pdf"))
        // Coordinates are barred from reaching a model anywhere in this
        // protocol, and this is not the exception.
        #expect(!AssistantPictureLimits.isShownToModel(kind: "location", mime: "application/json"))
    }

    @Test("the case of a declared type does not decide it")
    func mimeCaseIsIgnored() {
        #expect(AssistantPictureLimits.isShownToModel(kind: "photo", mime: "IMAGE/JPEG"))
    }

    /// Four, in the SENDER's order — which is the half that matters to a
    /// member reading "the first four go". The rest are named to the model,
    /// never silently dropped, which is the server's job; saying which ones
    /// they are is this side's.
    @Test("four photos off one message, in the order they were attached")
    func fourInSenderOrder() {
        let attachments: [(String, String, Int)] = [
            ("photo", "image/jpeg", 1),
            ("video", "video/mp4", 2),
            ("photo", "image/png", 3),
            ("photo", "image/jpeg", 4),
            ("photo", "image/heic", 5),
            ("photo", "image/jpeg", 6),
            ("photo", "image/jpeg", 7),
        ]
        let shown = AssistantPictureLimits.shown(
            attachments, kind: { $0.0 }, mime: { $0.1 })
        #expect(shown.map(\.2) == [1, 3, 4, 6])
        #expect(shown.count == AssistantPictureLimits.maxPerQuestion)
    }

    @Test("nothing to show is an empty answer, not a nil one")
    func nothingShowable() {
        let attachments = [("video", "video/mp4"), ("file", "text/plain")]
        #expect(AssistantPictureLimits.shown(attachments, kind: { $0.0 }, mime: { $0.1 }).isEmpty)
    }

    /// THE BOUND THE CLIENT USED TO HOLD AND NEVER APPLY.
    ///
    /// `maxBytes` was a constant with no call site: the composer's notice
    /// promised that a staged photograph "goes to the model your server is
    /// set up to use" whatever its size, while the server would leave a
    /// 6 MiB one out. A rule a client states and does not apply is worse
    /// than no rule at all — it is a promise made to somebody about their
    /// own photographs.
    @Test("a photograph over the ceiling is not shown to the model")
    func theSizeCeilingIsApplied() {
        let ceiling = AssistantPictureLimits.maxBytes
        #expect(AssistantPictureLimits.isShownToModel(
            kind: "photo", mime: "image/jpeg", bytes: ceiling))
        #expect(!AssistantPictureLimits.isShownToModel(
            kind: "photo", mime: "image/jpeg", bytes: ceiling + 1))
        // Not knowing is not the same as being over: an attachment on
        // somebody else's message has no size this client can measure, and
        // the two rules that ARE knowable still decide it.
        #expect(AssistantPictureLimits.isShownToModel(
            kind: "photo", mime: "image/jpeg", bytes: nil))
    }

    /// What actually travels is the PREVIEW where the client made one — so
    /// that, and not the original, is what a client must judge by. It is
    /// also why the HEIC rule almost never fires on this platform: a
    /// preview is a JPEG by definition.
    @Test("the preview is what a client judges, because it is what travels")
    func thePreviewIsWhatIsJudged() {
        #expect(AssistantPictureLimits.wireMIME(mime: "image/heic", hasPreview: true)
            == "image/jpeg")
        #expect(AssistantPictureLimits.wireMIME(mime: "image/heic", hasPreview: false)
            == "image/heic")
        #expect(AssistantPictureLimits.wireBytes(previewBytes: 40_000, originalBytes: 9_000_000)
            == 40_000)
        #expect(AssistantPictureLimits.wireBytes(previewBytes: nil, originalBytes: 9_000_000)
            == 9_000_000)
    }

    /// The composer's own question: of what is staged, what CAN travel —
    /// before the cap, because "too many" and "cannot travel at all" are
    /// different facts and it says different things about them.
    @Test("what can travel is decided by kind, encoding and size together")
    func carriedAppliesAllThreeRules() {
        let staged: [(kind: String, mime: String, bytes: Int, id: Int)] = [
            ("photo", "image/jpeg", 40_000, 1),
            ("photo", "image/jpeg", AssistantPictureLimits.maxBytes + 1, 2),
            ("photo", "image/heic", 40_000, 3),
            ("video", "video/mp4", 40_000, 4),
            ("photo", "image/png", AssistantPictureLimits.maxBytes, 5),
        ]
        let carried = AssistantPictureLimits.carried(
            staged, kind: { $0.kind }, mime: { $0.mime }, bytes: { $0.bytes })
        #expect(carried.map(\.id) == [1, 5])
        // …and the cap sits on top of exactly that set.
        let shown = AssistantPictureLimits.shown(
            staged, kind: { $0.kind }, mime: { $0.mime }, bytes: { $0.bytes })
        #expect(shown.map(\.id) == [1, 5])
    }

    @Test("the fixed bounds are the protocol's own numbers")
    func fixedBounds() {
        #expect(AssistantPictureLimits.maxPerQuestion == 4)
        #expect(AssistantPictureLimits.maxBytes == 5 * 1024 * 1024)
        #expect(AssistantPictureLimits.acceptedMIMETypes == ["image/jpeg", "image/png"])
    }

    // MARK: - What this client will offer

    /// The `/draw` capability check, spelling and all.
    ///
    /// `assistant.draw` exists so a client can be CERTAIN the server means
    /// the same five characters its own grammar does. A server that named a
    /// different token is one this build cannot compose for, and a button
    /// that typed the wrong thing would be exactly the affordance that
    /// silently does nothing.
    @Test("the picture-request affordance needs the capability and the spelling")
    func offersPictureRequests() {
        #expect(AssistantSurfaces.offersPictureRequests(
            serverCanDraw: true, serverToken: "/draw"))

        // No images deployment: `/draw` is just text on this server — no
        // error, no refusal, answered in words — so an affordance for it
        // would offer something that will not happen.
        #expect(!AssistantSurfaces.offersPictureRequests(
            serverCanDraw: false, serverToken: "/draw"))

        // A server naming a token this build does not speak.
        #expect(!AssistantSurfaces.offersPictureRequests(
            serverCanDraw: true, serverToken: "/picture"))

        // A server that named none at all predates pictures — where
        // `images` is false anyway, so this row is belt and braces.
        #expect(AssistantSurfaces.offersPictureRequests(
            serverCanDraw: true, serverToken: nil))
        #expect(!AssistantSurfaces.offersPictureRequests(
            serverCanDraw: false, serverToken: nil))
    }

    /// The vision gate the composers actually ask: three independent
    /// booleans, and ALL of them have to be true.
    ///
    /// Written out as a full table because every row but one must offer
    /// nothing, and a rule that had slipped to `||` — or that had forgotten
    /// the chat kind, which is what keeps this dedicated door out of the
    /// family chat — would still pass a test that only checked the happy
    /// row.
    @Test("a picture surface needs the chat, the server AND the family")
    func everyLock() {
        #expect(AssistantSurfaces.offersPictureAttach(
            isAssistantChat: true, serverCanSee: true, familyAllows: true))

        // The family's owner has not turned it on. Off by default, for
        // every family, including every family that existed before it.
        #expect(!AssistantSurfaces.offersPictureAttach(
            isAssistantChat: true, serverCanSee: true, familyAllows: false))

        // No vision deployment: absent, not disabled.
        #expect(!AssistantSurfaces.offersPictureAttach(
            isAssistantChat: true, serverCanSee: false, familyAllows: true))
        #expect(!AssistantSurfaces.offersPictureAttach(
            isAssistantChat: true, serverCanSee: false, familyAllows: false))

        // THE FAMILY CHAT, with both locks wide open: no dedicated door.
        // Not because a picture never reaches the assistant from there —
        // since #56 a photo on an `@ai` message, or on the message it
        // replies to, travels under these same two locks (protocol.md,
        // "Showing the assistant a picture from the family chat") — but
        // because that path rides the ordinary photo picker and the reply
        // affordance the family composer already has, and a second
        // "show the assistant" item there would offer nothing the first
        // does not. Same for a direct chat, and for anywhere else.
        #expect(!AssistantSurfaces.offersPictureAttach(
            isAssistantChat: false, serverCanSee: true, familyAllows: true))
        #expect(!AssistantSurfaces.offersPictureAttach(
            isAssistantChat: false, serverCanSee: true, familyAllows: false))
        #expect(!AssistantSurfaces.offersPictureAttach(
            isAssistantChat: false, serverCanSee: false, familyAllows: true))
        #expect(!AssistantSurfaces.offersPictureAttach(
            isAssistantChat: false, serverCanSee: false, familyAllows: false))
    }

    // MARK: - What a picture answer costs

    /// `images` is its OWN number, not a share of `questions`, because an
    /// image model reports no tokens at all: a picture answer is one
    /// question, zero prompt tokens, zero completion tokens and one image.
    /// A family shown only the token counts would see the expensive half of
    /// their assistant as free (protocol.md, "Family statistics").
    @Test("a picture answer costs one question, no tokens and one image")
    func pictureAnswerStatistics() throws {
        let ai = try decode(
            AiStatsDTO.self,
            """
            {"questions": 1, "prompt_tokens": 0, "completion_tokens": 0, "images": 1}
            """)
        #expect(ai.questions == 1)
        #expect(ai.promptTokens == 0)
        #expect(ai.completionTokens == 0)
        #expect(ai.images == 1)
    }

    /// Always present, `0` on a server that has never generated one — and
    /// ABSENT on a server that predates the field, which must still decode
    /// rather than take the whole statistics screen down with it.
    @Test("a server that predates pictures still decodes its statistics")
    func statisticsWithoutImages() throws {
        let ai = try decode(
            AiStatsDTO.self,
            """
            {"questions": 43, "prompt_tokens": 12040, "completion_tokens": 30512}
            """)
        #expect(ai.questions == 43)
        #expect(ai.images == 0)
    }

    /// The owner's switch travels as a plain boolean under `ai_vision`, and
    /// as NOTHING ELSE: it is not one of the two fields where a JSON null
    /// means something a missing key does not.
    @Test("turning the switch on sends one field")
    func patchBodyShape() async throws {
        let host = "ai-vision-patch.test"
        StubURLProtocol.register(host: host) { _ in
            .json(200, """
                {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                            "created_at": "2026-08-19T17:03:12Z",
                            "ai_history": true, "ai_vision": true}}
                """)
        }
        defer { StubURLProtocol.unregister(host: host) }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())

        let family = try await api.setAIVision(true)
        #expect(family.aiVision)

        let sent = StubURLProtocol.requests(host: host)
        #expect(sent.count == 1)
        #expect(sent.first?.method == "PATCH")
        // Suffix, not equality: every request carries the API prefix.
        #expect(sent.first?.url.path.hasSuffix("/families/mine") == true)
        let body = try #require(sent.first?.bodyJSON())
        #expect(body.count == 1)
        #expect(body["ai_vision"] as? Bool == true)
    }

    /// The third switch's PATCH: one key, and only when touched. The
    /// family that comes back is the truth for BOTH picture switches —
    /// turning `ai_vision` off turns this one off in the same write.
    @Test("ai_history_photos travels as one key on PATCH /families/mine")
    func patchBodyHistoryPhotos() async throws {
        let host = "ai-history-photos.test"
        StubURLProtocol.register(host: host) { _ in
            .json(200, """
                {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                            "created_at": "2026-08-19T17:03:12Z",
                            "ai_history": true, "ai_vision": true, "ai_history_photos": true}}
                """)
        }
        defer { StubURLProtocol.unregister(host: host) }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())

        let family = try await api.setAIHistoryPhotos(true)
        #expect(family.aiHistoryPhotos)
        #expect(family.aiVision)

        let sent = StubURLProtocol.requests(host: host)
        #expect(sent.count == 1)
        #expect(sent.first?.method == "PATCH")
        #expect(sent.first?.url.path.hasSuffix("/families/mine") == true)
        let body = try #require(sent.first?.bodyJSON())
        #expect(body.count == 1)
        #expect(body["ai_history_photos"] as? Bool == true)
        #expect(body["ai_vision"] == nil)
    }

    /// Turning `ai_vision` off: the request still carries only that key,
    /// and the server's answer carries the third switch OFF with it —
    /// which is what this client then shows, without having asked.
    @Test("turning ai_vision off brings ai_history_photos off in the answer")
    func patchVisionOffTakesHistoryPhotosWithIt() async throws {
        let host = "ai-vision-off-cascade.test"
        StubURLProtocol.register(host: host) { _ in
            .json(200, """
                {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                            "created_at": "2026-08-19T17:03:12Z",
                            "ai_history": true, "ai_vision": false, "ai_history_photos": false}}
                """)
        }
        defer { StubURLProtocol.unregister(host: host) }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())

        let family = try await api.setAIVision(false)
        #expect(!family.aiVision)
        #expect(!family.aiHistoryPhotos)
        let body = try #require(StubURLProtocol.requests(host: host).first?.bodyJSON())
        #expect(body.count == 1)
        #expect(body["ai_vision"] as? Bool == false)
    }

    @Test("turning it off sends false, not an absent key")
    func patchBodyOff() async throws {
        let host = "ai-vision-off.test"
        StubURLProtocol.register(host: host) { _ in
            .json(200, """
                {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                            "created_at": "2026-08-19T17:03:12Z",
                            "ai_history": true, "ai_vision": false}}
                """)
        }
        defer { StubURLProtocol.unregister(host: host) }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())

        let family = try await api.setAIVision(false)
        #expect(!family.aiVision)
        let body = try #require(StubURLProtocol.requests(host: host).first?.bodyJSON())
        #expect(body["ai_vision"] as? Bool == false)
    }
}
