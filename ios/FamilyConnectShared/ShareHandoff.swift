//
//  ShareHandoff.swift
//  FamilyConnect / FamilyConnectShareExtension
//
//  THE contract between the Share Extension and the app, in one file that
//  BOTH targets compile.
//
//  The extension copies each shared item into
//  `<group container>/ShareInbox/<uuid>/<name>` and opens
//  `familyconnect://share?ids=<uuid>,<uuid>`; the app parses that URL,
//  rebuilds the same paths, and moves the files out. Four constants and
//  two URL shapes have to agree ACROSS A PROCESS BOUNDARY for that to
//  work, and a mismatch is silent — the extension reports success, the
//  app finds nothing, and the share simply evaporates. Nothing in either
//  process ever fails loudly.
//
//  So they are declared once, here. The previous arrangement declared
//  them twice (ShareImport in the app, four `private let`s in
//  ShareViewController) with a comment explaining that duplication was
//  cheaper than sharing a file with an appex. It is not: the file is
//  dependency-free — `import Foundation` and nothing else, no UIKit, no
//  model types, nothing @MainActor — and lives in its own
//  FamilyConnectShared folder listed by both targets, so neither target
//  drags in the other's world. See issue #34.
//
//  Everything here is pure and total: the tests hammer producer straight
//  into consumer without a container, an appex process or a share sheet.
//

import Foundation

nonisolated enum ShareHandoff {

    // MARK: - The four hand-off constants

    /// The custom scheme both Info.plists register and the extension
    /// opens. One spelling, here.
    static let scheme = "familyconnect"
    /// The hand-off URL's host — `familyconnect://share`.
    static let host = "share"
    /// The App Group both the app and the extension are entitled to.
    /// Must match both .entitlements files.
    static let appGroup = "group.me.nettrash.FamilyConnect"
    /// The folder inside the group container the extension stages into.
    static let inboxFolder = "ShareInbox"

    /// The protocol's ceiling on one message's attachments
    /// (docs/protocol.md, "Limits": `limits.max_attachments_per_message`).
    /// It lives HERE, not on `StagedAttachment`, because the appex cannot
    /// see the app's model types and still has to cap what it stages;
    /// `StagedAttachment.maxPerMessage` is an alias of this number.
    /// The extension's Info.plist activation rule repeats it as
    /// `NSExtensionActivationSupports*WithMaxCount` — a plist cannot read
    /// a Swift constant, so ShareHandoffRoundTripTests pins the two
    /// against each other instead.
    static let maxAttachmentsPerMessage = 10

    // MARK: - The App Group container

    /// The shared container both processes stage through, or nil when the
    /// App Group entitlement is missing (an unsigned build, or a
    /// provisioning profile that lost the group).
    static func containerURL(
        fileManager: FileManager = .default
    ) -> URL? {
        fileManager.containerURL(forSecurityApplicationGroupIdentifier: appGroup)
    }

    /// `<container>/ShareInbox` — the folder the extension creates and
    /// stages into, and the only folder the app reads back.
    static func inboxURL(container: URL) -> URL {
        container.appendingPathComponent(inboxFolder, isDirectory: true)
    }

    /// The staging directory for one id, or nil when the id is not a
    /// UUID.
    ///
    /// The UUID check is the belt to `ids(from:)`'s braces and it guards
    /// the PRODUCER too: a path is built from this string on both sides,
    /// and "../../Documents" must die here rather than there. The
    /// extension only ever passes a `UUID().uuidString` it just minted,
    /// which is exactly why the check costs nothing.
    static func stagingDirectory(container: URL, id: String) -> URL? {
        guard UUID(uuidString: id) != nil else { return nil }
        return inboxURL(container: container).appendingPathComponent(id, isDirectory: true)
    }

    // MARK: - The hand-off URL, both directions

    /// PRODUCER (extension): `familyconnect://share?ids=<uuid>,<uuid>`.
    ///
    /// Ids only. A filename does not survive a query string with its
    /// commas intact, so names stay on the staged files and UUIDs are all
    /// that travels. Returns nil for an empty list — there is nothing to
    /// hand off — or if any id is not a UUID, so a producer bug cannot
    /// mint a URL its own consumer would refuse.
    static func handoffURL(ids: [String]) -> URL? {
        guard !ids.isEmpty, ids.allSatisfy({ UUID(uuidString: $0) != nil }) else { return nil }
        var components = URLComponents()
        components.scheme = scheme
        components.host = host
        components.queryItems = [
            URLQueryItem(name: "ids", value: ids.prefix(maxAttachmentsPerMessage).joined(separator: ","))
        ]
        return components.url
    }

    /// CONSUMER (app): the staged ids named by an incoming URL, or nil
    /// when the URL is not a share hand-off at all.
    ///
    /// Every id must be a well-formed UUID — the extension names its
    /// staging directories with nothing else, and an id that is anything
    /// else would become a PATH below. One bad id poisons the whole
    /// hand-off rather than being skipped: a URL this app did not write
    /// is a URL this app does not trust any part of. At most the
    /// protocol's ten, in the order shared.
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
        return Array(parts.prefix(maxAttachmentsPerMessage))
    }

    // MARK: - Names

    /// A name safe to create INSIDE a staging directory: no path
    /// separators, nothing hidden, never empty, never longer than a
    /// filesystem component.
    ///
    /// Shared because BOTH sides need it to agree — the extension writes
    /// the name and the app reads whatever it finds back — and because it
    /// is the second place an untrusted string could turn into a path.
    static func safeFileName(_ name: String) -> String {
        let cleaned = name
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: ":", with: "_")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let visible = String(cleaned.drop(while: { $0 == "." }))
        return visible.isEmpty ? "file" : String(visible.prefix(255))
    }
}
