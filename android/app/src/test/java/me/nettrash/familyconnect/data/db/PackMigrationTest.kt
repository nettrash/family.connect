/*
 * PackMigrationTest.kt
 * Family Connect (Android)
 *
 * v29: the family's sticker pack (docs/protocol.md, "Sticker pack").
 *
 * AppDatabase forbids fallbackToDestructiveMigration, so every migration
 * owes a test that what was there is still there afterwards — and a new
 * TABLE owes one that it arrives. This project once shipped a column with
 * no migration (issue #70); two tables with none would be the same mistake
 * with a different error message.
 *
 * Same shape as GoneNoteMigrationTest: the SQL runs against a hand-built
 * v28 database rather than through Room, because Room's open path checks an
 * identity hash a synthetic file cannot carry, and the risk being tested is
 * the statement itself. That the MIGRATED schema matches the entities is
 * the second test's business.
 */

package me.nettrash.familyconnect.data.db

import androidx.sqlite.db.SupportSQLiteDatabase
import androidx.sqlite.db.SupportSQLiteOpenHelper
import androidx.sqlite.db.framework.FrameworkSQLiteOpenHelperFactory
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.testutil.createTestDb
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@RunWith(RobolectricTestRunner::class)
class PackMigrationTest {

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
        // Enough of v28 to prove the history and the wall survive.
        db.execSQL(
            "CREATE TABLE messages (clientMsgId TEXT NOT NULL PRIMARY KEY, body TEXT NOT NULL, attachmentsJson TEXT)",
        )
        db.execSQL("CREATE TABLE goneNotes (noteId INTEGER NOT NULL, PRIMARY KEY(noteId))")
    }

    @After
    fun tearDown() {
        helper.close()
    }

    @Test
    fun `history survives and both pack tables arrive empty`() {
        db.execSQL(
            """INSERT INTO messages VALUES ('abc', '', '[{"id":34,"kind":"photo","mime":"image/jpeg","size":10}]')""",
        )
        db.execSQL("INSERT INTO goneNotes VALUES (12)")

        AppDatabase.MIGRATION_28_29.migrate(db)

        db.query("SELECT clientMsgId, attachmentsJson FROM messages").use { cursor ->
            assertThat(cursor.count).isEqualTo(1)
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("abc")
            // Untouched: a message cached before stickers is still exactly
            // the photo it was, with no flag to say otherwise.
            assertThat(cursor.getString(1)).doesNotContain("sticker")
        }
        db.query("SELECT COUNT(*) FROM goneNotes").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getLong(0)).isEqualTo(1)
        }
        // EMPTY, both: the pack fills from the next resync's full read, and
        // nothing a device upgrading today could know belongs in either.
        for (table in listOf("packItems", "gonePackItems")) {
            db.query("SELECT COUNT(*) FROM $table").use { cursor ->
                cursor.moveToFirst()
                assertThat(cursor.getLong(0)).isEqualTo(0)
            }
        }
    }

    @Test
    fun `the migrated tables have exactly the columns the entities declare`() {
        AppDatabase.MIGRATION_28_29.migrate(db)

        // name -> (type, notnull, pk), as SQLite reports a migrated table.
        fun columns(database: SupportSQLiteDatabase, table: String): Map<String, Triple<String, Int, Int>> =
            database.query("PRAGMA table_info($table)").use { cursor ->
                buildMap {
                    while (cursor.moveToNext()) {
                        put(
                            cursor.getString(1),
                            Triple(cursor.getString(2), cursor.getInt(3), cursor.getInt(5)),
                        )
                    }
                }
            }

        // Against what Room itself creates for a FRESH install — the one
        // comparison that catches a migration and an entity drifting apart,
        // which is precisely what Room's validation rejects on launch.
        val dispatcher = StandardTestDispatcher()
        val fresh = createTestDb(dispatcher)
        try {
            val freshDb = fresh.openHelper.writableDatabase
            for (table in listOf("packItems", "gonePackItems")) {
                assertThat(columns(db, table)).isEqualTo(columns(freshDb, table))
                assertThat(columns(db, table)).isNotEmpty()
            }
        } finally {
            fresh.close()
        }
    }

    @Test
    fun `the migration survives being run twice`() {
        AppDatabase.MIGRATION_28_29.migrate(db)
        // IF NOT EXISTS: a half-applied upgrade that is retried must not
        // throw on the tables it already made.
        AppDatabase.MIGRATION_28_29.migrate(db)

        db.execSQL("INSERT INTO gonePackItems (itemId) VALUES (5)")
        db.execSQL(
            "INSERT INTO packItems (id, addedBy, attachmentJson, label, createdAt, packSeq) " +
                "VALUES (5, 7, '[]', NULL, 1, 12)",
        )
        db.query("SELECT packSeq FROM packItems").use { cursor ->
            assertThat(cursor.count).isEqualTo(1)
        }
    }

    @Test
    fun `a pack written through Room reads back`() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val room = createTestDb(dispatcher)
        try {
            val dao = room.packDao()
            dao.upsert(
                PackItemEntity(
                    id = 5,
                    addedBy = 7,
                    attachmentJson = """[{"id":71,"kind":"photo","mime":"image/webp","size":2048}]""",
                    label = "party cat",
                    createdAt = 1,
                    packSeq = 12,
                ),
            )
            dao.remember(GonePackItemEntity(9))

            val item = dao.findById(5)!!
            assertThat(item.attachment?.id).isEqualTo(71)
            assertThat(item.attachment?.mime).isEqualTo("image/webp")
            assertThat(item.label).isEqualTo("party cat")
            assertThat(dao.isGone(9)).isTrue()
            assertThat(dao.isGone(5)).isFalse()
        } finally {
            room.close()
        }
    }
}
