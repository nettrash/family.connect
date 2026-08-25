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
