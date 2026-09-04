//
//  StubURLProtocol.swift
//  FamilyConnectTests
//
//  Canned-response URLSession for the APIClient / coordinator tests.
//  Handlers are ROUTED BY HOST, not held in one global slot: Swift
//  Testing runs suites in parallel, so each test picks a unique fake
//  host ("send-pipeline.test", …) and registers its handler under it —
//  no cross-test interference, no need to serialize everything.
//
//  Every request is also logged per host (method, URL, and the body —
//  drained from httpBodyStream, because URLSession strips httpBody
//  before a URLProtocol sees the request), so tests can assert exact
//  URLs, headers and payloads.
//

import Foundation

/// One recorded request: what a test asserts against.
struct RecordedRequest: @unchecked Sendable {
    let method: String
    let url: URL
    let headers: [String: String]
    let body: Data?

    func bodyJSON() -> [String: Any]? {
        guard let body else { return nil }
        return (try? JSONSerialization.jsonObject(with: body)) as? [String: Any]
    }
}

/// What a handler answers with.
struct StubResponse: Sendable {
    let status: Int
    let headers: [String: String]
    let body: Data
    /// When set, the request fails at the transport layer instead of
    /// answering — no HTTP response at all, which is what a phone losing
    /// its route actually produces and what the send pipeline's
    /// transient/terminal rule turns on.
    var transportError: URLError?

    static func json(_ status: Int, _ body: String, headers: [String: String] = [:]) -> StubResponse {
        var merged = headers
        merged["Content-Type"] = "application/json"
        return StubResponse(status: status, headers: merged, body: Data(body.utf8))
    }

    static func empty(_ status: Int) -> StubResponse {
        StubResponse(status: status, headers: [:], body: Data())
    }

    /// The request never gets an answer: DNS, refused, reset, timeout.
    static func failure(_ error: URLError) -> StubResponse {
        StubResponse(status: 0, headers: [:], body: Data(), transportError: error)
    }
}

final class StubURLProtocol: URLProtocol {
    typealias Handler = @Sendable (RecordedRequest) -> StubResponse

    private static let lock = NSLock()
    nonisolated(unsafe) private static var handlers: [String: Handler] = [:]
    nonisolated(unsafe) private static var log: [String: [RecordedRequest]] = [:]

    /// Register the canned behaviour for one fake host.
    static func register(host: String, handler: @escaping Handler) {
        lock.lock(); defer { lock.unlock() }
        handlers[host] = handler
        log[host] = []
    }

    static func unregister(host: String) {
        lock.lock(); defer { lock.unlock() }
        handlers[host] = nil
        log[host] = nil
    }

    /// Everything sent to a host, in order.
    static func requests(host: String) -> [RecordedRequest] {
        lock.lock(); defer { lock.unlock() }
        return log[host] ?? []
    }

    /// A URLSession whose traffic all lands in this protocol.
    static func makeSession() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [StubURLProtocol.self]
        return URLSession(configuration: configuration)
    }

    // MARK: - URLProtocol

    override class func canInit(with request: URLRequest) -> Bool {
        guard let host = request.url?.host() else { return false }
        lock.lock(); defer { lock.unlock() }
        return handlers[host] != nil
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        guard let url = request.url, let host = url.host() else {
            client?.urlProtocol(self, didFailWithError: URLError(.badURL))
            return
        }

        let recorded = RecordedRequest(
            method: request.httpMethod ?? "GET",
            url: url,
            headers: request.allHTTPHeaderFields ?? [:],
            body: Self.drainBody(of: request))

        let handler: Handler? = {
            Self.lock.lock(); defer { Self.lock.unlock() }
            Self.log[host, default: []].append(recorded)
            return Self.handlers[host]
        }()

        guard let handler else {
            client?.urlProtocol(self, didFailWithError: URLError(.cannotFindHost))
            return
        }

        let stub = handler(recorded)
        if let transportError = stub.transportError {
            client?.urlProtocol(self, didFailWithError: transportError)
            return
        }
        let response = HTTPURLResponse(
            url: url,
            statusCode: stub.status,
            httpVersion: "HTTP/1.1",
            headerFields: stub.headers)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: stub.body)
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}

    /// URLSession replaces httpBody with a stream before the protocol
    /// runs; read it back so tests can assert POST payloads.
    private static func drainBody(of request: URLRequest) -> Data? {
        if let body = request.httpBody { return body }
        guard let stream = request.httpBodyStream else { return nil }
        stream.open()
        defer { stream.close() }
        var data = Data()
        let bufferSize = 4096
        let buffer = UnsafeMutablePointer<UInt8>.allocate(capacity: bufferSize)
        defer { buffer.deallocate() }
        while stream.hasBytesAvailable {
            let read = stream.read(buffer, maxLength: bufferSize)
            guard read > 0 else { break }
            data.append(buffer, count: read)
        }
        return data
    }
}
