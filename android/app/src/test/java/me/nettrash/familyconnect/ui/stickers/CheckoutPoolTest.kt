/*
 * CheckoutPoolTest.kt
 * Family Connect (Android)
 *
 * The pool decoded stickers wait in while they are off screen
 * (StickerImage.kt, StickerDrawables): a value is TAKEN out — never shared,
 * because a Drawable has one callback and one set of bounds — given back
 * when its sticker leaves, and the least recently given go first once the
 * byte budget is passed.
 */

package me.nettrash.familyconnect.ui.stickers

import android.graphics.drawable.ColorDrawable
import com.google.common.truth.Truth.assertThat
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class CheckoutPoolTest {

    /** A value whose size is its own number, so the budget arithmetic reads plainly. */
    private class Sized(val bytes: Long)

    private fun pool(budget: Long) = CheckoutPool<String, Sized>(budget) { it.bytes }

    @Test
    fun aTakenValueIsGoneUntilItIsGivenBack() {
        val pool = pool(100)
        val sticker = Sized(10)
        pool.give("a", sticker)
        assertThat(pool.take("a")).isSameInstanceAs(sticker)
        // Taken = on screen: a second asker decodes its own.
        assertThat(pool.take("a")).isNull()
        assertThat(pool.heldBytes).isEqualTo(0)
        pool.give("a", sticker)
        assertThat(pool.take("a")).isSameInstanceAs(sticker)
    }

    @Test
    fun theLeastRecentlyGivenGoFirstPastTheBudget() {
        val pool = pool(25)
        pool.give("a", Sized(10))
        pool.give("b", Sized(10))
        pool.give("c", Sized(10)) // 30 > 25: "a" goes
        assertThat(pool.size).isEqualTo(2)
        assertThat(pool.heldBytes).isEqualTo(20)
        assertThat(pool.take("a")).isNull()
        assertThat(pool.take("b")).isNotNull()
        assertThat(pool.take("c")).isNotNull()
    }

    @Test
    fun givingUnderAHeldKeyReplacesItAndCountsOnce() {
        val pool = pool(100)
        pool.give("a", Sized(10))
        val newer = Sized(30)
        pool.give("a", newer)
        assertThat(pool.size).isEqualTo(1)
        assertThat(pool.heldBytes).isEqualTo(30)
        assertThat(pool.take("a")).isSameInstanceAs(newer)
    }

    @Test
    fun aValueBiggerThanTheWholeBudgetIsNotKept() {
        val pool = pool(10)
        pool.give("small", Sized(5))
        pool.give("huge", Sized(50))
        assertThat(pool.take("huge")).isNull()
        // ...and it did not push the small one out on its way past.
        assertThat(pool.take("small")).isNotNull()
    }

    @Test
    fun stickersArePooledPerBoxEdge() {
        StickerDrawables.clear()
        try {
            val small = ColorDrawable(0)
            StickerDrawables.give(key = 7L, edge = 160, drawable = small)
            // The enlarged view's box never gets the small decode.
            assertThat(StickerDrawables.take(key = 7L, edge = 864)).isNull()
            assertThat(StickerDrawables.take(key = 7L, edge = 160)).isSameInstanceAs(small)
        } finally {
            StickerDrawables.clear()
        }
    }
}
