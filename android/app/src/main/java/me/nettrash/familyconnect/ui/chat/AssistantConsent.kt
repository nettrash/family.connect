//
//  AssistantConsent.kt
//  FamilyConnect
//
//  Nothing a member writes reaches the model before that member has said
//  yes (docs/protocol.md, "Consenting to the assistant").
//
//  Arithmetic, kept free of Compose and Android like the rest of this
//  package, so a plain JUnit test can pin it. Two questions: which drafts
//  have to be asked about, and which must not be sent at all. The first is
//  the MIRROR of `model_surface` in `server/src/handlers_chat.rs` and of
//  `AssistantConsent.swift` — a disagreement is either a message refused
//  after it was typed or one sent having asked nothing.
//

package me.nettrash.familyconnect.ui.chat

object AssistantConsent {

    /**
     * Would a message with this body, in this chat, be sent to the model?
     *
     * The member's own `ai` chat, where everything goes, and the family
     * chat, where only an `@ai` does. `/draw` needs no case of its own:
     * in the family chat it is `@ai /draw`, a mention, and in the
     * assistant's own chat it is that chat.
     */
    fun reachesTheModel(chatKind: String?, body: String): Boolean =
        when (chatKind) {
            "ai" -> true
            "family" -> AssistantMention.mentions(body)
            else -> false
        }

    /**
     * Is there an assistant this client may offer at all?
     *
     * A server that names no processor has one this client must not use:
     * the disclosure would have a hole exactly where the person needs to
     * read, and "some third party" is not something anybody can weigh.
     */
    fun isAvailable(processor: String?): Boolean = !processor.isNullOrBlank()

    /** Must this member be asked before this message is sent? */
    fun isRequired(
        chatKind: String?,
        body: String,
        processor: String?,
        agreedAt: String?,
    ): Boolean =
        isAvailable(processor) && agreedAt.isNullOrBlank() && reachesTheModel(chatKind, body)

    /**
     * Would this message reach a model whose owner the server will not
     * name, so this client must hold it back entirely?
     *
     * `hasAssistant` is what keeps this from swallowing ordinary words: on
     * a server with no assistant, `@ai` in the family chat is three
     * characters that reach nobody, and refusing to send them would break
     * a conversation to protect nothing.
     */
    fun isWithheldFromAnUnnamedAssistant(
        chatKind: String?,
        body: String,
        hasAssistant: Boolean,
        processor: String?,
    ): Boolean =
        hasAssistant && !isAvailable(processor) && reachesTheModel(chatKind, body)
}
