//
//  DroppedAttachmentTests.swift
//  FamilyConnectTests
//
//  Drag and drop into a conversation (the Mac's composer). There is no
//  protocol change here, so what these check is the one decision a drop
//  forces that a picker never does: an NSOpenPanel is CONFIGURED to hand
//  back only files that can be sent, while a drag hands over whatever the
//  person happened to be holding.
//
//  The rule is asserted here rather than in a running window because a
//  drag cannot be synthesised: NSDraggingSession needs a real pointer, and
//  the shell around this rule — `.dropDestination(for: URL.self)` and the
//  sandbox extension AppKit attaches to a dropped URL — needs a signed,
//  sandboxed build and a hand on a trackpad. What CAN be pinned without
//  either is the part that decides, which is also the part that would
//  quietly diverge from the attach panel.
//
//  The cap is not re-tested here: it is `StagedAttachment.canAdd`, which
//  has its own tests, and the drop path reaches it through the same
//  `stage` the panel and the share import use.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Drop into a conversation")
struct DroppedAttachmentTests {

    private func file(_ path: String) -> URL { URL(fileURLWithPath: path) }
    private func link(_ string: String) -> URL { URL(string: string)! }

    /// Nothing is a directory unless a test says so.
    private let noDirectories: (URL) -> Bool = { _ in false }

    @Test("Dropped files are attached, in the order they were dropped")
    func filesAttach() {
        let urls = [file("/tmp/a.jpg"), file("/tmp/b.pdf")]
        #expect(DroppedAttachment.decide(urls, isDirectory: noDirectories) == .attach(urls))
    }

    /// The case that decides the rule's shape. Finder puts a file's own
    /// path on the drag as text as well, and some sources add a URL
    /// representation beside the file — so "there is a link here" cannot be
    /// the test, or dropping a file from Finder would type its path into
    /// the message instead of attaching it.
    @Test("A file wins over a link dropped alongside it")
    func fileBeatsLink() {
        let real = file("/tmp/photo.jpg")
        let decision = DroppedAttachment.decide(
            [link("https://example.com/photo.jpg"), real], isDirectory: noDirectories)
        #expect(decision == .attach([real]))
    }

    /// An image dragged out of a browser is a LINK: the bytes are on
    /// somebody else's server, so there is nothing to upload. It goes into
    /// the draft as words rather than being refused in silence.
    @Test("A dropped link becomes words, not an attachment")
    func linkBecomesText() {
        let url = link("https://example.com/page")
        #expect(DroppedAttachment.decide([url], isDirectory: noDirectories) == .link([url]))
    }

    @Test("Several links all land in the draft")
    func severalLinks() {
        let urls = [link("https://a.example"), link("https://b.example")]
        #expect(DroppedAttachment.decide(urls, isDirectory: noDirectories) == .link(urls))
    }

    /// The one the open panel cannot produce and a drag can. Staging a
    /// folder would copy it, call it a `kind=file`, and upload nothing.
    @Test("A dropped folder is refused")
    func folderRefused() {
        let folder = file("/tmp/Pictures")
        let decision = DroppedAttachment.decide([folder], isDirectory: { $0 == folder })
        #expect(decision == .nothing)
    }

    /// A bundle is a folder wearing an icon, and the same reasoning
    /// applies — but the files dropped WITH it are still perfectly good.
    @Test("A folder among files is dropped from the batch, not the batch")
    func folderAmongFiles() {
        let app = file("/Applications/Mail.app")
        let doc = file("/tmp/notes.txt")
        let decision = DroppedAttachment.decide([app, doc], isDirectory: { $0 == app })
        #expect(decision == .attach([doc]))
    }

    /// Unreadable answers `true` at the call site (MacConversationView), so
    /// the rule must treat that as "not attachable" and not as a file it
    /// can hand to MediaPrep.
    @Test("A file that cannot even be probed is not attached")
    func unprobableRefused() {
        #expect(DroppedAttachment.decide([file("/tmp/x")], isDirectory: { _ in true }) == .nothing)
    }

    @Test("An empty drop is nothing")
    func emptyIsNothing() {
        #expect(DroppedAttachment.decide([], isDirectory: noDirectories) == .nothing)
    }
}
