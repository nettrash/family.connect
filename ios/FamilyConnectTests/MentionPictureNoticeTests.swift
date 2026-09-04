//
//  MentionPictureNoticeTests.swift
//  FamilyConnectTests
//
//  The family composer's disclosure strip (docs/protocol.md, "Showing the
//  assistant a picture from the family chat" — "What a client's family-chat
//  composer must say").
//
//  Every row here is a sentence that would otherwise be a lie: shown when
//  nothing will leave, absent when something will, or a "4" that is not the
//  four the server will send. The counting is pinned against the server's
//  own — the mention's photos first, then the quote's, one budget across
//  both — because that is the whole reason the number is allowed to appear.
//

import Foundation
import Testing

@testable import FamilyConnect

@Suite("Family-chat picture notice")
struct MentionPictureNoticeTests {

    /// A photo this app made: a JPEG with a preview, well under the ceiling.
    private func photo(bytes: Int? = 40_000, mime: String = "image/jpeg")
        -> AssistantPictureCandidate
    {
        AssistantPictureCandidate(kind: "photo", mime: mime, bytes: bytes)
    }

    private let video = AssistantPictureCandidate(kind: "video", mime: "video/mp4", bytes: 900_000)

    private func notice(
        draft: String = "@ai what is this?",
        staged: [AssistantPictureCandidate] = [],
        quoted: [AssistantPictureCandidate] = [],
        serverCanSee: Bool = true,
        familyAllows: Bool = true,
        familyHistory: Bool = true,
        familyHistoryPhotos: Bool = false,
        serverCanDraw: Bool = true
    ) -> MentionPictureNotice? {
        MentionPictureNotice.of(
            draft: draft, staged: staged, quoted: quoted,
            serverCanSee: serverCanSee, familyAllows: familyAllows,
            familyHistory: familyHistory, familyHistoryPhotos: familyHistoryPhotos,
            serverCanDraw: serverCanDraw)
    }

    /// The same, with the owner's third switch in effect.
    private func recent(
        draft: String = "@ai what is this?",
        staged: [AssistantPictureCandidate] = [],
        quoted: [AssistantPictureCandidate] = [],
        serverCanSee: Bool = true,
        familyAllows: Bool = true,
        familyHistory: Bool = true,
        serverCanDraw: Bool = true
    ) -> MentionPictureNotice? {
        notice(
            draft: draft, staged: staged, quoted: quoted,
            serverCanSee: serverCanSee, familyAllows: familyAllows,
            familyHistory: familyHistory, familyHistoryPhotos: true,
            serverCanDraw: serverCanDraw)
    }

    // MARK: - When it is absent

    /// Either lock shut and nothing leaves — so nothing is announced. The
    /// family composer does not name the lock: a member who is not the
    /// owner can open neither, and naming it would be a way to probe.
    @Test("absent with either lock shut")
    func absentWithALockShut() {
        #expect(notice(staged: [photo()], serverCanSee: false) == nil)
        #expect(notice(staged: [photo()], familyAllows: false) == nil)
        #expect(notice(quoted: [photo()], serverCanSee: false, familyAllows: false) == nil)
        #expect(notice(staged: [photo()], quoted: [photo()], familyAllows: false) == nil)
    }

    /// Without `@ai` the photo is an ordinary attachment on an ordinary
    /// message, and the assistant is not in the conversation at all.
    @Test("absent when the draft does not mention the assistant")
    func absentWithoutAMention() {
        #expect(notice(draft: "what is this?", staged: [photo()]) == nil)
        #expect(notice(draft: "", staged: [photo()], quoted: [photo()]) == nil)
        // `anna@ai.example` is not a mention, by the same grammar the
        // server applies — and the strip must agree with the server.
        #expect(notice(draft: "mail anna@ai.example this", quoted: [photo()]) == nil)
    }

    @Test("absent when there is no photograph anywhere")
    func absentWithoutAPhotograph() {
        #expect(notice() == nil)
        #expect(notice(staged: [video]) == nil)
        #expect(notice(quoted: [video]) == nil)
    }

    /// `@ai /draw …` sends the words after the token and nothing else — not
    /// the photo — on a server that can draw. On one that cannot, it is an
    /// ordinary mention and the photo goes.
    @Test("absent for a picture request, unless the server cannot draw")
    func absentForADrawRequest() {
        #expect(notice(draft: "@ai /draw a cat", staged: [photo()]) == nil)
        #expect(notice(draft: "@ai /draw a cat", quoted: [photo()]) == nil)
        #expect(notice(draft: "@ai /draw a cat", staged: [photo()], serverCanDraw: false)
            == MentionPictureNotice(shownOnMention: 1, shownOnQuote: 0, extra: 0, unreadable: 0))
        // `/draw` not at the front is just a word; the photo goes.
        #expect(notice(draft: "@ai what does /draw do?", staged: [photo()]) != nil)
    }

    // MARK: - When it is present

    @Test("present with a staged photo on an @ai draft")
    func presentWithAStagedPhoto() {
        #expect(notice(staged: [photo()])
            == MentionPictureNotice(shownOnMention: 1, shownOnQuote: 0, extra: 0, unreadable: 0))
        // The token's case does not matter to the server, so not here.
        #expect(notice(draft: "@AI look", staged: [photo(), photo()])
            == MentionPictureNotice(shownOnMention: 2, shownOnQuote: 0, extra: 0, unreadable: 0))
    }

    @Test("present with a photo on the message being replied to")
    func presentWithAQuotedPhoto() {
        #expect(notice(quoted: [photo()])
            == MentionPictureNotice(shownOnMention: 0, shownOnQuote: 1, extra: 0, unreadable: 0))
        // Other attachments on the quoted message are not photos and do
        // not count, in either direction.
        #expect(notice(quoted: [video, photo(), video])
            == MentionPictureNotice(shownOnMention: 0, shownOnQuote: 1, extra: 0, unreadable: 0))
    }

    /// THE SHARED BUDGET. Four across both messages, the mention's first,
    /// exactly as `vision_images` is called twice on the server with one
    /// budget — never four each.
    @Test("the four are counted across mention and quote, mention first")
    func countIsHonestAcrossMentionAndQuote() {
        // Three of mine and three of theirs: mine all go, one of theirs.
        #expect(notice(staged: [photo(), photo(), photo()], quoted: [photo(), photo(), photo()])
            == MentionPictureNotice(shownOnMention: 3, shownOnQuote: 1, extra: 2, unreadable: 0))
        // Five of mine: the quote gets nothing, and the fifth is named.
        #expect(notice(staged: Array(repeating: photo(), count: 5), quoted: [photo()])
            == MentionPictureNotice(shownOnMention: 4, shownOnQuote: 0, extra: 2, unreadable: 0))
        // Two and two: exactly the budget, nothing left out.
        #expect(notice(staged: [photo(), photo()], quoted: [photo(), photo()])
            == MentionPictureNotice(shownOnMention: 2, shownOnQuote: 2, extra: 0, unreadable: 0))
        // Six of theirs alone: four go, two are named.
        #expect(notice(quoted: Array(repeating: photo(), count: 6))
            == MentionPictureNotice(shownOnMention: 0, shownOnQuote: 4, extra: 2, unreadable: 0))
        #expect(notice(staged: [photo(), photo(), photo()], quoted: [photo(), photo(), photo()])?
            .shown == AssistantPictureLimits.maxPerQuestion)
    }

    /// A photo that cannot travel does not spend a place and is said out
    /// loud — the same two rules the server applies, on either message.
    @Test("a photo that will not travel is counted as such, on either message")
    func unreadableIsNamed() {
        let heic = photo(bytes: 3_000_000, mime: "image/heic")
        let huge = photo(bytes: AssistantPictureLimits.maxBytes + 1)
        #expect(notice(staged: [heic])
            == MentionPictureNotice(shownOnMention: 0, shownOnQuote: 0, extra: 0, unreadable: 1))
        #expect(notice(quoted: [huge])
            == MentionPictureNotice(shownOnMention: 0, shownOnQuote: 0, extra: 0, unreadable: 1))
        // It does not take a place: four readable ones still go.
        #expect(notice(staged: [heic, photo(), photo()], quoted: [huge, photo(), photo()])
            == MentionPictureNotice(shownOnMention: 2, shownOnQuote: 2, extra: 0, unreadable: 2))
        // A size this client cannot know is not a size that is over.
        #expect(notice(quoted: [photo(bytes: nil)])
            == MentionPictureNotice(shownOnMention: 0, shownOnQuote: 1, extra: 0, unreadable: 0))
    }

    // MARK: - What the sentence says

    @Test("the sentence names which photo goes, and the token")
    func sentenceNamesWhichPhoto() throws {
        let mine = try #require(notice(staged: [photo()])).sentence
        #expect(mine.hasPrefix("This goes to the model"))
        #expect(mine.contains(AssistantMention.token))
        #expect(mine.contains("No other photo in this chat does."))

        let theirs = try #require(notice(quoted: [photo()])).sentence
        #expect(theirs.hasPrefix("The photo you're replying to goes"))
        #expect(theirs.contains(AssistantMention.token))

        let both = try #require(notice(staged: [photo()], quoted: [photo()])).sentence
        #expect(both.hasPrefix("This and the photo you're replying to go"))
    }

    /// The "4" a member reads is `AssistantPictureLimits.maxPerQuestion`
    /// and nothing else, and the sentence says the order it is counted in.
    @Test("past the budget the sentence carries the protocol's four and the order")
    func sentenceOverTheBudget() throws {
        let over = try #require(notice(staged: [photo(), photo(), photo()], quoted: [photo(), photo()]))
        #expect(over.extra == 1)
        #expect(over.sentence.contains("first \(AssistantPictureLimits.maxPerQuestion) photos"))
        #expect(over.sentence.contains("yours first, then the ones you're replying to"))
    }

    /// The private thread's own words for a photo that cannot travel —
    /// the same disclosure, so the same sentence.
    @Test("a photo that will not travel is said in the private strip's words")
    func sentenceForAnUnreadablePhoto() throws {
        let unreadable = try #require(notice(quoted: [photo(bytes: 10, mime: "image/heic")]))
        #expect(unreadable.sentence
            == "A photo here is too large, or in a format the model can't read, so it will be told it's here but won't be shown it.")
        // Ahead of the cap, on the rare draft that trips both.
        let both = try #require(notice(
            staged: Array(repeating: photo(), count: 5),
            quoted: [photo(bytes: 10, mime: "image/heic")]))
        #expect(both.extra == 1 && both.unreadable == 1)
        #expect(both.sentence.hasPrefix("A photo here is too large"))
    }

    // MARK: - Recent photos (the owner's third switch)

    /// Every row of #56 above ran with the switch off, and every notice
    /// they pinned carries `recentUpTo == nil` — so the strip under a
    /// family that never turned it on is byte for byte what #56 shipped.
    /// This row makes that explicit for the one case that could drift.
    @Test("without the third switch nothing about recent photos is said")
    func nothingRecentWithoutTheSwitch() throws {
        let plain = try #require(notice(staged: [photo()]))
        #expect(plain.recentUpTo == nil)
        #expect(plain.sentence.contains("No other photo in this chat does."))
        #expect(!plain.sentence.contains("most recent photos"))
        // A bare `@ai` draft has nothing to announce.
        #expect(notice() == nil)
    }

    /// The switch does nothing on its own: both locks still have to be
    /// open (the server refuses to even turn it on under a shut
    /// `ai_vision`, but a cached family may still say so), and it needs a
    /// transcript — with `ai_history` off no photo from the history can
    /// travel, so there is nothing to announce (protocol.md, "Recent photos
    /// from the family chat" — "It requires everything below it").
    @Test("the third switch is inert with a lock shut or without history")
    func recentIsInertWithoutItsPrerequisites() {
        #expect(recent(serverCanSee: false) == nil)
        #expect(recent(familyAllows: false) == nil)
        #expect(recent(staged: [photo()], familyAllows: false) == nil)
        // No transcript, no history photos — and with a photo of the
        // member's own the strip is exactly #56's.
        #expect(recent(familyHistory: false) == nil)
        #expect(recent(staged: [photo()], familyHistory: false)
            == MentionPictureNotice(shownOnMention: 1, shownOnQuote: 0, extra: 0, unreadable: 0))
        #expect(recent(staged: [photo()], familyHistory: false)?.recentUpTo == nil)
    }

    /// THE CASE #56'S STRIP WAS ABSENT FOR: an `@ai` draft with no photo
    /// of its own and none on its quote. Under the switch that is exactly
    /// the mention on which all four may be somebody else's picture, so
    /// the strip shows — with the whole budget left for history.
    @Test("with the switch on a bare @ai draft shows the strip with all four left")
    func bareMentionShowsUnderTheSwitch() throws {
        let bare = try #require(recent())
        #expect(bare == MentionPictureNotice(shownOnMention: 0, shownOnQuote: 0, extra: 0, unreadable: 0, recentUpTo: 4))
        #expect(bare.recentUpTo == AssistantPictureLimits.maxPerQuestion)
        #expect(bare.sentence.hasPrefix("Up to 4 of the most recent photos in this chat go"))
        #expect(bare.sentence.contains(AssistantMention.token))
        #expect(bare.sentence.contains("nobody pointed it at"))
        // Still not without a mention, and still not for a picture
        // request on a server that can draw — a `/draw` sends the words
        // after the token and nothing else, at every setting.
        #expect(recent(draft: "what is this?") == nil)
        #expect(recent(draft: "@ai /draw a cat") == nil)
        #expect(recent(draft: "@ai /draw a cat", staged: [photo()]) == nil)
        #expect(recent(draft: "@ai /draw a cat", serverCanDraw: false)?.recentUpTo == 4)
    }

    /// THE ARITHMETIC. History fills only what the mention and the quote
    /// left of the same four — N = 4, 3, 2, 1, 0 — exactly as the server
    /// walks the transcript newest-first for "what is left". A photo the
    /// member pointed at always beats an ambient one.
    @Test("history takes what is left of the four: 4, 3, 2, 1, 0")
    func recentFillsWhatIsLeft() throws {
        for own in 0...4 {
            let n = try #require(recent(staged: Array(repeating: photo(), count: own)))
            #expect(n.shownOnMention == own)
            #expect(n.recentUpTo == AssistantPictureLimits.maxPerQuestion - own)
        }
        // Across the two messages, the mention's first, then the quote's.
        #expect(recent(staged: [photo()], quoted: [photo()])?.recentUpTo == 2)
        #expect(recent(quoted: [photo(), photo(), photo()])?.recentUpTo == 1)
        #expect(recent(staged: [photo(), photo()], quoted: [photo(), photo()])?.recentUpTo == 0)
        // Past the budget nothing is left at all, and the cap sentence
        // needs no variant.
        let over = try #require(recent(staged: Array(repeating: photo(), count: 5)))
        #expect(over.recentUpTo == 0 && over.extra == 1)
        #expect(over.sentence.hasPrefix("Only the first 4 photos go"))
        // A photo that cannot travel spends no place, so history may
        // still fill it — and the private strip's sentence for it is kept
        // word for word, with the history clause after it.
        let heic = try #require(recent(staged: [photo(bytes: 10, mime: "image/heic")]))
        #expect(heic.recentUpTo == 4 && heic.unreadable == 1)
        #expect(heic.sentence.hasPrefix("A photo here is too large"))
        #expect(heic.sentence.hasSuffix("Up to 4 of the most recent photos in this chat may still go."))
        #expect(recent(staged: [photo(bytes: 10, mime: "image/heic"), photo()])?.recentUpTo == 3)
    }

    /// "No other photo in this chat does" survives only where it is true.
    /// With places left it becomes "up to N", and the sentence still says
    /// which of the member's own photos go and in what order the four are
    /// counted; with the four spent by the member's own photos, nothing
    /// from the history travels and #56's sentence is exactly right.
    @Test("the sentence says up to N with places left, and #56's when none are")
    func recentSentences() throws {
        let mine = try #require(recent(staged: [photo()])).sentence
        #expect(mine.hasPrefix("This goes to the model"))
        #expect(mine.contains("up to 3 of the most recent photos in this chat may go too"))
        #expect(!mine.contains("No other photo"))

        let theirs = try #require(recent(quoted: [photo(), photo()])).sentence
        #expect(theirs.hasPrefix("The photo you're replying to goes"))
        #expect(theirs.contains("up to 2 of the most recent photos"))

        let both = try #require(recent(staged: [photo()], quoted: [photo(), photo()])).sentence
        #expect(both.hasPrefix("This and the photo you're replying to go"))
        #expect(both.contains("up to 1 of the most recent photos"))

        // The budget spent: no recent photo goes, and the sentence says so
        // in #56's words — the same words a family without the switch
        // reads, because the same thing happens.
        let spent = try #require(recent(staged: [photo(), photo()], quoted: [photo(), photo()]))
        #expect(spent.recentUpTo == 0)
        #expect(spent.sentence == notice(staged: [photo(), photo()], quoted: [photo(), photo()])?.sentence)
        #expect(spent.sentence.contains("No other photo in this chat does."))
    }

    // MARK: - Reducing a message's attachment to the three facts

    /// The server prefers the preview, whose length is not on the wire;
    /// so a previewed attachment is judged as a JPEG of unknown size, and
    /// an un-previewed one by its own type and stored size.
    @Test("an attachment on somebody's message is judged as it will travel")
    func candidateFromAnAttachment() throws {
        let previewed = try APICoding.decoder().decode(
            AttachmentDTO.self,
            from: Data(
                """
                {"id": 34, "kind": "photo", "mime": "image/heic", "size": 9000000,
                 "width": 4032, "height": 3024, "has_preview": true}
                """.utf8))
        let candidate = AssistantPictureCandidate(attachment: previewed)
        #expect(candidate == AssistantPictureCandidate(kind: "photo", mime: "image/jpeg", bytes: nil))

        let original = try APICoding.decoder().decode(
            AttachmentDTO.self,
            from: Data(
                """
                {"id": 35, "kind": "photo", "mime": "image/heic", "size": 9000000,
                 "width": 4032, "height": 3024, "has_preview": false}
                """.utf8))
        #expect(AssistantPictureCandidate(attachment: original)
            == AssistantPictureCandidate(kind: "photo", mime: "image/heic", bytes: 9_000_000))
        #expect(notice(quoted: [AssistantPictureCandidate(attachment: original)])?.unreadable == 1)
        #expect(notice(quoted: [AssistantPictureCandidate(attachment: previewed)])?.shownOnQuote == 1)
    }
}
