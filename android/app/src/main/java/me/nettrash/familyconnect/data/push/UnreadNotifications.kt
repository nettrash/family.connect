/*
 * UnreadNotifications.kt
 * Family Connect (Android)
 *
 * The badge's SOURCE and its CLEARING, in one place: the store's unread
 * counts, watched for the life of the process, and the tray entries that
 * have to come down when one of them goes to nothing.
 *
 * The number itself is carried by PushNotifications (setNumber, per
 * chat); the arithmetic and the reasons for it are in UnreadBadge. This
 * file is the part that has to be RUNNING.
 *
 *   [total]     — the whole unread count, straight off Room. Correct
 *                 across process death because Room is where it lives,
 *                 and correct after a resync because `GET /chats` writes
 *                 the authoritative counts into those same rows. Nothing
 *                 draws it — Android has no icon to draw it on — but it
 *                 is the number a badge-capable launcher arrives at by
 *                 summing the per-chat ones, so it is the invariant they
 *                 have to add up to, and the one number a test can
 *                 assert without a launcher.
 *   the sweep   — a chat going from something unread to nothing takes
 *                 its tray entry with it. See UnreadBadge.nowRead for
 *                 why it is a transition and never the plain state.
 *   [onServerSaysRead]
 *               — the OTHER half, and the one the transition cannot
 *                 cover: a chat read on the user's OTHER device. This
 *                 process never watched a count go down — as far as it
 *                 knows nothing changed — so nothing transitions. What
 *                 changed is on the server, and `GET /chats` is where it
 *                 is learned (ChatRepository.refreshChats).
 *
 * A BADGE THAT NEVER CLEARS IS WORSE THAN NO BADGE, which is why the
 * clearing is a file of its own rather than a line at a call site. The
 * four ways this device can learn a chat has been read:
 *
 *   1. the user read it HERE — ChatViewModel's read reporter cancels the
 *      chat outright the moment it posts the marker (it also clears the
 *      count, so the sweep below fires as well; the direct call is what
 *      makes it immediate, and what covers a chat whose local count was
 *      already zero because this process never saw the pushed message);
 *   2. a `read` frame naming THIS user arrives from another of their
 *      devices — MessageRepository routes it to
 *      ChatRepository.applyMyReadMarker, which recounts and lands here
 *      through the sweep or through [onServerSaysRead];
 *   3. a resync finds the server reporting nothing unread —
 *      [onServerSaysRead], the trigger that actually fires for (2) on
 *      today's server (see ChatRepository.applyMyReadMarker);
 *   4. the chat itself goes away — a peer deleted their account, taking
 *      the direct chat with it (UnreadBadge.nowRead handles the vanished
 *      row).
 *
 * iOS counterpart: ios/FamilyConnect/Core/ChatNotifier.swift
 * (dismissDelivered) + UnreadBadge.swift.
 */

package me.nettrash.familyconnect.data.push

import android.content.Context
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.di.AppScope
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class UnreadNotifications @Inject constructor(
    /**
     * The application context, to reach the notification manager. It
     * outlives every screen, so there is nothing to leak — the same
     * exception ChatViewModel documents for `getString`.
     */
    @param:ApplicationContext private val appContext: Context,
    private val chatDao: ChatDao,
    @param:AppScope private val scope: CoroutineScope,
) {

    private val _total = MutableStateFlow(0)

    /** Every chat's unread, added up. See the file header. */
    val total: StateFlow<Int> = _total

    init {
        scope.launch {
            // The previous emission, held here rather than compared
            // against Room a second time: the question is what THIS
            // process watched change, and only a variable that has been
            // watching can answer it.
            var previous: Map<Long, Int> = emptyMap()
            chatDao.observeUnread().collect { rows ->
                val current = rows.associate { it.chatId to it.unreadCount }
                _total.value = UnreadBadge.total(rows.map { it.unreadCount })
                val swept = UnreadBadge.nowRead(previous, current)
                // BEFORE the cancel, not after: the cancel is a binder
                // round trip, and a second emission landing while it is
                // in flight would otherwise be compared against a
                // `previous` that still says these chats had unread and
                // sweep them a second time.
                previous = current
                PushNotifications.cancelChats(appContext, swept)
            }
        }
    }

    /**
     * The server says these chats have nothing unread, so whatever this
     * device is still showing about them is stale.
     *
     * The cross-device half. A read marker belongs to the PERSON, not the
     * device (docs/protocol.md, `GET /chats` → `last_read_message_id`):
     * reading a chat on a tablet leaves this phone with a notification
     * for a message that has been read, and no local count ever moved for
     * the sweep above to notice. The server's `unread_count` is the only
     * thing that knows, and zero from it is proof — the count is
     * "messages newer than MY read marker that I did not send", so zero
     * means the marker is past everything this chat pushed.
     *
     * Not a suspend function on purpose: the caller is mid-merge inside
     * `refreshChats`, and taking a stale notification down is not
     * something the merge should wait on or fail with.
     */
    fun onServerSaysRead(chatIds: Set<Long>) {
        PushNotifications.cancelChats(appContext, chatIds)
    }
}
