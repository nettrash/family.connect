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
    /// kind "report" — the owner's moderation inbox.
    ///
    /// A report raises a push and NO WebSocket frame, exactly as a join
    /// request does (protocol.md, "Reporting a member"), so this push is
    /// the owner's only live signal that one arrived — a tap that landed
    /// on the chat list instead would leave them hunting for it.
    case reports
    /// kind "invitation" — somebody invited this account into their family
    /// (docs/protocol.md, "Several families", 2026-10-22): the invitations,
    /// which are the banner over the chat list and rows in the switcher.
    case invitations
    /// kind "family_plan" — a paid family stopped being paid for, is
    /// deleted tomorrow, or was deleted (docs/protocol.md, "Paid families",
    /// 2026-10-30): the app opens in that family, where the banner over the
    /// chat list says the rest — or, for `stage: "deleted"`, which names a
    /// family that is gone, on the family list as it now is.
    case familyPlan
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
        case "report":
            return .reports
        case "invitation":
            return .invitations
        case "family_plan":
            return .familyPlan
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
    /// The family a push is about, when it names one: a family chat's,
    /// group's or `ai` chat's message, a board note, a report, a join
    /// request, an invitation (docs/protocol.md, "Several families" —
    /// Pushes). A direct chat's message names none.
    static func familyID(userInfo: [AnyHashable: Any]) -> Int64? {
        // A deleted family is not one to open in: the app stays on the
        // families it still has.
        if userInfo["kind"] as? String == "family_plan", userInfo["stage"] as? String == "deleted" {
            return nil
        }
        return int64(userInfo["family_id"])
    }

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
