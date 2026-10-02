//
//  PictureRequestHintTests.swift
//  FamilyConnectTests
//
//  The line a member reads while asking for a picture — "Describe people
//  and things in general words — real names and brands are often refused."
//  (docs/protocol.md, "Pictures" — "A refused description is reworded
//  once").
//
//  Three things are pinned here. The GRAMMAR half — "is this draft being
//  written as a picture request?" — is `drawPrompt`'s scan with the prompt
//  allowed to be empty, so it is run over the shared `/draw` vectors: a
//  draft the server would draw from must show the hint, and a draft the
//  server would never draw from, however it is finished, must not. The
//  COMPOSER rule, as a table over chat kind, capability and edit state.
//  And the BACKDROP rule, which is one answer for the control and the hint
//  so the sentence cannot sit beside a control that is not there.
//

import Foundation
import Testing

@testable import FamilyConnect

@Suite("The picture-request hint")
struct PictureRequestHintTests {

    // MARK: - The grammar: being written as a request

    /// Every shared vector that asks for a picture is, necessarily, being
    /// written as one — the looser question can never say no where the
    /// stricter one says yes.
    @Test("every draft that asks for a picture shows as being written as one")
    func everyDrawStartsARequest() {
        for (body, _) in AssistantMentionTests.draws {
            #expect(AssistantMention.startsPictureRequest(body), "\(body)")
        }
    }

    /// The shared NON-draws, and the one way they may differ: a token and
    /// whitespace with nothing after it yet. That is exactly the draft the
    /// paintbrush leaves behind, and the only row here the hint is for.
    /// Every other non-draw — a longer word, the token not first, a
    /// homoglyph, a combining mark, a separator that is not whitespace —
    /// stays a non-request however it is finished, so no hint.
    @Test("a non-draw is being written as one only while its prompt is still empty")
    func nonDrawsStartARequestOnlyWhenThePromptIsEmpty() {
        let emptyPrompt: Set<String> = ["  /draw  "]
        for body in AssistantMentionTests.notDraws {
            #expect(
                AssistantMention.startsPictureRequest(body) == emptyPrompt.contains(body),
                "\(body)")
        }
    }

    @Test("what the paintbrush types, and the shapes around it")
    func theTokenAndWhitespaceIsEnough() {
        // What `insertDrawToken` leaves in an empty draft.
        #expect(AssistantMention.startsPictureRequest("\(AssistantMention.drawToken) "))
        // Leading whitespace is skipped, the way `drawPrompt` skips it.
        #expect(AssistantMention.startsPictureRequest("\n\t /draw "))
        #expect(AssistantMention.startsPictureRequest("/DRAW "))
        #expect(AssistantMention.startsPictureRequest("/draw\n"))
        // U+00A0 is `White_Space`, so it ends the token as a space does.
        #expect(AssistantMention.startsPictureRequest("/draw\u{A0}"))
        // The family chat's one leading mention.
        #expect(AssistantMention.startsPictureRequest("@ai /draw "))
        #expect(AssistantMention.startsPictureRequest("  @AI\n/draw "))

        // No whitespace yet: `/draw` could still become `/drawer`.
        #expect(!AssistantMention.startsPictureRequest("/draw"))
        #expect(!AssistantMention.startsPictureRequest("/dra"))
        #expect(!AssistantMention.startsPictureRequest(""))
        #expect(!AssistantMention.startsPictureRequest("   "))
        // U+200B is not whitespace, so it makes a longer word.
        #expect(!AssistantMention.startsPictureRequest("/draw\u{200B}"))
        // Not first.
        #expect(!AssistantMention.startsPictureRequest("hey /draw "))
        #expect(!AssistantMention.startsPictureRequest("@ai @ai /draw "))
        #expect(!AssistantMention.startsPictureRequest("@aiden /draw "))
    }

    // MARK: - The composer

    @Test("the assistant's own chat shows it from the moment the token is typed")
    func assistantChatShowsIt() {
        for draft in ["/draw ", "/draw a cat", "  /Draw a cat", "@ai /draw a cat"] {
            #expect(PictureRequestHint.showsInComposer(
                chatKind: "ai", draft: draft, isEditing: false, offersPictures: true), "\(draft)")
        }
        for draft in ["", "a cat", "/draw", "/drawer", "please /draw a cat"] {
            #expect(!PictureRequestHint.showsInComposer(
                chatKind: "ai", draft: draft, isEditing: false, offersPictures: true), "\(draft)")
        }
    }

    /// In the family chat `/draw` only asks with the `@ai` in front of it —
    /// without one the draft never reaches the assistant, and advice about
    /// the picture provider would be advice about nothing.
    @Test("the family chat shows it only for @ai /draw")
    func familyChatNeedsTheMention() {
        #expect(PictureRequestHint.showsInComposer(
            chatKind: "family", draft: "@ai /draw ", isEditing: false, offersPictures: true))
        #expect(PictureRequestHint.showsInComposer(
            chatKind: "family", draft: "@ai /draw a cat", isEditing: false, offersPictures: true))
        #expect(!PictureRequestHint.showsInComposer(
            chatKind: "family", draft: "/draw a cat", isEditing: false, offersPictures: true))
        #expect(!PictureRequestHint.showsInComposer(
            chatKind: "family", draft: "@ai a cat", isEditing: false, offersPictures: true))
        #expect(!PictureRequestHint.showsInComposer(
            chatKind: "family", draft: "hey @ai /draw a cat", isEditing: false, offersPictures: true))
    }

    @Test("a chat that never reaches the assistant never shows it")
    func otherChatsNever() {
        for kind in ["direct", "group", nil] as [String?] {
            #expect(!PictureRequestHint.showsInComposer(
                chatKind: kind, draft: "/draw a cat", isEditing: false, offersPictures: true))
            #expect(!PictureRequestHint.showsInComposer(
                chatKind: kind, draft: "@ai /draw a cat", isEditing: false, offersPictures: true))
        }
    }

    /// No images deployment (or a token this build does not speak): `/draw`
    /// is answered in words, and there is no picture filter to warn about.
    @Test("a server that cannot draw never shows it")
    func noPicturesNoHint() {
        for kind in ["ai", "family"] {
            #expect(!PictureRequestHint.showsInComposer(
                chatKind: kind, draft: "@ai /draw a cat", isEditing: false, offersPictures: false))
        }
    }

    /// Rewriting an old message calls no model, so a `/draw` in the draft
    /// being fixed is not a picture being asked for.
    @Test("an edit never shows it")
    func editingNever() {
        for kind in ["ai", "family"] {
            #expect(!PictureRequestHint.showsInComposer(
                chatKind: kind, draft: "@ai /draw a cat", isEditing: true, offersPictures: true))
        }
    }

    // MARK: - The event backdrop

    /// Written out as the full table: sixteen rows, and every one but the
    /// last must offer nothing.
    @Test("the backdrop control, and the hint beside it, need all four")
    func backdropNeedsAllFour() {
        for isEvent in [false, true] {
            for isSaved in [false, true] {
                for isAuthor in [false, true] {
                    for serverCanDraw in [false, true] {
                        #expect(
                            PictureRequestHint.offersBackdrop(
                                isEvent: isEvent, isSaved: isSaved,
                                isAuthor: isAuthor, serverCanDraw: serverCanDraw)
                                == (isEvent && isSaved && isAuthor && serverCanDraw),
                            "event \(isEvent) saved \(isSaved) author \(isAuthor) draw \(serverCanDraw)")
                    }
                }
            }
        }
    }

    // MARK: - The sentence

    /// One key, looked up rather than spelled twice — and a key that is in
    /// the catalogue, so it is never shown as a bare identifier.
    @Test("the sentence is the catalogue's")
    func sentenceIsTheCatalogues() {
        #expect(PictureRequestHint.text == String(localized: "Describe people and things in general words — real names and brands are often refused."))
        #expect(!PictureRequestHint.text.isEmpty)
    }
}
