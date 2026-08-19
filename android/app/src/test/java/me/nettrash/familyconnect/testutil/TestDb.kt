/*
 * TestDb.kt
 * Family Connect (Android) — test fixtures
 *
 * In-memory Room with direct (same-thread) executors. Direct executors
 * matter: coroutine tests drive a virtual clock, and Room work hopping
 * to real background threads would race advanceUntilIdle — inline
 * execution keeps every DB operation on the test scheduler.
 */

package me.nettrash.familyconnect.testutil

import androidx.room.Room
import me.nettrash.familyconnect.data.db.AppDatabase
import org.robolectric.RuntimeEnvironment
import java.util.concurrent.Executor

fun createTestDb(): AppDatabase {
    val direct = Executor { it.run() }
    return Room.inMemoryDatabaseBuilder(
        RuntimeEnvironment.getApplication(),
        AppDatabase::class.java,
    )
        .setQueryExecutor(direct)
        .setTransactionExecutor(direct)
        .allowMainThreadQueries()
        .build()
}
