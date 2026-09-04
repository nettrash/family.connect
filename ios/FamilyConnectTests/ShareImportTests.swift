//
//  ShareImportTests.swift
//  FamilyConnectTests
//
//  The share extension's hand-off, from the app's side. The contract
//  itself now lives in ShareHandoff (issue #34) and is exercised producer
//  → consumer in ShareHandoffTests; what stays here is the app half: the
//  parser is
//  TOTAL and paranoid (an id that is not a UUID never becomes a path),
//  the chat-eligibility rule is one line that must never drift, and the
//  staging cap is the same ten the protocol caps a message at.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Share import")
struct ShareImportTests {

    private let a = "6a1f0c3e-0000-4000-8000-000000000001"
    private let b = "6a1f0c3e-0000-4000-8000-000000000002"

    // MARK: - The hand-off URL

    @Test("A well-formed hand-off yields its ids in shared order")
    func parsesIDs() {
        let url = URL(string: "familyconnect://share?ids=\(a),\(b)")!
        #expect(ShareHandoff.ids(from: url) == [a, b])
    }

    @Test("Anything that is not the share hand-off is refused")
    func refusesForeignURLs() {
        for text in [
            "https://share?ids=\(a)",                    // wrong scheme
            "familyconnect://open?ids=\(a)",             // wrong host
            "familyconnect://share",                     // no ids at all
            "familyconnect://share?ids=",                // empty ids
        ] {
            #expect(ShareHandoff.ids(from: URL(string: text)!) == nil, "\(text)")
        }
    }

    /// The whole reason ids are UUIDs: the app builds a PATH from each
    /// one, and an id like "../../Documents" must die here, not there.
    @Test("An id that is not a UUID poisons the whole hand-off")
    func refusesTraversal() {
        let url = URL(string: "familyconnect://share?ids=\(a),..%2F..%2FDocuments")!
        #expect(ShareHandoff.ids(from: url) == nil)
    }

    @Test("More ids than a message may carry are trimmed to the cap")
    func capsAtTen() {
        let ids = (0..<12).map { _ in UUID().uuidString }
        let url = URL(string: "familyconnect://share?ids=\(ids.joined(separator: ","))")!
        #expect(ShareHandoff.ids(from: url)?.count == StagedAttachment.maxPerMessage)
        #expect(ShareHandoff.ids(from: url) == Array(ids.prefix(10)))
    }

    // MARK: - The inbox path

    @Test("The staging directory is built only from a valid UUID")
    func inboxDirectoryValidatesItsID() {
        let container = URL(fileURLWithPath: "/tmp/group")
        let good = ShareHandoff.stagingDirectory(container: container, id: a)
        #expect(good?.path == "/tmp/group/ShareInbox/\(a)")
        #expect(ShareHandoff.stagingDirectory(container: container, id: "../evil") == nil)
        #expect(ShareHandoff.stagingDirectory(container: container, id: "") == nil)
    }

    // MARK: - Where a share may land

    /// Family and direct chats take attachments; the assistant's chat
    /// does not — the server would refuse the send. An unknown future
    /// kind is left eligible: the server is the authority on refusing.
    @Test("The chat picker excludes exactly the assistant's chat")
    func eligibilityRule() {
        #expect(ShareImport.isEligible(chatKind: "family"))
        #expect(ShareImport.isEligible(chatKind: "direct"))
        #expect(!ShareImport.isEligible(chatKind: "ai"))
        #expect(ShareImport.isEligible(chatKind: "somekind"))
    }

    // MARK: - Discarding, and the bytes

    /// An app-group-style container holding one staged file per id, the
    /// shape `handleShareURL` reads (ShareInbox/<uuid>/<name>).
    private func stagedContainer(ids: [String]) throws -> URL {
        let container = FileManager.default.temporaryDirectory
            .appendingPathComponent("fc-test-share-\(UUID().uuidString)", isDirectory: true)
        for id in ids {
            // Through the shared constant, so a rename of the inbox folder
            // shows up here as a failing test rather than as a share that
            // silently finds nothing.
            let inbox = ShareHandoff.inboxURL(container: container)
                .appendingPathComponent(id, isDirectory: true)
            try FileManager.default.createDirectory(at: inbox, withIntermediateDirectories: true)
            try Data("shared".utf8).write(to: inbox.appendingPathComponent("photo.jpg"))
        }
        return container
    }

    @MainActor
    private func makeSession() -> AppSession {
        AppSession(api: APIClient(serverURL: nil), defaultServerURL: { nil })
    }

    /// The purge invariant, on the path that used to leak: once the
    /// picker has ADDRESSED the files (`chooseShareTarget`), the purge's
    /// old `discardPendingShareImport` was a guard-return no-op and
    /// `shareImportTarget = nil` dropped the moved files without touching
    /// disk. `.leftFamily` is used because its scope skips the keychain
    /// and UserDefaults, and every purge reason runs the same discard.
    @Test("A purge after the picker chose a chat deletes the moved files")
    @MainActor
    func purgeDeletesAddressedShareFiles() throws {
        let session = makeSession()
        let container = try stagedContainer(ids: [a])
        defer { try? FileManager.default.removeItem(at: container) }

        session.handleShareURL(
            URL(string: "familyconnect://share?ids=\(a)")!, container: container)
        let parked = try #require(session.pendingShareImport?.first)
        session.chooseShareTarget(chatID: 5)
        #expect(session.pendingShareImport == nil)
        #expect(FileManager.default.fileExists(atPath: parked.path))

        session.purge(.leftFamily)
        #expect(session.shareImportTarget == nil)
        // The whole fc-shared-<id> directory, not just the file.
        #expect(!FileManager.default.fileExists(
            atPath: parked.deletingLastPathComponent().path))
    }

    @Test("A purge before the picker chose deletes the parked files")
    @MainActor
    func purgeDeletesParkedShareFiles() throws {
        let session = makeSession()
        let container = try stagedContainer(ids: [a, b])
        defer { try? FileManager.default.removeItem(at: container) }

        session.handleShareURL(
            URL(string: "familyconnect://share?ids=\(a),\(b)")!, container: container)
        let parked = try #require(session.pendingShareImport)
        #expect(parked.count == 2)

        session.purge(.leftFamily)
        #expect(session.pendingShareImport == nil)
        for url in parked {
            #expect(!FileManager.default.fileExists(
                atPath: url.deletingLastPathComponent().path))
        }
    }

    /// The sheet's onDismiss calls `discardPendingShareImport`
    /// unconditionally, AFTER `chooseShareTarget` on the choosing path —
    /// so it must stay a no-op for the addressed files, or choosing a
    /// chat would delete the very files it just addressed.
    @Test("Dismissing the picker after choosing keeps the addressed files")
    @MainActor
    func dismissAfterChoosingKeepsTheAddressedFiles() throws {
        let session = makeSession()
        let container = try stagedContainer(ids: [a])
        defer { try? FileManager.default.removeItem(at: container) }

        session.handleShareURL(
            URL(string: "familyconnect://share?ids=\(a)")!, container: container)
        let parked = try #require(session.pendingShareImport?.first)
        session.chooseShareTarget(chatID: 5)
        session.discardPendingShareImport()

        #expect(session.shareImportTarget?.urls == [parked])
        #expect(FileManager.default.fileExists(atPath: parked.path))

        // Cleanup through the walk-both discard, which asserts it too.
        session.discardShareImports()
        #expect(!FileManager.default.fileExists(
            atPath: parked.deletingLastPathComponent().path))
    }

    // MARK: - The staging cap

    /// The same ten as `limits.max_attachments_per_message` — the rule
    /// both composers and the share-import path ask before appending.
    @Test("The staging list caps at the protocol's ten")
    func stagingCap() {
        #expect(StagedAttachment.maxPerMessage == 10)
        #expect(StagedAttachment.canAdd(to: 0))
        #expect(StagedAttachment.canAdd(to: 9))
        #expect(!StagedAttachment.canAdd(to: 10))
        #expect(!StagedAttachment.canAdd(to: 11))
    }
}
