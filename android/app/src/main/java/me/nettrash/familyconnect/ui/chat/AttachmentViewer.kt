/*
 * AttachmentViewer.kt
 * Family Connect (Android)
 *
 * A message's photos and videos, full screen, one page each.
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
 * An album is a HorizontalPager over the message's media, opened on the
 * tapped item. The pager only composes the page on screen (and the one
 * sliding in), so a video that is paged away is disposed and stops the
 * way it always did on dismiss. The one piece of bookkeeping is the
 * other direction: a video starts only once its page has SETTLED, not
 * the moment a drag exposes it — see [VideoAttachment].
 *
 * iOS counterpart: ios/FamilyConnect/Views/AttachmentViewer.swift
 */

package me.nettrash.familyconnect.ui.chat

import android.widget.MediaController
import android.widget.VideoView
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroidSize
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.PointerInputScope
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChanged
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.net.toUri
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.ui.components.AttachmentAlbum
import me.nettrash.familyconnect.ui.components.rememberAttachmentImage
import kotlin.math.abs

/**
 * Full-screen viewer, as a dialog rather than a route: it is a transient
 * look at one message's media, and the thread underneath keeps its scroll
 * position and its socket without a navigation round trip.
 *
 * Share and Save act on the page SHOWING, whichever the album was opened
 * on — the buttons read the pager, not the album value. Paging is off
 * while the photo on screen is zoomed in: a drag then pans the photo, and
 * a page turn under a pan is the one thing a zoomed-in reader never means.
 */
@Composable
fun AttachmentViewer(
    album: AttachmentAlbum,
    streamUrl: suspend (Long) -> Pair<String, Map<String, String>>?,
    onShare: (AttachmentDto) -> Unit,
    onSave: (AttachmentDto) -> Unit,
    onDismiss: () -> Unit,
) {
    val items = album.items
    val pager = rememberPagerState(initialPage = album.clampedIndex) { items.size }
    // The page whose photo is zoomed in, if any. Only the page on screen
    // can be — paging stops the moment one zooms — so one slot suffices,
    // and the page reports itself out again when it resets or leaves.
    var zoomedPage by remember { mutableStateOf<Int?>(null) }
    val current = items[pager.currentPage]

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
            HorizontalPager(
                state = pager,
                modifier = Modifier.fillMaxSize(),
                userScrollEnabled = zoomedPage == null,
                key = { items[it].id },
            ) { page ->
                val item = items[page]
                Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    if (item.isVideo) {
                        VideoAttachment(
                            attachment = item,
                            streamUrl = streamUrl,
                            isCurrent = pager.settledPage == page,
                        )
                    } else {
                        ZoomablePhoto(
                            attachment = item,
                            onZoomedChange = { zoomed ->
                                zoomedPage = if (zoomed) page else zoomedPage.takeIf { it != page }
                            },
                        )
                    }
                }
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
            if (items.size > 1) {
                // "2 of 5", in the buttons' own capsule so the top edge
                // reads as one row of chrome. Absent on a lone photo: a
                // "1 of 1" would only say the viewer has nothing to page.
                Text(
                    text = stringResource(R.string.s_n_of_m, pager.currentPage + 1, items.size),
                    style = MaterialTheme.typography.labelLarge,
                    color = Color.White,
                    modifier = Modifier
                        .align(Alignment.TopCenter)
                        .padding(top = 12.dp + 8.dp)
                        .clip(CircleShape)
                        .background(Color.Black.copy(alpha = 0.45f))
                        .padding(horizontal = 12.dp, vertical = 6.dp),
                )
            }
            Row(
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .padding(12.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                IconButton(
                    onClick = { onSave(current) },
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
                    onClick = { onShare(current) },
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

/**
 * Pinch and double-tap to zoom, drag to pan while zoomed.
 *
 * [onZoomedChange] tells the pager whether this page holds the pan: true
 * while zoomed in, false again on reset and when the page leaves the
 * composition, so a page can never leave the pager locked behind it.
 */
@Composable
private fun ZoomablePhoto(
    attachment: AttachmentDto,
    onZoomedChange: (Boolean) -> Unit,
) {
    val full = rememberAttachmentImage(attachment, preview = false)
    val preview = rememberAttachmentImage(attachment, preview = true)
    val image = full ?: preview

    // Float state, not boxed: these change on every frame of a pinch.
    var zoom by remember { mutableFloatStateOf(1f) }
    var offsetX by remember { mutableFloatStateOf(0f) }
    var offsetY by remember { mutableFloatStateOf(0f) }

    val zoomed = zoom > 1f
    val reportZoomed by rememberUpdatedState(onZoomedChange)
    LaunchedEffect(zoomed) { reportZoomed(zoomed) }
    DisposableEffect(Unit) { onDispose { reportZoomed(false) } }

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
                detectPhotoTransform { pan, gestureZoom, pinching ->
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
                    // Ours only while there is something to pan, or while
                    // two fingers are on it; a one-finger drag on a photo
                    // at 1× is the pager's page turn, not a pan.
                    zoom > 1f || pinching
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
 * Foundation's detectTransformGestures, minus rotation and with one
 * change that is the whole point: it consumes a move only when
 * [onGesture] says the photo took it. The stock detector consumes every
 * move past touch slop, and a child that consumes is a child the pager
 * above it never hears from — a photo at 1× would swallow the swipe that
 * should turn the page.
 *
 * @param onGesture pan and zoom deltas, and whether a second finger is
 *   down; returns true when the photo owns this gesture.
 */
private suspend fun PointerInputScope.detectPhotoTransform(
    onGesture: (pan: Offset, zoom: Float, pinching: Boolean) -> Boolean,
) {
    awaitEachGesture {
        var zoom = 1f
        var pan = Offset.Zero
        var pastTouchSlop = false
        val touchSlop = viewConfiguration.touchSlop

        awaitFirstDown(requireUnconsumed = false)
        do {
            val event = awaitPointerEvent()
            val canceled = event.changes.any { it.isConsumed }
            if (!canceled) {
                val zoomChange = event.calculateZoom()
                val panChange = event.calculatePan()
                // A second finger settles it before any slop does: two
                // fingers are never a page turn, and waiting would let the
                // pager win the race on the first few pixels of a pinch.
                val pinching = event.changes.count { it.pressed } > 1

                if (!pastTouchSlop) {
                    zoom *= zoomChange
                    pan += panChange
                    val centroidSize = event.calculateCentroidSize(useCurrent = false)
                    val zoomMotion = abs(1 - zoom) * centroidSize
                    val panMotion = pan.getDistance()
                    if (pinching || zoomMotion > touchSlop || panMotion > touchSlop) {
                        pastTouchSlop = true
                    }
                }

                if (pastTouchSlop && (zoomChange != 1f || panChange != Offset.Zero)) {
                    if (onGesture(panChange, zoomChange, pinching)) {
                        event.changes.forEach { if (it.positionChanged()) it.consume() }
                    }
                }
            }
        } while (!canceled && event.changes.any { it.pressed })
    }
}

/**
 * Streams from the server with the session token on every range request.
 *
 * The URL is resolved in a LaunchedEffect rather than up front because it
 * needs the stored token, which is a suspending read — and only once the
 * page is the CURRENT one. The pager composes a neighbour the moment a
 * drag exposes a sliver of it, and a clip that started streaming then
 * would be heard before it was seen; the iOS viewer gates its player on
 * the same condition, for the same reason. Dragging away pauses it the
 * moment another page settles, and the disposal that follows once it is
 * off screen stops it for good.
 */
@Composable
private fun VideoAttachment(
    attachment: AttachmentDto,
    streamUrl: suspend (Long) -> Pair<String, Map<String, String>>?,
    /** Whether this is the page the pager has settled on. */
    isCurrent: Boolean,
) {
    var source by remember(attachment.id) {
        mutableStateOf<Pair<String, Map<String, String>>?>(null)
    }
    LaunchedEffect(attachment.id, isCurrent) {
        if (isCurrent && source == null) source = streamUrl(attachment.id)
    }

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
                setOnPreparedListener { player -> player.isLooping = false }
            }
        },
        // start() and pause() before the player is prepared only record
        // the state it should open in — VideoView keeps a target state and
        // applies it on prepare — so the first update, which runs before
        // any bytes arrive, is what starts playback; a later one pauses
        // the clip when the reader pages off it, and resumes it where it
        // stopped if they page straight back.
        update = { view ->
            if (isCurrent) view.start() else view.pause()
        },
        // Paging away disposes the page (see the file header), so this
        // is where a video stops whether the reader closed the viewer or
        // swiped past it.
        onRelease = { view ->
            view.stopPlayback()
        },
    )
}

private const val MAX_ZOOM = 4f
