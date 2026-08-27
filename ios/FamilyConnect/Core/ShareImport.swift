//
//  ShareImport.swift
//  FamilyConnect
//
//  The app's half of the share extension: the URL the extension opens,
//  the App Group inbox it staged the files into, and the one rule about
//  which chats a shared file may land in.
//
//  The extension (FamilyConnectShareExtension) copies each shared item
//  into `<group>/ShareInbox/<uuid>/<name>` and opens
//  `familyconnect://share?ids=<uuid>,<uuid>` — ids only, because a
//  filename does not survive a query string with its commas intact, and
//  because an id that is not a UUID is an id this app must refuse: the
//  path is built from it, and "..%2Fetc" style traversal is exactly what
//  the UUID check exists to stop. Everything here is pure and total so
//  the tests can hammer it without a container or an extension process.
//
//  NOTHING AUTO-SENDS. The staged files land in a chat's composer as
//  staged attachments, and the user presses Send there — sharing into a
//  family chat is choosing to say something, not having said it.
//

import Foundation

nonisolated enum ShareImport {

    /// The custom scheme both Info.plists register and the extension
    /// opens. One spelling, here.
    static let scheme = "familyconnect"
    static let host = "share"
    /// The App Group both the app and the extension are entitled to.
    static let appGroup = "group.me.nettrash.FamilyConnect"
    /// The folder inside the group container the extension stages into.
    static let inboxFolder = "ShareInbox"

    /// The staged ids named by an incoming URL, or nil when the URL is
    /// not a share hand-off at all. Every id must be a well-formed UUID
    /// — the extension names its staging directories with nothing else,
    /// and an id that is anything else would become a PATH below. At
    /// most the protocol's ten, in the order shared.
    static func ids(from url: URL) -> [String]? {
        guard url.scheme?.lowercased() == scheme,
              url.host?.lowercased() == host,
              let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
              let raw = components.queryItems?.first(where: { $0.name == "ids" })?.value,
              !raw.isEmpty
        else { return nil }
        let parts = raw.split(separator: ",").map(String.init)
        guard !parts.isEmpty, parts.allSatisfy({ UUID(uuidString: $0) != nil }) else {
            return nil
        }
        return Array(parts.prefix(StagedAttachment.maxPerMessage))
    }

    /// The staging directory for one validated id, or nil when the id is
    /// not a UUID — the belt to `ids(from:)`'s braces, so a caller that
    /// somehow holds an unvalidated string still cannot build a path
    /// from it.
    static func inboxDirectory(container: URL, id: String) -> URL? {
        guard UUID(uuidString: id) != nil else { return nil }
        return container
            .appendingPathComponent(inboxFolder, isDirectory: true)
            .appendingPathComponent(id, isDirectory: true)
    }

    /// May a shared file land in this chat? Family and direct chats take
    /// attachments; the assistant's chat ("ai") does not — it is a text
    /// conversation with a model, and the server would refuse the send.
    static func isEligible(chatKind: String) -> Bool {
        chatKind != "ai"
    }
}
