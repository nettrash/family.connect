//
//  BackdropDoorTests.swift
//  FamilyConnectTests
//
//  What "Draw a backdrop" is on one event (docs/protocol.md, "Board" and
//  "Consenting to the assistant"): absent, open, or asking the consent
//  question first. The backdrop is drawn from the title — the author's
//  words going to the model — so it is asked the question a `/draw` is,
//  on the phone's sheet and the Mac's menu alike.
//

import Foundation
import Testing

@testable import FamilyConnect

@Suite("The backdrop door")
struct BackdropDoorTests {

    private static let agreed = ISO8601DateFormatter().date(from: "2026-09-01T12:00:00Z")!

    /// The whole table: every row `PictureRequestHint.offersBackdrop`
    /// refuses stays absent whatever the consent, and every row it offers
    /// asks first until the author has agreed.
    @Test("absent where the control is not offered, asking until agreed where it is")
    func fullTable() {
        for isEvent in [false, true] {
            for isSaved in [false, true] {
                for isAuthor in [false, true] {
                    for serverCanDraw in [false, true] {
                        for agreedAt in [nil, Self.agreed] {
                            let door = BackdropDoor.of(
                                isEvent: isEvent, isSaved: isSaved, isAuthor: isAuthor,
                                serverCanDraw: serverCanDraw,
                                processor: "Example AI", agreedAt: agreedAt)
                            let offered = isEvent && isSaved && isAuthor && serverCanDraw
                            let expected: BackdropDoor =
                                !offered ? .absent : (agreedAt == nil ? .asksFirst : .open)
                            #expect(
                                door == expected,
                                "event \(isEvent) saved \(isSaved) author \(isAuthor) draw \(serverCanDraw) agreed \(agreedAt != nil)")
                            #expect(door.isOffered == offered)
                        }
                    }
                }
            }
        }
    }

    /// A server that names no processor has an assistant this client must
    /// not use: there is no honest way to ask, so no backdrop at all —
    /// even for an author whose agreement is on record.
    @Test("an unnamed processor offers no backdrop")
    func unnamedProcessorIsAbsent() {
        for processor in [nil, "", "   ", "\n"] as [String?] {
            for agreedAt in [nil, Self.agreed] {
                #expect(
                    BackdropDoor.of(
                        isEvent: true, isSaved: true, isAuthor: true, serverCanDraw: true,
                        processor: processor, agreedAt: agreedAt) == .absent,
                    "processor \(String(describing: processor)) agreed \(agreedAt != nil)")
            }
        }
    }

    /// The same facts the composer's `/draw` is asked with give the same
    /// answer: asking first here is exactly `AssistantConsent.isRequired`
    /// for a message in the assistant's own chat.
    @Test("asks first exactly when a /draw in the assistant's chat would")
    func matchesTheComposer() {
        for processor in ["Example AI", nil] as [String?] {
            for agreedAt in [nil, Self.agreed] {
                let door = BackdropDoor.of(
                    isEvent: true, isSaved: true, isAuthor: true, serverCanDraw: true,
                    processor: processor, agreedAt: agreedAt)
                let composerAsks = AssistantConsent.isRequired(
                    chatKind: "ai", body: "/draw a picnic", processor: processor,
                    agreedAt: agreedAt)
                #expect((door == .asksFirst) == composerAsks)
            }
        }
    }

    /// One backdrop request per note at a time (the Mac's board, which
    /// shows every note's menu at once): a second click while one is out
    /// sends nothing, another note is not held up, and the answer — any
    /// answer — frees the control, so the retry after a consent "yes" is
    /// not refused as a duplicate.
    @Test("a note draws one backdrop at a time")
    func oneDrawPerNote() {
        var draws = BackdropDraws()
        #expect(!draws.isDrawing(12))
        let first = draws.begin(12)
        #expect(first)
        #expect(draws.isDrawing(12))
        let second = draws.begin(12)
        #expect(!second, "a second click while the first is out sends nothing")
        let other = draws.begin(13)
        #expect(other, "another note is not held up")
        draws.end(12)
        #expect(!draws.isDrawing(12))
        #expect(draws.isDrawing(13))
        let again = draws.begin(12)
        #expect(again, "answered, it can be asked again")
    }
}
