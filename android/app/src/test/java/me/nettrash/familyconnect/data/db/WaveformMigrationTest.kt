/*
 * WaveformMigrationTest.kt
 * Family Connect (Android)
 *
 * v31 → v32: a queued voice note's waveform (#79; docs/protocol.md, "A
 * voice note's waveform"). The RoundFlagMigrationTest shape: what is
 * already queued survives and reads as having no waveform — it was queued
 * before there were any — and the migrated column byte-matches what the
 * entity declares (the check Room runs on launch).
 */

package me.nettrash.familyconnect.data.db

import androidx.room.Room
import androidx.sqlite.db.SupportSQLiteDatabase
import androidx.sqlite.db.SupportSQLiteOpenHelper
import androidx.sqlite.db.framework.FrameworkSQLiteOpenHelperFactory
import com.google.common.truth.Truth.assertThat
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@RunWith(RobolectricTestRunner::class)
class WaveformMigrationTest {

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
        // Enough of v31's `pending_attachments` to prove a queued item survives.
        db.execSQL(
            """
            CREATE TABLE pending_attachments (
                localId INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
                clientMsgId TEXT NOT NULL,
                position INTEGER NOT NULL,
                localPath TEXT,
                mime TEXT NOT NULL,
                kind TEXT NOT NULL
            )
            """.trimIndent(),
        )
    }

    @After
    fun tearDown() {
        helper.close()
    }

    @Test
    fun aQueuedNoteSurvivesWithNoWaveform() {
        db.execSQL("INSERT INTO pending_attachments VALUES (1, 'c1', 0, '/x/voice.m4a', 'audio/mp4', 'audio')")

        AppDatabase.MIGRATION_31_32.migrate(db)

        db.query("SELECT localPath, waveform FROM pending_attachments WHERE localId = 1").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("/x/voice.m4a")
            assertThat(cursor.isNull(1)).isTrue()
        }
    }

    @Test
    fun theNewColumnMatchesWhatTheEntityDeclares() {
        AppDatabase.MIGRATION_31_32.migrate(db)

        val fresh = Room.inMemoryDatabaseBuilder(RuntimeEnvironment.getApplication(), AppDatabase::class.java).build()
        val expected: Map<String, String>
        try {
            expected = columnsOf(fresh.openHelper.writableDatabase, "pending_attachments")
        } finally {
            fresh.close()
        }
        val migrated = columnsOf(db, "pending_attachments")
        assertThat(migrated["waveform"]).isEqualTo("TEXT|0|null")
        assertThat(migrated["waveform"]).isEqualTo(expected["waveform"])
    }

    /** name → "type|notnull|default", the three things Room validates. */
    private fun columnsOf(db: SupportSQLiteDatabase, table: String): Map<String, String> =
        db.query("PRAGMA table_info($table)").use { cursor ->
            buildMap {
                while (cursor.moveToNext()) {
                    val name = cursor.getString(cursor.getColumnIndexOrThrow("name"))
                    val type = cursor.getString(cursor.getColumnIndexOrThrow("type"))
                    val notNull = cursor.getInt(cursor.getColumnIndexOrThrow("notnull"))
                    val default = cursor.getString(cursor.getColumnIndexOrThrow("dflt_value"))
                    put(name, "$type|$notNull|$default")
                }
            }
        }
}
