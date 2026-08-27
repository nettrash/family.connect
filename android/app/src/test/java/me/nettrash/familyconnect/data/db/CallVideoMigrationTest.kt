/*
 * CallVideoMigrationTest.kt
 * Family Connect (Android)
 *
 * v16 → v17: the call record's video flag (docs/protocol.md, "Video").
 * Same shape as CallMigrationTest: history survives untouched — and reads
 * as VOICE, which is what every pre-video record was — a video record
 * written afterwards reads back whole, and the migrated column
 * byte-matches what the entity declares (the check Room runs on launch).
 */

package me.nettrash.familyconnect.data.db

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
class CallVideoMigrationTest {

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
        // The v16 shape of `messages` — everything up to and including
        // voice calls and multi-attachments.
        db.execSQL(
            """
            CREATE TABLE messages (
                clientMsgId TEXT NOT NULL PRIMARY KEY,
                serverId INTEGER,
                chatId INTEGER NOT NULL,
                senderId INTEGER NOT NULL,
                body TEXT NOT NULL,
                createdAt INTEGER NOT NULL,
                status TEXT NOT NULL,
                reactionsJson TEXT,
                reactionSeq INTEGER NOT NULL DEFAULT 0,
                replyToMessageId INTEGER,
                replySenderId INTEGER,
                replyExcerpt TEXT,
                replyParentMessageId INTEGER,
                replyParentSenderId INTEGER,
                replyParentExcerpt TEXT,
                editSeq INTEGER NOT NULL DEFAULT 0,
                editedAt INTEGER,
                attachmentId INTEGER,
                attachmentKind TEXT,
                attachmentMime TEXT,
                attachmentSize INTEGER NOT NULL DEFAULT 0,
                attachmentWidth INTEGER,
                attachmentHeight INTEGER,
                attachmentDurationMs INTEGER,
                attachmentHasPreview INTEGER NOT NULL DEFAULT 0,
                attachmentName TEXT,
                attachmentLatitude REAL,
                attachmentLongitude REAL,
                attachmentAccuracyM INTEGER,
                pollJson TEXT,
                pollSeq INTEGER NOT NULL DEFAULT 0,
                callOutcome TEXT,
                callDurationSecs INTEGER,
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
    fun aVoiceRecordSurvivesTheMigrationAsAVoiceRecord() {
        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, " +
                "status, callOutcome, callDurationSecs) VALUES ('a', 1, 42, 7, 'Voice call', 1000, " +
                "'SENT', 'completed', 222)",
        )

        AppDatabase.MIGRATION_16_17.migrate(db)

        db.query("SELECT callOutcome, callDurationSecs, callVideo FROM messages WHERE clientMsgId = 'a'").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("completed")
            assertThat(cursor.getInt(1)).isEqualTo(222)
            // Every record written before video WAS a voice call.
            assertThat(cursor.getInt(2)).isEqualTo(0)
        }
    }

    @Test
    fun aVideoRecordWrittenAfterTheMigrationReadsBackWhole() {
        AppDatabase.MIGRATION_16_17.migrate(db)

        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, " +
                "status, callOutcome, callDurationSecs, callVideo) VALUES ('b', 2, 42, 7, " +
                "'Video call', 2000, 'SENT', 'completed', 222, 1)",
        )

        db.query("SELECT callOutcome, callVideo FROM messages WHERE clientMsgId = 'b'").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("completed")
            assertThat(cursor.getInt(1)).isEqualTo(1)
        }
    }

    @Test
    fun theNewColumnMatchesWhatTheEntityDeclares() {
        AppDatabase.MIGRATION_16_17.migrate(db)

        val fresh = androidx.room.Room.inMemoryDatabaseBuilder(
            RuntimeEnvironment.getApplication(),
            AppDatabase::class.java,
        ).build()
        val expected: Map<String, String>
        try {
            expected = columnsOf(fresh.openHelper.writableDatabase, "messages")
        } finally {
            fresh.close()
        }
        val migrated = columnsOf(db, "messages")
        assertThat(migrated["callVideo"]).isNotNull()
        assertThat(migrated["callVideo"]).isEqualTo(expected["callVideo"])
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
