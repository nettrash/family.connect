package me.nettrash.familyconnect.ui.chat

import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class ZzTempLinkBenchTest {

    private val bodies = listOf(
        "ok",
        "see you at 7 tonight, bring the bag",
        "haha that's amazing 😄 tell mum I'll call later ok?",
        "the order number is 1234567890 if you need it",
        "docs are at https://example.com/notes?v=1 and mail nettrash@nettrash.me",
        "call +1 555-123-4567 tonight",
        "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod " +
            "tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim " +
            "veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea " +
            "commodo consequat. Duis aute irure dolor in reprehenderit in voluptate.",
    )

    @Test
    fun bench() {
        // Cold: first ever call in this process (libphonenumber metadata load).
        val cold = System.nanoTime()
        MessageLinks.linkSpans("call +1 555-123-4567 tonight")
        val coldNs = System.nanoTime() - cold
        println("COLD first linkSpans: ${coldNs / 1_000_000.0} ms")

        // Warm up JIT.
        repeat(2000) { for (b in bodies) MessageLinks.linkSpans(b) }

        for (b in bodies) {
            val n = 2000
            val t = System.nanoTime()
            repeat(n) { MessageLinks.linkSpans(b) }
            val per = (System.nanoTime() - t).toDouble() / n
            println("WARM ${"%8.1f".format(per / 1000.0)} us/call  len=${b.length}  :: ${b.take(40)}")
        }
    }

    @Test
    fun falsePositives() {
        for (b in listOf(
            "the order number is 1234567890 if you need it",
            "my code is 483920",
            "flight BA 2490 lands at 18:45",
            "it cost 1500 dollars",
            "meeting 2026-08-21 at 14:30",
            "ref 12 34 56 78",
        )) {
            println("FP '$b' -> " + MessageLinks.linkSpans(b).map { b.substring(it.start, it.end) to it.url })
        }
    }
}
