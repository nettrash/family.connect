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

import androidx.sqlite.db.SupportSQLiteDatabase
import androidx.sqlite.db.SupportSQLiteOpenHelper
import androidx.sqlite.db.framework.FrameworkSQLiteOpenHelperFactory
import com.google.common.truth.Truth.assertThat
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
class AppDatabaseMigrationsTest {

    @Test
    fun `every migration is registered, in order, with no gap`() {
        val migrations = AppDatabase.ALL_MIGRATIONS
        migrations.forEachIndexed { index, migration ->
            assertThat(migration.startVersion).isEqualTo(index + 1)
            assertThat(migration.endVersion).isEqualTo(index + 2)
        }
    }

    /**
     * The NEWEST step, named. It has to be edited by whoever adds one, which is the point: the
     * generic guard is MigrationCoverageTest (every step from 1 to the version Room stamps is
     * registered), and this is the one that makes a bump a deliberate act rather than a diff
     * nobody read.
     */
    @Test
    fun `the gone-notes migration is the last one and reaches the current schema`() {
        val last = AppDatabase.ALL_MIGRATIONS.last()
        assertThat(last).isSameInstanceAs(AppDatabase.MIGRATION_27_28)
        assertThat(last.endVersion).isEqualTo(28)
        // And the note-lists one is still registered right before it.
        assertThat(AppDatabase.ALL_MIGRATIONS[AppDatabase.ALL_MIGRATIONS.size - 2])
            .isSameInstanceAs(AppDatabase.MIGRATION_26_27)
    }

    /**
     * The last migration actually adds the two columns its notes need,
     * run against a real SQLite rather than read off the source.
     *
     * The regression this pins is mine: `notes.mentionsJson` arrived with
     * the note-mentions work and NO migration at all, with the schema
     * version left at 26 — a fresh install was fine and every upgraded one
     * would have met a schema Room could not verify. The list check above
     * cannot see that, because a column added with no step leaves the list
     * exactly as it was.
     */
    @Test
    @Config(sdk = [34])
    fun `the note-lists migration adds the columns a note now carries`() {
        val configuration = SupportSQLiteOpenHelper.Configuration
            .builder(RuntimeEnvironment.getApplication())
            // In-memory, and the table as it stood BEFORE this step: id
            // and the columns the check reads, which is all the ALTERs
            // need to find.
            .name(null)
            .callback(object : SupportSQLiteOpenHelper.Callback(1) {
                override fun onCreate(db: SupportSQLiteDatabase) {
                    db.execSQL("CREATE TABLE notes (id INTEGER PRIMARY KEY NOT NULL, text TEXT)")
                }

                override fun onUpgrade(db: SupportSQLiteDatabase, from: Int, to: Int) = Unit
            })
            .build()
        val helper = FrameworkSQLiteOpenHelperFactory().create(configuration)
        val db = helper.writableDatabase

        AppDatabase.MIGRATION_26_27.migrate(db)

        val columns = mutableListOf<String>()
        db.query("PRAGMA table_info(notes)").use { cursor ->
            val name = cursor.getColumnIndexOrThrow("name")
            while (cursor.moveToNext()) columns += cursor.getString(name)
        }
        helper.close()

        assertThat(columns).containsAtLeast("mentionsJson", "itemsJson")
    }
}
