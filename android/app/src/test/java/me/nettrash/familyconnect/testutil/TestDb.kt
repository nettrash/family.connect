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
 *
 * allowMainThreadQueries for the same reason, one step on: that scheduler
 * runs on the test's own thread, which Robolectric calls the main thread,
 * and Room's main-thread guard sits in beginTransaction — so the first
 * suspending transaction (a page of catch-up, DbTransactions) refused to
 * start in a test, though in the app it runs on Room's own executor. The
 * guard is about the APP's main thread; nothing here is one.
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
        .allowMainThreadQueries()
        .build()
