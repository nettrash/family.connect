//
//  LinkPreviewFixtures.swift
//  FamilyConnectTests
//
//  Real pages, kept as bytes, for the link-preview tests.
//
//  Fixtures/youtube-watch.html is a genuine capture of
//  https://www.youtube.com/watch?v=dQw4w9WgXcQ, fetched with exactly the
//  headers LinkPreviewLoader sends (its own User-Agent, Accept:
//  text/html) — markup, entities, Cyrillic locale attributes and all.
//
//  ONE thing was changed, and it is the reason the file is 12KB rather
//  than 1.3MB: the ~700KB of inline player JSON that YouTube puts BEFORE
//  its og: tags is replaced by a marker comment naming its exact byte
//  count. `reinflated()` swaps that marker back for a filler script of
//  precisely that size, so every tag in the page ends up at the offset
//  it really has — <title> at 704,923, og:title at 706,842, </head> at
//  715,108. Those offsets ARE the bug in #50: they sit far past the 256K
//  the fetcher used to read and the 200K the parser used to scan, so the
//  page yielded no card at all.
//
//  Android mirror: app/src/test/resources/fixtures/youtube-watch.html
//  with LinkPreviewFixtures.kt.
//

import Foundation

enum LinkPreviewFixtures {

    /// The captured YouTube watch page, restored to its real size.
    static func youTubeWatchPage() throws -> String {
        try reinflated(named: "youtube-watch")
    }

    /// Real offsets in that page, as measured on the live capture. The
    /// tests assert against these rather than "some big number" so a
    /// future cap change has to face what it is actually up against.
    enum YouTube {
        static let titleTagOffset = 704_923
        static let ogTitleOffset = 706_842
        static let headEndOffset = 715_108
        static let title = "Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)"
        static let imageURL = "https://i.ytimg.com/vi/dQw4w9WgXcQ/maxresdefault.jpg"
    }

    // MARK: - Loading

    /// Fixture bytes with the elision marker expanded back to its real
    /// length, so offsets match the page as served.
    private static func reinflated(named name: String) throws -> String {
        let raw = try String(contentsOf: url(for: name), encoding: .utf8)
        guard let open = raw.range(of: markerPrefix),
              let close = raw.range(of: "-->", range: open.upperBound..<raw.endIndex),
              let count = Int(raw[open.upperBound...]
                  .prefix { $0.isNumber }) else {
            // Not an Issue.record: every test here depends on the bytes,
            // so a broken fixture must stop the run, not colour it.
            throw FixtureError.markerMissing(name)
        }
        // A <script> rather than bare filler, because that is what really
        // sits there — and the parser must ignore it either way. The
        // wrapper is 31 bytes, so the padding makes up the rest.
        let script = "<script>var elided=\"" + String(repeating: "a", count: count - 31) + "\";</script>"
        guard script.utf8.count == count else { throw FixtureError.markerMissing(name) }
        return raw.replacingCharacters(in: open.lowerBound..<close.upperBound, with: script)
    }

    private static let markerPrefix = "<!--FAMILY-CONNECT-FIXTURE: "

    /// Bundle first — the fixtures folder is a resource of the test
    /// bundle — falling back to the source tree, which is where a
    /// synchronized Xcode group can decide not to copy a .html at all.
    private static func url(for name: String) -> URL {
        if let bundled = Bundle(for: FixtureAnchor.self).url(forResource: name, withExtension: "html") {
            return bundled
        }
        return URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .appending(path: "Fixtures")
            .appending(path: name + ".html")
    }

    /// Bundle(for:) needs a class; a Swift Testing suite is a struct.
    private final class FixtureAnchor {}

    enum FixtureError: Error {
        case markerMissing(String)
    }
}
