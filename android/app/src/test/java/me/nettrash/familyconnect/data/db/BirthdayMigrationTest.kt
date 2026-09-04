/*
 * BirthdayMigrationTest.kt
 * Family Connect (Android)
 *
 * AppDatabase forbids fallbackToDestructiveMigration — this database is
 * the family's message history, and a schema bump that quietly drops it
 * is data loss the user notices. So every migration owes a test that the
 * rows are still there afterwards.
 *
 * Same shape as AvatarMigrationTest, and for the same reason: the 11→12
 * SQL runs against a hand-built v11 `members` table rather than through
 * Room, because Room's open path checks an identity hash that a synthetic
 * file cannot carry, and the risk being tested is the statement itself.
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
class BirthdayMigrationTest {

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
        // The v11 shape of `members`: avatars (v3) and the has-left flag
        // (v9), which is everything this table had before birthdays.
        db.execSQL(
            """
            CREATE TABLE members (
                userId INTEGER NOT NULL PRIMARY KEY,
                username TEXT NOT NULL,
                displayName TEXT NOT NULL,
                role TEXT NOT NULL,
                avatarVersion INTEGER NOT NULL DEFAULT 0,
                hasLeft INTEGER NOT NULL DEFAULT 0
            )
            """.trimIndent(),
        )
    }

    @After
    fun tearDown() {
        helper.close()
    }

    @Test
    fun `the roster survives the birthdays migration with nobody having one`() {
        db.execSQL("INSERT INTO members VALUES (7, 'anna', 'Anna', 'owner', 3, 0)")
        db.execSQL("INSERT INTO members VALUES (8, 'ben', 'Ben', 'member', 0, 1)")

        AppDatabase.MIGRATION_11_12.migrate(db)

        db.query(
            """
            SELECT userId, displayName, avatarVersion, hasLeft, birthdayMonth, birthdayDay
            FROM members ORDER BY userId
            """.trimIndent(),
        ).use { cursor ->
            assertThat(cursor.count).isEqualTo(2)
            cursor.moveToFirst()
            assertThat(cursor.getString(1)).isEqualTo("Anna")
            // Everything the row already carried is untouched.
            assertThat(cursor.getLong(2)).isEqualTo(3)
            assertThat(cursor.getLong(3)).isEqualTo(0)
            // NULL, not 0: nobody in the table has told us a birthday, and
            // there is no month 0 for a default to claim they had.
            assertThat(cursor.isNull(4)).isTrue()
            assertThat(cursor.isNull(5)).isTrue()
            cursor.moveToNext()
            assertThat(cursor.getString(1)).isEqualTo("Ben")
            // A departed member keeps their flag as well as their name.
            assertThat(cursor.getLong(3)).isEqualTo(1)
            assertThat(cursor.isNull(4)).isTrue()
        }
    }

    @Test
    fun `rows inserted after the migration default to no birthday`() {
        AppDatabase.MIGRATION_11_12.migrate(db)

        // No DEFAULT on either column, matching MemberEntity — a nullable
        // column whose absence IS the meaning must not gain one, or Room's
        // schema validation rejects a migrated database.
        db.execSQL(
            "INSERT INTO members (userId, username, displayName, role) " +
                "VALUES (9, 'cara', 'Cara', 'member')",
        )

        db.query("SELECT birthdayMonth, birthdayDay FROM members WHERE userId = 9").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.isNull(0)).isTrue()
            assertThat(cursor.isNull(1)).isTrue()
        }
    }

    @Test
    fun `a birthday written after the migration reads back as two numbers`() {
        AppDatabase.MIGRATION_11_12.migrate(db)
        db.execSQL("INSERT INTO members VALUES (7, 'anna', 'Anna', 'owner', 0, 0, NULL, NULL)")

        // 29 February: valid here precisely because there is no year for
        // it to fail to exist in (docs/protocol.md, "Birthdays").
        db.execSQL("UPDATE members SET birthdayMonth = 2, birthdayDay = 29 WHERE userId = 7")

        db.query("SELECT birthdayMonth, birthdayDay FROM members WHERE userId = 7").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getInt(0)).isEqualTo(2)
            assertThat(cursor.getInt(1)).isEqualTo(29)
        }
    }
}
