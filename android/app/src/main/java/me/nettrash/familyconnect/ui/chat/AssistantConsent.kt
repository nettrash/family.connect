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

    /** What becomes of a sticker tapped in a composer. */
    enum class StickerGate {
        /** It goes. */
        SEND,

        /** It would reach the model and this member has not agreed: ask first, send nothing. */
        ASK,

        /** It would reach a model whose owner the server will not name: it goes nowhere. */
        WITHHELD,
    }

    /**
     * A sticker is a send like any other, and in the member's own `ai` chat
     * it is a photo to the assistant — so it goes through the SAME two
     * questions a typed message does, never around them. One function for
     * every composer that has a sticker button (the chat's and the
     * thread's), so neither can grow a way past the question.
     *
     * A sticker has no body, so in the family chat it can never say `@ai`
     * and always goes.
     */
    fun stickerGate(
        chatKind: String?,
        hasAssistant: Boolean,
        processor: String?,
        agreedAt: String?,
    ): StickerGate = when {
        isWithheldFromAnUnnamedAssistant(chatKind, "", hasAssistant, processor) -> StickerGate.WITHHELD
        isRequired(chatKind, "", processor, agreedAt) -> StickerGate.ASK
        else -> StickerGate.SEND
    }

    /** What becomes of "Draw a backdrop" on an event. */
    enum class BackdropGate {
        /** It is asked for. */
        DRAW,

        /** The title would reach the model and this author has not agreed: ask first, send nothing. */
        ASK,

        /** The server will not name who would receive it: it goes nowhere, and is not offered. */
        WITHHELD,
    }

    /**
     * An event's backdrop is drawn from its TITLE — words the author wrote,
     * going to `processor` — so it is asked about exactly as a `/draw` is
     * (docs/protocol.md, "Consenting to the assistant", amended
     * 2026-09-30). Always reaches the model: there is no chat kind or
     * mention to decide it, only whether the assistant can be named and
     * whether this member has agreed.
     */
    fun backdropGate(processor: String?, agreedAt: String?): BackdropGate = when {
        !isAvailable(processor) -> BackdropGate.WITHHELD
        agreedAt.isNullOrBlank() -> BackdropGate.ASK
        else -> BackdropGate.DRAW
    }

    /**
     * Is "Draw a backdrop" offered at all? The server must be able to draw
     * (`assistant.images`), and it must name who draws: a client that
     * cannot name `processor` does not offer the assistant at all, and
     * could not ask the consent a backdrop needs.
     */
    fun offersBackdrop(serverCanDraw: Boolean, processor: String?): Boolean =
        serverCanDraw && isAvailable(processor)

    /** What becomes of "Show text" under a recording. */
    enum class TranscriptGate {
        /** It is asked for. */
        ASK_FOR_TEXT,

        /** The sound would go to `processor` and this member has not agreed: ask first, send nothing. */
        ASK_CONSENT,

        /** The server will not name who would receive it: it goes nowhere, and is not offered. */
        WITHHELD,
    }

    /**
     * The member who asks for the text of a recording is the member sending
     * its sound to `processor`, so their OWN consent is required — the same
     * consent a `/draw` needs (docs/protocol.md, "Transcripts on request").
     * Asked before the request when this device knows the answer is no,
     * and again on the server's `assistant_consent_required`.
     */
    fun transcriptGate(processor: String?, agreedAt: String?): TranscriptGate = when {
        !isAvailable(processor) -> TranscriptGate.WITHHELD
        agreedAt.isNullOrBlank() -> TranscriptGate.ASK_CONSENT
        else -> TranscriptGate.ASK_FOR_TEXT
    }

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
