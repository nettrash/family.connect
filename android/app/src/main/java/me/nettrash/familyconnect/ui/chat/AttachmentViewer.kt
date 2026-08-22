/*
 * AttachmentViewer.kt
 * Family Connect (Android)
 *
 * A photo or video, full screen.
 *
 * The two halves work differently on purpose. A photo is FETCHED — the
 * whole JPEG lands in AttachmentRepository's disk cache and is then
 * zoomable offline, which is what you want for something you will open
 * again. A video is STREAMED: the player pulls byte ranges from the
 * server as it plays (the endpoint honours Range — docs/protocol.md), so
 * a 90 MB clip starts in a second and never occupies 90 MB anywhere.
 *
 * Playback is the platform's own VideoView rather than ExoPlayer. It
 * takes the Authorization header the protocol needs, and keeping it means
 * media3 is in this build for compression alone — see the note in
 * gradle/libs.versions.toml.
 *
 * iOS counterpart: ios/FamilyConnect/Views/AttachmentViewer.swift
 */

package me.nettrash.familyconnect.ui.chat

import android.widget.MediaController
import android.widget.VideoView
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.net.toUri
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.ui.components.rememberAttachmentImage

/**
 * Full-screen viewer, as a dialog rather than a route: it is a transient
 * look at one message's media, and the thread underneath keeps its scroll
 * position and its socket without a navigation round trip.
 */
@Composable
fun AttachmentViewer(
    attachment: AttachmentDto,
    streamUrl: suspend (Long) -> Pair<String, Map<String, String>>?,
    onShare: () -> Unit,
    onSave: () -> Unit,
    onDismiss: () -> Unit,
) {
    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                // Black in both themes: a photo full screen wants no
                // surrounding colour competing with it.
                .background(Color.Black),
            contentAlignment = Alignment.Center,
        ) {
            if (attachment.isVideo) {
                VideoAttachment(attachment = attachment, streamUrl = streamUrl)
            } else {
                ZoomablePhoto(attachment = attachment)
            }
            IconButton(
                onClick = onDismiss,
                modifier = Modifier
                    .align(Alignment.TopStart)
                    .padding(12.dp),
                colors = IconButtonDefaults.iconButtonColors(
                    containerColor = Color.Black.copy(alpha = 0.45f),
                    contentColor = Color.White,
                ),
            ) {
                Icon(imageVector = Icons.Filled.Close, contentDescription = stringResource(R.string.s_close))
            }
            Row(
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .padding(12.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                IconButton(
                    onClick = onSave,
                    colors = IconButtonDefaults.iconButtonColors(
                        containerColor = Color.Black.copy(alpha = 0.45f),
                        contentColor = Color.White,
                    ),
                ) {
                    Icon(
                        imageVector = Icons.Filled.Download,
                        contentDescription = stringResource(R.string.s_save_to_gallery),
                    )
                }
                IconButton(
                    onClick = onShare,
                    colors = IconButtonDefaults.iconButtonColors(
                        containerColor = Color.Black.copy(alpha = 0.45f),
                        contentColor = Color.White,
                    ),
                ) {
                    Icon(imageVector = Icons.Filled.Share, contentDescription = stringResource(R.string.s_share))
                }
            }
        }
    }
}

/** Pinch and double-tap to zoom, drag to pan while zoomed. */
@Composable
private fun ZoomablePhoto(attachment: AttachmentDto) {
    val full = rememberAttachmentImage(attachment, preview = false)
    val preview = rememberAttachmentImage(attachment, preview = true)
    val image = full ?: preview

    // Float state, not boxed: these change on every frame of a pinch.
    var zoom by remember { mutableFloatStateOf(1f) }
    var offsetX by remember { mutableFloatStateOf(0f) }
    var offsetY by remember { mutableFloatStateOf(0f) }

    fun reset() {
        zoom = 1f
        offsetX = 0f
        offsetY = 0f
    }

    if (image == null) {
        CircularProgressIndicator(color = Color.White)
        return
    }
    Image(
        bitmap = image,
        contentDescription = stringResource(R.string.s_photo),
        contentScale = ContentScale.Fit,
        modifier = Modifier
            .fillMaxSize()
            .pointerInput(Unit) {
                detectTransformGestures { _, pan, gestureZoom, _ ->
                    zoom = (zoom * gestureZoom).coerceIn(1f, MAX_ZOOM)
                    if (zoom > 1f) {
                        offsetX += pan.x
                        offsetY += pan.y
                    } else {
                        // Panning an unzoomed photo would just slide it
                        // off the screen with nothing to reveal.
                        offsetX = 0f
                        offsetY = 0f
                    }
                }
            }
            .pointerInput(Unit) {
                detectTapGestures(
                    onDoubleTap = { if (zoom > 1f) reset() else zoom = 2.5f },
                )
            }
            .graphicsLayer(
                scaleX = zoom,
                scaleY = zoom,
                translationX = offsetX,
                translationY = offsetY,
            ),
    )
}

/**
 * Streams from the server with the session token on every range request.
 *
 * The URL is resolved in a LaunchedEffect rather than up front because it
 * needs the stored token, which is a suspending read.
 */
@Composable
private fun VideoAttachment(
    attachment: AttachmentDto,
    streamUrl: suspend (Long) -> Pair<String, Map<String, String>>?,
) {
    var source by remember(attachment.id) {
        mutableStateOf<Pair<String, Map<String, String>>?>(null)
    }
    LaunchedEffect(attachment.id) { source = streamUrl(attachment.id) }

    val resolved = source
    if (resolved == null) {
        CircularProgressIndicator(color = Color.White)
        return
    }
    AndroidView(
        modifier = Modifier.fillMaxSize(),
        factory = { context ->
            VideoView(context).apply {
                setMediaController(
                    MediaController(context).also { it.setAnchorView(this) },
                )
                // setVideoURI's header overload is the only way to send
                // an Authorization header — the attachment endpoint needs
                // one on every byte-range request the player makes.
                setVideoURI(resolved.first.toUri(), resolved.second)
                setOnPreparedListener { player ->
                    player.isLooping = false
                    start()
                }
            }
        },
        onRelease = { view ->
            view.stopPlayback()
        },
    )
}

private const val MAX_ZOOM = 4f
