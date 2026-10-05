/*
 * CoveredChats.kt
 * Family Connect (Android)
 *
 * The chats whose OWN thread or polls screen has come over them (#79,
 * docs/audio-video-messages-2026-10-04.md, S4 "Leaving the chat").
 *
 * Opening a chat's thread or its polls is not leaving the chat: S4 lists the
 * back button, another chat, the rail and a notification tap, and on the
 * iPhone both are sheets over a conversation that never goes away. Here they
 * are screens, and the chat's screen leaves composition under them — but its
 * ViewModel stays, and its composer comes back exactly as it was. So a voice
 * message in review stays in review then, with the photos, the words and the
 * reply it shares the composer with; only a RECORDING stops (ON_STOP), and is
 * kept as "not sent".
 *
 * What makes that safe is this list. A covered chat whose screen never comes
 * back must not keep a recording out of sight: on a tablet, picking another
 * chat in the list pane while a thread is open drops that pane's whole
 * NavHost and leaves the chat's ViewModel alive with nobody to show it. So
 * ANOTHER chat's screen coming on is the person leaving every chat still
 * covered — their notes in review become "not sent" (S2.8) — and a chat's
 * own screen coming back takes it off the list. A chat that is cleared
 * leaves as it always did (ChatViewModel.onCleared).
 *
 * Driven from the main thread only — screen callbacks and onCleared — and
 * synchronised anyway, so a stray caller cannot tear it.
 */

package me.nettrash.familyconnect.ui.chat

import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class CoveredChats @Inject constructor() {

    /** One chat behind its own thread or polls screen, and what leaving it does. */
    fun interface Covered {
        fun leave()
    }

    private val covered = LinkedHashSet<Covered>()

    /** [chat]'s screen went away under its own thread or polls screen. */
    @Synchronized
    fun cover(chat: Covered) {
        covered += chat
    }

    /** [chat] is back on screen, or gone for good: not covered any more. */
    @Synchronized
    fun uncover(chat: Covered) {
        covered -= chat
    }

    /**
     * [chat]'s screen came on. Every OTHER chat still covered has been left —
     * the person is somewhere else now — and is told so, outside the lock.
     */
    fun leaveAllBut(chat: Covered) {
        val left = synchronized(this) {
            covered.filter { it !== chat }.also { covered.removeAll(it.toSet()) }
        }
        left.forEach { it.leave() }
    }

    /** How many chats are covered now — for the tests. */
    @get:Synchronized
    internal val size: Int get() = covered.size
}
