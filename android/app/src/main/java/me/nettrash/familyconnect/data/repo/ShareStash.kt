/*
 * ShareStash.kt
 * Family Connect (Android)
 *
 * The hand-off between the OS share flow and the one chat composer that
 * may take it: MainViewModel deposits what it prepared, the chat picker
 * names the target, and the target chat's ChatViewModel drains it on
 * open — mirroring iOS's pendingShareImport.
 *
 * A singleton holding FILES, so discipline matters: a deposit that is
 * never claimed is replaced (and its cache files deleted) by the next
 * share, and cancelling the picker discards it outright. Claiming is
 * target-checked — a chat opened by any other route finds nothing here.
 */

package me.nettrash.familyconnect.data.repo

import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class ShareStash @Inject constructor() {

    /** What a share left for one composer: staged-ready items, and/or words. */
    data class Claim(val items: List<MediaPrep.Prepared>, val text: String?)

    private var pending: Claim? = null
    private var targetChatId: Long? = null

    /** A new share replaces an unclaimed older one — files and all. */
    @Synchronized
    fun deposit(items: List<MediaPrep.Prepared>, text: String?) {
        discardLocked()
        pending = Claim(items = items, text = text)
    }

    /** The picker chose; only this chat's composer may claim now. */
    @Synchronized
    fun target(chatId: Long) {
        targetChatId = chatId
    }

    /**
     * The chosen chat's composer collects — exactly once. Any other chat
     * (or the same one opened again later) finds nothing.
     */
    @Synchronized
    fun claim(chatId: Long): Claim? {
        if (targetChatId != chatId) return null
        val claimed = pending
        pending = null
        targetChatId = null
        return claimed
    }

    /** The share was abandoned; the cache files go with it. */
    @Synchronized
    fun discard() {
        discardLocked()
    }

    private fun discardLocked() {
        pending?.items?.forEach { it.file.delete() }
        pending = null
        targetChatId = null
    }
}
