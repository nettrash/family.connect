/*
 * AppDatabaseMigrationsTest.kt
 * Family Connect (Android) — tests
 *
 * The migration list is contiguous and ends at the schema's version. The
 * regression this pins: Room 19 -> 21 once shipped with MIGRATION_20_21
 * written, reviewed and never REGISTERED, and every upgraded install
 * crash-looped. A migration that is not in ALL_MIGRATIONS does not exist.
 */

package me.nettrash.familyconnect.data.db

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class AppDatabaseMigrationsTest {

    @Test
    fun `every migration is registered, in order, with no gap`() {
        val migrations = AppDatabase.ALL_MIGRATIONS
        migrations.forEachIndexed { index, migration ->
            assertThat(migration.startVersion).isEqualTo(index + 1)
            assertThat(migration.endVersion).isEqualTo(index + 2)
        }
    }

    @Test
    fun `the mentions migration is the last one and reaches the current schema`() {
        val last = AppDatabase.ALL_MIGRATIONS.last()
        assertThat(last).isSameInstanceAs(AppDatabase.MIGRATION_22_23)
        assertThat(last.endVersion).isEqualTo(23)
        // And the threads one is still registered right before it.
        assertThat(AppDatabase.ALL_MIGRATIONS[AppDatabase.ALL_MIGRATIONS.size - 2])
            .isSameInstanceAs(AppDatabase.MIGRATION_21_22)
    }
}
