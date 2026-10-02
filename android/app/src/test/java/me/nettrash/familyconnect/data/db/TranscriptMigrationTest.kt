/*
 * TranscriptMigrationTest.kt
 * Family Connect (Android)
 *
 * v30: the texts of recordings this member asked for (docs/protocol.md,
 * "Transcripts on request"). A new table owes a test that it arrives, that
 * what was there survives, and that the migrated table is exactly what Room
 * creates for a fresh install — the PackMigrationTest shape.
 */

package me.nettrash.familyconnect.data.db

import androidx.sqlite.db.SupportSQLiteDatabase
import androidx.sqlite.db.SupportSQLiteOpenHelper
import androidx.sqlite.db.framework.FrameworkSQLiteOpenHelperFactory
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.test.StandardTestDispatcher
import me.nettrash.familyconnect.testutil.createTestDb
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@RunWith(RobolectricTestRunner::class)
class TranscriptMigrationTest {

    private lateinit var helper: SupportSQLiteOpenHelper
    private lateinit var db: SupportSQLiteDatabase

    @Before
    fun setUp() {
        val config = SupportSQLiteOpenHelper.Configuration
            .builder(RuntimeEnvironment.getApplication())
            .name(null)
            .callback(object : SupportSQLiteOpenHelper.Callback(1) {
                override fun onCreate(db: SupportSQLiteDatabase) = Unit
                override fun onUpgrade(db: SupportSQLiteDatabase, oldVersion: Int, newVersion: Int) = Unit
            })
            .build()
        helper = FrameworkSQLiteOpenHelperFactory().create(config)
        db = helper.writableDatabase
        // Enough of v29 to prove the history survives.
        db.execSQL(
            "CREATE TABLE messages (clientMsgId TEXT NOT NULL PRIMARY KEY, body TEXT NOT NULL, attachmentsJson TEXT)",
        )
    }

    @After
    fun tearDown() {
        helper.close()
    }

    private fun columns(database: SupportSQLiteDatabase, table: String): Map<String, Triple<String, Int, Int>> =
        database.query("PRAGMA table_info($table)").use { cursor ->
            buildMap {
                while (cursor.moveToNext()) {
                    put(cursor.getString(1), Triple(cursor.getString(2), cursor.getInt(3), cursor.getInt(5)))
                }
            }
        }

    @Test
    fun `history survives and the transcripts table arrives empty`() {
        db.execSQL(
            """INSERT INTO messages VALUES ('abc', '', '[{"id":40,"kind":"audio","mime":"audio/mp4","size":10}]')""",
        )

        AppDatabase.MIGRATION_29_30.migrate(db)

        db.query("SELECT clientMsgId FROM messages").use { cursor ->
            assertThat(cursor.count).isEqualTo(1)
        }
        db.query("SELECT COUNT(*) FROM transcripts").use { cursor ->
            cursor.moveToFirst()
            // Nothing held before the feature was ever asked.
            assertThat(cursor.getLong(0)).isEqualTo(0)
        }
    }

    @Test
    fun `the migrated table has exactly the columns the entity declares`() {
        AppDatabase.MIGRATION_29_30.migrate(db)

        val fresh = createTestDb(StandardTestDispatcher())
        try {
            val freshDb = fresh.openHelper.writableDatabase
            assertThat(columns(db, "transcripts")).isNotEmpty()
            assertThat(columns(db, "transcripts")).isEqualTo(columns(freshDb, "transcripts"))
        } finally {
            fresh.close()
        }
    }

    @Test
    fun `the migration survives being run twice`() {
        AppDatabase.MIGRATION_29_30.migrate(db)
        AppDatabase.MIGRATION_29_30.migrate(db)

        db.execSQL(
            "INSERT INTO transcripts (attachmentId, text, language, source, hidden) VALUES (40, '', NULL, 'stored', 0)",
        )
        db.query("SELECT COUNT(*) FROM transcripts").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getLong(0)).isEqualTo(1)
        }
    }
}
