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

/**
 * v14 → v15: the two call-record columns (docs/protocol.md, "Voice
 * calls"). Same shape as PollMigrationTest: history survives untouched,
 * a record written afterwards reads back whole, and the migrated columns
 * byte-match what the entity declares — the check Room runs on launch.
 */
@RunWith(RobolectricTestRunner::class)
class CallMigrationTest {

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
        // The v14 shape of `messages` — everything up to and including polls.
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
                pollSeq INTEGER NOT NULL DEFAULT 0
            )
            """.trimIndent(),
        )
    }

    @After
    fun tearDown() {
        helper.close()
    }

    @Test
    fun historySurvivesTheMigrationCarryingNoCall() {
        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, " +
                "status, pollJson, pollSeq) VALUES ('a', 1, 42, 7, 'See you at six', 1000, 'SENT', " +
                "'{\"poll_seq\":88}', 88)",
        )

        AppDatabase.MIGRATION_14_15.migrate(db)

        db.query(
            "SELECT body, pollJson, pollSeq, callOutcome, callDurationSecs FROM messages WHERE clientMsgId = 'a'",
        ).use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("See you at six")
            // Untouched by the new columns.
            assertThat(cursor.getString(1)).contains("88")
            assertThat(cursor.getLong(2)).isEqualTo(88)
            // "Not a call" is the ABSENCE of one — null, both columns.
            assertThat(cursor.isNull(3)).isTrue()
            assertThat(cursor.isNull(4)).isTrue()
        }
    }

    @Test
    fun aRecordWrittenAfterTheMigrationReadsBackWhole() {
        AppDatabase.MIGRATION_14_15.migrate(db)

        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, " +
                "status, callOutcome, callDurationSecs) VALUES ('b', 2, 42, 7, 'Voice call', 2000, " +
                "'SENT', 'completed', 222)",
        )
        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, " +
                "status, callOutcome) VALUES ('c', 3, 42, 7, 'Missed voice call', 3000, 'SENT', 'missed')",
        )

        db.query("SELECT callOutcome, callDurationSecs FROM messages WHERE clientMsgId = 'b'").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("completed")
            assertThat(cursor.getInt(1)).isEqualTo(222)
        }
        db.query("SELECT callOutcome, callDurationSecs FROM messages WHERE clientMsgId = 'c'").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("missed")
            // A missed call was never answered, so it never has a duration.
            assertThat(cursor.isNull(1)).isTrue()
        }
    }

    @Test
    fun theNewColumnsMatchWhatTheEntityDeclares() {
        AppDatabase.MIGRATION_14_15.migrate(db)

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
        for (name in listOf("callOutcome", "callDurationSecs")) {
            assertThat(migrated[name]).isNotNull()
            assertThat(migrated[name]).isEqualTo(expected[name])
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
