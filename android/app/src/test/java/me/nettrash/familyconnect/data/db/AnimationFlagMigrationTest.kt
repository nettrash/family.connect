/*
 * AnimationFlagMigrationTest.kt
 * Family Connect (Android)
 *
 * v32 → v33: whether a message's stored attachment set knows the ANIMATION
 * flag (#96; docs/protocol.md, "Animations"). RoundFlagMigrationTest's shape:
 * history survives, every row already held reads as NOT known — a build
 * before #96 wrote each received set without `animation`, which is what the
 * resync repair looks for — and the migrated column byte-matches what the
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
class AnimationFlagMigrationTest {

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
        // Enough of v32's `messages` to prove the history survives.
        db.execSQL(
            """
            CREATE TABLE messages (
                clientMsgId TEXT NOT NULL PRIMARY KEY,
                serverId INTEGER,
                body TEXT NOT NULL,
                attachmentKind TEXT,
                attachmentsJson TEXT
            )
            """.trimIndent(),
        )
    }

    @After
    fun tearDown() {
        helper.close()
    }

    @Test
    fun aCachedVideoSurvivesAndReadsAsNotYetKnown() {
        val json = """[{"id":50,"kind":"video","mime":"video/mp4","size":4000,"width":480,"height":270}]"""
        db.execSQL("INSERT INTO messages VALUES ('s50', 50, 'lol', 'video', '$json')")

        AppDatabase.MIGRATION_32_33.migrate(db)

        db.query("SELECT attachmentsJson, attachmentsKnowAnimation FROM messages WHERE clientMsgId = 's50'").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo(json)
            // Written by a build that knew no `animation`: the repair's to read again.
            assertThat(cursor.getInt(1)).isEqualTo(0)
        }
    }

    @Test
    fun theNewColumnMatchesWhatTheEntityDeclares() {
        AppDatabase.MIGRATION_32_33.migrate(db)

        val fresh = Room.inMemoryDatabaseBuilder(RuntimeEnvironment.getApplication(), AppDatabase::class.java).build()
        val expected: Map<String, String>
        try {
            expected = columnsOf(fresh.openHelper.writableDatabase, "messages")
        } finally {
            fresh.close()
        }
        val migrated = columnsOf(db, "messages")
        assertThat(migrated["attachmentsKnowAnimation"]).isEqualTo("INTEGER|1|0")
        assertThat(migrated["attachmentsKnowAnimation"]).isEqualTo(expected["attachmentsKnowAnimation"])
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
