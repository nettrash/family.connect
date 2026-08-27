/*
 * AttachmentsMigrationTest.kt
 * Family Connect (Android)
 *
 * v15 → v16: multiple attachments per message (`attachmentsJson`).
 *
 * Same rules as every migration test here — AppDatabase forbids
 * fallbackToDestructiveMigration, so the SQL owes proof that the rows
 * survive AND that the column Room would have created is the column the
 * statement creates. And one rule of this migration's own: nothing is
 * backfilled — a pre-plurality row IS its twelve flat columns, and the
 * entity's fallback is what reads them.
 *
 * The table is hand-built at its v15 shape rather than opened through
 * Room, because Room's open path checks an identity hash a synthetic
 * file cannot carry, and the risk under test is the statement itself.
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
class AttachmentsMigrationTest {

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
        // The v15 shape of `messages` — everything up to and including
        // voice-call records (v15).
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
                callDurationSecs INTEGER
            )
            """.trimIndent(),
        )
    }

    @After
    fun tearDown() {
        helper.close()
    }

    @Test
    fun aPrePluralityRowSurvivesWithItsFlatAttachmentUntouched() {
        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, " +
                "status, attachmentId, attachmentKind, attachmentMime, attachmentSize, " +
                "attachmentWidth, attachmentHeight, attachmentHasPreview) " +
                "VALUES ('a', 1, 42, 7, '', 1000, 'SENT', 34, 'photo', 'image/jpeg', 182734, " +
                "1600, 1200, 1)",
        )

        AppDatabase.MIGRATION_15_16.migrate(db)

        db.query(
            "SELECT attachmentId, attachmentKind, attachmentWidth, attachmentsJson " +
                "FROM messages WHERE clientMsgId = 'a'",
        ).use { cursor ->
            cursor.moveToFirst()
            // The flat columns are untouched — a migration that clears what
            // was already there is the exact bug this table has been bitten
            // by before.
            assertThat(cursor.getLong(0)).isEqualTo(34)
            assertThat(cursor.getString(1)).isEqualTo("photo")
            assertThat(cursor.getLong(2)).isEqualTo(1600)
            // And NOTHING is backfilled: a pre-plurality row IS its flat
            // columns, and the entity's fallback reads them.
            assertThat(cursor.isNull(3)).isTrue()
        }
    }

    @Test
    fun anAlbumWrittenAfterTheMigrationReadsBackWhole() {
        AppDatabase.MIGRATION_15_16.migrate(db)

        val album = "[{\"id\":34,\"kind\":\"photo\",\"mime\":\"image/jpeg\",\"size\":182734}," +
            "{\"id\":35,\"kind\":\"video\",\"mime\":\"video/mp4\",\"size\":999999}]"
        db.execSQL(
            "INSERT INTO messages (clientMsgId, serverId, chatId, senderId, body, createdAt, " +
                "status, attachmentId, attachmentKind, attachmentMime, attachmentSize, " +
                "attachmentsJson) VALUES ('b', 2, 42, 7, '', 2000, 'SENT', 34, 'photo', " +
                "'image/jpeg', 182734, ?)",
            arrayOf<Any>(album),
        )

        db.query("SELECT attachmentsJson FROM messages WHERE clientMsgId = 'b'").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo(album)
        }
    }

    @Test
    fun theNewColumnMatchesWhatTheEntityDeclares() {
        AppDatabase.MIGRATION_15_16.migrate(db)

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

        assertThat(migrated["attachmentsJson"]).isNotNull()
        assertThat(migrated["attachmentsJson"]).isEqualTo(expected["attachmentsJson"])
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
