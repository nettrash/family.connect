//
//  UnreadBadge.swift
//  FamilyConnect
//
//  The app-icon badge. One number — the total unread across chats — put
//  on the icon by whichever mechanism the platform has for it.
//
//  It used to be a Mac-only file, because on iOS the number was taken to
//  belong to the SERVER: it rides on every push as `aps.badge`
//  (docs/protocol.md, "Push notifications") and the app's only job was to
//  clear it on the way in. That was wrong in the direction that costs a
//  message. The server pushes only to a device with no live socket, so
//  once the phone is in the foreground no push ever comes to correct the
//  number — the app cleared the icon on every single foreground and then
//  had nothing that could ever put a count back. The per-chat capsules
//  said three and the icon said nothing.
//
//  So both platforms now derive it locally, from the same rows the chat
//  list draws its per-chat badges from. That matches the server's own
//  definition (total unread across chats, `build_unread_badge_query`),
//  needs no network, and is fed from the single seam every unread write
//  passes through — ChatSyncCoordinator.saveContext.
//
//  The two mechanisms are NOT symmetric and the asymmetry is the reason
//  this file exists rather than one line at the call site:
//
//    - macOS uses `NSApp.dockTile`, which needs no permission at all. A
//      Mac user who declined notifications — an entirely reasonable thing
//      to do for an app that is always open — still gets the Dock badge.
//    - iOS has only `UNUserNotificationCenter.setBadgeCount`, which is
//      gated on the `.badge` authorization and is async. A denial is a
//      decision, not a fault: it is swallowed, never logged (it would
//      repeat on every save) and never prompts.
//
//  WHAT THE ICON SHOWS AT LAUNCH is a third question, and not the same
//  one as "what does it show while the app runs". Deriving the number
//  from the store is hung off ChatSyncCoordinator.saveContext, so until
//  something SAVES, no code in this app has touched the icon at all:
//
//    - on iOS it shows whatever the last APNs push left there, which is
//      a count of a server state this device may never have seen and can
//      be weeks old;
//    - on the Mac it shows nothing, because `dockTile.badgeLabel` is
//      per-process and starts nil — so a Mac launched with three unread
//      has a bare icon beside a sidebar full of capsules.
//
//  Both are closed by publishing the store's total ONCE at launch, from
//  RootView, before bootstrap. protocol.md is explicit about the split:
//  the pushed `badge` is what the SYSTEM puts on the icon while the app
//  is NOT running, and a RUNNING client derives its own from its store —
//  so launching is precisely where the app takes the number over. It is
//  also the only answer that exists with no network, which is the one
//  case a resync can never help with.
//
//  That seed can be LOWER than a correct pushed number for one round
//  trip: the store cannot know about messages that arrived while the
//  process was dead until the resync fetches them. It is neither a
//  regression nor observable — the icon is behind the app that is
//  drawing it, and by the time anybody leaves the app the resync has
//  written the real number. What the seed must never become is a CLEAR:
//  it publishes what the store says, which is zero only when the store
//  is genuinely empty. Clearing on launch, or on every foreground, is
//  the reverted design above wearing a different hat.
//
//  READING SOMEWHERE ELSE is the last question, and the only one this file
//  does not answer by itself. The number comes from the store, and the
//  store learns the truth from `unread_count` on GET /chats — so a chat
//  read on another of this person's devices corrects this icon at the next
//  resync and not before. The live `read` frame cannot help: the server
//  relays it to the OTHER members of the chat, so a reader's own devices
//  never see their own read go past. On a phone that is a moment, because
//  foregrounding resyncs. On a Mac it is not — the socket stays up for as
//  long as the app is open and nothing resyncs on its own — so the Dock
//  tile can sit on a number somebody cleared on their phone an hour ago
//  until a ⌘R, a reconnect or a relaunch.
//
//  What a resync additionally applies now is the caller's own
//  `last_read_message_id` (ChatSyncCoordinator, step 3), monotonically.
//  A marker that moved FORWARD is the evidence that takes this device's
//  stale banners out of Notification Center with it, which no resync used
//  to do at all. The icon and the notification list are the same
//  statement, and one of them being right is not enough.
//

import Foundation

#if os(macOS)
import AppKit
#else
import UserNotifications
#endif

@MainActor
enum UnreadBadge {

    #if os(iOS)
    /// The number the icon should be showing, and the single task applying
    /// it. `setBadgeCount` is async while every caller is not, so a burst
    /// of saves inside one resync would otherwise queue several awaits that
    /// can complete in any order — and the wrong one landing last leaves a
    /// stale count on the icon until something else happens to save. The
    /// applier re-reads `desired` after each attempt instead, so the last
    /// value written always wins and intermediate ones are coalesced away.
    private static var desired = 0
    private static var applyTask: Task<Void, Never>?
    #endif

    /// The one number, from the per-chat counts.
    ///
    /// Pure, and separate from `show`, so the arithmetic can be asserted
    /// without an icon to read it back off. The clamp is per chat rather
    /// than on the sum: one negative count — a store somebody has been
    /// poking at, a column default gone wrong — must not be able to
    /// cancel out real unread messages sitting in another chat.
    nonisolated static func total(unreadCounts: [Int]) -> Int {
        unreadCounts.reduce(0) { $0 + max(0, $1) }
    }

    /// Show `count` unread on the app icon, or nothing when it is zero.
    static func show(_ count: Int) {
        #if os(macOS)
        NSApp?.dockTile.badgeLabel = count > 0 ? String(count) : nil
        #else
        desired = count
        guard applyTask == nil else { return }
        applyTask = Task { @MainActor in
            while true {
                let value = desired
                try? await UNUserNotificationCenter.current().setBadgeCount(value)
                if desired == value { break }
            }
            applyTask = nil
        }
        #endif
    }
}
