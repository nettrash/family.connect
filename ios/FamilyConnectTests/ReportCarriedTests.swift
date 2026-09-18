//
//  ReportCarriedTests.swift
//  FamilyConnectTests
//
//  WHAT THE OWNER'S INBOX SAYS A REPORTED MESSAGE CARRIED (docs/protocol.md,
//  "Reporting a member").
//
//  A photo sent without a caption has an EMPTY body by design, and
//  "inappropriate" is very often exactly that message — so a row drawn from
//  the excerpt alone is a reason word and two names, which is the screen a
//  reviewer probing moderation walks into. `message_attachments` is what
//  fills it, and the wording is the chat-list preview's own, matching the web
//  and Windows clients so that one family's owner reads the same row
//  whichever app they open.
//

import Foundation
import Testing

@testable import FamilyConnect

struct ReportCarriedTests {
    private func decode(_ json: String) throws -> [ReportedAttachmentDTO] {
        try APICoding.decoder().decode([ReportedAttachmentDTO].self, from: Data(json.utf8))
    }

    @Test("a caption-less photo still says what was reported")
    func onePhoto() throws {
        let carried = try decode(#"[{"kind": "photo", "name": null}]"#)

        #expect(carried.first?.name == nil)
        #expect(ReportDTO.carried(carried) == "Photo")
    }

    @Test("several photos become a count, as the chat list says it")
    func severalPhotos() throws {
        let carried = try decode(
            #"[{"kind": "photo", "name": null}, {"kind": "photo", "name": null}, {"kind": "photo", "name": null}]"#)

        #expect(ReportDTO.carried(carried) == "3 Photos")
    }

    @Test("a video, a voice note and a place each say their kind")
    func kinds() throws {
        #expect(try ReportDTO.carried(decode(#"[{"kind": "video", "name": null}]"#)) == "Video")
        #expect(try ReportDTO.carried(decode(#"[{"kind": "audio", "name": null}]"#)) == "Audio")
        #expect(try ReportDTO.carried(decode(#"[{"kind": "location", "name": null}]"#)) == "Location")
    }

    /// A document's name is its whole identity — "attachment 34" tells a
    /// moderator nothing — and a file that arrived without one still has to
    /// say something.
    @Test("a file says its own name, or the word for it")
    func file() throws {
        #expect(try ReportDTO.carried(decode(#"[{"kind": "file", "name": "budget.pdf"}]"#)) == "budget.pdf")
        #expect(try ReportDTO.carried(decode(#"[{"kind": "file", "name": null}]"#)) == "File")
        #expect(try ReportDTO.carried(decode(#"[{"kind": "file", "name": ""}]"#)) == "File")
    }

    /// A kind added by a newer server must never leave the row blank: the
    /// owner is the moderator, and "something was attached" is the least this
    /// screen may say.
    @Test("a kind this build has never heard of still draws")
    func unknownKind() throws {
        #expect(try ReportDTO.carried(decode(#"[{"kind": "hologram", "name": "spin.h"}]"#)) == "spin.h")
        #expect(try ReportDTO.carried(decode(#"[{"kind": "hologram", "name": null}]"#)) == "File")
    }

    /// A report that names a PERSON carries no attachment key at all, and one
    /// whose message retention has swept carries none either — there the
    /// frozen excerpt is the whole row, and the line must simply not appear.
    @Test("nothing carried draws no line")
    func nothing() throws {
        #expect(ReportDTO.carried(nil) == nil)
        #expect(try ReportDTO.carried(decode("[]")) == nil)
    }

    /// The trimmed shape is the whole of what a report brings: a moderator
    /// needs to know that a place was sent, never where the sender stood. The
    /// decode must not depend on anything else being there.
    @Test("kind and name are all it needs to decode")
    func trimmed() throws {
        let carried = try decode(#"[{"kind": "photo", "name": null}]"#)

        #expect(carried.count == 1)
        #expect(carried.first?.kind == "photo")
    }
}
