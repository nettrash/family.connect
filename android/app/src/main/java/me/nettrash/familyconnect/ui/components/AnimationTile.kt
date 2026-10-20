/*
 * AnimationTile.kt
 * Family Connect (Android)
 *
 * How an ANIMATION is drawn (issue #96; docs/protocol.md, "Animations" →
 * "How it is drawn"): a PICTURE that moves, not a video. In the box a
 * photograph of its shape would take, with no play button, no duration, no
 * sound control and no scrubber — only a small "GIF" label in a corner. It
 * plays by itself, muted, and loops for ever while it is on screen, and
 * stops when it scrolls away. It shows its poster, still, with the label:
 *
 *  - when the system asks for less motion (Android's animations turned off,
 *    ANIMATOR_DURATION_SCALE 0) — a tap then plays it in the viewer;
 *  - when its size is above 8 MiB, where it plays when tapped. Below that the
 *    tile DOWNLOADS the video to draw itself (the original-bytes cache a
 *    sticker draws from, and where the sender's own copy was seeded), the one
 *    exception to "a tile never downloads a video".
 *
 * THE PLAYER is the platform's MediaPlayer on a TextureView — the player the
 * rest of this app uses (VideoView, RoundVideoPlayback), so no ExoPlayer joins
 * the build. Muted, looping, and it never asks for audio focus: MediaPlayer
 * does not request it on its own (VideoView would), and the file has no sound
 * track anyway, so somebody's music plays on through a chat full of GIFs. A
 * TextureView rather than a SurfaceView because the tile scrolls, clips to
 * rounded corners and sits under other views. It is transparent until its
 * first frame, so the poster underneath is what shows while it loads.
 *
 * "While it is on screen": the player exists while the tile is composed and
 * attached — a LazyColumn disposes a row that scrolls away, and that releases
 * it — and pauses while the activity is not resumed.
 */

package me.nettrash.familyconnect.ui.components

import android.content.Context
import android.graphics.Matrix
import android.graphics.SurfaceTexture
import android.media.MediaPlayer
import android.view.Surface
import android.view.TextureView
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.net.toUri
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.ui.chat.animationsRemoved
import me.nettrash.familyconnect.ui.chat.drawsHairline
import java.io.File
import java.io.FileInputStream

/** The rules an animation's tile follows (#96), as plain functions the tests pin. */
object AnimationPlayback {

    /**
     * The largest animation a tile downloads to play by itself: 8 MiB
     * (docs/protocol.md, "Animations"). Above it the tile is its poster and
     * plays when tapped.
     */
    const val TILE_DOWNLOAD_LIMIT: Long = 8L * 1024 * 1024

    /** Whether the tile plays by itself — or shows its poster, still. */
    fun tilePlays(sizeBytes: Long, animationsRemoved: Boolean): Boolean =
        !animationsRemoved && sizeBytes <= TILE_DOWNLOAD_LIMIT

    /** Whether the viewer plays the downloaded file (true) or streams it (false, over 8 MiB). */
    fun viewerDownloads(sizeBytes: Long): Boolean = sizeBytes <= TILE_DOWNLOAD_LIMIT

    /**
     * How a [videoWidth] × [videoHeight] picture is fitted to a [viewWidth] ×
     * [viewHeight] TextureView, which stretches it to its own size: the
     * x and y scale to apply about the centre. [crop] fills the view (the
     * tile's box, cut at the long edges, as a photo tile is); otherwise it
     * fits whole (the viewer). (1, 1) when anything is unknown.
     */
    fun textureScale(
        viewWidth: Int,
        viewHeight: Int,
        videoWidth: Int,
        videoHeight: Int,
        crop: Boolean,
    ): Pair<Float, Float> {
        if (viewWidth <= 0 || viewHeight <= 0 || videoWidth <= 0 || videoHeight <= 0) return 1f to 1f
        val horizontal = viewWidth.toFloat() / videoWidth
        val vertical = viewHeight.toFloat() / videoHeight
        val scale = if (crop) maxOf(horizontal, vertical) else minOf(horizontal, vertical)
        return (videoWidth * scale / viewWidth) to (videoHeight * scale / viewHeight)
    }
}

/** Where a looping video's bytes are: a file this device holds, or the authenticated stream. */
sealed interface LoopSource {
    data class Local(val file: File) : LoopSource
    data class Stream(val url: String, val headers: Map<String, String>) : LoopSource
}

/**
 * An animation in a bubble — see the file header. The gestures are the photo
 * tile's trio, for the same reason: a caption-less animation IS the balloon.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun AnimationTile(
    attachment: AttachmentDto,
    onOpen: () -> Unit,
    onLongPress: () -> Unit,
    onDoubleTap: () -> Unit,
    modifier: Modifier = Modifier,
    onBalloon: Boolean = true,
) {
    val poster = rememberTileImage(attachment)
    val plays = AnimationPlayback.tilePlays(attachment.size, animationsRemoved())
    val attachments = LocalAttachments.current
    var file by remember(attachment.id) { mutableStateOf<File?>(null) }
    if (plays && attachments != null) {
        LaunchedEffect(attachment.id, attachments.retryToken) {
            if (file == null) file = attachments.originalFile(attachment.id)
        }
    }
    val label = stringResource(R.string.s_gif)
    Box(
        modifier = modifier
            .widthIn(max = attachmentMaxWidth())
            .aspectRatio(attachment.aspectRatio.coerceIn(MIN_RATIO, MAX_RATIO))
            .clip(RoundedCornerShape(14.dp))
            .background(
                Brush.verticalGradient(
                    listOf(
                        LocalContentColor.current.copy(alpha = 0.14f),
                        LocalContentColor.current.copy(alpha = 0.06f),
                    ),
                ),
            )
            .then(
                if (drawsHairline(onBalloon = onBalloon, hasImage = poster != null)) {
                    Modifier.border(1.dp, LocalContentColor.current.copy(alpha = 0.12f), RoundedCornerShape(14.dp))
                } else {
                    Modifier
                },
            )
            .combinedClickable(onClick = onOpen, onLongClick = onLongPress, onDoubleClick = onDoubleTap)
            .semantics { contentDescription = label }
            .testTag(ANIMATION_TILE_TAG),
        contentAlignment = Alignment.Center,
    ) {
        if (poster != null) {
            Image(
                bitmap = poster,
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier.fillMaxSize(),
            )
        }
        val playing = file
        if (plays && playing != null) {
            LoopingVideo(LoopSource.Local(playing), crop = true, modifier = Modifier.fillMaxSize())
        }
        GifLabel()
    }
}

/** The small "GIF" in the corner — the word every messenger uses, the same in every language. */
@Composable
internal fun BoxScope.GifLabel() {
    Text(
        text = stringResource(R.string.s_gif),
        style = MaterialTheme.typography.labelSmall,
        fontWeight = FontWeight.Bold,
        color = Color.White,
        modifier = Modifier
            .align(Alignment.TopStart)
            .padding(6.dp)
            .clip(RoundedCornerShape(4.dp))
            .background(Color.Black.copy(alpha = 0.45f))
            .padding(horizontal = 5.dp, vertical = 2.dp)
            .testTag(GIF_LABEL_TAG),
    )
}

/**
 * A muted video that loops for ever while it is composed — the tile's and
 * the viewer's. [crop] fills the box; otherwise the picture is fitted whole.
 */
@Composable
internal fun LoopingVideo(source: LoopSource, crop: Boolean, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    // Held still while the thread scrolls or a viewer is over it
    // (LocalMotionHold) — read in an effect, so a scroll edge reaches the
    // player without recomposing the tile. Never held in the viewer itself,
    // which is not under the list's provider. The player is MADE knowing
    // whether it is held, because its surface can arrive before the effect
    // below has run, and a tile flung onto the screen must not open a
    // player in that gap; read without observation, for the same reason
    // the effect exists.
    val hold = LocalMotionHold.current
    val player = remember(source) {
        LoopingPlayer(
            context.applicationContext,
            source,
            crop,
            initiallyHeld = Snapshot.withoutReadObservation { hold.value },
        )
    }
    DisposableEffect(player) { onDispose { player.release() } }
    LaunchedEffect(player, hold) { snapshotFlow { hold.value }.collect { player.setHeld(it) } }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(lifecycle, player) {
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_RESUME -> player.resume()
                Lifecycle.Event.ON_PAUSE -> player.pause()
                else -> Unit
            }
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    AndroidView(
        modifier = modifier,
        factory = { viewContext ->
            TextureView(viewContext).apply {
                // Transparent until the first frame: the poster below shows while it loads.
                isOpaque = false
                player.attach(this)
            }
        },
    )
}

/**
 * One looping, muted MediaPlayer on one TextureView. Made when the view's
 * surface arrives, let go when it goes or the composable leaves. Main thread
 * — except the letting go itself, which [PlayerReaper] does off it.
 *
 * Not made at all while the thread is being scrolled ([setHeld]): a fling
 * through a chat full of GIFs used to create a player — a binder round trip
 * to the media server, a decoder spun up — for every tile that flew past,
 * and tear each down again a moment later. A tile that arrives during a
 * scroll shows its poster, and starts when the scroll comes to rest.
 */
private class LoopingPlayer(
    private val context: Context,
    private val source: LoopSource,
    private val crop: Boolean,
    initiallyHeld: Boolean = false,
) : TextureView.SurfaceTextureListener {

    private var view: TextureView? = null
    private var surface: Surface? = null
    private var player: MediaPlayer? = null
    private var prepared = false
    private var resumed = true
    private var released = false
    private var held = initiallyHeld
    /** The surface that arrived while held, waiting for the hold to lift. */
    private var waiting: SurfaceTexture? = null
    private var videoWidth = 0
    private var videoHeight = 0

    fun attach(textureView: TextureView) {
        view = textureView
        textureView.surfaceTextureListener = this
        textureView.surfaceTexture?.let { open(it) }
    }

    fun resume() {
        resumed = true
        if (prepared && !held) runCatching { player?.start() }
    }

    /**
     * Hold still (pause, and make no player for a surface that arrives
     * meanwhile), or carry on: open what arrived while held, or start the
     * paused loop again — never past the activity's own pause.
     */
    fun setHeld(value: Boolean) {
        if (held == value) return
        held = value
        if (value) {
            runCatching { if (player?.isPlaying == true) player?.pause() }
            return
        }
        waiting?.let { texture ->
            waiting = null
            open(texture)
        }
        if (prepared && resumed) runCatching { player?.start() }
    }

    fun pause() {
        resumed = false
        runCatching { if (player?.isPlaying == true) player?.pause() }
    }

    fun release() {
        released = true
        waiting = null
        close()
        // The listener STAYS: the view is detached after this, and its
        // surface's destruction is what hands the texture to the reaper
        // behind the player still drawing into it (see
        // onSurfaceTextureDestroyed). With `released` set nothing else in
        // it does anything.
        view = null
    }

    override fun onSurfaceTextureAvailable(texture: SurfaceTexture, width: Int, height: Int) = open(texture)

    override fun onSurfaceTextureSizeChanged(texture: SurfaceTexture, width: Int, height: Int) = fit()

    override fun onSurfaceTextureDestroyed(texture: SurfaceTexture): Boolean {
        if (waiting === texture) waiting = null
        close()
        // false = this listener releases the texture, not the view — and it
        // does so on the reaper, AFTER the player that was drawing into it
        // (close() queued that first, and the reaper is one thread, in
        // order). Released here at once, as `true` asked the view to do, the
        // player's last frames would land in a buffer queue that no longer
        // exists while its own release is still in flight.
        PlayerReaper.release(texture)
        return false
    }

    override fun onSurfaceTextureUpdated(texture: SurfaceTexture) = Unit

    private fun open(texture: SurfaceTexture) {
        if (released || player != null) return
        if (held) {
            waiting = texture
            return
        }
        val made = Surface(texture)
        surface = made
        player = runCatching {
            MediaPlayer().apply {
                setSurface(made)
                when (source) {
                    is LoopSource.Local -> FileInputStream(source.file).use { setDataSource(it.fd) }
                    is LoopSource.Stream -> setDataSource(context, source.url.toUri(), source.headers)
                }
                isLooping = true
                // Muted twice over: the file has no sound track, and the player says none.
                setVolume(0f, 0f)
                // Both callbacks ask whether the player is still THIS
                // owner's: one handed to the reaper can still deliver a late
                // event before the media server has let it go, and it must
                // not mark the owner's next player prepared, or start it.
                setOnVideoSizeChangedListener { reporting, width, height ->
                    if (reporting !== player) return@setOnVideoSizeChangedListener
                    this@LoopingPlayer.videoWidth = width
                    this@LoopingPlayer.videoHeight = height
                    fit()
                }
                setOnPreparedListener { ready ->
                    if (ready !== player) return@setOnPreparedListener
                    prepared = true
                    if (resumed && !held) runCatching { ready.start() }
                }
                // A failure leaves the poster: nothing to say over a picture.
                setOnErrorListener { _, _, _ -> true }
                prepareAsync()
            }
        }.getOrNull()
    }

    private fun fit() {
        val textureView = view ?: return
        val (sx, sy) = AnimationPlayback.textureScale(textureView.width, textureView.height, videoWidth, videoHeight, crop)
        textureView.setTransform(
            Matrix().apply { setScale(sx, sy, textureView.width / 2f, textureView.height / 2f) },
        )
    }

    private fun close() {
        prepared = false
        // Handed to the reaper rather than released here: MediaPlayer.release()
        // waits for the media server to tear the decoder down, and this runs
        // on the main thread as a tile scrolls out — one stall per GIF that
        // left the screen, in the middle of the scroll that moved it.
        PlayerReaper.release(player, surface)
        player = null
        surface = null
    }
}

/**
 * Where looping players are let go: ONE background thread, in the order
 * things are handed to it — a player before its Surface, both before the
 * SurfaceTexture they draw into — which is the order the main thread used to
 * release them in, without the main thread waiting for the media server.
 *
 * A player handed here may still deliver a late event on the main thread
 * before the media server has let it go; its owner has already forgotten it,
 * and LoopingPlayer's listeners ignore any player that is not its current
 * one.
 */
internal object PlayerReaper {
    private val thread = java.util.concurrent.Executors.newSingleThreadExecutor { task ->
        Thread(task, "fc-player-reaper").apply { isDaemon = true }
    }

    fun release(player: MediaPlayer?, surface: Surface?) {
        if (player == null && surface == null) return
        thread.execute {
            runCatching { player?.release() }
            runCatching { surface?.release() }
        }
    }

    fun release(texture: SurfaceTexture) {
        thread.execute { runCatching { texture.release() } }
    }
}

/** Test tags: the tile, and its label. */
internal const val ANIMATION_TILE_TAG = "animation-tile"
internal const val GIF_LABEL_TAG = "animation-gif-label"
