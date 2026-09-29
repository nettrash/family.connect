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
 * NOISE, by default, is what makes a clip expensive: an encoder cannot
 * squeeze random luma, so a clip asked for at 20 Mbit/s really is about
 * 20 Mbit/s — and rule D will not keep it over a 2 Mbit/s transcode.
 */

package me.nettrash.familyconnect.data.repo

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.MediaMuxer
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
        noise: Boolean = true,
        audioChannels: Int? = 2,
        audioBitrate: Int = 128_000,
    ) {
        val video = encodeVideo(width, height, fps, frames, bitrate, noise)
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

    /** A 16-bit PCM WAV of a sine tone: what a picked lossless file looks like to the probe. */
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

    private class Sample(val data: ByteArray, val presentationUs: Long, val flags: Int)
    private class Encoded(val format: MediaFormat, val samples: List<Sample>)

    private fun encodeVideo(width: Int, height: Int, fps: Int, frames: Int, bitrate: Int, noise: Boolean): Encoded {
        val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, width, height).apply {
            setInteger(
                MediaFormat.KEY_COLOR_FORMAT,
                MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible,
            )
            setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
            setInteger(MediaFormat.KEY_FRAME_RATE, fps)
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
        }
        // A pool of noise twice a frame's size, read from a different offset each
        // frame: every frame differs, and filling one is a row of bulk copies.
        val pool = ByteArray(width * height * 2).also { Random(74).nextBytes(it) }
        return encode(format, frames) { codec, index, frame ->
            val image = checkNotNull(codec.getInputImage(index))
            val luma = image.planes[0]
            val buffer = luma.buffer
            for (row in 0 until height) {
                buffer.position(row * luma.rowStride)
                if (noise) {
                    buffer.put(pool, (frame * 7_919 + row * width) % (pool.size - width), width)
                } else {
                    // Bars drifting sideways: motion an encoder predicts almost for free.
                    buffer.put(ByteArray(width) { ((it + frame * 4) and 0xFF).toByte() })
                }
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
                    if (!config && info.size > 0) {
                        val bytes = ByteArray(info.size)
                        buffer.position(info.offset)
                        buffer.get(bytes)
                        val flags = info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM.inv()
                        samples += Sample(bytes, info.presentationTimeUs, flags)
                    }
                    codec.releaseOutputBuffer(out, false)
                    if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) break
                }
            }
            codec.stop()
            return Encoded(checkNotNull(outputFormat), samples)
        } finally {
            codec.release()
        }
    }
}
