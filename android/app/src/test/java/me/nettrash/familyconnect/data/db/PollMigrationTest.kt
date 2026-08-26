/*
 * PollMigrationTest.kt
 * Family Connect (Android)
 *
 * v13 → v14: a poll on a message, and the chat's poll cursor.
 *
 * Same rules as every migration test here — AppDatabase forbids
 * fallbackToDestructiveMigration, so the SQL owes proof that the rows
 * survive AND that the columns Room would have created are the columns
 * the statements create. The second half is the one that bites: a
 * DEFAULT that does not byte-match the entity's @ColumnInfo does not
 * fail a test, it crashes every upgraded install on launch when Room
 * validates the schema.
 *
 * The tables are hand-built at their v13 shape rather than opened
 * through Room, because Room's open path checks an identity hash a
 * synthetic file cannot carry, and the risk under test is the statements
 * themselves.
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
class PollMigrationTest {

    private lateinit var helper: SupportSQLiteOpenHelper
    private lateinit var db: SupportSQLiteDatabase

    @Before
    fun setUp() {
        val config = SupportSQLiteOpenHelper.Configuration
            .builder(RuntimeEnvironment.getApplication())
            // Null name = in-memory, so nothing survives between tests.
            .name(null)
            .callback(object : SupportSQLiteOpenHelper.Callback(1) {
                override fun onCreate(db: SupportSQLiteDatabase) = Unit
                override fun onUpgrade(db: SupportSQLiteDatabase, oldVersion: Int, newVersion: Int) = Unit
            })
            .build()
        helper = FrameworkSQLiteOpenHelperFactory().create(config)
        db = helper.writableDatabase
        // The v13 shape of `messages` — everything up to and including
        // locations (v11).
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
                attachmentAccuracyM INTEGER
            )
            """.trimIndent(),
        )
        // The v13 shape of `chats`: the reaction (v2) and edit (v5) cursors.
        db.execSQL(
            """
            CREATE TABLE chats (
                id INTEGER NOT NULL PRIMARY KEY,
                kind TEXT NOT NULL,
                peerUserId INTEGER,
                title TEXT NOT NULL,
                unreadCount INTEGER NOT NULL,
                myLastReadId INTEGER,
                peerLastReadId INTEGER,
                lastMessageBody TEXT,
                lastMessageAt INTEGER,
                lastMessageSenderId INTEGER,
                maxReactionSeq INTEGER NOT NULL DEFAULT 0,
                maxEditSeq INTEGER NOT NULL DEFAULT 0
            )
            """.trimIndent(),
        )
    }

    @After
    fun tearDown() {
        helper.close()
    }

    @Test
    fun historySurvivesTheMigrationCarryingNoPoll() {
        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, " +
                "status, reactionsJson, reactionSeq, editSeq) " +
                "VALUES ('a', 1, 42, 7, 'See you at six', 1000, 'SENT', " +
                "'[{\"user_id\":9,\"emoji\":\"❤️\"}]', 5, 3)",
        )
        db.execSQL(
            "INSERT INTO chats (id, kind, title, unreadCount, maxReactionSeq, maxEditSeq) " +
                "VALUES (42, 'family', 'The Smiths', 2, 12, 7)",
        )

        AppDatabase.MIGRATION_13_14.migrate(db)

        db.query(
            "SELECT body, reactionsJson, reactionSeq, editSeq, pollJson, pollSeq " +
                "FROM messages WHERE clientMsgId = 'a'",
        ).use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("See you at six")
            // Untouched by the new columns — a migration that clears what
            // was already there is the exact bug this table has been
            // bitten by before.
            assertThat(cursor.getString(1)).contains("❤️")
            assertThat(cursor.getLong(2)).isEqualTo(5)
            assertThat(cursor.getLong(3)).isEqualTo(3)
            // "Not a poll" is the ABSENCE of one — null, not "{}".
            assertThat(cursor.isNull(4)).isTrue()
            // And no poll state has ever been applied to it.
            assertThat(cursor.getLong(5)).isEqualTo(0)
        }
        db.query(
            "SELECT maxReactionSeq, maxEditSeq, maxPollSeq FROM chats WHERE id = 42",
        ).use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getLong(0)).isEqualTo(12)
            assertThat(cursor.getLong(1)).isEqualTo(7)
            assertThat(cursor.getLong(2)).isEqualTo(0)
        }
    }

    @Test
    fun aPollWrittenAfterTheMigrationReadsBackWhole() {
        AppDatabase.MIGRATION_13_14.migrate(db)

        val poll = "{\"poll_seq\":88,\"closed\":false," +
            "\"options\":[{\"id\":5,\"text\":\"Pizza\",\"votes\":[7,9]}]}"
        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, " +
                "status, pollJson, pollSeq) VALUES ('b', 2, 42, 7, 'Pizza or pasta?', 2000, " +
                "'SENT', ?, 88)",
            arrayOf<Any>(poll),
        )

        db.query("SELECT body, pollJson, pollSeq FROM messages WHERE clientMsgId = 'b'")
            .use { cursor ->
                cursor.moveToFirst()
                // The QUESTION is the body, and the poll carries no copy.
                assertThat(cursor.getString(0)).isEqualTo("Pizza or pasta?")
                assertThat(cursor.getString(1)).isEqualTo(poll)
                assertThat(cursor.getLong(2)).isEqualTo(88)
            }
    }

    @Test
    fun theNewColumnsMatchWhatTheEntitiesDeclare() {
        AppDatabase.MIGRATION_13_14.migrate(db)

        val fresh = androidx.room.Room.inMemoryDatabaseBuilder(
            RuntimeEnvironment.getApplication(),
            AppDatabase::class.java,
        ).build()
        val expectedMessages: Map<String, String>
        val expectedChats: Map<String, String>
        try {
            expectedMessages = columnsOf(fresh.openHelper.writableDatabase, "messages")
            expectedChats = columnsOf(fresh.openHelper.writableDatabase, "chats")
        } finally {
            fresh.close()
        }
        val migratedMessages = columnsOf(db, "messages")
        val migratedChats = columnsOf(db, "chats")

        for (name in listOf("pollJson", "pollSeq")) {
            assertThat(migratedMessages[name]).isNotNull()
            assertThat(migratedMessages[name]).isEqualTo(expectedMessages[name])
        }
        assertThat(migratedChats["maxPollSeq"]).isNotNull()
        assertThat(migratedChats["maxPollSeq"]).isEqualTo(expectedChats["maxPollSeq"])
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
