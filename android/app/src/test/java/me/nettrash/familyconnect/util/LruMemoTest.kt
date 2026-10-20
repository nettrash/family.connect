/*
 * LruMemoTest.kt
 * Family Connect (Android)
 *
 * The memo the stored-JSON codecs and the bubble bodies keep their answers
 * in (util/LruMemo.kt): least recently USED goes first, a null answer is
 * never remembered, and a held answer is handed back without computing.
 */

package me.nettrash.familyconnect.util

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class LruMemoTest {

    @Test
    fun aHeldAnswerIsHandedBackWithoutComputingAgain() {
        val memo = LruMemo<String, List<Int>>(4)
        var computed = 0
        val first = memo.getOrCompute("a") { computed++; listOf(1) }
        val second = memo.getOrCompute("a") { computed++; listOf(2) }
        assertThat(computed).isEqualTo(1)
        assertThat(second).isSameInstanceAs(first)
    }

    @Test
    fun theLeastRecentlyUsedKeyIsTheOneThatGoes() {
        val memo = LruMemo<String, String>(2)
        memo.getOrCompute("a") { "A" }
        memo.getOrCompute("b") { "B" }
        // Touching "a" makes "b" the eldest.
        memo.getOrCompute("a") { error("held") }
        memo.getOrCompute("c") { "C" }
        assertThat(memo.size).isEqualTo(2)
        assertThat(memo.get("a")).isEqualTo("A")
        assertThat(memo.get("b")).isNull()
        assertThat(memo.get("c")).isEqualTo("C")
    }

    @Test
    fun aNullAnswerIsNotRemembered() {
        val memo = LruMemo<String, String>(2)
        var computed = 0
        assertThat(memo.getOrCompute("bad") { computed++; null }).isNull()
        assertThat(memo.getOrCompute("bad") { computed++; null }).isNull()
        assertThat(computed).isEqualTo(2)
        assertThat(memo.size).isEqualTo(0)
    }

    @Test
    fun clearForgetsEverything() {
        val memo = LruMemo<String, String>(2)
        memo.put("a", "A")
        memo.clear()
        assertThat(memo.get("a")).isNull()
    }
}
