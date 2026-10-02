/*
 * TranscriptApi.kt
 * Family Connect (Android)
 *
 * The one transcript call of docs/protocol.md, "Transcripts on request":
 *
 *   POST /chats/{chat_id}/messages/{message_id}/attachments/{attachment_id}/transcript
 *
 * Two shapes:
 *  - NO body asks the server to send its own STORED copy of the recording;
 *    the answer is kept on the server and handed to later askers who pass
 *    the checks.
 *  - `multipart/form-data` with one part named `audio` — an AAC M4A this
 *    device took out of a video, or out of a recording the server will not
 *    send itself (TranscriptSound). That answer is returned and never kept
 *    or shared by the server; this device keeps it.
 *
 * Interface + impl split so the repository's tests can script the server
 * without an HTTP stack.
 */

package me.nettrash.familyconnect.data.net

import me.nettrash.familyconnect.data.net.dto.TranscriptResponse
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.MultipartBody
import okhttp3.RequestBody.Companion.asRequestBody
import java.io.File
import javax.inject.Inject
import javax.inject.Singleton

interface TranscriptApi {
    /** Ask for the text of one stored recording. Slow: its own timeout. */
    suspend fun transcribeStored(
        chatId: Long,
        messageId: Long,
        attachmentId: Long,
    ): ApiResult<TranscriptResponse>

    /** Ask for the text of [sound], an AAC M4A this device made. Slow: its own timeout. */
    suspend fun transcribeSupplied(
        chatId: Long,
        messageId: Long,
        attachmentId: Long,
        sound: File,
    ): ApiResult<TranscriptResponse>
}

/**
 * The supplied-sound body: ONE part named `audio`, typed `audio/mp4` (AAC in
 * MPEG-4, the only thing the server takes there), streamed from [sound] —
 * never read into memory. The file name is fixed: it says what the part is,
 * and nothing about the attachment.
 */
internal fun transcriptSoundBody(sound: File): MultipartBody =
    MultipartBody.Builder()
        .setType(MultipartBody.FORM)
        .addFormDataPart(
            TRANSCRIPT_SOUND_PART,
            TRANSCRIPT_SOUND_FILENAME,
            sound.asRequestBody(TRANSCRIPT_SOUND_TYPE.toMediaType()),
        )
        .build()

internal const val TRANSCRIPT_SOUND_PART = "audio"
internal const val TRANSCRIPT_SOUND_FILENAME = "sound.m4a"
internal const val TRANSCRIPT_SOUND_TYPE = "audio/mp4"

/** The request's path, as a value a test can hold. */
internal fun transcriptPath(chatId: Long, messageId: Long, attachmentId: Long): String =
    "/chats/$chatId/messages/$messageId/attachments/$attachmentId/transcript"

@Singleton
class DefaultTranscriptApi @Inject constructor(
    private val client: ApiClient,
) : TranscriptApi {

    // An EMPTY body with no Content-Type: anything that is not
    // multipart/form-data is the stored-copy shape. SLOW, so never the
    // shared client's 20 s — see ApiClient.TRANSCRIPT_TIMEOUT.
    override suspend fun transcribeStored(
        chatId: Long,
        messageId: Long,
        attachmentId: Long,
    ): ApiResult<TranscriptResponse> =
        client.postEmpty(
            transcriptPath(chatId, messageId, attachmentId),
            timeout = ApiClient.TRANSCRIPT_TIMEOUT,
        )

    // Uploading up to 25 MB and then waiting on the provider: the same
    // budget, which is per operation, so a slow uplink that keeps moving is
    // not cut off.
    override suspend fun transcribeSupplied(
        chatId: Long,
        messageId: Long,
        attachmentId: Long,
        sound: File,
    ): ApiResult<TranscriptResponse> =
        client.decode(
            client.rawBody(
                "POST",
                transcriptPath(chatId, messageId, attachmentId),
                transcriptSoundBody(sound),
                timeout = ApiClient.TRANSCRIPT_TIMEOUT,
            ),
        )
}
