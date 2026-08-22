/*
 * Attachments.kt
 * Family Connect (Android)
 *
 * The seam between the attachment cache and composition, and the two
 * pieces of chrome a photo or video bubble is made of.
 *
 * LocalAttachments carries the app-scoped AttachmentRepository down the
 * tree the same way LocalAvatars carries the avatar cache, and for the
 * same reason: a bubble deep inside a LazyColumn should not have to be
 * handed one by every ViewModel above it. Null by default so a @Preview
 * renders the placeholder instead of crashing.
 *
 * iOS counterpart: ios/FamilyConnect/Views/AttachmentView.swift
 */

package me.nettrash.familyconnect.ui.components

import android.content.Context
import android.text.format.Formatter
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.material.icons.filled.AudioFile
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.FolderZip
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.filled.InsertDriveFile
import androidx.compose.material.icons.filled.PictureAsPdf
import androidx.compose.material.icons.filled.VideoFile
import androidx.compose.material3.LocalContentColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.repo.AttachmentRepository

val LocalAttachments = staticCompositionLocalOf<AttachmentRepository?> { null }

/**
 * The bubble's image for this attachment, or null while it loads.
 *
 * Preview first, then the full photo: a message sent before its preview
 * upload landed still shows something rather than a permanent spinner.
 * A video has nothing to fall back to — its bytes are a video — so it
 * shows the play badge over the placeholder until the poster arrives.
 */
@Composable
fun rememberAttachmentImage(attachment: AttachmentDto, preview: Boolean): ImageBitmap? {
    val attachments = LocalAttachments.current ?: return null
    if (preview && !attachment.hasPreview) return null
    if (!preview && attachment.isVideo) return null
    // retryToken is in the key on purpose — see rememberAvatar.
    LaunchedEffect(attachment.id, preview, attachments.retryToken) {
        attachments.load(attachment.id, preview)
    }
    return attachments.cached(attachment.id, preview)
}

/**
 * A photo or video inside a bubble: the preview at the attachment's own
 * aspect ratio, with a play badge and duration on a video.
 *
 * The SHAPE is reserved before any bytes arrive — the attachment carries
 * its dimensions, so the placeholder is exactly the size the image will
 * be. Without that, every photo that finished loading would resize its
 * bubble and shove the whole thread.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun AttachmentBlock(
    attachment: AttachmentDto,
    onOpen: () -> Unit,
    modifier: Modifier = Modifier,
    /**
     * The bubble's own gestures, forwarded.
     *
     * A caption-less photo IS the balloon, and a child `clickable`
     * consumes the press — so long-pressing to react or double-tapping to
     * heart only worked on the few dp of padding at the edge. Same trap as
     * the link spans (see MessageLinks): on Compose the child wins.
     */
    onLongPress: () -> Unit = {},
    onDoubleTap: () -> Unit = {},
) {
    if (attachment.isFile) {
        FileRow(
            attachment = attachment,
            onOpen = onOpen,
            onLongPress = onLongPress,
            onDoubleTap = onDoubleTap,
            modifier = modifier,
        )
    } else {
        MediaThumbnail(
            attachment = attachment,
            onOpen = onOpen,
            onLongPress = onLongPress,
            onDoubleTap = onDoubleTap,
            modifier = modifier,
        )
    }
}

/**
 * A file has nothing to show but what it is: an icon, its name, its size.
 * Deliberately a ROW, not a tile — a document is a line item in a
 * conversation, and a 240dp square of grey would be a lie about how much
 * there is to look at.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun FileRow(
    attachment: AttachmentDto,
    onOpen: () -> Unit,
    onLongPress: () -> Unit,
    onDoubleTap: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    Row(
        modifier = modifier
            .widthIn(max = MAX_WIDTH.dp)
            .clip(RoundedCornerShape(10.dp))
            .background(Color.Black.copy(alpha = 0.06f))
            .combinedClickable(
                onClick = onOpen,
                onLongClick = onLongPress,
                onDoubleClick = onDoubleTap,
            )
            .padding(horizontal = 8.dp, vertical = 6.dp)
            .semantics {
                contentDescription =
                    "${attachment.displayName}, ${formatSize(context, attachment.size)}"
            },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(
            imageVector = iconFor(attachment.mime),
            contentDescription = null,
            tint = MaterialTheme.colorScheme.primary,
            modifier = Modifier.size(30.dp),
        )
        Column(modifier = Modifier.weight(1f, fill = false)) {
            Text(
                text = attachment.displayName,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Medium,
                maxLines = 2,
                overflow = TextOverflow.MiddleEllipsis,
            )
            Text(
                text = formatSize(context, attachment.size),
                style = MaterialTheme.typography.bodySmall,
                color = LocalContentColor.current.copy(alpha = 0.7f),
            )
        }
    }
}

/** Material icons only — a document icon set would be a pile of assets
 *  for something the platform already draws. */
private fun iconFor(mime: String): ImageVector = when {
    mime == "application/pdf" -> Icons.Filled.PictureAsPdf
    mime.startsWith("audio/") -> Icons.Filled.AudioFile
    mime.startsWith("video/") -> Icons.Filled.VideoFile
    mime.startsWith("image/") -> Icons.Filled.Image
    mime.startsWith("text/") -> Icons.Filled.Description
    mime in ARCHIVE_TYPES -> Icons.Filled.FolderZip
    else -> Icons.Filled.InsertDriveFile
}

private val ARCHIVE_TYPES = setOf(
    "application/zip",
    "application/x-tar",
    "application/gzip",
    "application/x-7z-compressed",
    "application/vnd.rar",
)

/** "1.2 MB", localised, the way the system formats every other size. */
fun formatSize(context: Context, bytes: Long): String =
    Formatter.formatShortFileSize(context, bytes)

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun MediaThumbnail(
    attachment: AttachmentDto,
    onOpen: () -> Unit,
    onLongPress: () -> Unit,
    onDoubleTap: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val image = rememberAttachmentImage(attachment, preview = true)
        ?: rememberAttachmentImage(attachment, preview = false)

    Box(
        modifier = modifier
            .widthIn(max = MAX_WIDTH.dp)
            .aspectRatio(attachment.aspectRatio.coerceIn(MIN_RATIO, MAX_RATIO))
            .clip(RoundedCornerShape(10.dp))
            .background(Color.Black.copy(alpha = 0.08f))
            .combinedClickable(
                onClick = onOpen,
                onLongClick = onLongPress,
                onDoubleClick = onDoubleTap,
            )
            .semantics {
                contentDescription = if (attachment.isVideo) "Video" else "Photo"
            },
        contentAlignment = Alignment.Center,
    ) {
        if (image != null) {
            Image(
                bitmap = image,
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier.fillMaxSize(),
            )
        }
        if (attachment.isVideo) {
            Box(
                modifier = Modifier
                    .size(44.dp)
                    .clip(CircleShape)
                    .background(Color.Black.copy(alpha = 0.45f)),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = Icons.Filled.PlayArrow,
                    contentDescription = null,
                    tint = Color.White,
                )
            }
            attachment.durationMs?.let { millis ->
                Text(
                    text = formatDuration(millis),
                    style = MaterialTheme.typography.labelSmall,
                    color = Color.White,
                    modifier = Modifier
                        .align(Alignment.BottomEnd)
                        .padding(6.dp)
                        .clip(RoundedCornerShape(4.dp))
                        .background(Color.Black.copy(alpha = 0.45f))
                        .padding(horizontal = 5.dp, vertical = 2.dp),
                )
            }
        }
    }
}

/** m:ss, as every video player shows it. */
fun formatDuration(millis: Int): String {
    val total = (millis / 1000).coerceAtLeast(0)
    val minutes = total / 60
    val seconds = total % 60
    return "%d:%02d".format(minutes, seconds)
}

private const val MAX_WIDTH = 240

/**
 * Shape limits for the reserved box. A panorama or a very tall crop would
 * otherwise be a sliver or a column taller than the screen.
 */
private const val MIN_RATIO = 0.6f
private const val MAX_RATIO = 1.9f
