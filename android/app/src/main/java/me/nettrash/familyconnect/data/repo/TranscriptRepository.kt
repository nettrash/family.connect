/*
 * TranscriptRepository.kt
 * Family Connect (Android)
 *
 * The text of a recording, on request (docs/protocol.md, "Transcripts on
 * request"): asked of the server once, then kept on this device per
 * attachment so reopening the chat shows it without asking again.
 *
 * Nothing here is ever logged — not the text, not the language.
 */

package me.nettrash.familyconnect.data.repo

import kotlinx.coroutines.flow.Flow
import me.nettrash.familyconnect.data.db.TranscriptDao
import me.nettrash.familyconnect.data.db.TranscriptEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.TranscriptApi
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.TranscriptDto
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class TranscriptRepository @Inject constructor(
    private val api: TranscriptApi,
    private val dao: TranscriptDao,
    private val sound: TranscriptSoundSource,
    // Kotlin default for the tests that never end a session; Dagger ignores
    // it and injects the singleton SessionRepository advances.
    private val epoch: SessionEpoch = SessionEpoch(),
) {

    /** What this device holds for one attachment, live. */
    fun observe(attachmentId: Long): Flow<TranscriptEntity?> = dao.observe(attachmentId)

    /**
     * The text of one STORED recording.
     *
     * Held already → that, shown again, and no request: the device keeps
     * what it was given. Otherwise the server is asked with no body (its own
     * copy); an answer is written down, unfolded, before it is returned.
     */
    suspend fun fetchStored(chatId: Long, messageId: Long, attachmentId: Long): TranscriptOutcome {
        val startedAt = epoch.current()
        dao.find(attachmentId)?.let { held ->
            if (held.hidden) dao.setHidden(attachmentId, false)
            return TranscriptOutcome.Text(held.text, held.language)
        }
        return when (val result = api.transcribeStored(chatId, messageId, attachmentId)) {
            is ApiResult.Ok -> keep(startedAt, attachmentId, result.value.transcript, TranscriptEntity.SOURCE_STORED)
            else -> TranscriptOutcome.ofFailure(result)
        }
    }

    /**
     * The text of a recording the server will NOT send from its own copy —
     * a video, an Ogg file, a type outside the provider's list, one over
     * [maxBytes] — from sound this device takes out of the file
     * ([TranscriptSoundSource]).
     *
     * Held already → that, and nothing is sent. Otherwise the server is
     * asked FIRST with no body: it runs every check it has — the asker's
     * consent, the owner's switch, the sender's consent — before it looks at
     * the stored type, so `not_transcribable` means "allowed, send the
     * sound", and any other refusal arrives before this device downloads a
     * video and spends its battery on the sound of something it may not ask
     * about. A 200 there is an answer the server kept, or a stored copy that
     * qualifies after all (a ceiling this device knew lower): kept as stored.
     *
     * An answer from supplied sound is kept on THIS device only, marked
     * [TranscriptEntity.SOURCE_SUPPLIED]; the server keeps none.
     *
     * A recording whose stated length could not fit [maxBytes] even at
     * 64 kbit/s is [TranscriptOutcome.TooLong] before anything is
     * downloaded ([TranscriptSoundPlan.knownTooLong]).
     */
    suspend fun fetchSupplied(
        chatId: Long,
        messageId: Long,
        attachment: AttachmentDto,
        maxBytes: Long,
    ): TranscriptOutcome {
        val attachmentId = attachment.id
        val startedAt = epoch.current()
        dao.find(attachmentId)?.let { held ->
            if (held.hidden) dao.setHidden(attachmentId, false)
            return TranscriptOutcome.Text(held.text, held.language)
        }
        when (val first = api.transcribeStored(chatId, messageId, attachmentId)) {
            is ApiResult.Ok -> return keep(startedAt, attachmentId, first.value.transcript, TranscriptEntity.SOURCE_STORED)
            is ApiResult.HttpError ->
                if (first.code != TranscriptOutcome.NOT_TRANSCRIBABLE) return TranscriptOutcome.ofFailure(first)
            is ApiResult.NetworkError -> return TranscriptOutcome.Failed
        }
        if (TranscriptSoundPlan.knownTooLong(attachment.durationMs?.toLong(), maxBytes)) {
            return TranscriptOutcome.TooLong
        }
        return when (val made = sound.soundFor(attachment, maxBytes)) {
            TranscriptSoundPlan.Result.NotFetched -> TranscriptOutcome.Failed
            TranscriptSoundPlan.Result.TooLong -> TranscriptOutcome.TooLong
            TranscriptSoundPlan.Result.Unreadable -> TranscriptOutcome.Unreadable
            is TranscriptSoundPlan.Result.Ready -> try {
                when (val result = api.transcribeSupplied(chatId, messageId, attachmentId, made.file)) {
                    is ApiResult.Ok -> keep(startedAt, attachmentId, result.value.transcript, TranscriptEntity.SOURCE_SUPPLIED)
                    else -> TranscriptOutcome.ofFailure(result)
                }
            } finally {
                // Made for this one request; the answer is what is kept.
                // One unlink, so not worth a thread hop a cancellation could skip.
                made.file.delete()
            }
        }
    }

    /**
     * An answer written down, unfolded, then returned — only while the
     * session that asked ([startedAt]) is still this device's. One that
     * comes back after a sign-out is [TranscriptOutcome.Dropped]: written
     * after the wipe it would be the next account's to read (docs/protocol.md,
     * "An answer belongs to the account that asked"), and the line draws
     * from what is written, so not writing it is also not drawing it.
     */
    private suspend fun keep(
        startedAt: Long,
        attachmentId: Long,
        answer: TranscriptDto,
        source: String,
    ): TranscriptOutcome = epoch.whileCurrent(startedAt) {
        dao.upsert(
            TranscriptEntity(
                attachmentId = attachmentId,
                text = answer.text,
                language = answer.language,
                source = source,
                hidden = false,
            ),
        )
        TranscriptOutcome.Text(answer.text, answer.language)
    } ?: TranscriptOutcome.Dropped

    /** "Hide text" / "Show text" on a text already held: folds it, keeps it. */
    suspend fun setHidden(attachmentId: Long, hidden: Boolean) = dao.setHidden(attachmentId, hidden)
}
