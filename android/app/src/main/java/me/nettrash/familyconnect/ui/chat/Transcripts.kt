/*
 * Transcripts.kt
 * Family Connect (Android)
 *
 * "Show text" under a recording, as one object a screen's view model owns
 * and its bubbles reach through [LocalTranscripts] (docs/protocol.md,
 * "Transcripts on request").
 *
 * Owned by the view model rather than the repository because the consent
 * question it may raise is a screen's: the chat and the thread each draw
 * their own consent dialog from [consentAsk]. Reached through a
 * CompositionLocal, like LocalAttachments, because a bubble deep inside a
 * LazyColumn should not have to be handed it by every composable above it.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.runtime.staticCompositionLocalOf
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.TranscriptEntity
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.repo.TranscriptOutcome
import me.nettrash.familyconnect.data.repo.TranscriptRepository
import me.nettrash.familyconnect.data.settings.SettingsRepository

/** The screen's [Transcripts], or null where none is offered (previews, the viewer). */
val LocalTranscripts = staticCompositionLocalOf<Transcripts?> { null }

class Transcripts(
    private val scope: CoroutineScope,
    private val settings: SettingsRepository,
    private val repository: TranscriptRepository,
    /** POST /me/assistant-consent `{granted: true}`; whether it was recorded. */
    agree: suspend () -> Boolean,
) {
    /** Everything [TranscriptRules.offersShowText] needs that is not on the message. */
    data class Context(
        val serverTranscribes: Boolean = false,
        val maxBytes: Long = 0L,
        val processor: String? = null,
        val myUserId: Long? = null,
        val assistantUserId: Long? = null,
        val familyAllowsOthers: Boolean = false,
    ) {
        fun offers(
            chatKind: String?,
            messageServerId: Long?,
            senderId: Long,
            attachment: AttachmentDto,
        ): Boolean = TranscriptRules.offersShowText(
            serverTranscribes = serverTranscribes,
            maxBytes = maxBytes,
            processor = processor,
            chatKind = chatKind,
            messageServerId = messageServerId,
            senderId = senderId,
            myUserId = myUserId,
            assistantUserId = assistantUserId,
            familyAllowsOthers = familyAllowsOthers,
            attachment = attachment,
        )
    }

    val context: StateFlow<Context> = settings.state
        .map {
            Context(
                serverTranscribes = it.assistantTranscribe,
                maxBytes = it.assistantTranscribeMaxBytes,
                processor = it.assistantProcessor,
                myUserId = it.myUserId,
                assistantUserId = it.assistantUserId,
                familyAllowsOthers = it.familyAiTranscripts,
            )
        }
        .stateIn(scope, SharingStarted.Eagerly, Context())

    private val requests = TranscriptRequests(
        scope = scope,
        gate = {
            val settingsState = settings.state.first()
            AssistantConsent.transcriptGate(
                processor = settingsState.assistantProcessor,
                agreedAt = settingsState.assistantConsentAt,
            )
        },
        fetch = { ask -> fetch(ask) },
        agree = agree,
    )

    /** What each attachment's line says while there is no text to draw. */
    val status: StateFlow<Map<Long, TranscriptRequests.Status>> = requests.status

    /** The text this device holds for one attachment, live. */
    fun saved(attachmentId: Long): Flow<TranscriptEntity?> = repository.observe(attachmentId)

    /** "Show text" with nothing held: ask (the consent question first, if it is owed). */
    fun request(chatId: Long, messageId: Long, attachment: AttachmentDto) =
        requests.request(TranscriptRequests.Ask(chatId, messageId, attachment.id, attachment))

    /**
     * The server's stored copy, or sound this device takes out of the file
     * — a video, an Ogg file, a recording over the ceiling — by
     * [TranscriptRules.route], against the ceiling the server states now.
     */
    private suspend fun fetch(ask: TranscriptRequests.Ask): TranscriptOutcome {
        val attachment = ask.attachment
            ?: return repository.fetchStored(ask.chatId, ask.messageId, ask.attachmentId)
        val maxBytes = settings.state.first().assistantTranscribeMaxBytes
        return when (TranscriptRules.route(attachment.kind, attachment.mime, attachment.size, maxBytes)) {
            TranscriptRules.Route.STORED ->
                repository.fetchStored(ask.chatId, ask.messageId, ask.attachmentId)
            TranscriptRules.Route.SUPPLIED ->
                repository.fetchSupplied(ask.chatId, ask.messageId, attachment, maxBytes)
        }
    }

    /** "Show text" over text already held: unfold it. Nothing is sent. */
    fun reveal(attachmentId: Long) {
        scope.launch { repository.setHidden(attachmentId, false) }
    }

    /** "Hide text": fold it away. The device keeps the text it was given. */
    fun hide(attachmentId: Long) {
        scope.launch { repository.setHidden(attachmentId, true) }
    }

    /** What the consent screen must say, or null while it is not up. */
    val consentAsk: StateFlow<ChatViewModel.AssistantConsentAsk?> =
        combine(requests.asking, settings.state) { asking, settingsState ->
            val processor = settingsState.assistantProcessor
            if (!asking || processor.isNullOrBlank()) {
                null
            } else {
                ChatViewModel.AssistantConsentAsk(
                    processor = processor,
                    familyHistory = settingsState.familyAiHistory,
                    familyVision = settingsState.familyAiVision,
                    transcripts = settingsState.assistantTranscribe,
                )
            }
        }.stateIn(scope, SharingStarted.Eagerly, null)

    /** "I Agree" — the stamp is the server's — then the text that was waiting. */
    fun agreed() = requests.agreed()

    /** "Not Now": the screen closes and nothing was sent. */
    fun dismissed() = requests.dismissed()
}
