/*
 * AvatarMigrationTest.kt
 * Family Connect (Android)
 *
 * AppDatabase forbids fallbackToDestructiveMigration — this database is
 * the family's message history, and a schema bump that quietly drops it
 * is data loss the user notices. So every migration owes a test that the
 * rows are still there afterwards.
 *
 * This runs the 2→3 SQL against a hand-built v2 `members` table rather
 * than through Room: Room's own open path checks an identity hash that a
 * synthetic v2 file cannot carry, and the risk being tested here is the
 * SQL statement itself.
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
class AvatarMigrationTest {

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
                override fun onUpgrade(db: SupportSQLiteDatabase, old: Int, new: Int) = Unit
            })
            .build()
        helper = FrameworkSQLiteOpenHelperFactory().create(config)
        db = helper.writableDatabase
        // The v2 shape of `members`, as Room generated it before avatars.
        db.execSQL(
            """
            CREATE TABLE members (
                userId INTEGER NOT NULL PRIMARY KEY,
                username TEXT NOT NULL,
                displayName TEXT NOT NULL,
                role TEXT NOT NULL
            )
            """.trimIndent(),
        )
    }

    @After
    fun tearDown() {
        helper.close()
    }

    @Test
    fun `the roster survives the avatars migration with version zero`() {
        db.execSQL("INSERT INTO members VALUES (7, 'anna', 'Anna', 'owner')")
        db.execSQL("INSERT INTO members VALUES (8, 'ben', 'Ben', 'member')")

        AppDatabase.MIGRATION_2_3.migrate(db)

        db.query("SELECT userId, displayName, role, avatarVersion FROM members ORDER BY userId")
            .use { cursor ->
                assertThat(cursor.count).isEqualTo(2)
                cursor.moveToFirst()
                assertThat(cursor.getLong(0)).isEqualTo(7)
                assertThat(cursor.getString(1)).isEqualTo("Anna")
                assertThat(cursor.getString(2)).isEqualTo("owner")
                // Nobody has a picture until the roster is refreshed.
                assertThat(cursor.getLong(3)).isEqualTo(0)
                cursor.moveToNext()
                assertThat(cursor.getString(1)).isEqualTo("Ben")
                assertThat(cursor.getLong(3)).isEqualTo(0)
            }
    }

    @Test
    fun `rows inserted after the migration default to no picture`() {
        AppDatabase.MIGRATION_2_3.migrate(db)

        // The DEFAULT has to match MemberEntity's @ColumnInfo(defaultValue),
        // or Room's schema validation rejects a migrated database.
        db.execSQL("INSERT INTO members (userId, username, displayName, role) VALUES (9, 'cara', 'Cara', 'member')")

        db.query("SELECT avatarVersion FROM members WHERE userId = 9").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getLong(0)).isEqualTo(0)
        }
    }
}
