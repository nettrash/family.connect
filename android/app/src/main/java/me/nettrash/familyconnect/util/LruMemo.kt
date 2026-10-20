/*
 * LruMemo.kt
 * Family Connect (Android)
 *
 * A small least-recently-used memo: the answer to a pure function of its
 * key, kept for the keys asked about most recently.
 *
 * Why it exists: the chat re-reads the SAME stored strings over and over.
 * Every Room emission hands the item builder a fresh copy of each row in
 * the window — a hundred or more — and every row's attachments, reactions,
 * poll and mentions are JSON columns that used to be parsed again on each
 * of them, several times per row (MessageEntity.attachmentList is a getter,
 * and the bubble asks it three different questions). Opening a chat that
 * was away for a while means hundreds of emissions in a row while the
 * catch-up lands, and that parsing was most of what each one cost. The
 * column's text is the whole input, so the parsed value for an equal string
 * is the same value, and handing it back is not a cache that can go stale —
 * it is the same answer, earlier.
 *
 * Plain Kotlin over a LinkedHashMap in access order rather than
 * android.util.LruCache: the codecs that use it are exercised by plain JVM
 * tests, where the android.jar stub (with unitTests.isReturnDefaultValues)
 * would answer every get with null and quietly test nothing.
 *
 * Thread-safe by one lock. The compute runs OUTSIDE it, so two threads
 * missing on one key may both compute — harmless for a pure function, and
 * it keeps a slow parse from serialising every other reader behind it.
 *
 * Only for IMMUTABLE values: everybody asking about an equal key shares the
 * one instance it hands back.
 */

package me.nettrash.familyconnect.util

class LruMemo<K : Any, V : Any>(private val capacity: Int) {

    init {
        require(capacity > 0) { "capacity must be positive" }
    }

    private val map = object : LinkedHashMap<K, V>(16, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<K, V>?): Boolean = size > capacity
    }

    /** How many answers are held right now — for the tests. */
    val size: Int get() = synchronized(map) { map.size }

    fun get(key: K): V? = synchronized(map) { map[key] }

    fun put(key: K, value: V) {
        synchronized(map) { map[key] = value }
    }

    /**
     * The held answer for [key], or [compute]'s — which is kept unless it is
     * null. A null answer (a column that would not parse) is not remembered:
     * it is rare, and remembering it would need a second kind of entry.
     */
    inline fun getOrCompute(key: K, compute: () -> V?): V? {
        get(key)?.let { return it }
        val computed = compute() ?: return null
        put(key, computed)
        return computed
    }

    fun clear() {
        synchronized(map) { map.clear() }
    }
}
