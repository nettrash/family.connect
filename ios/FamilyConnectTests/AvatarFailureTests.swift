//
//  AvatarFailureTests.swift
//  FamilyConnectTests
//
//  Every way a profile-picture request can fail has to reach the user as
//  a different sentence — the whole point of the type. A regression here
//  is silent (the app still "works", it just stops explaining itself),
//  so each branch is pinned.
//

import Foundation
import Testing
@testable import FamilyConnect

struct AvatarFailureTests {

    @Test("an unreachable server is not confused with a refused photo")
    func transport() {
        let message = AvatarFailure.message(
            for: APIError.transport(URLError(.notConnectedToInternet)), verb: "upload")
        #expect(message.contains("Can't reach the server"))
    }

    /// The one that matters most in practice: both avatar mutations are
    /// "me"-scoped, so a 404 can only mean the route is absent — a server
    /// built before profile pictures. Saying "Couldn't upload the photo."
    /// there sends the user looking at their photo instead of their server.
    @Test("a 404 says the server is too old, not that the photo is bad")
    func missingEndpoint() {
        let message = AvatarFailure.message(for: APIError.notFound(code: nil), verb: "upload")
        #expect(message.contains("doesn't support profile pictures"))
    }

    @Test("the protocol's two picture refusals get their own wording")
    func pictureRefusals() {
        #expect(AvatarFailure.message(
            for: APIError.conflict(code: "avatar_too_large", message: "too big"),
            verb: "upload") == "That photo is too large for this server.")
        #expect(AvatarFailure.message(
            for: APIError.conflict(code: "invalid_image", message: "nope"),
            verb: "upload") == "That file isn't a photo we can use.")
    }

    @Test("an unknown 4xx falls back to the server's own sentence")
    func unknownConflict() {
        #expect(AvatarFailure.message(
            for: APIError.conflict(code: "something_new", message: "Try a smaller square."),
            verb: "upload") == "Try a smaller square.")
        // …and to something human when the server sent no message.
        #expect(AvatarFailure.message(
            for: APIError.conflict(code: nil, message: nil),
            verb: "upload") == "The server refused the photo.")
    }

    /// The one that produced a real bug report: a proxy in front of the
    /// server answers an oversize body with a bare 413 — no protocol error
    /// body, so no code — and "The server refused the photo." sent the
    /// reader looking in the wrong place entirely.
    @Test("a bare 413 from a proxy still says the photo is too large")
    func proxyPayloadTooLarge() {
        let message = AvatarFailure.message(for: APIError.payloadTooLarge, verb: "upload")
        #expect(message.contains("too large"))
    }

    @Test("a 5xx carries the status so a report can name it")
    func serverError() {
        let message = AvatarFailure.message(
            for: APIError.server(status: 502, message: nil), verb: "upload")
        #expect(message.contains("502"))
    }

    @Test("a bad body is blamed on the server, not the photo")
    func decoding() {
        let message = AvatarFailure.message(for: APIError.decoding, verb: "upload")
        #expect(message.contains("didn't understand"))
    }

    @Test("the fallback names what was being attempted")
    func fallbackVerb() {
        struct Odd: Error {}
        #expect(AvatarFailure.message(for: Odd(), verb: "remove") == "Couldn't remove the photo.")
        #expect(AvatarFailure.message(for: Odd(), verb: "upload") == "Couldn't upload the photo.")
    }

    @Test("every case produces a distinct sentence")
    func allDistinct() {
        let messages = [
            AvatarFailure.message(for: APIError.notConfigured, verb: "upload"),
            AvatarFailure.message(for: APIError.transport(URLError(.timedOut)), verb: "upload"),
            AvatarFailure.message(for: APIError.notFound(code: nil), verb: "upload"),
            AvatarFailure.message(for: APIError.forbidden(code: nil), verb: "upload"),
            AvatarFailure.message(for: APIError.conflict(code: "invalid_image", message: nil), verb: "upload"),
            AvatarFailure.message(for: APIError.payloadTooLarge, verb: "upload"),
            AvatarFailure.message(for: APIError.server(status: 500, message: nil), verb: "upload"),
            AvatarFailure.message(for: APIError.decoding, verb: "upload"),
        ]
        #expect(Set(messages).count == messages.count)
    }
}
