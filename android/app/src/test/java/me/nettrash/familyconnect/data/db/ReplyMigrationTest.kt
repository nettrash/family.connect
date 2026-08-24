/*
 * ReplyMigrationTest.kt
 * Family Connect (Android)
 *
 * AppDatabase forbids fallbackToDestructiveMigration — this database is the
 * family's message history — so every migration owes a test that the rows
 * are still there afterwards.
 *
 * Runs the 3→4 SQL against a hand-built v3 `messages` table rather than
 * through Room: Room's open path checks an identity hash a synthetic file
 * cannot carry, and the risk being tested is the SQL itself.
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
class ReplyMigrationTest {

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
        // The v3 shape of `messages`, as Room generated it before replies.
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
                reactionSeq INTEGER NOT NULL DEFAULT 0
            )
            """.trimIndent(),
        )
        // The v3 shape of `chats`; MIGRATION_4_5 alters this table too.
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
                maxReactionSeq INTEGER NOT NULL DEFAULT 0
            )
            """.trimIndent(),
        )
    }

    @After
    fun tearDown() {
        helper.close()
    }

    @Test
    fun `history survives the replies migration with no quote`() {
        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, status) " +
                "VALUES ('a', 1, 42, 7, 'See you at six', 1000, 'SENT')",
        )
        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, status) " +
                "VALUES ('b', 2, 42, 9, 'Six works', 2000, 'SENT')",
        )

        AppDatabase.MIGRATION_3_4.migrate(db)

        db.query(
            "SELECT clientMsgId, body, replyToMessageId, replySenderId, replyExcerpt " +
                "FROM messages ORDER BY createdAt",
        ).use { cursor ->
            assertThat(cursor.count).isEqualTo(2)
            cursor.moveToFirst()
            assertThat(cursor.getString(1)).isEqualTo("See you at six")
            // "Not a reply" is the ABSENCE of a quote — null, not zero.
            assertThat(cursor.isNull(2)).isTrue()
            assertThat(cursor.isNull(3)).isTrue()
            assertThat(cursor.isNull(4)).isTrue()
        }
    }

    @Test
    fun `a quote written after the migration reads back whole`() {
        AppDatabase.MIGRATION_3_4.migrate(db)

        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, status, " +
                "replyToMessageId, replySenderId, replyExcerpt) " +
                "VALUES ('c', 3, 42, 9, 'Six works', 3000, 'SENT', 1, 7, 'See you at six')",
        )

        db.query(
            "SELECT replyToMessageId, replySenderId, replyExcerpt FROM messages WHERE clientMsgId = 'c'",
        ).use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getLong(0)).isEqualTo(1)
            assertThat(cursor.getLong(1)).isEqualTo(7)
            assertThat(cursor.getString(2)).isEqualTo("See you at six")
        }
    }

    /**
     * The check that matters most: Room validates the schema on every open
     * and refuses to start when a MIGRATED database does not match what it
     * would have CREATED. Getting that wrong does not fail a test — it
     * bricks every existing install on launch.
     *
     * Earlier migrations here pair with `@ColumnInfo(defaultValue = "0")`
     * on their entity fields; these three columns deliberately have no
     * default (null IS the value for "not a reply"), so this compares the
     * migrated column definitions against the ones Room generates itself.
     */
    @Test
    fun `the migrated columns match what Room creates from the entity`() {
        AppDatabase.MIGRATION_3_4.migrate(db)
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

        for (name in listOf("replyToMessageId", "replySenderId", "replyExcerpt")) {
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

    /**
     * v5 on top of v4. `editSeq` is NOT NULL with a DEFAULT ("never
     * edited" is 0, so every existing row already has an answer);
     * `editedAt` is nullable with none (there, "never edited" is the
     * absence of a timestamp). Room refuses to open a migrated database
     * whose columns differ from what it would have created, so both are
     * compared against the real thing.
     */
    @Test
    fun `the edit columns match what Room creates from the entity`() {
        AppDatabase.MIGRATION_3_4.migrate(db)
        AppDatabase.MIGRATION_4_5.migrate(db)
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

        for (name in listOf("editSeq", "editedAt")) {
            assertThat(migrated[name]).isNotNull()
            assertThat(migrated[name]).isEqualTo(expected[name])
        }
    }

    /**
     * The SECOND quote level (v10). Same PRAGMA comparison as above: Room
     * refuses to open a migrated database whose columns differ from what it
     * would have created, and getting that wrong bricks every install that
     * upgrades rather than reinstalls.
     */
    @Test
    fun `the second quote level matches what Room creates from the entity`() {
        migrateToLatest()
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

        for (name in listOf(
            "replyParentMessageId",
            "replyParentSenderId",
            "replyParentExcerpt",
        )) {
            assertThat(migrated[name]).isNotNull()
            assertThat(migrated[name]).isEqualTo(expected[name])
        }
    }

    /**
     * A reply that predates two-level quotes keeps its own quote and simply
     * has nothing behind it — null is "there is no second level", which is
     * the normal case rather than a half-set row.
     */
    @Test
    fun `an existing reply survives with no second level`() {
        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, status) " +
                "VALUES ('a', 1, 42, 7, 'Six works', 1000, 'SENT')",
        )
        AppDatabase.MIGRATION_3_4.migrate(db)
        db.execSQL(
            "UPDATE messages SET replyToMessageId = 41, replySenderId = 9, " +
                "replyExcerpt = 'See you at six' WHERE clientMsgId = 'a'",
        )
        migrateFrom4ToLatest()

        db.query(
            "SELECT replyExcerpt, replyParentMessageId, replyParentExcerpt " +
                "FROM messages WHERE clientMsgId = 'a'",
        ).use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("See you at six")
            assertThat(cursor.isNull(1)).isTrue()
            assertThat(cursor.isNull(2)).isTrue()
        }
    }

    /** Every migration from the replies one up to the current schema. */
    private fun migrateToLatest() {
        AppDatabase.MIGRATION_3_4.migrate(db)
        migrateFrom4ToLatest()
    }

    private fun migrateFrom4ToLatest() {
        AppDatabase.MIGRATION_4_5.migrate(db)
        AppDatabase.MIGRATION_5_6.migrate(db)
        AppDatabase.MIGRATION_6_7.migrate(db)
        AppDatabase.MIGRATION_7_8.migrate(db)
        AppDatabase.MIGRATION_9_10.migrate(db)
        AppDatabase.MIGRATION_10_11.migrate(db)
    }

    /**
     * The LOCATION columns (v11). Same PRAGMA comparison as above, and for
     * the same reason: Room refuses to open a migrated database whose
     * columns differ from what it would have created, and getting that
     * wrong does not fail a test — it bricks every install that upgrades.
     *
     * All three are nullable with NO default, because null IS the normal
     * case: every attachment that is not a location has no coordinates,
     * and a `0` default would claim every photo ever sent was taken off
     * the coast of Africa.
     */
    @Test
    fun `the location columns match what Room creates from the entity`() {
        migrateToLatest()
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

        for (name in listOf(
            "attachmentLatitude",
            "attachmentLongitude",
            "attachmentAccuracyM",
        )) {
            assertThat(migrated[name]).isNotNull()
            assertThat(migrated[name]).isEqualTo(expected[name])
        }
    }

    /**
     * A message that predates locations keeps everything it had and simply
     * has no coordinates — which is what "this is not a location" means.
     */
    @Test
    fun `history survives the locations migration`() {
        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, status) " +
                "VALUES ('a', 1, 42, 7, 'Dinner at 7?', 1000, 'SENT')",
        )
        migrateToLatest()

        db.query(
            "SELECT body, attachmentLatitude, attachmentLongitude, attachmentAccuracyM " +
                "FROM messages WHERE clientMsgId = 'a'",
        ).use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("Dinner at 7?")
            assertThat(cursor.isNull(1)).isTrue()
            assertThat(cursor.isNull(2)).isTrue()
            assertThat(cursor.isNull(3)).isTrue()
        }
    }

    /** A location written after the migration reads back whole. */
    @Test
    fun `a location written after the migration reads back whole`() {
        migrateToLatest()
        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, status, " +
                "attachmentId, attachmentKind, attachmentMime, attachmentSize, " +
                "attachmentLatitude, attachmentLongitude, attachmentAccuracyM) " +
                "VALUES ('b', 2, 42, 7, '', 2000, 'SENT', 61, 'location', " +
                "'application/vnd.family-connect.location', 0, 55.7558, 37.6173, 12)",
        )

        db.query(
            "SELECT attachmentKind, attachmentLatitude, attachmentLongitude, attachmentAccuracyM " +
                "FROM messages WHERE clientMsgId = 'b'",
        ).use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("location")
            assertThat(cursor.getDouble(1)).isEqualTo(55.7558)
            assertThat(cursor.getDouble(2)).isEqualTo(37.6173)
            assertThat(cursor.getInt(3)).isEqualTo(12)
        }
    }

    @Test
    fun `history survives the edits migration unedited`() {
        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, status) " +
                "VALUES ('a', 1, 42, 7, 'Dinner at 7?', 1000, 'SENT')",
        )
        AppDatabase.MIGRATION_3_4.migrate(db)
        AppDatabase.MIGRATION_4_5.migrate(db)

        db.query("SELECT body, editSeq, editedAt FROM messages WHERE clientMsgId = 'a'").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("Dinner at 7?")
            // "Never edited" is 0 for the seq and NULL for the stamp.
            assertThat(cursor.getLong(1)).isEqualTo(0)
            assertThat(cursor.isNull(2)).isTrue()
        }
    }
}
