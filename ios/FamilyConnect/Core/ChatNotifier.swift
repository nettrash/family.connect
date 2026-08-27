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
//     as a stale badge, from the other side. Reading is not only something
//     that happens HERE, either: the read marker belongs to the PERSON and
//     not to the device, so a resync that finds `last_read_message_id`
//     further along than this device had recorded has learned that the
//     banners sitting here below it are stale, whichever device did the
//     reading. That is the only way this device ever finds out — the live
//     `read` frame is relayed to the OTHER members of the chat, so a
//     reader's own devices never see their own read go past.
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
//  Authorization is never asked for here: PushRegistrar asks once per
//  install, for [.alert, .sound, .badge] — at Mac launch and on every
//  arrival at .active, not only at the resync's tail. A refusal is final
//  but no longer silent: a failed `add` is logged (chat and message ids
//  only, never bodies), because a silently-failing `add` is how this app
//  once ran for days on a Mac that had never registered with Notification
//  Center at all. And the Dock badge does NOT carry on alone: modern
//  macOS gates it behind the same per-app Notification Center settings
//  ("Badge application icon") — see UnreadBadge.
//

import Foundation
import os
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
    static func body(text: String, attachment: AttachmentDTO?, call: CallDTO? = nil) -> String {
        body(text: text, attachments: attachment.map { [$0] } ?? [], call: call)
    }

    /// The plural spelling, mirroring the server's push-summary rules:
    /// one attachment by name or kind, several of one kind as a count
    /// ("3 Photos"), a mixed set as "N attachments" — and a caption still
    /// wins (docs/protocol.md, "Push notifications").
    static func body(text: String, attachments: [AttachmentDTO], call: CallDTO? = nil) -> String {
        // A call record is announced from the CALLEE's side — the caller
        // is never told about their own call — so the wording is theirs.
        let summary = ChatSyncCoordinator.preview(
            body: text, attachments: attachments, call: call, isMine: false)
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

    // MARK: - Incoming calls (macOS)

    /// The category an incoming call's banner carries, and its two buttons.
    static let incomingCallCategoryID = "me.nettrash.FamilyConnect.incomingCall"
    static let answerActionID = "answer"
    static let declineActionID = "decline"

    static func incomingCallCategory() -> UNNotificationCategory {
        UNNotificationCategory(
            identifier: incomingCallCategoryID,
            actions: [
                UNNotificationAction(
                    identifier: answerActionID,
                    title: String(localized: "Answer"),
                    options: [.foreground]),
                UNNotificationAction(
                    identifier: declineActionID,
                    title: String(localized: "Decline"),
                    options: [.destructive]),
            ],
            intentIdentifiers: [],
            options: [])
    }

    #if os(macOS)
    /// Ring: a Mac has no CallKit, and the offer arrives over the socket
    /// whether or not the app is in front. Answer and Decline are on the
    /// banner itself, so somebody working in another app never has to find
    /// the window (docs/protocol.md, "Voice calls").
    static func announceIncomingCall(callID: String, callerName: String) {
        let content = UNMutableNotificationContent()
        content.title = String(localized: "Incoming call")
        content.body = String(localized: "\(callerName) is calling")
        content.sound = .default
        content.categoryIdentifier = incomingCallCategoryID
        content.interruptionLevel = .timeSensitive
        content.userInfo = ["kind": "call", "call_id": callID, localKey: true]
        let request = UNNotificationRequest(identifier: "call-\(callID)", content: content, trigger: nil)
        let center = UNUserNotificationCenter.current()
        Task {
            do {
                try await center.add(request)
            } catch {
                AppLog.push.error("Incoming-call notification failed for call \(callID, privacy: .public): \(String(describing: error), privacy: .public)")
            }
        }
    }

    /// The call was answered, declined, or rang out — on any device.
    static func dismissIncomingCall(callID: String) {
        let center = UNUserNotificationCenter.current()
        center.removeDeliveredNotifications(withIdentifiers: ["call-\(callID)"])
        center.removePendingNotificationRequests(withIdentifiers: ["call-\(callID)"])
    }

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
        // Failures are logged — ids only, never the title or body. The
        // common failure is still a declined authorization, and the line
        // repeating per message is the point: this exact call once sat
        // behind a bare `try?` while the app was not even registered with
        // Notification Center, and nothing anywhere said so.
        Task {
            do {
                try await center.add(request)
            } catch {
                AppLog.push.error("Local notification failed for chat \(chatID, privacy: .public) message \(messageID, privacy: .public): \(String(describing: error), privacy: .public)")
            }
        }
    }
    #endif

    // MARK: - Taking them back

    /// The inverse of `threadIdentifier(chatID:)`, kept beside it so the
    /// spelling of that key exists in exactly one place.
    static func chatID(threadIdentifier: String) -> Int64? {
        let prefix = "chat-"
        guard threadIdentifier.hasPrefix(prefix) else { return nil }
        return Int64(threadIdentifier.dropFirst(prefix.count))
    }

    /// Has the reader already passed the message this delivered
    /// notification is about?
    ///
    /// `readMarkers` is chatID → that chat's `last_read_message_id`, used
    /// exactly as protocol.md defines the field: an id THRESHOLD, compared
    /// against and never fetched (retention may already have swept the
    /// message it names). A message at or below it has been read on SOME
    /// device this person owns, so its banner states something that is no
    /// longer true; a message above it has not been read anywhere, and its
    /// banner stays.
    ///
    /// Per MESSAGE rather than per chat, because a read is partial far more
    /// often than it is total. A marker at 53 with 54 already delivered is
    /// the ordinary case on a Mac — which raises its own banners off the
    /// socket the whole time it runs — and taking that chat's notifications
    /// down wholesale would destroy the announcement of a message nobody
    /// has seen. That is the one failure this file exists to prevent.
    ///
    /// The chat is read out of the payload two ways because the
    /// notifications come from two places: this app's own carry the thread
    /// key, and APNs-delivered ones carry the chat id among their custom
    /// keys — which PushRoute already knows how to read, so there is no
    /// second parser here.
    ///
    /// A notification with no message id is never dismissed by this rule.
    /// Nothing this app raises and nothing this server sends for a chat
    /// lacks one, so it is not a case that arises; if it ever did, leaving
    /// the banner up costs a glance and taking it down costs the message,
    /// which is the bias ChatPresence sets for everything on this side.
    ///
    /// Pure and separate from the removal below so the rule can be asserted
    /// without a notification centre to put anything in.
    static func isRead(
        by readMarkers: [Int64: Int64],
        threadIdentifier thread: String,
        userInfo: [AnyHashable: Any]
    ) -> Bool {
        guard !readMarkers.isEmpty else { return false }
        let chatID: Int64?
        if case .chat(let id) = PushRoute.parse(userInfo: userInfo) {
            chatID = id
        } else {
            chatID = Self.chatID(threadIdentifier: thread)
        }
        guard let chatID,
              let marker = readMarkers[chatID],
              let messageID = PushRoute.messageID(userInfo: userInfo)
        else { return false }
        return messageID <= marker
    }

    /// Drop every delivered notification belonging to `chatID`.
    ///
    /// `markRead`'s door, and the one case where a whole chat goes at once:
    /// the person is looking at its newest message with the app in front of
    /// them (ChatPresence), so there is nothing left in it to announce.
    static func dismissDelivered(chatID: Int64) {
        let thread = threadIdentifier(chatID: chatID)
        dismissDelivered { userInfo, threadIdentifier in
            threadIdentifier == thread || PushRoute.parse(userInfo: userInfo) == .chat(chatID)
        }
    }

    /// Drop every delivered notification the read markers have passed.
    ///
    /// The resync's door, and the reason it takes a threshold per chat
    /// rather than a chat id: "becomes read" is not only this device's
    /// doing. The marker belongs to the person and not to the device — one
    /// value, shared across every device they own — so a resync that finds
    /// it further along than this device had recorded has learned that the
    /// chat was read somewhere else, and that the banners sitting here for
    /// the messages below it are stale. Nothing else ever tells this device
    /// that: the live `read` frame is relayed only to OTHER members, so a
    /// reader's own devices never see their own read go past.
    ///
    /// A whole map rather than a chat at a time because a resync answers
    /// for the entire list at once, and one pass over the delivered
    /// notifications settles all of it.
    static func dismissDelivered(readMarkers: [Int64: Int64]) {
        guard !readMarkers.isEmpty else { return }
        dismissDelivered { userInfo, threadIdentifier in
            isRead(by: readMarkers, threadIdentifier: threadIdentifier, userInfo: userInfo)
        }
    }

    /// The single pass over Notification Center, shared by both doors.
    ///
    /// The completion-handler API rather than the async one on purpose: the
    /// notifications never leave the callback (only their identifier
    /// strings do), so nothing non-Sendable has to cross an isolation
    /// boundary to get this done. The predicate is handed the two pieces
    /// the doors judge on rather than the notification itself, for the same
    /// reason.
    private static func dismissDelivered(
        where isStale: @escaping @Sendable ([AnyHashable: Any], String) -> Bool
    ) {
        // `current()` is re-read inside the callback rather than captured:
        // the centre is not Sendable, and it is a singleton accessor, so
        // there is nothing to carry across.
        UNUserNotificationCenter.current().getDeliveredNotifications { delivered in
            let identifiers = delivered.filter { note in
                isStale(note.request.content.userInfo, note.request.content.threadIdentifier)
            }.map(\.request.identifier)
            guard !identifiers.isEmpty else { return }
            UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: identifiers)
        }
    }
}
