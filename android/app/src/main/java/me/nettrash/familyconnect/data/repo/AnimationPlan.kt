/*
 * AnimationPlan.kt
 * Family Connect (Android)
 *
 * An animated picture sent in a chat as a silent looping video (issue #96,
 * docs/protocol.md "Animations"): the TIMING half — which source frame is
 * shown when, how long the clip is, its size and its bitrate. The port of
 * `fc_text::animation` (web/text/src/animation.rs), held to the reference's
 * vectors (animation-plan-vectors.json, AnimationPlanVectorsTest), because
 * four encoders have to make the same clip of the same picture.
 *
 * THE GRID. The animation is SAMPLED thirty times a second: at each tick the
 * frame showing is the last one that has started, and ticks in a row that
 * show the same frame are ONE video frame, that many ticks long. A held
 * frame stays held, nothing runs faster than 30 a second, and a slow
 * animation is never given frames it did not have (rule B).
 *
 * THE SHORTEST FRAME. A frame stated at 10 ms or less is shown for 100 ms —
 * what every browser does with the many GIFs that say "0".
 *
 * Decoding the frames is AnimatedPicture's; encoding the video is
 * AnimationEncoder's. Integer arithmetic throughout, exactly as the
 * reference: nothing here may depend on a floating-point library.
 *
 * Plain Kotlin, nothing from android.*.
 */

package me.nettrash.familyconnect.data.repo

object AnimationPlan {

    /** Samples a second: the grid the animation is read on, and the timescale every plan counts in. */
    const val TICKS_PER_SECOND: Int = 30

    /** A frame stated at this or less is shown for [DEFAULT_FRAME_MS]. */
    const val SHORTEST_FRAME_MS: Int = 10

    /** How long such a frame is shown. */
    const val DEFAULT_FRAME_MS: Int = 100

    /** What shows through a transparent pixel: a video has no transparency. */
    val BACKGROUND_RGB: IntArray get() = intArrayOf(0xFF, 0xFF, 0xFF)

    /** [BACKGROUND_RGB] as one opaque ARGB colour. */
    const val BACKGROUND_ARGB: Int = 0xFFFFFFFF.toInt()

    /** A GIF frame's delay, stated in hundredths of a second, in milliseconds as stated. */
    fun gifDelayMs(centiseconds: Int): Long = (centiseconds.toLong() and 0xFFFF) * 10

    /**
     * An APNG frame's delay from its `fcTL`: `num ÷ den` seconds, a `den` of 0
     * meaning hundredths, in whole milliseconds rounded half up.
     */
    fun apngDelayMs(num: Int, den: Int): Long {
        val n = num.toLong() and 0xFFFF
        val d = (den.toLong() and 0xFFFF).let { if (it == 0L) 100L else it }
        return (2 * n * 1000 + d) / (2 * d)
    }

    /** How long a frame stated at [statedMs] is shown: as stated, unless it is [SHORTEST_FRAME_MS] or less. */
    fun frameDurationMs(statedMs: Long): Long =
        if (statedMs <= SHORTEST_FRAME_MS) DEFAULT_FRAME_MS.toLong() else statedMs

    /** One frame of the video: which source frame (0-based, composited) and for how many ticks. */
    data class Sample(val frame: Int, val ticks: Int)

    /** What an animation becomes. */
    data class Plan(
        /** Even, the source's orientation, never larger than the source: the profile's size rule. */
        val width: Int,
        val height: Int,
        /** In order; together they last [totalTicks]. */
        val samples: List<Sample>,
        val totalTicks: Int,
        /** [totalTicks] in milliseconds, rounded half up — the upload's `duration_ms`. */
        val durationMs: Long,
        /** The profile's bitrate for the target size at 30 frames a second. */
        val videoBitrate: Long,
    )

    /**
     * The plan for an animation of [width] × [height] whose frames are stated
     * to last [statedMs] each, in the source's order. Null when there is no
     * size or no frame.
     */
    fun plan(width: Long, height: Long, statedMs: List<Long>): Plan? {
        if (width <= 0 || height <= 0 || statedMs.isEmpty()) return null
        val (targetWidth, targetHeight) = MediaPlan.targetSize(width, height)
        if (targetWidth <= 0 || targetHeight <= 0) return null
        // Where each frame starts, in milliseconds; and where the last one ends.
        val starts = LongArray(statedMs.size)
        var total = 0L
        statedMs.forEachIndexed { index, stated ->
            starts[index] = total
            total += frameDurationMs(stated)
        }
        val tps = TICKS_PER_SECOND.toLong()
        // ceil(total × 30 ÷ 1000), at least one tick.
        val totalTicks = ((total * tps + 999) / 1000).coerceAtLeast(1)
        if (totalTicks > Int.MAX_VALUE) return null
        val samples = ArrayList<Sample>()
        var frame = 0
        var tick = 0L
        while (tick < totalTicks) {
            // The frame showing at tick ÷ 30 s: the last one that has started,
            // compared as tick × 1000 against start × 30, in integers.
            while (frame + 1 < starts.size && starts[frame + 1] * tps <= tick * 1000) frame++
            val last = samples.lastOrNull()
            if (last != null && last.frame == frame) {
                samples[samples.lastIndex] = last.copy(ticks = last.ticks + 1)
            } else {
                samples += Sample(frame, 1)
            }
            tick++
        }
        return Plan(
            width = targetWidth.toInt(),
            height = targetHeight.toInt(),
            samples = samples,
            totalTicks = totalTicks.toInt(),
            durationMs = (2 * totalTicks * 1000 + tps) / (2 * tps),
            videoBitrate = MediaPlan.profileVideoBitrate(targetWidth, targetHeight, MediaPlan.MAX_FRAME_RATE),
        )
    }

    /**
     * Where tick [tick] falls, in microseconds — rounded half up, so a muxer
     * that rounds back to a 90 kHz (or any multiple-of-30) timescale lands on
     * the tick exactly: the error is under half a microsecond.
     */
    fun tickToUs(tick: Long): Long = (2 * tick * 1_000_000 + TICKS_PER_SECOND) / (2 * TICKS_PER_SECOND)
}
