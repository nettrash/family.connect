/*
 * NoteContentSeqMigrationTest.kt
 * Family Connect (Android)
 *
 * v18 → v19: the seq a board BADGE counts (issue #53, docs/protocol.md,
 * "Board").
 *
 * Same rules as every migration test here — AppDatabase forbids
 * fallbackToDestructiveMigration, so the SQL owes proof that the rows
 * survive AND that the column Room would have created is the column the
 * statement creates. The second half is the one that bites: a DEFAULT that
 * does not byte-match the entity's @ColumnInfo does not fail a test, it
 * crashes every upgraded install on launch when Room validates the schema.
 *
 * The table is hand-built at its v18 shape rather than opened through Room,
 * because Room's open path checks an identity hash a synthetic file cannot
 * carry, and the risk under test is the statement itself.
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
class NoteContentSeqMigrationTest {

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
        // The v18 shape of `notes`: the board (v6) plus sizes (v18).
        db.execSQL(
            """
            CREATE TABLE notes (
                id INTEGER NOT NULL PRIMARY KEY,
                authorId INTEGER NOT NULL,
                text TEXT NOT NULL,
                color TEXT NOT NULL,
                x REAL NOT NULL,
                y REAL NOT NULL,
                createdAt INTEGER NOT NULL,
                updatedAt INTEGER NOT NULL,
                boardSeq INTEGER NOT NULL,
                size TEXT NOT NULL DEFAULT 'medium'
            )
            """.trimIndent(),
        )
    }

    @After
    fun tearDown() {
        helper.close()
    }

    @Test
    fun theWallSurvivesAndArrivesWithNoContentSeq() {
        db.execSQL(
            "INSERT INTO notes (id, authorId, text, color, x, y, createdAt, updatedAt, " +
                "boardSeq, size) VALUES (12, 7, 'Milk', 'pink', 0.4, 0.6, 1000, 2000, 38, 'large')",
        )

        AppDatabase.MIGRATION_18_19.migrate(db)

        db.query(
            "SELECT text, color, x, y, boardSeq, size, contentSeq FROM notes WHERE id = 12",
        ).use { cursor ->
            cursor.moveToFirst()
            // Untouched by the new column — a migration that clears what
            // was already there is the bug these tests exist for.
            assertThat(cursor.getString(0)).isEqualTo("Milk")
            assertThat(cursor.getString(1)).isEqualTo("pink")
            assertThat(cursor.getDouble(2)).isEqualTo(0.4)
            assertThat(cursor.getDouble(3)).isEqualTo(0.6)
            assertThat(cursor.getLong(4)).isEqualTo(38)
            assertThat(cursor.getString(5)).isEqualTo("large")
            // 0 = "nobody has said when this text was written", which is the
            // truth about a row cached before any server could say. The
            // badge judges these by note id, exactly as it always did.
            assertThat(cursor.getLong(6)).isEqualTo(0)
        }
    }

    @Test
    fun theNewColumnMatchesWhatTheEntityDeclares() {
        AppDatabase.MIGRATION_18_19.migrate(db)

        val fresh = androidx.room.Room.inMemoryDatabaseBuilder(
            RuntimeEnvironment.getApplication(),
            AppDatabase::class.java,
        ).build()
        val expected: Map<String, String>
        try {
            expected = columnsOf(fresh.openHelper.writableDatabase, "notes")
        } finally {
            fresh.close()
        }

        val migrated = columnsOf(db, "notes")
        assertThat(migrated["contentSeq"]).isNotNull()
        assertThat(migrated["contentSeq"]).isEqualTo(expected["contentSeq"])
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
