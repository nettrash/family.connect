/*
 * CallBackRegistryTest.kt
 * Family Connect (Android)
 *
 * The call-log index over an in-memory store: remember, find, a repeat
 * UUID replaces, the bound holds, garbage reads as empty, clear empties.
 */

package me.nettrash.familyconnect.calls

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class CallBackRegistryTest {

    private class MemoryStore : CallBackStore {
        var value: String? = null
        override fun read(): String? = value
        override fun write(value: String?) { this.value = value }
    }

    @Test
    fun rememberAndFind() {
        val store = MemoryStore()
        val registry = CallBackRegistry(store)
        assertThat(registry.all()).isEmpty()
        registry.remember(CallBackEntry("u1", 42L, 9L, video = false))
        registry.remember(CallBackEntry("u2", 43L, 10L, video = true))
        assertThat(registry.find("u1")).isEqualTo(CallBackEntry("u1", 42L, 9L, false))
        assertThat(registry.find("u2")?.video).isTrue()
        assertThat(registry.find("nope")).isNull()
        // Survives a re-read of the same store.
        assertThat(CallBackRegistry(store).all()).hasSize(2)
    }

    @Test
    fun aRepeatUuidReplacesAndTheBoundHolds() {
        val registry = CallBackRegistry(MemoryStore())
        registry.remember(CallBackEntry("u1", 42L, 9L, false))
        registry.remember(CallBackEntry("u1", 42L, 9L, true))
        assertThat(registry.all()).hasSize(1)
        assertThat(registry.find("u1")?.video).isTrue()
        for (n in 0 until CallBackRegistry.LIMIT + 10) registry.remember(CallBackEntry("x$n", 1L, 1L, false))
        assertThat(registry.all()).hasSize(CallBackRegistry.LIMIT)
        assertThat(registry.find("u1")).isNull()
        assertThat(registry.find("x${CallBackRegistry.LIMIT + 9}")).isNotNull()
    }

    @Test
    fun garbageReadsAsEmptyAndClearEmpties() {
        val store = MemoryStore().apply { value = "not json" }
        val registry = CallBackRegistry(store)
        assertThat(registry.all()).isEmpty()
        registry.remember(CallBackEntry("u1", 42L, 9L, false))
        assertThat(registry.all()).hasSize(1)
        registry.clear()
        assertThat(store.value).isNull()
        assertThat(registry.all()).isEmpty()
    }
}
