//
//  AvatarFailure.swift
//  FamilyConnect
//
//  Turning an APIError from the profile-picture endpoints into the one
//  sentence the user sees.
//
//  It is its own type because collapsing every failure into "Couldn't
//  upload the photo." costs real time: the API layer already knows
//  whether the server was unreachable, answered 404 because it predates
//  profile pictures, refused the bytes, or sent something undecodable —
//  and each of those wants a different response from the person holding
//  the phone. A pure function is also the piece worth pinning with tests.
//
//  Android counterpart: SettingsViewModel.uploadFailure — same wording.
//

import Foundation

nonisolated enum AvatarFailure {

    /// - Parameter verb: "upload" or "remove", for the fallback sentence.
    static func message(for error: Error, verb: String) -> String {
        switch error {
        case APIError.notConfigured:
            return "No server address is set."
        case APIError.transport:
            return "Can't reach the server. Check your connection."
        case APIError.notFound:
            // The endpoint itself is missing. Both avatar mutations are
            // "me"-scoped, so the server can only 404 them by not having
            // the route at all — i.e. it was built before profile
            // pictures existed.
            return "This server doesn't support profile pictures yet — it needs updating."
        case APIError.forbidden:
            return "The server refused that."
        case APIError.payloadTooLarge:
            // Either the server's own cap or — far more often — a proxy
            // in front of it with a smaller `client_max_body_size`.
            return "That photo is too large for this server."
        case APIError.conflict(let code, let message):
            return switch code {
            case "avatar_too_large": "That photo is too large for this server."
            case "invalid_image": "That file isn't a photo we can use."
            default: message ?? "The server refused the photo."
            }
        case APIError.server(let status, _):
            return "The server had a problem (\(status)). Try again."
        case APIError.decoding:
            return "The server sent an answer this app didn't understand."
        default:
            return "Couldn't \(verb) the photo."
        }
    }
}
