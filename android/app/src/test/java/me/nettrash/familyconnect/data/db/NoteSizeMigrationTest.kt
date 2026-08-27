/*
 * NoteSizeMigrationTest.kt
 * Family Connect (Android)
 *
 * v17 → v18: a note's size (docs/protocol.md, "Board"). Same shape as
 * CallVideoMigrationTest: the wall survives untouched — and every note on
 * it reads as MEDIUM, which is the size every note had before there was
 * one — a large note written afterwards reads back whole, and the
 * migrated column byte-matches what the entity declares (the check Room
 * runs on launch, and the one that rejects every upgraded database if the
 * DEFAULT is spelled differently).
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
class NoteSizeMigrationTest {

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
        // The v17 shape of `notes` — unchanged since the board arrived in v6.
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
                boardSeq INTEGER NOT NULL
            )
            """.trimIndent(),
        )
    }

    @After
    fun tearDown() {
        helper.close()
    }

    @Test
    fun anExistingNoteSurvivesTheMigrationAsMedium() {
        db.execSQL(
            "INSERT INTO notes (id, authorId, text, color, x, y, createdAt, updatedAt, boardSeq) " +
                "VALUES (1, 7, 'Milk', 'yellow', 0.42, 0.13, 1000, 1000, 88)",
        )

        AppDatabase.MIGRATION_17_18.migrate(db)

        db.query("SELECT text, color, size, boardSeq FROM notes WHERE id = 1").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("Milk")
            assertThat(cursor.getString(1)).isEqualTo("yellow")
            // Every note written before sizes WAS medium.
            assertThat(cursor.getString(2)).isEqualTo("medium")
            assertThat(cursor.getLong(3)).isEqualTo(88)
        }
    }

    @Test
    fun aLargeNoteWrittenAfterTheMigrationReadsBackWhole() {
        AppDatabase.MIGRATION_17_18.migrate(db)

        db.execSQL(
            "INSERT INTO notes (id, authorId, text, color, size, x, y, createdAt, updatedAt, boardSeq) " +
                "VALUES (2, 7, 'DENTIST 9AM', 'pink', 'large', 0.5, 0.5, 2000, 2000, 89)",
        )

        db.query("SELECT size FROM notes WHERE id = 2").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("large")
        }
    }

    @Test
    fun theNewColumnMatchesWhatTheEntityDeclares() {
        AppDatabase.MIGRATION_17_18.migrate(db)

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
        assertThat(migrated["size"]).isNotNull()
        assertThat(migrated["size"]).isEqualTo(expected["size"])
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
