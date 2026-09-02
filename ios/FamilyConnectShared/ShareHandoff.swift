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

    // MARK: - What never completed

    /// How long a staging entry is left alone before the sweep calls it
    /// abandoned. 24 hours — the SERVER's number.
    ///
    /// `limits.attachment_grace_hours` defaults to 24 (server/config.rs,
    /// config.example.toml) and docs/protocol.md states it as a promise:
    /// "Unclaimed attachments are deleted after 24 hours". That is the
    /// same abandoned send as this one, one hop later in the same
    /// pipeline, and a client that swept sooner than the server would be
    /// making a stricter promise than the protocol documents.
    ///
    /// It is also enormous next to the real in-flight window, which is
    /// the number that actually has to be safe. A hand-off is live from
    /// the appex's `copyItem` to the app's `onOpenURL`: seconds against a
    /// running app, and on the worst cold launch — a big video, a slow
    /// device, a SwiftData store that has to be rebuilt first — still far
    /// short of a minute. Nothing is bought by tightening it: a shorter
    /// floor reclaims a few directory entries sooner, and risks deleting
    /// a share while its own app is still launching.
    static let stagingGrace: TimeInterval = 24 * 60 * 60

    /// Delete everything in the inbox older than `age`; answer how many
    /// entries went.
    ///
    /// A hand-off that COMPLETES cleans up after itself — the app removes
    /// each staging directory as it imports it (`AppSession.handleShareURL`).
    /// This is for every hand-off that does not: the person cancels after
    /// the appex has already staged, the open is dropped, the app is
    /// killed mid-import, the import throws, or the app is simply never
    /// opened again. Nothing in either process ever looks at those
    /// directories again, so without this they sit in the App Group
    /// container for the life of the install — with a shared video's
    /// hundreds of megabytes in each (issue #35).
    ///
    /// AGE IS THE ONLY TEST, and both halves of that are deliberate:
    ///
    ///   - A directory IN FLIGHT must never be deleted. `age` is the
    ///     entire protection, so an entry whose timestamp cannot be read
    ///     is LEFT rather than guessed at, and so is one stamped in the
    ///     future (a clock moved back: the subtraction goes negative and
    ///     nothing matches). A directory kept one launch too long costs
    ///     nothing; a share that evaporates mid-hand-off is the failure
    ///     this whole file exists to prevent.
    ///   - The NAME is not consulted. An entry the consumer would refuse
    ///     to name — `stagingDirectory` builds a path from a UUID and
    ///     nothing else — is precisely an entry no import will ever
    ///     claim, and so is a stray FILE dropped straight into the inbox
    ///     (a `.DS_Store`, a half-written copy). Sparing those would be
    ///     sparing exactly the garbage this exists to collect. `ShareInbox`
    ///     is written by this appex and read by this app; there is no
    ///     third party whose file could be in there.
    ///
    /// TOTAL AND SILENT. Every filesystem call is `try?`: a missing
    /// container (no App Group entitlement), a missing inbox (nothing has
    /// ever been shared) and an unreadable entry all return a count
    /// rather than throw, because both call sites are paths that must not
    /// fail — an app launch, and an appex about to stage a share. It logs
    /// nothing, because this file is compiled into the extension and stays
    /// dependency-free; a caller with a logger can log the count.
    static func sweepOrphanedStaging(
        container: URL? = ShareHandoff.containerURL(),
        olderThan age: TimeInterval = ShareHandoff.stagingGrace,
        now: Date = Date(),
        fileManager: FileManager = .default
    ) -> Int {
        guard let container else { return 0 }
        let inbox = inboxURL(container: container)
        guard let names = try? fileManager.contentsOfDirectory(atPath: inbox.path) else {
            return 0
        }
        var removed = 0
        for name in names {
            let entry = inbox.appendingPathComponent(name)
            let attributes = try? fileManager.attributesOfItem(atPath: entry.path)
            // Modified, not created: a staging directory's modification
            // date moves when the appex copies the file INTO it, which is
            // the moment the hand-off became live. Creation date is the
            // fallback for anything that carries one and not the other.
            guard let stamped = (attributes?[.modificationDate] as? Date)
                    ?? (attributes?[.creationDate] as? Date),
                  now.timeIntervalSince(stamped) > age
            else { continue }
            if (try? fileManager.removeItem(at: entry)) != nil { removed += 1 }
        }
        return removed
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
