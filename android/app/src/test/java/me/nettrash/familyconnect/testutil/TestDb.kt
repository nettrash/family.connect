/*
 * TestDb.kt
 * Family Connect (Android) — test fixtures
 *
 * In-memory Room pinned to the test's coroutine scheduler.
 *
 * Why setQueryCoroutineContext(dispatcher) and not direct executors:
 * Room 2.7+ routes *suspend* DAO calls and Flow re-queries through its
 * coroutine context (executors only feed the legacy callback paths), so
 * with default config that work lands on a real background dispatcher.
 * Coroutine tests drive a virtual clock — advanceUntilIdle() sees an
 * empty scheduler queue while Room is still mid-hop on a real thread
 * and returns too early, which makes every background-coroutine
 * assertion a coin flip. Pinning Room to the StandardTestDispatcher
 * keeps the entire pipeline on the test scheduler: fully deterministic.
 */

package me.nettrash.familyconnect.testutil

import androidx.room.Room
import kotlin.coroutines.CoroutineContext
import me.nettrash.familyconnect.data.db.AppDatabase
import org.robolectric.RuntimeEnvironment

fun createTestDb(queryContext: CoroutineContext): AppDatabase =
    Room.inMemoryDatabaseBuilder(
        RuntimeEnvironment.getApplication(),
        AppDatabase::class.java,
    )
        .setQueryCoroutineContext(queryContext)
        .build()
