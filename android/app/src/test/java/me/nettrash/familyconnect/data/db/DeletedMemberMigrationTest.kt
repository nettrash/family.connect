/*
 * DeletedMemberMigrationTest.kt
 * Family Connect (Android)
 *
 * v12 → v13: the `deleted` flag on the roster.
 *
 * Same rules as every migration test here — AppDatabase forbids
 * fallbackToDestructiveMigration, so the SQL owes proof that the rows
 * survive AND that the column Room would have created is the column the
 * statement creates. The second half is the one that bites: a DEFAULT
 * that does not byte-match MemberEntity's @ColumnInfo does not fail a
 * test, it crashes every upgraded install on launch when Room validates
 * the schema.
 *
 * The table is hand-built at its v12 shape rather than opened through
 * Room, because Room's open path checks an identity hash a synthetic file
 * cannot carry, and the risk under test is the statement itself.
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
class DeletedMemberMigrationTest {

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
        // The v12 shape of `members`: avatars (v3), the has-left flag (v9)
        // and birthdays (v12).
        db.execSQL(
            """
            CREATE TABLE members (
                userId INTEGER NOT NULL PRIMARY KEY,
                username TEXT NOT NULL,
                displayName TEXT NOT NULL,
                role TEXT NOT NULL,
                avatarVersion INTEGER NOT NULL DEFAULT 0,
                hasLeft INTEGER NOT NULL DEFAULT 0,
                birthdayMonth INTEGER,
                birthdayDay INTEGER
            )
            """.trimIndent(),
        )
    }

    @After
    fun tearDown() {
        helper.close()
    }

    @Test
    fun `the roster survives the migration with nobody deleted`() {
        db.execSQL("INSERT INTO members VALUES (7, 'anna', 'Anna', 'owner', 3, 0, 3, 14)")
        db.execSQL("INSERT INTO members VALUES (8, 'ben', 'Ben', 'member', 0, 1, NULL, NULL)")

        AppDatabase.MIGRATION_12_13.migrate(db)

        db.query(
            """
            SELECT userId, displayName, avatarVersion, hasLeft, birthdayMonth, deleted
            FROM members ORDER BY userId
            """.trimIndent(),
        ).use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getLong(0)).isEqualTo(7)
            assertThat(cursor.getString(1)).isEqualTo("Anna")
            assertThat(cursor.getLong(2)).isEqualTo(3)
            assertThat(cursor.getInt(3)).isEqualTo(0)
            // Untouched by the new column — a migration that clears a
            // birthday is the exact bug this table has been bitten by.
            assertThat(cursor.getInt(4)).isEqualTo(3)
            // Everybody already in the table has an answer: their account
            // exists.
            assertThat(cursor.getInt(5)).isEqualTo(0)

            cursor.moveToNext()
            assertThat(cursor.getLong(0)).isEqualTo(8)
            // Somebody who LEFT is not somebody who was deleted: the two
            // flags are independent and the old one has to survive.
            assertThat(cursor.getInt(3)).isEqualTo(1)
            assertThat(cursor.getInt(5)).isEqualTo(0)
        }
    }

    @Test
    fun `the new column matches what the entity declares`() {
        AppDatabase.MIGRATION_12_13.migrate(db)

        db.query("PRAGMA table_info(members)").use { cursor ->
            var found = false
            while (cursor.moveToNext()) {
                if (cursor.getString(cursor.getColumnIndexOrThrow("name")) != "deleted") continue
                found = true
                assertThat(cursor.getString(cursor.getColumnIndexOrThrow("type")))
                    .isEqualTo("INTEGER")
                assertThat(cursor.getInt(cursor.getColumnIndexOrThrow("notnull"))).isEqualTo(1)
                // Byte-for-byte what MemberEntity's @ColumnInfo says, or
                // Room's validation rejects every upgraded database.
                assertThat(cursor.getString(cursor.getColumnIndexOrThrow("dflt_value")))
                    .isEqualTo("0")
            }
            assertThat(found).isTrue()
        }
    }

    @Test
    fun `a tombstone written after the migration reads back whole`() {
        AppDatabase.MIGRATION_12_13.migrate(db)

        db.execSQL(
            "INSERT INTO members (userId, username, displayName, role, avatarVersion, hasLeft, deleted) " +
                "VALUES (11, 'junior', 'Deleted account', 'member', 0, 1, 1)",
        )

        db.query(
            "SELECT hasLeft, deleted, avatarVersion, birthdayMonth FROM members WHERE userId = 11",
        ).use { cursor ->
            cursor.moveToFirst()
            // Both flags: a deleted account has also left the family, and
            // the roster filter reads both.
            assertThat(cursor.getInt(0)).isEqualTo(1)
            assertThat(cursor.getInt(1)).isEqualTo(1)
            assertThat(cursor.getLong(2)).isEqualTo(0)
            assertThat(cursor.isNull(3)).isTrue()
        }
    }
}
