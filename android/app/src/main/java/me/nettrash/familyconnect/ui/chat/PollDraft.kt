/*
 * PollDraft.kt
 * Family Connect (Android)
 *
 * The poll composer's state, as a pure value — no Compose, no Android,
 * so ChatViewModelTest can pin every rule directly.
 *
 * The rules are the SERVER's, mirrored (docs/protocol.md, "Polls" and
 * "Limits"): 2–10 options, each trimmed, non-empty, at most 100
 * characters, no two the same ignoring case, and a question that may not
 * be empty — a poll's body, unlike a message carrying an attachment, is
 * never allowed to be blank. Mirrored rather than trusted to the round
 * trip because a form that only learns it was wrong from a 400 has
 * already thrown the words away.
 *
 * EMPTY option boxes are dropped rather than refused: the form always
 * shows at least two, and somebody who adds a third and leaves it blank
 * meant a two-option poll, not an error.
 *
 * iOS counterpart: the poll sheet's draft in
 * ios/FamilyConnect/Views/ConversationView.swift.
 */

package me.nettrash.familyconnect.ui.chat

import me.nettrash.familyconnect.data.repo.MessageRepository

data class PollDraft(
    val question: String = "",
    /** Always at least [MIN_OPTIONS] boxes on screen, blank or not. */
    val options: List<String> = List(MIN_OPTIONS) { "" },
) {

    /** What would actually be sent: trimmed, with the blanks dropped. */
    val sendableOptions: List<String>
        get() = options.map { it.trim() }.filter { it.isNotEmpty() }

    /** Another box may be added — the protocol caps a poll at ten. */
    val canAddOption: Boolean get() = options.size < MAX_OPTIONS

    /** A box may be taken away — never below the two that make a question. */
    val canRemoveOption: Boolean get() = options.size > MIN_OPTIONS

    /**
     * Whether Send may be pressed. Every rule the server would apply,
     * asked of what would actually be sent.
     */
    val isValid: Boolean
        get() {
            if (question.trim().isEmpty()) return false
            val sendable = sendableOptions
            if (sendable.size < MIN_OPTIONS || sendable.size > MAX_OPTIONS) return false
            if (sendable.any { it.length > MAX_OPTION_CHARS }) return false
            // "No two the same ignoring case" — the server's words. A
            // poll cannot rename an option afterwards, so two identical
            // ones would be unfixable as well as unanswerable.
            return sendable.map { it.lowercase() }.toSet().size == sendable.size
        }

    fun withQuestion(text: String): PollDraft = copy(question = text)

    fun withOption(index: Int, text: String): PollDraft {
        if (index !in options.indices) return this
        return copy(options = options.toMutableList().also { it[index] = text })
    }

    fun plusOption(): PollDraft =
        if (canAddOption) copy(options = options + "") else this

    fun minusOption(index: Int): PollDraft {
        if (!canRemoveOption || index !in options.indices) return this
        return copy(options = options.toMutableList().also { it.removeAt(index) })
    }

    companion object {
        const val MIN_OPTIONS = MessageRepository.MIN_POLL_OPTIONS
        const val MAX_OPTIONS = MessageRepository.MAX_POLL_OPTIONS
        const val MAX_OPTION_CHARS = MessageRepository.MAX_POLL_OPTION_CHARS
    }
}
