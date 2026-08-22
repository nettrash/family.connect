/*
 * AttachmentMigrationTest.kt
 * Family Connect (Android)
 *
 * v6 → v7 (photos and videos) and v7 → v8 (files). Same rules as ReplyMigrationTest: this
 * database holds the family's message history and forbids destructive
 * migration, so the SQL owes proof that the rows survive AND that the
 * migrated columns are byte-for-byte what Room would have created —
 * getting the second one wrong does not fail a test, it bricks every
 * existing install on launch.
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
class AttachmentMigrationTest {

    private lateinit var helper: SupportSQLiteOpenHelper
    private lateinit var db: SupportSQLiteDatabase

    @Before
    fun setUp() {
        val config = SupportSQLiteOpenHelper.Configuration
            .builder(RuntimeEnvironment.getApplication())
            .name(null)
            .callback(object : SupportSQLiteOpenHelper.Callback(1) {
                override fun onCreate(db: SupportSQLiteDatabase) = Unit
                override fun onUpgrade(db: SupportSQLiteDatabase, old: Int, new: Int) = Unit
            })
            .build()
        helper = FrameworkSQLiteOpenHelperFactory().create(config)
        db = helper.writableDatabase
        // The v6 shape of `messages` — everything up to and including edits.
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
                editSeq INTEGER NOT NULL DEFAULT 0,
                editedAt INTEGER
            )
            """.trimIndent(),
        )
    }

    @After
    fun tearDown() {
        helper.close()
    }

    @Test
    fun `history survives the attachment migration carrying no media`() {
        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, status) " +
                "VALUES ('a', 1, 42, 7, 'See you at six', 1000, 'SENT')",
        )

        AppDatabase.MIGRATION_6_7.migrate(db)

        db.query(
            "SELECT body, attachmentId, attachmentKind, attachmentSize, attachmentHasPreview " +
                "FROM messages WHERE clientMsgId = 'a'",
        ).use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("See you at six")
            // "No attachment" is the ABSENCE of one — null, not zero.
            assertThat(cursor.isNull(1)).isTrue()
            assertThat(cursor.isNull(2)).isTrue()
            // The two NOT NULL columns answer for an old row from their
            // defaults: no bytes, no preview.
            assertThat(cursor.getLong(3)).isEqualTo(0)
            assertThat(cursor.getInt(4)).isEqualTo(0)
        }
    }

    @Test
    fun `an attachment written after the migration reads back whole`() {
        AppDatabase.MIGRATION_6_7.migrate(db)

        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, status, " +
                "attachmentId, attachmentKind, attachmentMime, attachmentSize, attachmentWidth, " +
                "attachmentHeight, attachmentDurationMs, attachmentHasPreview) " +
                "VALUES ('b', 2, 42, 7, '', 2000, 'SENT', 34, 'video', 'video/mp4', 12345678, " +
                "1080, 1920, 8400, 1)",
        )

        db.query(
            "SELECT attachmentId, attachmentKind, attachmentMime, attachmentSize, attachmentWidth, " +
                "attachmentHeight, attachmentDurationMs, attachmentHasPreview " +
                "FROM messages WHERE clientMsgId = 'b'",
        ).use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getLong(0)).isEqualTo(34)
            assertThat(cursor.getString(1)).isEqualTo("video")
            assertThat(cursor.getString(2)).isEqualTo("video/mp4")
            assertThat(cursor.getLong(3)).isEqualTo(12345678)
            assertThat(cursor.getInt(4)).isEqualTo(1080)
            assertThat(cursor.getInt(5)).isEqualTo(1920)
            assertThat(cursor.getInt(6)).isEqualTo(8400)
            assertThat(cursor.getInt(7)).isEqualTo(1)
        }
    }

    @Test
    fun `a file name survives the v8 migration`() {
        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, status) " +
                "VALUES ('a', 1, 42, 7, 'See you at six', 1000, 'SENT')",
        )
        AppDatabase.MIGRATION_6_7.migrate(db)
        AppDatabase.MIGRATION_7_8.migrate(db)

        // An existing message has no attachment and therefore no name —
        // null, not "".
        db.query("SELECT attachmentName FROM messages WHERE clientMsgId = 'a'").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.isNull(0)).isTrue()
        }

        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, status, " +
                "attachmentId, attachmentKind, attachmentMime, attachmentSize, attachmentName) " +
                "VALUES ('c', 3, 42, 7, '', 3000, 'SENT', 40, 'file', 'application/pdf', 1536, " +
                "'Rechnung März.pdf')",
        )
        db.query("SELECT attachmentName FROM messages WHERE clientMsgId = 'c'").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("Rechnung März.pdf")
        }
    }

    @Test
    fun `the v8 column matches what Room creates from the entity`() {
        AppDatabase.MIGRATION_6_7.migrate(db)
        AppDatabase.MIGRATION_7_8.migrate(db)
        val migrated = columnsOf(db, "messages")

        val fresh = androidx.room.Room.inMemoryDatabaseBuilder(
            RuntimeEnvironment.getApplication(),
            AppDatabase::class.java,
        ).build()
        val expected = try {
            columnsOf(fresh.openHelper.writableDatabase, "messages")
        } finally {
            fresh.close()
        }

        assertThat(migrated["attachmentName"]).isNotNull()
        assertThat(migrated["attachmentName"]).isEqualTo(expected["attachmentName"])
    }

    @Test
    fun `the migrated columns match what Room creates from the entity`() {
        AppDatabase.MIGRATION_6_7.migrate(db)
        AppDatabase.MIGRATION_7_8.migrate(db)
        val migrated = columnsOf(db, "messages")

        val fresh = androidx.room.Room.inMemoryDatabaseBuilder(
            RuntimeEnvironment.getApplication(),
            AppDatabase::class.java,
        ).build()
        val expected = try {
            columnsOf(fresh.openHelper.writableDatabase, "messages")
        } finally {
            fresh.close()
        }

        val columns = listOf(
            "attachmentId",
            "attachmentKind",
            "attachmentMime",
            "attachmentSize",
            "attachmentWidth",
            "attachmentHeight",
            "attachmentDurationMs",
            "attachmentHasPreview",
        )
        for (name in columns) {
            assertThat(migrated[name]).isNotNull()
            assertThat(migrated[name]).isEqualTo(expected[name])
        }
    }

    /** name → "type|notnull|default", straight out of SQLite. */
    private fun columnsOf(db: SupportSQLiteDatabase, table: String): Map<String, String> {
        val columns = mutableMapOf<String, String>()
        db.query("PRAGMA table_info($table)").use { cursor ->
            val name = cursor.getColumnIndexOrThrow("name")
            val type = cursor.getColumnIndexOrThrow("type")
            val notNull = cursor.getColumnIndexOrThrow("notnull")
            val default = cursor.getColumnIndexOrThrow("dflt_value")
            while (cursor.moveToNext()) {
                columns[cursor.getString(name)] =
                    "${cursor.getString(type)}|${cursor.getInt(notNull)}|${cursor.getString(default)}"
            }
        }
        return columns
    }
}
