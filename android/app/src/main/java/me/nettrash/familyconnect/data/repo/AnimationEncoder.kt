/*
 * AnimationEncoder.kt
 * Family Connect (Android)
 *
 * An animated picture made into the protocol's silent looping video (issue
 * #96, docs/protocol.md "Animations" → "The file"): AnimatedPicture's
 * composited frames, sampled on AnimationPlan's thirty-a-second grid, laid on
 * white, scaled to the profile's size, encoded as H.264 with NO audio track,
 * each video frame lasting exactly its ticks, `moov` first, and the first
 * frame kept as the poster JPEG.
 *
 * THE ENCODER is MediaCodec fed YUV through `getInputImage` — byte buffers,
 * not an input Surface: a Surface stamps its own clock on a frame unless it
 * is driven through EGL, and every frame here has to carry the timestamp the
 * plan gives it. The image's own plane strides are honoured, so a planar and
 * a semi-planar encoder take the same path. The pixels are converted with the
 * BT.709 limited-range matrix the format declares, so white is (235,128,128).
 *
 * THE MUXER is Media3's Mp4Muxer (already in the build: media3-transformer
 * depends on media3-muxer and writes with it, so nothing is added to the
 * catalog — the same way media3-common's classes are used here), not the
 * platform's MediaMuxer, for two reasons that are both
 * about the timing being exact on every phone: its video timescale is 90 000
 * — a multiple of 30, as the protocol asks — with each sample's duration the
 * difference of two timestamps rounded half up, and it takes the LAST
 * sample's duration from an end-of-stream timestamp. The platform muxer's
 * timescale and its last-sample rule have changed between Android releases.
 * A timestamp here is AnimationPlan.tickToUs of the tick it starts on, which
 * rounds back to exactly 3 000 units a tick.
 *
 * Then Mp4Faststart puts the `moov` in front; a file it cannot fix is a
 * failure, because the protocol's container row says `moov` before `mdat`.
 *
 * Anything that fails — no encoder for the size, a codec error, a stall — is
 * null, and the caller sends the picture the way it went before (rule C).
 * Runs on a background thread and checks for cancellation between frames.
 */

package me.nettrash.familyconnect.data.repo

import android.graphics.Bitmap
import android.media.Image
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.util.Log
import androidx.annotation.OptIn
import androidx.core.graphics.scale
import androidx.media3.common.C
import androidx.media3.common.ColorInfo
import androidx.media3.common.Format
import androidx.media3.common.MimeTypes
import androidx.media3.common.util.UnstableApi
import androidx.media3.muxer.BufferInfo
import androidx.media3.muxer.Mp4Muxer
import androidx.media3.muxer.SeekableMuxerOutput
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import java.io.ByteArrayOutputStream
import java.io.File
import java.nio.ByteBuffer

/** Makes an animation's video. An interface so the composer's routing is testable without a codec. */
interface AnimationMaker {

    /** What [make] wrote: the upload's `width`, `height` and `duration_ms`, and the poster. */
    class Made(val width: Int, val height: Int, val durationMs: Long, val poster: ByteArray?)

    /**
     * The animated picture in [source] as the protocol's MP4, written to
     * [output]; null — [output] then gone — when it cannot be (rule C).
     */
    suspend fun make(source: ByteArray, output: File): Made?
}

/** The app's [AnimationMaker]: AnimatedPicture's frames through [AnimationEncoder]. */
object PlatformAnimationMaker : AnimationMaker {
    override suspend fun make(source: ByteArray, output: File): AnimationMaker.Made? = withContext(Dispatchers.Default) {
        val made = runCatching {
            val picture = AnimatedPicture.open(source) ?: return@runCatching null
            val plan = AnimationPlan.plan(
                picture.width.toLong(),
                picture.height.toLong(),
                picture.frames.map { it.statedMs },
            ) ?: return@runCatching null
            AnimationEncoder.encode(picture, plan, output)
        }.getOrElse { error ->
            if (error is kotlinx.coroutines.CancellationException) {
                output.delete()
                throw error
            }
            Log.w(AnimationEncoder.TAG, "the animation could not be made; it goes as a file", error)
            null
        }
        if (made == null) output.delete()
        made
    }
}

object AnimationEncoder {

    internal const val TAG = "AnimationEncoder"

    /**
     * Encode [picture] by [plan] into [output]. Null on any failure; the
     * caller deletes [output] then.
     */
    @OptIn(UnstableApi::class)
    suspend fun encode(picture: AnimatedPicture.Source, plan: AnimationPlan.Plan, output: File): AnimationMaker.Made? {
        val codec = openEncoder(plan) ?: run {
            Log.w(TAG, "no H.264 encoder takes ${plan.width}x${plan.height}")
            return null
        }
        var poster: ByteArray? = null
        var muxer: Mp4Muxer? = null
        try {
            codec.start()
            muxer = Mp4Muxer.Builder(SeekableMuxerOutput.of(output.path))
                .setLastSampleDurationBehavior(
                    Mp4Muxer.LAST_SAMPLE_DURATION_BEHAVIOR_SET_FROM_END_OF_STREAM_BUFFER_OR_DUPLICATE_PREVIOUS,
                )
                .setAttemptStreamableOutputEnabled(false)
                .build()
            val drain = Drain(codec, muxer, plan)
            val frames = Frames(picture, plan)
            var tick = 0L
            for ((index, sample) in plan.samples.withIndex()) {
                currentCoroutineContext().ensureActive()
                val pixels = frames.composited(sample.frame) ?: return null
                if (index == 0) poster = posterOf(frames.flattened, picture.width, picture.height)
                val scaled = frames.scaled(pixels)
                queue(codec, drain, scaled, plan.width, plan.height, AnimationPlan.tickToUs(tick))
                tick += sample.ticks
            }
            // The end of the input: an empty buffer carrying the flag.
            queueEnd(codec, drain)
            drain.untilEnd()
            if (!drain.started) return null
            // The last sample lasts to the end of the plan, not "as long as the one before".
            muxer.writeSampleData(
                drain.track,
                ByteBuffer.allocate(0),
                BufferInfo(AnimationPlan.tickToUs(plan.totalTicks.toLong()), 0, C.BUFFER_FLAG_END_OF_STREAM),
            )
            muxer.close()
            muxer = null
            if (drain.samples != plan.samples.size) {
                Log.w(TAG, "the encoder wrote ${drain.samples} frames of ${plan.samples.size}")
            }
            if (!Mp4Faststart.apply(output)) {
                Log.w(TAG, "the animation's moov could not be moved to the front")
                return null
            }
            return AnimationMaker.Made(plan.width, plan.height, plan.durationMs, poster)
        } finally {
            runCatching { muxer?.close() }
            runCatching { codec.stop() }
            runCatching { codec.release() }
        }
    }

    /** The sampled frames, composited in order — every source frame drawn, only the sampled ones read. */
    private class Frames(private val picture: AnimatedPicture.Source, private val plan: AnimationPlan.Plan) {
        private val compositor = AnimatedPicture.Compositor(picture.width, picture.height)
        private var drawnUpTo = -1

        /** The last frame read, on white, at the picture's own size. */
        val flattened = IntArray(picture.width * picture.height)
        private val scaledPixels = IntArray(plan.width * plan.height)

        /** Frame [frame] as showing — every frame before it drawn first. Null when one cannot be decoded. */
        fun composited(frame: Int): IntArray? {
            while (drawnUpTo < frame) {
                drawnUpTo++
                val info = picture.frames[drawnUpTo]
                val pixels = picture.pixels(drawnUpTo) ?: return null
                compositor.render(info, pixels)
            }
            return AnimatedPicture.flattenOnWhite(compositor.canvas, flattened)
        }

        /** [pixels] (the picture's size) at the plan's size. */
        fun scaled(pixels: IntArray): IntArray {
            if (picture.width == plan.width && picture.height == plan.height) return pixels
            val full = Bitmap.createBitmap(pixels, picture.width, picture.height, Bitmap.Config.ARGB_8888)
            val scaled = full.scale(plan.width, plan.height)
            scaled.getPixels(scaledPixels, 0, plan.width, 0, 0, plan.width, plan.height)
            if (scaled !== full) scaled.recycle()
            full.recycle()
            return scaledPixels
        }
    }

    /** The first frame as the poster: within [MediaPrep.PREVIEW_EDGE], JPEG — the poster every video carries. */
    private fun posterOf(argb: IntArray, width: Int, height: Int): ByteArray? = runCatching {
        val full = Bitmap.createBitmap(argb, width, height, Bitmap.Config.ARGB_8888)
        val longest = maxOf(width, height)
        val bitmap = if (longest <= MediaPrep.PREVIEW_EDGE) {
            full
        } else {
            val factor = MediaPrep.PREVIEW_EDGE.toFloat() / longest
            full.scale((width * factor).toInt().coerceAtLeast(1), (height * factor).toInt().coerceAtLeast(1))
        }
        ByteArrayOutputStream().use { out ->
            bitmap.compress(Bitmap.CompressFormat.JPEG, MediaPrep.PREVIEW_QUALITY, out)
            if (bitmap !== full) bitmap.recycle()
            full.recycle()
            out.toByteArray()
        }
    }.getOrNull()

    // -- The codec ------------------------------------------------------------

    /**
     * An H.264 encoder configured for [plan], started by the caller: hardware
     * first, then software, each at High where it offers it, Main where it
     * offers nothing else, its own default if neither configures. Null when
     * no encoder takes the size — a very small picture on some hardware.
     */
    private fun openEncoder(plan: AnimationPlan.Plan): MediaCodec? {
        val candidates = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
            .filter { info -> info.isEncoder && info.supportedTypes.any { it.equals(MIME, ignoreCase = true) } }
            .sortedBy { if (isSoftware(it)) 1 else 0 }
        for (info in candidates) {
            val caps = runCatching { info.getCapabilitiesForType(MIME) }.getOrNull() ?: continue
            if (MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible !in caps.colorFormats) continue
            if (caps.videoCapabilities?.isSizeSupported(plan.width, plan.height) != true) continue
            val levels = caps.profileLevels.orEmpty()
            val choices = buildList {
                levels.filter { it.profile == MediaCodecInfo.CodecProfileLevel.AVCProfileHigh }
                    .maxByOrNull { it.level }?.let(::add)
                levels.filter { it.profile == MediaCodecInfo.CodecProfileLevel.AVCProfileMain }
                    .maxByOrNull { it.level }?.let(::add)
                add(null)
            }
            for (choice in choices) {
                val codec = runCatching { MediaCodec.createByCodecName(info.name) }.getOrNull() ?: break
                val configured = runCatching {
                    codec.configure(format(plan, choice), null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                }.isSuccess
                if (configured) return codec
                runCatching { codec.release() }
            }
        }
        return null
    }

    private fun isSoftware(info: MediaCodecInfo): Boolean =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            info.isSoftwareOnly
        } else {
            info.name.startsWith("OMX.google.") || info.name.startsWith("c2.android.")
        }

    private fun format(plan: AnimationPlan.Plan, profile: MediaCodecInfo.CodecProfileLevel?): MediaFormat =
        MediaFormat.createVideoFormat(MIME, plan.width, plan.height).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible)
            setInteger(MediaFormat.KEY_BIT_RATE, plan.videoBitrate.toInt())
            setInteger(MediaFormat.KEY_FRAME_RATE, AnimationPlan.TICKS_PER_SECOND)
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
            setInteger(MediaFormat.KEY_COLOR_STANDARD, MediaFormat.COLOR_STANDARD_BT709)
            setInteger(MediaFormat.KEY_COLOR_RANGE, MediaFormat.COLOR_RANGE_LIMITED)
            setInteger(MediaFormat.KEY_COLOR_TRANSFER, MediaFormat.COLOR_TRANSFER_SDR_VIDEO)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
            profile?.let {
                setInteger(MediaFormat.KEY_PROFILE, it.profile)
                setInteger(MediaFormat.KEY_LEVEL, it.level)
            }
        }

    /** One frame in: waits for an input buffer, draining output meanwhile so the codec never blocks on us. */
    private fun queue(codec: MediaCodec, drain: Drain, argb: IntArray, width: Int, height: Int, ptsUs: Long) {
        val deadline = System.nanoTime() + STALL_NS
        while (true) {
            drain.available()
            val index = codec.dequeueInputBuffer(WAIT_US)
            if (index >= 0) {
                val image = codec.getInputImage(index) ?: error("no input image from the encoder")
                writeYuv(image, argb, width, height)
                codec.queueInputBuffer(index, 0, width * height * 3 / 2, ptsUs, 0)
                return
            }
            if (System.nanoTime() > deadline) error("the encoder took no input")
        }
    }

    private fun queueEnd(codec: MediaCodec, drain: Drain) {
        val deadline = System.nanoTime() + STALL_NS
        while (true) {
            drain.available()
            val index = codec.dequeueInputBuffer(WAIT_US)
            if (index >= 0) {
                codec.queueInputBuffer(index, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                return
            }
            if (System.nanoTime() > deadline) error("the encoder took no end of input")
        }
    }

    /** The encoder's output into the muxer, the track added once the encoder says what it makes. */
    @OptIn(UnstableApi::class)
    private class Drain(private val codec: MediaCodec, private val muxer: Mp4Muxer, private val plan: AnimationPlan.Plan) {
        private val info = MediaCodec.BufferInfo()
        var track = -1
            private set
        val started: Boolean get() = track >= 0
        var samples = 0
            private set
        private var ended = false

        /** Whatever is ready now. */
        fun available() {
            while (!ended && step(0)) continue
        }

        /** Everything, up to the encoder's end of stream. */
        fun untilEnd() {
            val deadline = System.nanoTime() + STALL_NS
            while (!ended) {
                if (step(WAIT_US)) continue
                if (System.nanoTime() > deadline) error("the encoder never finished")
            }
        }

        /** One output event; false when there was none within [waitUs]. */
        private fun step(waitUs: Long): Boolean {
            val index = codec.dequeueOutputBuffer(info, waitUs)
            when {
                index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                    if (track < 0) track = muxer.addTrack(trackFormat(codec.outputFormat))
                }
                index >= 0 -> {
                    val buffer = codec.getOutputBuffer(index)
                    val config = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                    if (buffer != null && !config && info.size > 0) {
                        check(track >= 0) { "a frame before the format" }
                        buffer.position(info.offset)
                        buffer.limit(info.offset + info.size)
                        val flags = if (info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0) C.BUFFER_FLAG_KEY_FRAME else 0
                        muxer.writeSampleData(track, buffer, BufferInfo(info.presentationTimeUs, info.size, flags))
                        samples++
                    }
                    codec.releaseOutputBuffer(index, false)
                    if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) ended = true
                }
                else -> return false
            }
            return true
        }

        private fun trackFormat(output: MediaFormat): Format {
            val csd = listOfNotNull(output.getByteBuffer("csd-0"), output.getByteBuffer("csd-1")).map { buffer ->
                ByteArray(buffer.remaining()).also { buffer.duplicate().get(it) }
            }
            return Format.Builder()
                .setSampleMimeType(MimeTypes.VIDEO_H264)
                .setWidth(plan.width)
                .setHeight(plan.height)
                .setFrameRate(AnimationPlan.TICKS_PER_SECOND.toFloat())
                .setColorInfo(ColorInfo.SDR_BT709_LIMITED)
                .setInitializationData(csd)
                .build()
        }
    }

    // -- Pixels -----------------------------------------------------------------

    /**
     * [argb] (opaque, [width] × [height]) into the encoder's YUV 4:2:0 image,
     * BT.709 limited range; chroma is the average of each 2 × 2 block.
     */
    internal fun writeYuv(image: Image, argb: IntArray, width: Int, height: Int) {
        val planes = image.planes
        val yPlane = planes[0]
        val uPlane = planes[1]
        val vPlane = planes[2]
        val yBuffer = yPlane.buffer
        val row = ByteArray(width)
        for (y in 0 until height) {
            val base = y * width
            for (x in 0 until width) row[x] = luma(argb[base + x]).toByte()
            if (yPlane.pixelStride == 1) {
                yBuffer.position(y * yPlane.rowStride)
                yBuffer.put(row, 0, width)
            } else {
                for (x in 0 until width) yBuffer.put(y * yPlane.rowStride + x * yPlane.pixelStride, row[x])
            }
        }
        val uBuffer = uPlane.buffer
        val vBuffer = vPlane.buffer
        for (cy in 0 until (height + 1) / 2) {
            for (cx in 0 until (width + 1) / 2) {
                var r = 0
                var g = 0
                var b = 0
                var n = 0
                for (dy in 0..1) {
                    val y = cy * 2 + dy
                    if (y >= height) continue
                    for (dx in 0..1) {
                        val x = cx * 2 + dx
                        if (x >= width) continue
                        val p = argb[y * width + x]
                        r += (p shr 16) and 0xFF
                        g += (p shr 8) and 0xFF
                        b += p and 0xFF
                        n++
                    }
                }
                r /= n
                g /= n
                b /= n
                uBuffer.put(cy * uPlane.rowStride + cx * uPlane.pixelStride, chromaBlue(r, g, b).toByte())
                vBuffer.put(cy * vPlane.rowStride + cx * vPlane.pixelStride, chromaRed(r, g, b).toByte())
            }
        }
    }

    /**
     * BT.709, limited range: Y = 16 + 219 × (0.2126 R
     * + 0.7152 G + 0.0722 B) ÷ 255, the coefficients scaled by 256 and rounded
     * so that they sum to 220 — white lands on 235 and black on 16.
     */
    internal fun luma(pixel: Int): Int {
        val r = (pixel shr 16) and 0xFF
        val g = (pixel shr 8) and 0xFF
        val b = pixel and 0xFF
        return 16 + ((47 * r + 157 * g + 16 * b + 128) shr 8)
    }

    /** Cb = 128 + 224 × (−0.1146 R − 0.3854 G + 0.5 B) ÷ 255; the coefficients sum to 0, so grey is 128. */
    internal fun chromaBlue(r: Int, g: Int, b: Int): Int =
        (128 + ((-26 * r - 86 * g + 112 * b + 128) shr 8)).coerceIn(16, 240)

    /** Cr = 128 + 224 × (0.5 R − 0.4542 G − 0.0458 B) ÷ 255; the coefficients sum to 0. */
    internal fun chromaRed(r: Int, g: Int, b: Int): Int =
        (128 + ((112 * r - 102 * g - 10 * b + 128) shr 8)).coerceIn(16, 240)

    private const val MIME = MediaFormat.MIMETYPE_VIDEO_AVC

    /** How long one dequeue waits, in microseconds. */
    private const val WAIT_US = 10_000L

    /** How long the encoder may make no progress before it is given up on. */
    private const val STALL_NS = 10_000_000_000L
}
