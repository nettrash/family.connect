/*
 * TranscriptLine.kt
 * Family Connect (Android)
 *
 * The line under a recording's player — a voice note, an audio file or a
 * video: "Show text", then the text itself —
 * selectable, with "Hide text" — or why there is none (docs/protocol.md,
 * "Transcripts on request").
 *
 * Draws nothing at all where the recording is not offered and nothing is
 * held, so a bubble on a server without transcripts looks as it always did.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.net.dto.AttachmentDto

@Composable
fun TranscriptLine(
    attachment: AttachmentDto,
    chatKind: String?,
    chatId: Long,
    messageServerId: Long?,
    senderId: Long,
    modifier: Modifier = Modifier,
    /**
     * Which video of an album pile this is the line of, counted among the
     * pile's videos — drawn as "Video 2" above the line, and only when the
     * line draws anything. Null for a lone recording.
     */
    videoNumber: Int? = null,
) {
    val transcripts = LocalTranscripts.current ?: return
    val context by transcripts.context.collectAsStateWithLifecycle()
    val saved by remember(attachment.id) { transcripts.saved(attachment.id) }
        .collectAsStateWithLifecycle(initialValue = null)
    val statuses by transcripts.status.collectAsStateWithLifecycle()
    val status = statuses[attachment.id]
    val offered = context.offers(chatKind, messageServerId, senderId, attachment)
    val held = saved

    // Nothing held, nothing to offer, nothing to say: the bubble is as it was.
    if (held == null && status == null && !offered) return

    val ink = LocalContentColor.current
    val quiet = ink.copy(alpha = 0.75f)
    val buttonColors = ButtonDefaults.textButtonColors(contentColor = ink)
    val buttonPadding = PaddingValues(horizontal = 4.dp, vertical = 0.dp)

    Column(modifier = modifier.padding(horizontal = 2.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        if (videoNumber != null) {
            Text(
                text = "${stringResource(R.string.s_video)} $videoNumber",
                style = MaterialTheme.typography.labelSmall,
                color = quiet,
            )
        }
        when {
            held != null && !held.hidden && status == null -> {
                val label = stringResource(R.string.s_transcript_a11y)
                // Labelled for a screen reader, so the words are not read
                // out as if they were the message itself.
                Column(modifier = Modifier.semantics { contentDescription = label }) {
                    if (held.text.isEmpty()) {
                        // Nothing was said: an answer, not an error.
                        Text(
                            text = stringResource(R.string.s_transcript_no_speech),
                            style = MaterialTheme.typography.bodyMedium,
                            fontStyle = FontStyle.Italic,
                            color = quiet,
                        )
                    } else {
                        SelectionContainer {
                            Text(
                                text = held.text,
                                style = MaterialTheme.typography.bodyMedium,
                                color = ink,
                            )
                        }
                    }
                }
                TextButton(
                    onClick = { transcripts.hide(attachment.id) },
                    colors = buttonColors,
                    contentPadding = buttonPadding,
                ) {
                    Text(stringResource(R.string.s_transcript_hide))
                }
            }
            status == TranscriptRequests.Status.LOADING -> Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                modifier = Modifier.padding(vertical = 6.dp),
            ) {
                CircularProgressIndicator(
                    modifier = Modifier.size(14.dp),
                    strokeWidth = 2.dp,
                    color = ink,
                )
                Text(
                    text = stringResource(R.string.s_transcript_getting),
                    style = MaterialTheme.typography.bodySmall,
                    color = quiet,
                )
            }
            status == TranscriptRequests.Status.REFUSED -> Text(
                text = stringResource(R.string.s_transcript_refused),
                style = MaterialTheme.typography.bodySmall,
                fontStyle = FontStyle.Italic,
                color = quiet,
                modifier = Modifier.padding(vertical = 6.dp),
            )
            status == TranscriptRequests.Status.UNAVAILABLE ||
                status == TranscriptRequests.Status.TOO_LONG ||
                status == TranscriptRequests.Status.UNREADABLE -> Text(
                text = stringResource(
                    when (status) {
                        // What THIS DEVICE could not do with the file, in the
                        // words every client uses — never the refusal's.
                        TranscriptRequests.Status.TOO_LONG -> R.string.s_transcript_too_long
                        TranscriptRequests.Status.UNREADABLE -> R.string.s_transcript_unreadable
                        else -> R.string.s_transcript_unavailable
                    },
                ),
                style = MaterialTheme.typography.bodySmall,
                fontStyle = FontStyle.Italic,
                color = quiet,
                modifier = Modifier.padding(vertical = 6.dp),
            )
            else -> {
                if (status == TranscriptRequests.Status.FAILED) {
                    Text(
                        text = stringResource(R.string.s_transcript_failed),
                        style = MaterialTheme.typography.bodySmall,
                        fontStyle = FontStyle.Italic,
                        color = quiet,
                    )
                }
                // "Show text": over text already held it only unfolds it;
                // otherwise it asks — and after a failure, asks again.
                if (held != null || offered) {
                    TextButton(
                        onClick = {
                            if (held != null) {
                                transcripts.reveal(attachment.id)
                            } else if (messageServerId != null) {
                                transcripts.request(chatId, messageServerId, attachment)
                            }
                        },
                        colors = buttonColors,
                        contentPadding = buttonPadding,
                    ) {
                        Text(stringResource(R.string.s_transcript_show))
                    }
                }
            }
        }
    }
}
