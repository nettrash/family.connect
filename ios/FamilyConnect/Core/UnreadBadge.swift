//
//  UnreadBadge.swift
//  FamilyConnect
//
//  The app-icon badge, which each platform gets from a different place —
//  and on the Mac used to get from nowhere at all.
//
//  On iOS the number comes from the SERVER: it rides on every push as
//  `aps.badge`, computed as that user's total unread across chats
//  (docs/protocol.md, "Push notifications"). The app only ever clears it.
//
//  On macOS that never happens, and the reason is not a bug — it is the
//  push rule working as designed. The server pushes ONLY to users with no
//  live WebSocket, and a Mac holds its socket open for as long as a window
//  is open, including while the app sits behind everything else. So a Mac
//  is almost never push-eligible, never receives a badge, and the Dock icon
//  looks identical whether or not the family has been talking.
//
//  The fix is to stop waiting for a push and derive the number locally from
//  the same rows the chat list already draws its per-chat badges from. That
//  matches the server's definition (total unread across chats), needs no
//  network, and works while the app is in the foreground — which on a Mac
//  is most of the time.
//
//  `NSApp.dockTile` deliberately, NOT `UNUserNotificationCenter
//  .setBadgeCount`: the latter is gated on the `.badge` notification
//  authorization, so a Mac user who declined notifications — an entirely
//  reasonable thing to do for an app that is always open — would get no
//  Dock badge either. The Dock tile needs no permission.
//

import Foundation

#if os(macOS)
import AppKit
#endif

enum UnreadBadge {
    /// Show `count` unread on the app icon, or nothing when it is zero.
    ///
    /// A no-op on iOS, where the badge belongs to the push payload: setting
    /// it locally would fight the server's number and lose, because the
    /// next notification overwrites it anyway.
    @MainActor
    static func show(_ count: Int) {
        #if os(macOS)
        NSApp?.dockTile.badgeLabel = count > 0 ? String(count) : nil
        #endif
    }
}
