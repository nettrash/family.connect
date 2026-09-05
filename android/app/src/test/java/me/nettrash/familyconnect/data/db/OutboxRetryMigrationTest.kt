/*
 * OutboxRetryMigrationTest.kt
 * Family Connect (Android)
 *
 * v19 → v20: the outbox's own retry schedule (docs/protocol.md, "Sending
 * on an unreliable network").
 *
 * Same rules as every migration test here: the rows survive, and the two
 * new columns byte-match what MessageEntity declares — `sendAttempts`
 * NOT NULL DEFAULT 0 because a stranded row deserves the full budget
 * rather than none, `nextAttemptAt` nullable because "due now" is a real
 * state and 0 would be a timestamp in 1970 that means it by accident. A
 * default that does not byte-match does not fail a test, it crashes every
 * upgraded install when Room validates the schema on launch.
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
class OutboxRetryMigrationTest {

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
        // The v19 shape of `messages`: v16 plus the video flag (v17).
        // v18 and v19 landed on `notes`, not here.
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
                attachmentsJson TEXT,
                callVideo INTEGER NOT NULL DEFAULT 0
            )
            """.trimIndent(),
        )
    }

    @After
    fun tearDown() {
        helper.close()
    }

    @Test
    fun aMessageStrandedByTheUpgradeArrivesWithTheFullBudgetAndIsDueNow() {
        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, " +
                "status) VALUES ('a', NULL, 42, 7, 'Sent as the update landed', 1000, 'SENDING')",
        )

        AppDatabase.MIGRATION_19_20.migrate(db)

        db.query(
            "SELECT body, status, sendAttempts, nextAttemptAt FROM messages WHERE clientMsgId = 'a'",
        ).use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("Sent as the update landed")
            assertThat(cursor.getString(1)).isEqualTo("SENDING")
            // This device has no record of what it already tried, and 0 is
            // the honest answer — it gives the row the full budget rather
            // than stranding it at the limit.
            assertThat(cursor.getInt(2)).isEqualTo(0)
            // Null = due as soon as the outbox next runs.
            assertThat(cursor.isNull(3)).isTrue()
        }
    }

    @Test
    fun theNewColumnsMatchWhatTheEntityDeclares() {
        AppDatabase.MIGRATION_19_20.migrate(db)

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
        for (column in listOf("sendAttempts", "nextAttemptAt")) {
            assertThat(migrated[column]).isNotNull()
            assertThat(migrated[column]).isEqualTo(expected[column])
        }
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
