/*
 * TestClips.kt
 * Family Connect (Android)
 *
 * The clips MediaPrepDeviceTest feeds MediaPrep, MADE ON THE DEVICE with
 * the platform's own encoder and muxer rather than committed as fixtures:
 * the properties that matter (size, turn, frame rate, bitrate, tracks) are
 * then parameters, not a file somebody has to regenerate with a tool this
 * repo does not depend on.
 *
 * A SCENE, NOT NOISE. Each frame is a smooth picture panning sideways under
 * a little grain — what a phone camera hands over: structure an encoder can
 * predict, and sensor noise it keeps only while it has bits to spare. Asked
 * for at 20 Mbit/s, the grain is kept and the clip really is expensive; asked
 * for at 2, it is the first thing to go. The first version of this file fed
 * the encoder pure random luma instead, and the run on a device showed what
 * that proves: nothing. Random luma has no rate at which it fits — the
 * platform's H.264 encoder, asked for 2 Mbit/s, returned 15, and a
 * "300 kbit/s" fixture of hard-edged bars came out at 9 — so a test of
 * whether a requested bitrate governs could only fail, whatever the code did.
 */

package me.nettrash.familyconnect.data.repo

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.MediaMuxer
import android.os.Build
import androidx.annotation.RequiresApi
import java.io.File
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.PI
import kotlin.math.sin
import kotlin.random.Random

object TestClips {

    /**
     * An MP4 of H.264 video, and AAC-LC audio when [audioChannels] is not
     * null. [rotation] is the container's turn, as a phone writes it: a
     * portrait clip is [width] × [height] landscape with 90.
     */
    fun video(
        file: File,
        width: Int,
        height: Int,
        fps: Int,
        frames: Int,
        bitrate: Int,
        rotation: Int = 0,
        /** The grain's amplitude in luma steps; 0 is a clean picture that costs almost nothing. */
        grain: Int = 6,
        audioChannels: Int? = 2,
        audioBitrate: Int = 128_000,
    ) {
        val video = encodeVideo(width, height, fps, frames, bitrate, grain)
        val durationUs = frames * 1_000_000L / fps
        val audio = audioChannels?.let { encodeAudio(it, audioBitrate, durationUs) }
        val muxer = MediaMuxer(file.absolutePath, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
        try {
            muxer.setOrientationHint(rotation)
            val videoTrack = muxer.addTrack(video.format)
            val audioTrack = audio?.let { muxer.addTrack(it.format) }
            muxer.start()
            val all = video.samples.map { videoTrack to it } +
                (audio?.samples.orEmpty().map { checkNotNull(audioTrack) to it })
            val info = MediaCodec.BufferInfo()
            for ((track, sample) in all.sortedBy { it.second.presentationUs }) {
                info.set(0, sample.data.size, sample.presentationUs, sample.flags)
                muxer.writeSampleData(track, ByteBuffer.wrap(sample.data), info)
            }
            muxer.stop()
        } finally {
            muxer.release()
        }
    }

    /**
     * A 16-bit PCM WAV of a sine tone: what a picked lossless file looks like
     * to the probe. [channels] up to six (a plain PCM header, which is how a
     * 5.1 export from an editor arrives) and any [sampleRate] — 96 000 is the
     * hi-res download an AAC encoder cannot take as it is.
     */
    fun wav(file: File, seconds: Int, channels: Int, sampleRate: Int = 44_100) {
        val frames = seconds * sampleRate
        val data = ByteBuffer.allocate(frames * channels * 2).order(ByteOrder.LITTLE_ENDIAN)
        for (frame in 0 until frames) {
            val value = (sin(2 * PI * 440.0 * frame / sampleRate) * 12_000).toInt().toShort()
            repeat(channels) { data.putShort(value) }
        }
        val header = ByteBuffer.allocate(44).order(ByteOrder.LITTLE_ENDIAN).apply {
            put("RIFF".toByteArray()); putInt(36 + data.capacity()); put("WAVE".toByteArray())
            put("fmt ".toByteArray()); putInt(16); putShort(1); putShort(channels.toShort())
            putInt(sampleRate); putInt(sampleRate * channels * 2)
            putShort((channels * 2).toShort()); putShort(16)
            put("data".toByteArray()); putInt(data.capacity())
        }
        RandomAccessFile(file, "rw").use { out ->
            out.setLength(0)
            out.write(header.array())
            out.write(data.array())
        }
    }

    /**
     * A FLAC file of a tone under a little hiss, stereo: the platform's own
     * FLAC encoder, whose stream header (`fLaC` and STREAMINFO) and frames,
     * written one after the other, ARE the file format — there is no
     * container to mux into.
     *
     * [broken] keeps that header and replaces every frame with noise: a file
     * the probe still calls FLAC, and that nothing can decode.
     */
    fun flac(file: File, seconds: Int, sampleRate: Int = 44_100, broken: Boolean = false) {
        val channels = 2
        val format = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_FLAC, sampleRate, channels).apply {
            setInteger(MediaFormat.KEY_FLAC_COMPRESSION_LEVEL, 5)
        }
        val framesPerBuffer = 1_024
        val random = Random(74)
        val audio = encode(format, seconds * sampleRate / framesPerBuffer) { codec, index, buffer ->
            val input = checkNotNull(codec.getInputBuffer(index)).order(ByteOrder.LITTLE_ENDIAN)
            input.clear()
            for (frame in 0 until framesPerBuffer) {
                val t = (buffer * framesPerBuffer + frame).toDouble() / sampleRate
                val value = (sin(2 * PI * 330.0 * t) * 8_000).toInt() + random.nextInt(-200, 201)
                repeat(channels) { input.putShort(value.toShort()) }
            }
            framesPerBuffer * channels * 2 to buffer * framesPerBuffer * 1_000_000L / sampleRate
        }
        check(audio.config.size >= 4 && String(audio.config, 0, 4, Charsets.US_ASCII) == "fLaC") {
            "this device's FLAC encoder did not hand over a stream header"
        }
        file.outputStream().use { out ->
            out.write(audio.config)
            for (sample in audio.samples) {
                out.write(if (broken) random.nextBytes(sample.data.size) else sample.data)
            }
        }
    }

    /** How far the picture travels: its wavelength, and its speed — a slow pan. */
    private const val PAN_PERIOD = 480
    private const val PAN_PIXELS_PER_FRAME = 2

    /**
     * An Ogg file of Opus, mono, 48 kHz — what a browser's voice recording
     * is, and what Android can read and an iPhone cannot. API 29+, which is
     * when MediaMuxer learnt to write the container.
     */
    @RequiresApi(Build.VERSION_CODES.Q)
    fun oggOpus(file: File, seconds: Int, bitrate: Int) {
        val sampleRate = 48_000
        val format = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_OPUS, sampleRate, 1).apply {
            setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
        }
        // 20 ms a buffer: an Opus frame.
        val framesPerBuffer = 960
        val random = Random(74)
        val audio = encode(format, seconds * sampleRate / framesPerBuffer) { codec, index, buffer ->
            val input = checkNotNull(codec.getInputBuffer(index)).order(ByteOrder.LITTLE_ENDIAN)
            input.clear()
            // Hiss, not a tone: Opus spends almost nothing on a sine, and a
            // fixture asked for at 96 kbit/s has to BE about 96 kbit/s.
            repeat(framesPerBuffer) { input.putShort(random.nextInt(-6_000, 6_001).toShort()) }
            framesPerBuffer * 2 to buffer * framesPerBuffer * 1_000_000L / sampleRate
        }
        val muxer = MediaMuxer(file.absolutePath, MediaMuxer.OutputFormat.MUXER_OUTPUT_OGG)
        try {
            val track = muxer.addTrack(audio.format)
            muxer.start()
            val info = MediaCodec.BufferInfo()
            for (sample in audio.samples) {
                info.set(0, sample.data.size, sample.presentationUs, sample.flags)
                muxer.writeSampleData(track, ByteBuffer.wrap(sample.data), info)
            }
            muxer.stop()
        } finally {
            muxer.release()
        }
    }

    private class Sample(val data: ByteArray, val presentationUs: Long, val flags: Int)
    /** [config] is the codec's own header bytes — what a muxer takes from the format instead. */
    private class Encoded(val format: MediaFormat, val samples: List<Sample>, val config: ByteArray)

    private fun encodeVideo(width: Int, height: Int, fps: Int, frames: Int, bitrate: Int, grain: Int): Encoded {
        val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, width, height).apply {
            setInteger(
                MediaFormat.KEY_COLOR_FORMAT,
                MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible,
            )
            setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
            setInteger(MediaFormat.KEY_FRAME_RATE, fps)
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
        }
        // The picture: two slow waves, one along each axis, so there is
        // something in every block and nothing an encoder cannot follow.
        val across = IntArray(PAN_PERIOD) { (56 * sin(2 * PI * it / PAN_PERIOD)).toInt() }
        val down = IntArray(height) { 128 + (40 * sin(2 * PI * it / 211.0)).toInt() }
        // The grain: a pool read from a different offset each row and frame,
        // so no two frames share it — which is what makes it cost bits.
        val random = Random(74)
        val pool = IntArray(width * 3) { if (grain == 0) 0 else random.nextInt(-grain, grain + 1) }
        val row = ByteArray(width)
        return encode(format, frames) { codec, index, frame ->
            val image = checkNotNull(codec.getInputImage(index))
            val luma = image.planes[0]
            val buffer = luma.buffer
            val pan = frame * PAN_PIXELS_PER_FRAME
            for (y in 0 until height) {
                val base = down[y]
                val from = (frame * 7_919 + y * 131) % (pool.size - width)
                for (x in 0 until width) {
                    row[x] = (base + across[(x + pan) % PAN_PERIOD] + pool[from + x]).coerceIn(16, 235).toByte()
                }
                buffer.position(y * luma.rowStride)
                buffer.put(row)
            }
            // Grey, written every frame: an input buffer comes back holding
            // whatever it held last, and chroma nobody wrote is not "none".
            for (plane in listOf(image.planes[1], image.planes[2])) {
                val chroma = plane.buffer
                chroma.position(0)
                chroma.put(ByteArray(chroma.remaining()) { 128.toByte() })
            }
            width * height * 3 / 2 to frame * 1_000_000L / fps
        }
    }

    private fun encodeAudio(channels: Int, bitrate: Int, durationUs: Long): Encoded {
        val sampleRate = 44_100
        val format = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, sampleRate, channels).apply {
            setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectLC)
            setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
        }
        val framesPerBuffer = 1_024
        val buffers = (durationUs * sampleRate / 1_000_000 / framesPerBuffer).toInt()
        return encode(format, buffers) { codec, index, buffer ->
            val input = checkNotNull(codec.getInputBuffer(index)).order(ByteOrder.LITTLE_ENDIAN)
            input.clear()
            for (frame in 0 until framesPerBuffer) {
                val t = (buffer * framesPerBuffer + frame).toDouble() / sampleRate
                val value = (sin(2 * PI * 330.0 * t) * 8_000).toInt().toShort()
                repeat(channels) { input.putShort(value) }
            }
            framesPerBuffer * channels * 2 to buffer * framesPerBuffer * 1_000_000L / sampleRate
        }
    }

    /**
     * Drive an encoder synchronously: [count] inputs from [fill] (which
     * returns their size and time), then end of stream, collecting every
     * encoded sample and the output format the muxer needs.
     */
    private fun encode(
        format: MediaFormat,
        count: Int,
        fill: (codec: MediaCodec, index: Int, ordinal: Int) -> Pair<Int, Long>,
    ): Encoded {
        val codec = MediaCodec.createEncoderByType(checkNotNull(format.getString(MediaFormat.KEY_MIME)))
        try {
            codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            codec.start()
            val samples = mutableListOf<Sample>()
            var header = ByteArray(0)
            var outputFormat: MediaFormat? = null
            val info = MediaCodec.BufferInfo()
            var submitted = 0
            var lastUs = 0L
            while (true) {
                if (submitted <= count) {
                    val index = codec.dequeueInputBuffer(10_000)
                    if (index >= 0) {
                        if (submitted == count) {
                            codec.queueInputBuffer(index, 0, 0, lastUs, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                        } else {
                            val (size, presentationUs) = fill(codec, index, submitted)
                            lastUs = presentationUs
                            codec.queueInputBuffer(index, 0, size, presentationUs, 0)
                        }
                        submitted++
                    }
                }
                val out = codec.dequeueOutputBuffer(info, 10_000)
                if (out == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                    outputFormat = codec.outputFormat
                } else if (out >= 0) {
                    val buffer = checkNotNull(codec.getOutputBuffer(out))
                    val config = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                    val bytes = ByteArray(info.size)
                    buffer.position(info.offset)
                    buffer.get(bytes)
                    if (config) {
                        header += bytes
                    } else if (info.size > 0) {
                        val flags = info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM.inv()
                        samples += Sample(bytes, info.presentationTimeUs, flags)
                    }
                    codec.releaseOutputBuffer(out, false)
                    if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) break
                }
            }
            codec.stop()
            return Encoded(checkNotNull(outputFormat), samples, header)
        } finally {
            codec.release()
        }
    }
}
