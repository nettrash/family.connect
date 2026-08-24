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
import androidx.compose.foundation.layout.height
import com.google.android.gms.maps.GoogleMapOptions
import com.google.android.gms.maps.model.CameraPosition
import com.google.android.gms.maps.model.LatLng
import com.google.maps.android.compose.GoogleMap
import com.google.maps.android.compose.MapUiSettings
import com.google.maps.android.compose.Marker
import com.google.maps.android.compose.rememberCameraPositionState
import com.google.maps.android.compose.rememberUpdatedMarkerState
import me.nettrash.familyconnect.BuildConfig
import android.content.Intent
import android.net.Uri
import androidx.compose.material.icons.filled.Place
import java.util.Locale
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
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.Spacer
import android.media.MediaPlayer
import androidx.compose.material3.IconButton
import androidx.compose.material3.Slider
import androidx.compose.material.icons.filled.Pause
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.core.net.toUri
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import androidx.compose.ui.graphics.Brush
import androidx.compose.foundation.border
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
import androidx.compose.ui.res.stringResource
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
import me.nettrash.familyconnect.R
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
     * Where audio streams from, with the auth header it needs. Threaded
     * explicitly like the gesture callbacks rather than reached for through
     * a CompositionLocal — this file already takes what it needs as
     * parameters, and a hidden dependency here would be harder to follow.
     */
    streamUrl: suspend (Long) -> Pair<String, Map<String, String>>? = { null },
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
    /**
     * Whether a shared location draws a map. The reader's own setting —
     * drawing one asks Google for tiles, so it is theirs to switch off, the
     * same trade the link previews make.
     */
    showMapPreviews: Boolean = true,
) {
    if (attachment.isLocation) {
        // A location has no bytes at all, so none of the download machinery
        // below applies to it (docs/protocol.md, "Locations").
        LocationRow(
            attachment = attachment,
            onLongPress = onLongPress,
            onDoubleTap = onDoubleTap,
            showMap = showMapPreviews && BuildConfig.HAS_MAPS_KEY,
            modifier = modifier,
        )
    } else if (attachment.isAudio) {
        // Audio has nothing to look at, so it gets a player rather than a
        // tile or a document row (protocol.md, "Audio").
        AudioPlayerRow(
            attachment = attachment,
            streamUrl = streamUrl,
            onLongPress = onLongPress,
            onDoubleTap = onDoubleTap,
            modifier = modifier,
        )
    } else if (attachment.isFile) {
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
 * A shared place.
 *
 * A ROW rather than a map tile, and that is a platform decision rather than
 * a design one: drawing map tiles on Android needs the Play Services Maps
 * SDK and an API key, and this app's dependency catalog states plainly that
 * firebase-messaging is the only Google service it will carry — "nothing
 * else may phone Google". So the pin, its label and its coordinates are
 * drawn from what arrived with the message, and TAPPING hands off to
 * whichever map app the person already has, through a `geo:` intent. That
 * hand-off is a choice the reader makes, not a request the app makes on
 * their behalf, which is the stricter half of the same privacy trade iOS
 * offers as a switch.
 */
@Composable
private fun LocationRow(
    attachment: AttachmentDto,
    onLongPress: () -> Unit,
    onDoubleTap: () -> Unit,
    /**
     * Draw the map. Off when the reader switched it off, and off when this
     * build carries no Maps API key — without a key Google renders a grey
     * "authorization failure" tile, which is worse than the card alone.
     */
    showMap: Boolean,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val latitude = attachment.latitude
    val longitude = attachment.longitude
    // The row is drawn even with NO coordinate, and that is a deliberate
    // floor rather than defensive noise. A location has no bytes to fall
    // back on, so anything that loses the pin — a sync bug, a row written
    // by an older build — used to `return` here and render an absolutely
    // EMPTY balloon, which reads as the app being broken rather than as one
    // missing detail. Saying "Location" and nothing else is a bad outcome;
    // saying nothing at all is a worse one.
    val label = attachment.name?.takeIf { it.isNotEmpty() }
        ?: stringResource(R.string.s_location)
    val coordinates = remember(latitude, longitude, attachment.accuracyM) {
        if (latitude == null || longitude == null) {
            ""
        } else {
            val lat = String.format(Locale.ROOT, "%.5f", latitude)
            val lon = String.format(Locale.ROOT, "%.5f", longitude)
            val accuracy = attachment.accuracyM
            if (accuracy == null) "$lat, $lon" else "$lat, $lon · ±$accuracy m"
        }
    }
    // Same rule as the file row: everything derives from the balloon's own
    // content colour, so it reads on both the tinted and the neutral ground.
    val ink = LocalContentColor.current
    Column(
        modifier = modifier
            .widthIn(max = MAX_WIDTH.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(ink.copy(alpha = 0.10f))
            .border(1.dp, ink.copy(alpha = 0.12f), RoundedCornerShape(12.dp))
            .combinedClickable(
                onClick = {
                    if (latitude != null && longitude != null) {
                        openInMaps(context, latitude, longitude, label)
                    }
                },
                onLongClick = onLongPress,
                onDoubleClick = onDoubleTap,
            )
            .semantics { contentDescription = "$label, $coordinates" },
    ) {
        if (showMap && latitude != null && longitude != null) {
            LocationMap(latitude = latitude, longitude = longitude)
        }
        Row(
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
        Box(
            modifier = Modifier
                .size(36.dp)
                .clip(RoundedCornerShape(9.dp))
                .background(ink.copy(alpha = 0.12f)),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = Icons.Filled.Place,
                contentDescription = null,
                tint = ink,
                modifier = Modifier.size(22.dp),
            )
        }
            Column(modifier = Modifier.weight(1f, fill = false)) {
                Text(
                    text = label,
                    style = MaterialTheme.typography.bodyMedium,
                    color = ink,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (coordinates.isNotEmpty()) {
                    Text(
                        text = coordinates,
                        style = MaterialTheme.typography.labelSmall,
                        color = ink.copy(alpha = 0.72f),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }
}

/**
 * The map itself.
 *
 * **LITE MODE, and that is not a detail.** A normal `MapView` is a live
 * OpenGL surface; one per row in a scrolling thread is not affordable, and
 * a thread full of them stutters and drains the battery. Lite mode renders
 * a static bitmap with no GL context and no gestures — which is also
 * exactly what a bubble wants, since a pannable map inside a scrolling list
 * fights the list for every drag.
 *
 * Interaction is deliberately none: the whole row is clickable and hands
 * off to a real map app, which is where panning belongs.
 */
@Composable
private fun LocationMap(latitude: Double, longitude: Double) {
    val position = remember(latitude, longitude) { LatLng(latitude, longitude) }
    val cameraPositionState = rememberCameraPositionState {
        // ~600 m across: close enough to recognise the street, wide enough
        // to place it in a neighbourhood. Matches the Apple clients.
        this.position = CameraPosition.fromLatLngZoom(position, 15f)
    }
    GoogleMap(
        modifier = Modifier
            .fillMaxWidth()
            .height(140.dp),
        cameraPositionState = cameraPositionState,
        googleMapOptionsFactory = { GoogleMapOptions().liteMode(true) },
        uiSettings = MapUiSettings(
            compassEnabled = false,
            mapToolbarEnabled = false,
            zoomControlsEnabled = false,
            scrollGesturesEnabled = false,
            zoomGesturesEnabled = false,
            tiltGesturesEnabled = false,
            rotationGesturesEnabled = false,
        ),
    ) {
        Marker(state = rememberUpdatedMarkerState(position = position))
    }
}

/**
 * Hand a place to whichever map app is installed.
 *
 * `geo:lat,lon?q=lat,lon(label)` rather than a Google Maps URL: the scheme
 * is what every map app on Android registers for, so this opens the one the
 * person actually uses. The `q` is what puts a PIN there — a bare
 * `geo:lat,lon` only centres the map, with nothing marked.
 */
private fun openInMaps(context: Context, latitude: Double, longitude: Double, label: String) {
    val point = "${String.format(Locale.ROOT, "%.6f", latitude)}," +
        String.format(Locale.ROOT, "%.6f", longitude)
    val uri = Uri.parse("geo:$point?q=$point(${Uri.encode(label)})")
    val intent = Intent(Intent.ACTION_VIEW, uri)
    // No map app installed is a real state on a stripped device; failing
    // silently beats an ActivityNotFoundException taking the app down.
    runCatching { context.startActivity(intent) }
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
    // Everything here takes its colour from LocalContentColor — the balloon's
    // own content colour — rather than from a fixed black wash or the primary
    // role. That is what makes it read correctly on BOTH grounds: it lightens
    // over the tinted own balloon and darkens over the neutral one, and the
    // icon can never end up primary-on-primaryContainer.
    val ink = LocalContentColor.current
    Row(
        modifier = modifier
            .widthIn(max = MAX_WIDTH.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(ink.copy(alpha = 0.10f))
            .border(1.dp, ink.copy(alpha = 0.12f), RoundedCornerShape(12.dp))
            .combinedClickable(
                onClick = onOpen,
                onLongClick = onLongPress,
                onDoubleClick = onDoubleTap,
            )
            .padding(horizontal = 10.dp, vertical = 8.dp)
            .semantics {
                contentDescription =
                    "${attachment.displayName}, ${formatSize(context, attachment.size)}"
            },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Box(
            modifier = Modifier
                .size(36.dp)
                .clip(RoundedCornerShape(9.dp))
                .background(ink.copy(alpha = 0.12f)),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = iconFor(attachment.mime),
                contentDescription = null,
                tint = ink,
                modifier = Modifier.size(22.dp),
            )
        }
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
    // Resolved out here: a semantics block is not a composable context.
    val mediaDescription = stringResource(
        if (attachment.isVideo) R.string.s_video else R.string.s_photo)

    Box(
        modifier = modifier
            .widthIn(max = MAX_WIDTH.dp)
            .aspectRatio(attachment.aspectRatio.coerceIn(MIN_RATIO, MAX_RATIO))
            .clip(RoundedCornerShape(14.dp))
            // A soft ramp rather than a flat slab: while the bytes are still
            // coming this rectangle is all there is to look at. Derived from
            // the balloon's content colour so it works on either ground.
            .background(
                Brush.verticalGradient(
                    listOf(
                        LocalContentColor.current.copy(alpha = 0.14f),
                        LocalContentColor.current.copy(alpha = 0.06f),
                    ),
                ),
            )
            .border(1.dp, LocalContentColor.current.copy(alpha = 0.12f), RoundedCornerShape(14.dp))
            .combinedClickable(
                onClick = onOpen,
                onLongClick = onLongPress,
                onDoubleClick = onDoubleTap,
            )
            .semantics {
                contentDescription = mediaDescription
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

/**
 * A piece of audio inside a bubble: play, elapsed/total, and a scrubber.
 *
 * Deliberately NOT a waveform — that is a second artefact the sender would
 * have to generate, upload and version, for something the ear does not need
 * and the protocol therefore does not carry (protocol.md, "Audio").
 *
 * MediaPlayer rather than ExoPlayer, for the same reason VideoView plays the
 * videos: `setDataSource(context, uri, headers)` carries the Authorization
 * header the stream needs, and adding a player library for one row would be
 * a large dependency for a small feature.
 *
 * iOS/macOS counterpart: ios/FamilyConnect/Views/AudioPlayerView.swift
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun AudioPlayerRow(
    attachment: AttachmentDto,
    streamUrl: suspend (Long) -> Pair<String, Map<String, String>>?,
    onLongPress: () -> Unit,
    onDoubleTap: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val ink = LocalContentColor.current

    val totalMs = (attachment.durationMs ?: 0).coerceAtLeast(1)
    var player by remember(attachment.id) { mutableStateOf<MediaPlayer?>(null) }
    var isPlaying by remember(attachment.id) { mutableStateOf(false) }
    var positionMs by remember(attachment.id) { mutableIntStateOf(0) }
    var scrubbing by remember(attachment.id) { mutableStateOf(false) }

    // Release with the composable, or a scrolled-away bubble keeps the
    // decoder and the socket open.
    DisposableEffect(attachment.id) {
        onDispose {
            player?.runCatching { release() }
            player = null
        }
    }

    LaunchedEffect(isPlaying) {
        while (isPlaying) {
            if (!scrubbing) positionMs = player?.currentPosition ?: positionMs
            delay(200)
        }
    }

    Row(
        modifier = modifier
            .widthIn(max = MAX_WIDTH.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(ink.copy(alpha = 0.10f))
            .border(1.dp, ink.copy(alpha = 0.12f), RoundedCornerShape(12.dp))
            .combinedClickable(
                onClick = {},
                onLongClick = onLongPress,
                onDoubleClick = onDoubleTap,
            )
            .padding(horizontal = 10.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        IconButton(
            onClick = {
                val active = player
                if (isPlaying && active != null) {
                    active.pause()
                    isPlaying = false
                    return@IconButton
                }
                scope.launch {
                    val ready = active ?: createPlayer(context, attachment, streamUrl) {
                        isPlaying = false
                        positionMs = totalMs
                    }
                    if (ready == null) return@launch
                    player = ready
                    // Replaying after it ran to the end: without this the
                    // button does nothing, the item being already at its end.
                    if (positionMs >= totalMs - 200) {
                        ready.seekTo(0)
                        positionMs = 0
                    }
                    ready.start()
                    isPlaying = true
                }
            },
        ) {
            Icon(
                imageVector = if (isPlaying) Icons.Filled.Pause else Icons.Filled.PlayArrow,
                contentDescription = stringResource(
                    if (isPlaying) R.string.s_pause else R.string.s_play,
                ),
                tint = ink,
            )
        }
        Column(modifier = Modifier.weight(1f)) {
            Slider(
                value = positionMs.coerceIn(0, totalMs).toFloat(),
                onValueChange = {
                    scrubbing = true
                    positionMs = it.toInt()
                },
                onValueChangeFinished = {
                    scrubbing = false
                    player?.seekTo(positionMs)
                },
                valueRange = 0f..totalMs.toFloat(),
            )
            Row(modifier = Modifier.fillMaxWidth()) {
                Text(
                    text = formatMillis(positionMs.toLong()),
                    style = MaterialTheme.typography.labelSmall,
                    color = ink.copy(alpha = 0.75f),
                )
                Spacer(Modifier.weight(1f))
                Text(
                    text = formatMillis(totalMs.toLong()),
                    style = MaterialTheme.typography.labelSmall,
                    color = ink.copy(alpha = 0.75f),
                )
            }
        }
    }
}

/**
 * A MediaPlayer pointed at the stream, with the auth header attached.
 * Returns null when it cannot be prepared — a bubble that will not play is
 * better than a crash.
 */
private suspend fun createPlayer(
    context: android.content.Context,
    attachment: AttachmentDto,
    streamUrl: suspend (Long) -> Pair<String, Map<String, String>>?,
    onCompleted: () -> Unit,
): MediaPlayer? {
    val entry = streamUrl(attachment.id) ?: return null
    return withContext(Dispatchers.IO) {
        runCatching {
            MediaPlayer().apply {
                setDataSource(context, entry.first.toUri(), entry.second)
                setOnCompletionListener { onCompleted() }
                prepare()
            }
        }.getOrNull()
    }
}

private fun formatMillis(ms: Long): String {
    val whole = (ms / 1000).coerceAtLeast(0)
    return "%d:%02d".format(whole / 60, whole % 60)
}
