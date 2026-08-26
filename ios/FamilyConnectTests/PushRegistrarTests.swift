//
//  PushRegistrarTests.swift
//  FamilyConnectTests
//
//  The push-registration state machine over the stubbed URLSession:
//  token hex encoding, POST-once semantics, re-POST on rotation, retry
//  after a failed POST, and logout deregistration. Persistence goes
//  through the registrar's closure seams into an in-memory box, so
//  these tests never touch the process-global UserDefaults (and can run
//  in parallel with the AppSession suite, which does).
//

import Foundation
import Testing
import UserNotifications
@testable import FamilyConnect

@MainActor
@Suite("Push registration")
struct PushRegistrarTests {

    // MARK: - Pure pieces

    @Test("APNs token data hex-encodes lowercase, byte for byte")
    func hexEncoding() {
        #expect(PushRegistrationLogic.hexToken(Data([0x00, 0x0F, 0xAB, 0xFF])) == "000fabff")
        #expect(PushRegistrationLogic.hexToken(Data([0xDE, 0xAD, 0xBE, 0xEF])) == "deadbeef")
        #expect(PushRegistrationLogic.hexToken(Data()) == "")
    }

    @Test("authorization: never-asked prompts, denial abstains, any grant registers")
    func authorizationActions() {
        // .notDetermined is the ONLY status that may prompt; a prior denial
        // stands down (re-checked every pass, so System Settings heals it);
        // anything already granted skips straight to APNs registration.
        #expect(PushRegistrationLogic.authorizationAction(for: .notDetermined) == .request)
        #expect(PushRegistrationLogic.authorizationAction(for: .denied) == .abstain)
        #expect(PushRegistrationLogic.authorizationAction(for: .authorized) == .registerOnly)
        #expect(PushRegistrationLogic.authorizationAction(for: .provisional) == .registerOnly)
    }

    @Test("needsRegistration: fresh / rotated / half-stored yes, confirmed no")
    func decisionTable() {
        #expect(PushRegistrationLogic.needsRegistration(token: "aa", storedToken: nil, storedDeviceID: nil))
        #expect(PushRegistrationLogic.needsRegistration(token: "bb", storedToken: "aa", storedDeviceID: 5))
        #expect(PushRegistrationLogic.needsRegistration(token: "aa", storedToken: "aa", storedDeviceID: nil))
        #expect(!PushRegistrationLogic.needsRegistration(token: "aa", storedToken: "aa", storedDeviceID: 5))
    }

    // MARK: - State machine plumbing

    /// In-memory stand-in for the AppSettings pair (see file header).
    @MainActor
    private final class StoredBox {
        var token: String?
        var deviceID: Int64?
    }

    private func makeRegistrar(
        host: String,
        handler: @escaping StubURLProtocol.Handler
    ) -> (PushRegistrar, StoredBox) {
        StubURLProtocol.register(host: host, handler: handler)
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let registrar = PushRegistrar(api: api)
        let box = StoredBox()
        registrar.loadStored = { (box.token, box.deviceID) }
        registrar.saveStored = { box.token = $0; box.deviceID = $1 }
        return (registrar, box)
    }

    /// Answers POST /devices with incrementing device ids and records
    /// DELETE paths; every other route 404s so a stray request fails loud.
    private static func deviceHandler(counter: Counter) -> StubURLProtocol.Handler {
        { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/devices"):
                return .json(201, #"{"device_id": \#(counter.increment() + 16)}"#)
            case ("DELETE", let path) where path.hasPrefix("/api/v1/devices/"):
                return .empty(204)
            default:
                return .json(404, #"{"error": {"code": "not_found", "message": "no"}}"#)
            }
        }
    }

    // MARK: - Tests

    @Test("first token POSTs {platform: ios, push_token: hex} and stores the pair")
    func firstRegistration() async throws {
        let host = "push-first.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (registrar, box) = makeRegistrar(host: host, handler: Self.deviceHandler(counter: Counter()))

        await registrar.register(tokenHex: "0a1b2c")

        let requests = StubURLProtocol.requests(host: host)
        #expect(requests.count == 1)
        #expect(requests[0].method == "POST")
        #expect(requests[0].url.path() == "/api/v1/devices")
        let body = requests[0].bodyJSON()
        // Whatever this build calls itself — "ios" on a phone, "macos" on
        // a Mac. Both are values the server routes to APNs; hard-coding
        // one made this fail the moment the suite started running on the
        // Mac, which is exactly what it should have done.
        #expect(body?["platform"] as? String == PushRegistrar.platform)
        #expect(body?["push_token"] as? String == "0a1b2c")
        #expect(box.token == "0a1b2c")
        #expect(box.deviceID == 17)
    }

    @Test("the VoIP token rides along when known, is absent when not, and re-POSTs when it changes")
    func voipToken() async throws {
        let host = "push-voip.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (registrar, box) = makeRegistrar(host: host, handler: Self.deviceHandler(counter: Counter()))
        let voipBox = StoredBox()
        registrar.loadStoredVoIP = { voipBox.token }
        registrar.saveStoredVoIP = { voipBox.token = $0 }

        // A launch where only the APNs token has arrived: the key must be
        // ABSENT, so the server keeps whatever VoIP token it holds.
        await registrar.register(tokenHex: "aaaa")
        var requests = StubURLProtocol.requests(host: host)
        #expect(requests.count == 1)
        #expect(requests[0].bodyJSON()?.keys.contains("voip_token") == false)

        // PushKit speaks: the pair is re-POSTed with the VoIP token beside
        // the unchanged APNs one, and the confirmed VoIP token is stored.
        registrar.handleVoIPToken("v0ip")
        await registrar.pendingRegistration?.value
        requests = StubURLProtocol.requests(host: host)
        #expect(requests.count == 2)
        #expect(requests[1].bodyJSON()?["push_token"] as? String == "aaaa")
        #expect(requests[1].bodyJSON()?["voip_token"] as? String == "v0ip")
        #expect(voipBox.token == "v0ip")
        #expect(box.token == "aaaa")

        // The same VoIP token again is not news.
        registrar.handleVoIPToken("v0ip")
        await registrar.pendingRegistration?.value
        #expect(StubURLProtocol.requests(host: host).count == 2)

        // Invalidated: an explicit null clears the server's copy.
        registrar.handleVoIPToken(nil)
        await registrar.pendingRegistration?.value
        requests = StubURLProtocol.requests(host: host)
        #expect(requests.count == 3)
        #expect(requests[2].bodyJSON()?.keys.contains("voip_token") == true)
        #expect(requests[2].bodyJSON()?["voip_token"] is NSNull)
        #expect(voipBox.token == nil)
    }

    @Test("same token again → no second POST; rotated token → re-POST")
    func rotation() async throws {
        let host = "push-rotate.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (registrar, box) = makeRegistrar(host: host, handler: Self.deviceHandler(counter: Counter()))

        await registrar.register(tokenHex: "aaaa")
        await registrar.register(tokenHex: "aaaa") // OS re-delivered, unchanged
        #expect(StubURLProtocol.requests(host: host).count == 1)

        await registrar.register(tokenHex: "bbbb") // OS rotated the token
        let requests = StubURLProtocol.requests(host: host)
        #expect(requests.count == 2)
        #expect(requests[1].bodyJSON()?["push_token"] as? String == "bbbb")
        #expect(box.token == "bbbb")
        #expect(box.deviceID == 18) // upsert answered a fresh row id
    }

    @Test("failed POST stores nothing; the next delivery of the same token retries")
    func retryAfterFailure() async throws {
        let host = "push-retry.test"
        defer { StubURLProtocol.unregister(host: host) }
        let counter = Counter()
        let (registrar, box) = makeRegistrar(host: host) { _ in
            counter.increment() == 1
                ? .json(500, #"{"error": {"code": "internal", "message": "restarting"}}"#)
                : .json(201, #"{"device_id": 17}"#)
        }

        await registrar.register(tokenHex: "cccc")
        #expect(box.token == nil)
        #expect(box.deviceID == nil)

        await registrar.register(tokenHex: "cccc")
        #expect(StubURLProtocol.requests(host: host).count == 2)
        #expect(box.token == "cccc")
        #expect(box.deviceID == 17)
    }

    @Test("deregister DELETEs the stored row, clears the pair, and re-arms registration")
    func deregister() async throws {
        let host = "push-logout.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (registrar, box) = makeRegistrar(host: host, handler: Self.deviceHandler(counter: Counter()))

        await registrar.register(tokenHex: "dddd")
        await registrar.deregister()

        var requests = StubURLProtocol.requests(host: host)
        #expect(requests.count == 2)
        #expect(requests[1].method == "DELETE")
        #expect(requests[1].url.path() == "/api/v1/devices/17")
        #expect(box.token == nil)
        #expect(box.deviceID == nil)

        // Logged out and back in: the very same OS token must POST again.
        await registrar.register(tokenHex: "dddd")
        requests = StubURLProtocol.requests(host: host)
        #expect(requests.count == 3)
        #expect(requests[2].method == "POST")
    }

    @Test("deregister with nothing stored issues no request")
    func deregisterEmpty() async throws {
        let host = "push-logout-empty.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (registrar, _) = makeRegistrar(host: host, handler: Self.deviceHandler(counter: Counter()))

        await registrar.deregister()
        #expect(StubURLProtocol.requests(host: host).isEmpty)
    }
}
