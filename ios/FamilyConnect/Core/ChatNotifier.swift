//
//  ChatNotifier.swift
//  FamilyConnect
//
//  Local notifications, and the removal of delivered ones. Two jobs, both
//  of which exist because an icon badge is not a message.
//
//  1. On the MAC, a message arriving over the socket used to produce
//     nothing at all — no banner, no sound, one silent number on the Dock
//     tile that a click then erased. There was no
//     UNMutableNotificationContent anywhere in the target. And this is not
//     a gap APNs can fill: the server pushes only to a device whose
//     session is not live, and a Mac holds its socket open for as long as
//     the app is running, so the app that received the frame is the only
//     thing here that can say anything. That is how every desktop chat
//     client works.
//
//  2. On BOTH platforms, reading a chat has to take that chat's banners
//     out of Notification Center with it. Otherwise the icon says nothing
//     and the notification list still says three — the same contradiction
//     as a stale badge, from the other side.
//
//  WORDING IS NOT INVENTED HERE. `server/src/push_payload.rs` composes
//  what a phone is told about a message, and a Mac saying something
//  different about the same message reads as two apps. Title: the sender
//  for a direct chat, "<Family> — <Sender>" in the family chat. Body: the
//  text, or what kind of thing arrived when there is none — and for a
//  location its label or the word, NEVER its coordinates, which is the
//  server's rule and applies wherever an alert can be read off a screen
//  nobody unlocked.
//
//  Authorization is never asked for here: PushRegistrar already asks once
//  per install, for [.alert, .sound, .badge], at the end of the first
//  resync. A refusal is final and silent — `add` simply fails, the Dock
//  badge needs no permission and carries on alone.
//

import Foundation
import UserNotifications

nonisolated enum ChatNotifier {

    /// Marks a notification this app raised itself.
    ///
    /// The notification-center delegates suppress presentation while the
    /// app is frontmost, because a REMOTE notification arriving then is the
    /// race where a push sent to a backgrounded app lands just after
    /// resume, and the socket already delivered the same event. A local one
    /// is the opposite case by construction — it was raised precisely
    /// BECAUSE nobody is looking at that chat — so it must not be swept up
    /// by the same rule.
    static let localKey = "local"

    /// The grouping key all of one chat's notifications share, spelled the
    /// way the server spells its APNs `thread-id` (`chat-<id>`,
    /// docs/protocol.md) — so dismissing a chat's notifications catches the
    /// ones APNs delivered as well as the ones this app raised.
    static func threadIdentifier(chatID: Int64) -> String { "chat-\(chatID)" }

    // MARK: - Wording (mirrors server/src/push_payload.rs)

    /// Deliberately NOT localized, and that is the point: `push_payload.rs`
    /// builds this string with no notion of a language, and mirroring it is
    /// what makes a Mac and a phone say the SAME thing about the same
    /// message. The words inside it — names — are not ours to translate
    /// either.
    static func title(chatKind: String, chatTitle: String, senderName: String) -> String {
        chatKind == "family" ? "\(chatTitle) — \(senderName)" : senderName
    }

    /// The text, or what kind of thing arrived when a message carries no
    /// words — which is the ORDINARY case for a photo, not an edge one, and
    /// an alert with a name and a blank line tells the reader nothing.
    ///
    /// `ChatSyncCoordinator.preview` already computes exactly the server's
    /// `attachment_summary` (a file's name, a location's label or the word,
    /// never coordinates); the only difference is what an empty message
    /// with no attachment says, and the server says "New message".
    static func body(text: String, attachment: AttachmentDTO?) -> String {
        let summary = ChatSyncCoordinator.preview(body: text, attachment: attachment)
        return summary.isEmpty ? String(localized: "New message") : summary
    }

    // MARK: - Raising one

    /// The custom keys the notification carries, which are the SAME keys an
    /// APNs payload carries (protocol.md, "Push notifications") — so a
    /// click routes through PushRoute exactly as a push's click does, one
    /// routing scheme rather than a second one to keep in step with it.
    ///
    /// `localKey` is the only addition, and it is what stops the
    /// notification-center delegate from suppressing this the way it
    /// suppresses a remote notification arriving in the foreground.
    static func userInfo(chatID: Int64, messageID: Int64) -> [String: Any] {
        [
            "kind": "message",
            "chat_id": chatID,
            "message_id": messageID,
            localKey: true,
        ]
    }

    #if os(macOS)
    /// Tell the person at the Mac that something arrived.
    ///
    /// The caller decides WHETHER — that is ChatPresence and nothing else
    /// (see ChatSyncCoordinator.announce). This only says it.
    static func announce(chatID: Int64, messageID: Int64, title: String, body: String) {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default
        content.threadIdentifier = threadIdentifier(chatID: chatID)
        content.userInfo = userInfo(chatID: chatID, messageID: messageID)
        // Deterministic, so the same message announced twice — a frame
        // re-delivered by a reconnect racing a catch-up page — replaces its
        // own banner instead of stacking a second copy of itself.
        let request = UNNotificationRequest(
            identifier: "\(threadIdentifier(chatID: chatID))-\(messageID)",
            content: content,
            trigger: nil)
        let center = UNUserNotificationCenter.current()
        // No error handling and no log: the one failure that happens in
        // practice is a declined authorization, which is a decision, not a
        // fault, and repeats on every single message.
        Task { try? await center.add(request) }
    }
    #endif

    // MARK: - Taking them back

    /// Drop every delivered notification belonging to `chatID`.
    ///
    /// Called when a chat becomes read, so the badge and Notification
    /// Center never contradict each other. Matched two ways because the
    /// notifications come from two places: this app's own carry the thread
    /// key, and APNs-delivered ones carry the chat id among their custom
    /// keys — which PushRoute already knows how to read, so there is no
    /// second parser here.
    ///
    /// The completion-handler API rather than the async one on purpose: the
    /// notifications never leave the callback (only their identifier
    /// strings do), so nothing non-Sendable has to cross an isolation
    /// boundary to get this done.
    static func dismissDelivered(chatID: Int64) {
        let thread = threadIdentifier(chatID: chatID)
        // `current()` is re-read inside the callback rather than captured:
        // the centre is not Sendable, and it is a singleton accessor, so
        // there is nothing to carry across.
        UNUserNotificationCenter.current().getDeliveredNotifications { delivered in
            let identifiers = delivered.filter { note in
                note.request.content.threadIdentifier == thread
                    || PushRoute.parse(userInfo: note.request.content.userInfo) == .chat(chatID)
            }.map(\.request.identifier)
            guard !identifiers.isEmpty else { return }
            UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: identifiers)
        }
    }
}
