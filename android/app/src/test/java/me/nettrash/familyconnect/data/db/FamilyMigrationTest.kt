/*
 * FamilyMigrationTest.kt
 * Family Connect (Android)
 *
 * v34 → v35: several families (#88; docs/protocol.md, "Several families"),
 * and v35 → v36, the roster's `password_reset` that came with them.
 * GroupMigrationTest's shape: every row already held survives — chats not
 * yet told their families, notes, pack items and the roster filed under
 * family 0 for the first `GET /me` to claim — the roster's primary key
 * becomes (family, member), and every migrated column byte-matches what the
 * entities declare (the check Room runs on launch). Never destructive.
 */

package me.nettrash.familyconnect.data.db

import android.database.sqlite.SQLiteConstraintException
import androidx.room.Room
import androidx.sqlite.db.SupportSQLiteDatabase
import androidx.sqlite.db.SupportSQLiteOpenHelper
import androidx.sqlite.db.framework.FrameworkSQLiteOpenHelperFactory
import com.google.common.truth.Truth.assertThat
import org.junit.After
import org.junit.Assert.assertThrows
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@RunWith(RobolectricTestRunner::class)
class FamilyMigrationTest {

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
        // Enough of v34's chats, notes and pack to prove the rows survive —
        // and ALL of v34's roster, because the migration copies it whole.
        db.execSQL("CREATE TABLE chats (id INTEGER NOT NULL PRIMARY KEY, kind TEXT NOT NULL, title TEXT NOT NULL)")
        db.execSQL("CREATE TABLE notes (id INTEGER NOT NULL PRIMARY KEY, text TEXT NOT NULL)")
        db.execSQL("CREATE TABLE packItems (id INTEGER NOT NULL PRIMARY KEY, packSeq INTEGER NOT NULL)")
        db.execSQL(
            """
            CREATE TABLE members (
                userId INTEGER NOT NULL PRIMARY KEY,
                username TEXT NOT NULL,
                displayName TEXT NOT NULL,
                role TEXT NOT NULL,
                avatarVersion INTEGER NOT NULL DEFAULT 0,
                hasLeft INTEGER NOT NULL DEFAULT 0,
                deleted INTEGER NOT NULL DEFAULT 0,
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
    fun everyRowSurvivesFiledForTheFirstMeToClaim() {
        db.execSQL("INSERT INTO chats VALUES (1, 'family', 'The Smiths')")
        db.execSQL("INSERT INTO chats VALUES (42, 'direct', 'Ben')")
        db.execSQL("INSERT INTO notes VALUES (12, 'Milk')")
        db.execSQL("INSERT INTO packItems VALUES (5, 14)")
        db.execSQL("INSERT INTO members VALUES (7, 'anna', 'Anna', 'owner', 3, 0, 0, 3, 14)")
        db.execSQL("INSERT INTO members VALUES (9, 'ben', 'Ben', 'member', 0, 1, 0, NULL, NULL)")

        AppDatabase.MIGRATION_34_35.migrate(db)

        db.query("SELECT id, title, familyIdsJson FROM chats ORDER BY id").use { cursor ->
            assertThat(cursor.count).isEqualTo(2)
            while (cursor.moveToNext()) {
                // Not told yet: null until the next GET /chats says.
                assertThat(cursor.isNull(2)).isTrue()
            }
        }
        db.query("SELECT text, familyId FROM notes").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getString(0)).isEqualTo("Milk")
            assertThat(cursor.getLong(1)).isEqualTo(0L)
        }
        db.query("SELECT packSeq, familyId FROM packItems").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getLong(0)).isEqualTo(14L)
            assertThat(cursor.getLong(1)).isEqualTo(0L)
        }
        db.query(
            "SELECT userId, username, displayName, role, avatarVersion, hasLeft, deleted, " +
                "birthdayMonth, birthdayDay, familyId FROM members ORDER BY userId",
        ).use { cursor ->
            assertThat(cursor.count).isEqualTo(2)
            cursor.moveToFirst()
            assertThat(cursor.getLong(0)).isEqualTo(7L)
            assertThat(cursor.getString(1)).isEqualTo("anna")
            assertThat(cursor.getString(2)).isEqualTo("Anna")
            assertThat(cursor.getString(3)).isEqualTo("owner")
            assertThat(cursor.getLong(4)).isEqualTo(3L)
            assertThat(cursor.getInt(5)).isEqualTo(0)
            assertThat(cursor.getInt(6)).isEqualTo(0)
            assertThat(cursor.getInt(7)).isEqualTo(3)
            assertThat(cursor.getInt(8)).isEqualTo(14)
            assertThat(cursor.getLong(9)).isEqualTo(0L)
            cursor.moveToNext()
            assertThat(cursor.getLong(0)).isEqualTo(9L)
            assertThat(cursor.getInt(5)).isEqualTo(1)
            assertThat(cursor.isNull(7)).isTrue()
            assertThat(cursor.getLong(9)).isEqualTo(0L)
        }
    }

    /** One row per (family, member): the same person in two families, never twice in one. */
    @Test
    fun theRosterIsKeyedByFamilyAndMember() {
        db.execSQL("INSERT INTO members VALUES (7, 'anna', 'Anna', 'owner', 0, 0, 0, NULL, NULL)")
        AppDatabase.MIGRATION_34_35.migrate(db)

        db.execSQL(
            "INSERT INTO members (userId, username, displayName, role, familyId) VALUES (7, 'anna', 'Anna', 'member', 5)",
        )
        db.query("SELECT COUNT(*) FROM members WHERE userId = 7").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getInt(0)).isEqualTo(2)
        }
        assertThrows(SQLiteConstraintException::class.java) {
            db.execSQL(
                "INSERT INTO members (userId, username, displayName, role, familyId) VALUES (7, 'anna', 'Anna', 'member', 5)",
            )
        }
    }

    @Test
    fun theMigratedColumnsMatchWhatTheEntitiesDeclare() {
        AppDatabase.MIGRATION_34_35.migrate(db)

        val fresh = Room.inMemoryDatabaseBuilder(RuntimeEnvironment.getApplication(), AppDatabase::class.java).build()
        try {
            val freshDb = fresh.openHelper.writableDatabase
            assertThat(columnsOf(db, "chats")["familyIdsJson"]).isEqualTo("TEXT|0|null|0")
            assertThat(columnsOf(db, "chats")["familyIdsJson"]).isEqualTo(columnsOf(freshDb, "chats")["familyIdsJson"])
            assertThat(columnsOf(db, "notes")["familyId"]).isEqualTo("INTEGER|1|0|0")
            assertThat(columnsOf(db, "notes")["familyId"]).isEqualTo(columnsOf(freshDb, "notes")["familyId"])
            assertThat(columnsOf(db, "packItems")["familyId"]).isEqualTo("INTEGER|1|0|0")
            assertThat(columnsOf(db, "packItems")["familyId"]).isEqualTo(columnsOf(freshDb, "packItems")["familyId"])
            // The rebuilt roster, whole — primary-key positions included —
            // but for the one column v36 added after it.
            assertThat(columnsOf(db, "members")).isEqualTo(columnsOf(freshDb, "members") - "passwordReset")
            assertThat(columnsOf(db, "members")["familyId"]).isEqualTo("INTEGER|1|0|1")
            assertThat(columnsOf(db, "members")["userId"]).isEqualTo("INTEGER|1|null|2")
        } finally {
            fresh.close()
        }
    }

    /**
     * v36: whether the owner may reset a member's password, per (family,
     * member). Every row held survives as "not told" — null, the old rule —
     * and the roster then byte-matches the entity, whole.
     */
    @Test
    fun thePasswordResetColumnArrivesNotTold() {
        db.execSQL("INSERT INTO members VALUES (9, 'ben', 'Ben', 'member', 0, 0, 0, NULL, NULL)")
        AppDatabase.MIGRATION_34_35.migrate(db)
        AppDatabase.MIGRATION_35_36.migrate(db)

        db.query("SELECT userId, passwordReset FROM members").use { cursor ->
            assertThat(cursor.count).isEqualTo(1)
            cursor.moveToFirst()
            assertThat(cursor.getLong(0)).isEqualTo(9L)
            assertThat(cursor.isNull(1)).isTrue()
        }
        val fresh = Room.inMemoryDatabaseBuilder(RuntimeEnvironment.getApplication(), AppDatabase::class.java).build()
        try {
            val freshDb = fresh.openHelper.writableDatabase
            assertThat(columnsOf(db, "members")["passwordReset"]).isEqualTo("INTEGER|0|null|0")
            assertThat(columnsOf(db, "members")).isEqualTo(columnsOf(freshDb, "members"))
        } finally {
            fresh.close()
        }
    }

    /** The stored form of a chat's families, both ways — null and empty are different answers. */
    @Test
    fun aChatsFamiliesRoundTrip() {
        val listed = chat(familyIdsJson = GroupMembersCodec.encode(listOf(3L, 5L)))
        assertThat(listed.familyIdsJson).isEqualTo("[3,5]")
        assertThat(listed.familyIds).containsExactly(3L, 5L).inOrder()
        assertThat(chat(familyIdsJson = null).familyIds).isNull()
        assertThat(chat(familyIdsJson = GroupMembersCodec.encode(emptyList())).familyIds).isEmpty()
    }

    private fun chat(familyIdsJson: String?) = ChatEntity(
        id = 1,
        kind = "direct",
        peerUserId = 9,
        title = "Ben",
        unreadCount = 0,
        myLastReadId = null,
        peerLastReadId = null,
        lastMessageBody = null,
        lastMessageAt = null,
        lastMessageSenderId = null,
        familyIdsJson = familyIdsJson,
    )

    /** name → "type|notnull|default|pk", the things Room validates. */
    private fun columnsOf(db: SupportSQLiteDatabase, table: String): Map<String, String> =
        db.query("PRAGMA table_info($table)").use { cursor ->
            buildMap {
                while (cursor.moveToNext()) {
                    val name = cursor.getString(cursor.getColumnIndexOrThrow("name"))
                    val type = cursor.getString(cursor.getColumnIndexOrThrow("type"))
                    val notNull = cursor.getInt(cursor.getColumnIndexOrThrow("notnull"))
                    val default = cursor.getString(cursor.getColumnIndexOrThrow("dflt_value"))
                    val pk = cursor.getInt(cursor.getColumnIndexOrThrow("pk"))
                    put(name, "$type|$notNull|$default|$pk")
                }
            }
        }
}
