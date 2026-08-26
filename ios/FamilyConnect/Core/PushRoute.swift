//
//  PushRoute.swift
//  FamilyConnect
//
//  Where a tapped push notification routes the app, and the pure parser
//  from an APNs userInfo dictionary (protocol.md §Push notifications:
//  custom keys kind / chat_id / message_id / family_id ride next to the
//  aps dictionary). Kept free of UIKit and UserNotifications types so
//  the payload → route mapping is a plain unit-testable table — the
//  notification delegate hands in the dictionary, everything else
//  happens here.
//
//  The parser is total: unknown kinds, missing keys and malformed ids
//  all degrade to `.chatList` (the app's home in the active phase),
//  never a crash and never a dropped tap.
//

import Foundation

nonisolated enum PushRoute: Equatable, Sendable {
    /// kind "message" — open the chat the message landed in.
    case chat(Int64)
    /// kind "board_note" — the family board.
    case board
    /// kind "join_request" — the owner's pending-requests screen.
    case joinRequests
    /// kind "joined" and anything unrecognised — the chat list.
    case chatList

    /// Map one notification payload to a route.
    static func parse(userInfo: [AnyHashable: Any]) -> PushRoute {
        guard let kind = userInfo["kind"] as? String else { return .chatList }
        switch kind {
        case "message":
            guard let chatID = int64(userInfo["chat_id"]) else { return .chatList }
            return .chat(chatID)
        case "board_note":
            return .board
        case "join_request":
            return .joinRequests
        default:
            // "joined" plus forward compatibility with future kinds.
            return .chatList
        }
    }

    /// The message a `message` push is ABOUT, rather than where tapping it
    /// goes.
    ///
    /// One caller, and it wants the id precisely because the id is a
    /// threshold: ChatNotifier compares it against `last_read_message_id`
    /// to decide whether a delivered notification is still true (see
    /// `isRead`). It lives here because this is the payload parser — the
    /// custom keys are `kind` / `chat_id` / `message_id` / `family_id` and
    /// there is no reason for a second thing that knows that.
    ///
    /// nil for every other kind, on purpose: `board_note`, `join_request`
    /// and `joined` carry no message id, and a read marker has nothing to
    /// say about them.
    static func messageID(userInfo: [AnyHashable: Any]) -> Int64? {
        guard userInfo["kind"] as? String == "message" else { return nil }
        return int64(userInfo["message_id"])
    }

    /// Int64 from the shapes an id actually arrives in: NSNumber for the
    /// JSON numbers our server sends over APNs, String as tolerance for
    /// stringly-typed payloads (FCM's data map is strings-only, and a
    /// server refactor drifting to strings should not break routing).
    private static func int64(_ value: Any?) -> Int64? {
        switch value {
        case let number as NSNumber:
            return number.int64Value
        case let text as String:
            return Int64(text)
        default:
            return nil
        }
    }
}
