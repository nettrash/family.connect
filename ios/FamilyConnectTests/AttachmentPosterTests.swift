//
//  AttachmentPosterTests.swift
//  FamilyConnectTests
//
//  A video's poster, and the flag that used to be allowed to decide
//  whether anyone ever asked for one.
//
//  `has_preview` is a snapshot the server took when this device read the
//  message, and it is the one attachment field that changes afterwards. A
//  copy that says false can never correct itself — history sync is
//  `after_id` only, so it cannot see a mutation of an older row, and a
//  message already held is not rewritten when it is re-delivered. So a
//  video stored with the flag down was a grey tile through leaving the
//  chat, through a relaunch, for the life of the install: it had nothing
//  to fall back on, because a photo shows its full bytes and a video's
//  bytes are a video.
//
//  These pin the two halves that have to hold together: the server is
//  asked whatever the flag says, and a poster that genuinely does not
//  exist SETTLES — a few small requests once per launch, then silence.
//
//  Android counterpart: app/src/test/java/me/nettrash/familyconnect/
//  data/repo/AttachmentPosterTest.kt and ui/components/
//  AttachmentPreviewGateTest.kt.
//

import Foundation
import Testing
@testable import FamilyConnect

/// Rungs measured in milliseconds: the ladder's LENGTH is what these tests
/// are about, and nobody should wait twelve seconds to watch it.
///
/// At file scope rather than on the suite: a default argument is evaluated
/// in a nonisolated context, and a @MainActor static read from one is an
/// error under Swift 6.
private let quickLadder: [Duration] = [.milliseconds(20), .milliseconds(20)]

@MainActor
struct AttachmentPosterTests {

    private func makeStore(
        host: String,
        session: URLSession,
        posterRetryDelays: [Duration] = quickLadder
    ) -> AttachmentStore {
        let api = APIClient(serverURL: URL(string: "https://\(host)"), session: session)
        let directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("attachment-poster-\(UUID().uuidString)")
        return AttachmentStore(api: api, directory: directory, posterRetryDelays: posterRetryDelays)
    }

    private func waitUntil(_ condition: @MainActor () -> Bool, timeout: TimeInterval = 5) async {
        let deadline = Date().addingTimeInterval(timeout)
        while !condition() && Date() < deadline {
            try? await Task.sleep(for: .milliseconds(10))
        }
    }

    private func video(id: Int64, hasPreview: Bool) -> AttachmentDTO {
        AttachmentDTO(
            id: id,
            kind: AttachmentDTO.Kind.video,
            mime: "video/mp4",
            size: 4096,
            width: 1600,
            height: 1200,
            durationMS: 8400,
            hasPreview: hasPreview,
            name: nil,
            latitude: nil,
            longitude: nil,
            accuracyM: nil)
    }

    private func photo(id: Int64, hasPreview: Bool) -> AttachmentDTO {
        AttachmentDTO(
            id: id,
            kind: AttachmentDTO.Kind.photo,
            mime: "image/jpeg",
            size: 4096,
            width: 1600,
            height: 1200,
            durationMS: nil,
            hasPreview: hasPreview,
            name: nil,
            latitude: nil,
            longitude: nil,
            accuracyM: nil)
    }

    private func previewRequests(host: String) -> Int {
        StubURLProtocol.requests(host: host)
            .filter { $0.url.path.hasSuffix("/preview") }
            .count
    }

    #if os(iOS)
    /// THE REGRESSION. The row says there is no poster; the server has
    /// one. Before the fix this tile made no request at all, and no amount
    /// of leaving the chat or relaunching changed that — the flag it read
    /// can never be corrected.
    @Test("a video whose stored flag says no poster still asks the server")
    func aStaleFlagStillAsks() async throws {
        let host = "attachment-poster-stale-flag.test"
        StubURLProtocol.register(host: host) { _ in
            StubResponse(status: 200, headers: ["Content-Type": "image/jpeg"],
                         body: TestImages.photograph(width: 32, height: 32))
        }
        defer { StubURLProtocol.unregister(host: host) }

        let store = makeStore(host: host, session: StubURLProtocol.makeSession())
        let attachment = video(id: 41, hasPreview: false)

        #expect(AttachmentView.image(for: attachment, in: store) == nil)
        await waitUntil { AttachmentView.image(for: attachment, in: store) != nil }

        #expect(AttachmentView.image(for: attachment, in: store) != nil,
                "the poster was on the server and the bubble never asked for it")
        // The poster, and only the poster: a video's own bytes are never
        // pulled into the image cache — AVPlayer streams them.
        #expect(previewRequests(host: host) >= 1)
        #expect(StubURLProtocol.requests(host: host).allSatisfy { $0.url.path.hasSuffix("/preview") })
    }

    /// The normal path, unchanged. A photo with the flag down asks for its
    /// FULL bytes and never for a preview: being wrong about a photo costs
    /// nothing, so the request that would 404 is still not made.
    @Test("a photo whose flag says no preview asks only for the full bytes")
    func aPhotoStillSkipsThePreview() async throws {
        let host = "attachment-poster-photo.test"
        StubURLProtocol.register(host: host) { _ in
            StubResponse(status: 200, headers: ["Content-Type": "image/jpeg"],
                         body: TestImages.photograph(width: 32, height: 32))
        }
        defer { StubURLProtocol.unregister(host: host) }

        let store = makeStore(host: host, session: StubURLProtocol.makeSession())
        let attachment = photo(id: 42, hasPreview: false)

        #expect(AttachmentView.image(for: attachment, in: store) == nil)
        await waitUntil { AttachmentView.image(for: attachment, in: store) != nil }

        #expect(AttachmentView.image(for: attachment, in: store) != nil)
        #expect(previewRequests(host: host) == 0,
                "a photo asked for a preview its message says does not exist")
    }
    #endif

    /// The poster was not there the first time it was asked for, and the
    /// picture must still turn up — without a relaunch, without the
    /// network dropping, without the reader doing anything at all.
    @Test("a poster that lands late is picked up without a relaunch")
    func aLatePosterArrives() async throws {
        let host = "attachment-poster-late.test"
        StubURLProtocol.register(host: host) { _ in
            if StubURLProtocol.requests(host: host).count <= 1 { return .empty(404) }
            return StubResponse(status: 200, headers: ["Content-Type": "image/jpeg"],
                                body: TestImages.photograph(width: 32, height: 32))
        }
        defer { StubURLProtocol.unregister(host: host) }

        let store = makeStore(host: host, session: StubURLProtocol.makeSession())

        #expect(store.image(id: 43, preview: true, mayArriveLate: true) == nil)
        await waitUntil { store.image(id: 43, preview: true) != nil }

        #expect(store.image(id: 43, preview: true) != nil,
                "a 404 answered before the poster landed was treated as final")
        #expect(previewRequests(host: host) == 2)
    }

    /// The other half, and the one that keeps the fix honest: a poster
    /// that will NEVER exist — the sender's frame grab failed, or its
    /// upload did — must stop being asked for.
    @Test("a video with no poster settles after a bounded number of tries")
    func aPosterlessVideoSettles() async throws {
        let host = "attachment-poster-settles.test"
        StubURLProtocol.register(host: host) { _ in .empty(404) }
        defer { StubURLProtocol.unregister(host: host) }

        let store = makeStore(host: host, session: StubURLProtocol.makeSession())

        let settledAt = store.generation
        #expect(store.image(id: 44, preview: true, mayArriveLate: true) == nil)
        // The ladder is spent once the key settles, which is the only
        // thing `generation` moves for here.
        await waitUntil { store.generation != settledAt }

        let spent = previewRequests(host: host)
        #expect(spent == quickLadder.count + 1)

        // Scrolled back to, re-entered, redrawn: the key is settled and
        // nothing asks again.
        for _ in 0..<5 { _ = store.image(id: 44, preview: true, mayArriveLate: true) }
        try? await Task.sleep(for: .milliseconds(100))
        #expect(previewRequests(host: host) == spent,
                "a settled poster was re-requested on every render pass")
    }

    /// A poster that is there costs exactly one request — the ladder is a
    /// consequence of being told "no", not a schedule.
    @Test("a poster that is there costs one request")
    func aPresentPosterCostsOneRequest() async throws {
        let host = "attachment-poster-present.test"
        StubURLProtocol.register(host: host) { _ in
            StubResponse(status: 200, headers: ["Content-Type": "image/jpeg"],
                         body: TestImages.photograph(width: 32, height: 32))
        }
        defer { StubURLProtocol.unregister(host: host) }

        let store = makeStore(host: host, session: StubURLProtocol.makeSession())

        #expect(store.image(id: 45, preview: true, mayArriveLate: true) == nil)
        await waitUntil { store.image(id: 45, preview: true) != nil }

        #expect(previewRequests(host: host) == 1)
    }
}
