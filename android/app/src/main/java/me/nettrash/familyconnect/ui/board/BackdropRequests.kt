/*
 * BackdropRequests.kt
 * Family Connect (Android)
 *
 * Asking the assistant for an event's backdrop, with the consent question
 * in front of it (docs/protocol.md, "Board" and "Consenting to the
 * assistant", amended 2026-09-30).
 *
 * The title is the author's own words going to the model, so the backdrop
 * follows the chat's rule for a `/draw`: an author who has not agreed is
 * asked FIRST and nothing is sent; and an author the server says has not
 * agreed (`assistant_consent_required`, 403 — a stamp withdrawn on another
 * device, say) is asked on that answer. Agreeing finishes the backdrop the
 * author already asked for, as agreeing in the chat finishes the send.
 *
 * Kept free of Compose and of the view model's other dependencies so a
 * plain test can drive it: the three things it needs are handed in.
 */

package me.nettrash.familyconnect.ui.board

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.repo.BackdropOutcome
import me.nettrash.familyconnect.ui.chat.AssistantConsent

class BackdropRequests(
    private val scope: CoroutineScope,
    /** Read fresh at every ask, so a consent given elsewhere counts at once. */
    private val gate: suspend () -> AssistantConsent.BackdropGate,
    /** POST …/backdrop, read. */
    private val draw: suspend (noteId: Long) -> BackdropOutcome,
    /** POST /me/assistant-consent `{granted: true}`; whether it was recorded. */
    private val agree: suspend () -> Boolean,
) {
    /** A backdrop held while the author is asked. */
    private class Waiting(val noteId: Long, val onSettled: (BackdropOutcome) -> Unit)

    private val waiting = MutableStateFlow<Waiting?>(null)

    /** Whether the consent screen should be up: true while a backdrop waits on the answer. */
    val asking: Flow<Boolean> = waiting.map { it != null }

    /**
     * Ask for a backdrop. [onSettled] hears exactly once what it came to —
     * never [BackdropOutcome.ConsentRequired], which is answered with the
     * question rather than handed back.
     */
    fun request(noteId: Long, onSettled: (BackdropOutcome) -> Unit) {
        scope.launch {
            when (gate()) {
                // Not offered, so not normally reachable: settled rather
                // than left hanging on a button that says "Drawing…".
                AssistantConsent.BackdropGate.WITHHELD -> onSettled(BackdropOutcome.Failed)
                AssistantConsent.BackdropGate.ASK -> hold(Waiting(noteId, onSettled))
                AssistantConsent.BackdropGate.DRAW -> drawNow(Waiting(noteId, onSettled))
            }
        }
    }

    /** "I Agree": record it, then draw the backdrop that was waiting. */
    fun agreed() {
        // Taken before anything suspends, so a second tap finds nothing.
        val held = waiting.value ?: return
        waiting.value = null
        scope.launch {
            if (agree()) {
                drawNow(held)
            } else {
                // The consent was not recorded, so nothing was drawn; the
                // author tapped Draw and is told it did not.
                held.onSettled(BackdropOutcome.Failed)
            }
        }
    }

    /** "Not Now": nothing was sent, and nothing needs saying. */
    fun dismissed() {
        val held = waiting.value ?: return
        waiting.value = null
        held.onSettled(BackdropOutcome.Declined)
    }

    private suspend fun drawNow(request: Waiting) {
        when (val outcome = draw(request.noteId)) {
            BackdropOutcome.ConsentRequired ->
                // The server's answer outranks this device's stamp: ask,
                // and draw again on a yes. Unless the assistant cannot be
                // named, in which case there is no screen to show.
                if (gate() == AssistantConsent.BackdropGate.WITHHELD) {
                    request.onSettled(BackdropOutcome.Failed)
                } else {
                    hold(request)
                }
            else -> request.onSettled(outcome)
        }
    }

    private fun hold(request: Waiting) {
        // One question at a time: a backdrop already waiting on it is
        // settled as not asked for, so its button does not hang.
        waiting.value?.onSettled(BackdropOutcome.Declined)
        waiting.value = request
    }
}
