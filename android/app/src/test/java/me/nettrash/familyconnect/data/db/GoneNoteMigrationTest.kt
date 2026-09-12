/*
 * GoneNoteMigrationTest.kt
 * Family Connect (Android)
 *
 * AppDatabase forbids fallbackToDestructiveMigration — this database is the
 * family's message history, and a schema bump that quietly drops it is data
 * loss the user notices. So every migration owes a test that the rows are
 * still there afterwards, and a new TABLE owes one that it arrives.
 *
 * Same shape as BirthdayMigrationTest: the SQL runs against a hand-built
 * v27 database rather than through Room, because Room's open path checks an
 * identity hash a synthetic file cannot carry, and the risk being tested is
 * the statement itself.
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
class GoneNoteMigrationTest {

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
        // Enough of the v27 `notes` table to prove the wall survives.
        db.execSQL(
            """
            CREATE TABLE notes (
                id INTEGER NOT NULL PRIMARY KEY,
                authorId INTEGER NOT NULL,
                text TEXT NOT NULL,
                color TEXT NOT NULL,
                x REAL NOT NULL,
                y REAL NOT NULL,
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
    fun `the wall survives the gone-notes migration and the table arrives empty`() {
        db.execSQL("INSERT INTO notes VALUES (12, 7, 'Milk', 'yellow', 0.25, 0.5, 11)")

        AppDatabase.MIGRATION_27_28.migrate(db)

        db.query("SELECT id, text FROM notes").use { cursor ->
            assertThat(cursor.count).isEqualTo(1)
            cursor.moveToFirst()
            assertThat(cursor.getLong(0)).isEqualTo(12)
            assertThat(cursor.getString(1)).isEqualTo("Milk")
        }
        // EMPTY, and it has to be: a device upgrading today cannot know which notes it has
        // already been told about, so the set fills from the next tombstone on. That is exactly
        // the guarantee the protocol asks for going forward, and no backfill could invent the past.
        db.query("SELECT COUNT(*) FROM goneNotes").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getLong(0)).isEqualTo(0)
        }
    }

    @Test
    fun `the migration is idempotent enough to survive being run twice`() {
        AppDatabase.MIGRATION_27_28.migrate(db)
        // IF NOT EXISTS, because a half-applied upgrade that is retried must not throw on the
        // table it already made.
        AppDatabase.MIGRATION_27_28.migrate(db)

        db.execSQL("INSERT INTO goneNotes (noteId) VALUES (12)")
        db.query("SELECT noteId FROM goneNotes").use { cursor ->
            assertThat(cursor.count).isEqualTo(1)
        }
    }
}
