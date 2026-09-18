//
//  SafetyRules.kt
//  FamilyConnect
//
//  WHICH SAFETY ROW A BUBBLE GETS, as arithmetic — kept free of Compose and
//  Android, like the rest of this package, so a plain JUnit test can pin it.
//
//  The two reports are EXCLUSIVE, and the exclusion is the protocol's rather
//  than a tidiness: a member report names somebody in your family and the
//  OWNER reads it, while an assistant report names a reply from an account
//  that belongs to no family at all and the OPERATOR reads it (docs/protocol.md,
//  "Reporting a member" and "Reporting the assistant"). Each endpoint refuses
//  the other's subject — `not_same_family` one way, `message_not_found` the
//  other — so a menu offering the wrong row fails visibly on the one screen
//  that must not: a safety screen whose whole design is that refusals look
//  innocent.
//
//  iOS and macOS decide the same two questions in their own `SafetyRules`,
//  and the three apps have to agree: a member who can report an `@ai` answer
//  on a phone and cannot on a tablet has been told something about the
//  product that is not true.
//

package me.nettrash.familyconnect.ui.chat

object SafetyRules {

    /**
     * A member's message: acked, not the reader's own, and not the
     * assistant's. Blocking hangs off the same question, which is why this
     * is the gate for the whole Safety page on a member's bubble.
     */
    fun canModerateMember(
        senderId: Long,
        myUserId: Long?,
        assistantUserId: Long?,
        isAiChat: Boolean,
        hasServerId: Boolean,
    ): Boolean =
        hasServerId &&
            myUserId != null &&
            senderId != myUserId &&
            !isAssistantSender(senderId, myUserId, assistantUserId, isAiChat)

    /**
     * An assistant reply: acked, and from the assistant rather than from the
     * reader. There is no Block half — the assistant is not a member, and
     * whether it speaks at all is the owner's `ai_greeting` and the
     * operator's `[ai]` switch.
     */
    fun canReportAssistant(
        senderId: Long,
        myUserId: Long?,
        assistantUserId: Long?,
        isAiChat: Boolean,
        hasServerId: Boolean,
    ): Boolean =
        hasServerId &&
            myUserId != null &&
            senderId != myUserId &&
            isAssistantSender(senderId, myUserId, assistantUserId, isAiChat)

    /**
     * The assistant, by its account or by the chat it is speaking in. Both,
     * because a client that has not read the assistant's id yet still knows
     * whose chat this is — and in an `ai` chat anybody who is not the reader
     * is the assistant.
     */
    private fun isAssistantSender(
        senderId: Long,
        myUserId: Long?,
        assistantUserId: Long?,
        isAiChat: Boolean,
    ): Boolean =
        senderId != myUserId && (isAiChat || (assistantUserId != null && senderId == assistantUserId))
}
