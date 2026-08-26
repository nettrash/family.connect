/*
 * UnreadBadge.kt
 * Family Connect (Android)
 *
 * The unread number — and the arithmetic behind it, kept away from
 * anything that has to be running to be asked.
 *
 * ANDROID HAS NO ICON-BADGE API. There is no `setBadgeCount`, no dock
 * tile, nothing an app may write a number onto. The only number an app
 * can offer a launcher is `Notification.number`
 * (NotificationCompat.Builder.setNumber), and a launcher that draws a
 * count SUMS it across that app's live notifications. Most AOSP-derived
 * launchers — Pixel included — draw a dot and never a number at all, and
 * that is a correct rendering of the same data, not a bug to work around.
 * No third-party badge library is used or wanted: they poke vendor-
 * specific content providers, and the ones that still work do exactly
 * what `setNumber` does.
 *
 * That is why the number on the wire is PER CHAT
 * (`android.notification.notification_count`, docs/protocol.md) and not
 * the APNs `badge` total: this app posts one notification per chat
 * (`tag: "chat-<id>"`), so putting the total on each of three chats
 * would render as three times the total. [total] is therefore not a
 * number anything here draws — it is the number the launcher ARRIVES at
 * by summing, and so the invariant the per-chat numbers have to add up
 * to. It exists as a flow (UnreadNotifications) because the clearing
 * rule is derived from it, and as a function here because arithmetic
 * that decides what a user sees deserves to be asserted without an icon
 * to read it back off.
 *
 * THE TOTAL IS NOT A LICENCE TO SWEEP, and this is the trap worth
 * writing down. "The store says nothing is unread" and "nothing is
 * unread" are different statements at exactly one moment: process start.
 * A push that arrived while the app was dead was rendered by the SYSTEM
 * tray, which this process never saw — the store knows nothing about
 * that message and honestly reports zero. Cancelling notifications
 * because the total is zero would, at that instant, delete the one
 * notification the user has not seen. So nothing here acts on zero; only
 * on a chat GOING to zero from a count this process watched ([nowRead]),
 * or on the server saying so out of a resync.
 *
 * iOS counterpart: ios/FamilyConnect/Core/UnreadBadge.swift (which has an
 * icon to put the total on, and puts it there).
 */

package me.nettrash.familyconnect.data.push

object UnreadBadge {

    /**
     * The one number, from the per-chat counts.
     *
     * The clamp is per chat rather than on the sum: one negative count —
     * a store somebody has been poking at, a column default gone wrong —
     * must not be able to cancel out real unread messages sitting in
     * another chat.
     */
    fun total(unreadCounts: List<Int>): Int = unreadCounts.sumOf { it.coerceAtLeast(0) }

    /**
     * The chats whose notifications have just gone stale: the ones this
     * process watched go from something unread to nothing.
     *
     * A TRANSITION, deliberately, and never the plain state — see the
     * file header. [before] having a count above zero is the proof this
     * process was actually keeping that chat's number, so a zero after it
     * is a read and not merely an empty store.
     *
     * A chat that DISAPPEARS with unread messages on it counts too: a
     * peer deleting their account takes the direct chat with them
     * (docs/protocol.md, "Deleting an account"), and the row it was
     * counted on goes with it. Leaving the notification behind would
     * leave a badge for a conversation that no longer exists and cannot
     * be opened.
     */
    fun nowRead(before: Map<Long, Int>, after: Map<Long, Int>): Set<Long> =
        before.asSequence()
            .filter { (chatId, count) -> count > 0 && (after[chatId] ?: 0) <= 0 }
            .mapTo(LinkedHashSet()) { it.key }
}
