/*
 * GroupMigrationTest.kt
 * Family Connect (Android)
 *
 * v33 → v34: groups (#107; docs/protocol.md, "Groups"). ReplyMigrationTest's
 * shape: every chat already held survives with no member list and no admin
 * — the truth for a family, direct or assistant chat — and the two migrated
 * columns byte-match what the entity declares (the check Room runs on
 * launch).
 */

package me.nettrash.familyconnect.data.db

import androidx.room.Room
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
class GroupMigrationTest {

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
        // Enough of v33's `chats` to prove the rows survive.
        db.execSQL(
            """
            CREATE TABLE chats (
                id INTEGER NOT NULL PRIMARY KEY,
                kind TEXT NOT NULL,
                peerUserId INTEGER,
                title TEXT NOT NULL,
                unreadCount INTEGER NOT NULL
            )
            """.trimIndent(),
        )
    }

    @After
    fun tearDown() {
        helper.close()
    }

    @Test
    fun everyChatSurvivesWithNoMembersAndNoAdmin() {
        db.execSQL("INSERT INTO chats VALUES (1, 'family', NULL, 'The Smiths', 2)")
        db.execSQL("INSERT INTO chats VALUES (42, 'direct', 9, 'Ben', 0)")

        AppDatabase.MIGRATION_33_34.migrate(db)

        db.query("SELECT id, kind, title, unreadCount, memberIdsJson, adminUserId FROM chats ORDER BY id").use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getLong(0)).isEqualTo(1L)
            assertThat(cursor.getString(1)).isEqualTo("family")
            assertThat(cursor.getString(2)).isEqualTo("The Smiths")
            assertThat(cursor.getInt(3)).isEqualTo(2)
            assertThat(cursor.isNull(4)).isTrue()
            assertThat(cursor.isNull(5)).isTrue()
            cursor.moveToNext()
            assertThat(cursor.getLong(0)).isEqualTo(42L)
            assertThat(cursor.isNull(4)).isTrue()
            assertThat(cursor.isNull(5)).isTrue()
        }
    }

    @Test
    fun theNewColumnsMatchWhatTheEntityDeclares() {
        AppDatabase.MIGRATION_33_34.migrate(db)

        val fresh = Room.inMemoryDatabaseBuilder(RuntimeEnvironment.getApplication(), AppDatabase::class.java).build()
        val expected: Map<String, String>
        try {
            expected = columnsOf(fresh.openHelper.writableDatabase, "chats")
        } finally {
            fresh.close()
        }
        val migrated = columnsOf(db, "chats")
        assertThat(migrated["memberIdsJson"]).isEqualTo("TEXT|0|null")
        assertThat(migrated["adminUserId"]).isEqualTo("INTEGER|0|null")
        assertThat(migrated["memberIdsJson"]).isEqualTo(expected["memberIdsJson"])
        assertThat(migrated["adminUserId"]).isEqualTo(expected["adminUserId"])
    }

    /** The stored form of a group's member list, both ways. */
    @Test
    fun theMemberListRoundTrips() {
        assertThat(GroupMembersCodec.encode(listOf(7, 9, 11))).isEqualTo("[7,9,11]")
        assertThat(GroupMembersCodec.decode("[7,9,11]")).containsExactly(7L, 9L, 11L).inOrder()
        assertThat(GroupMembersCodec.encode(null)).isNull()
        assertThat(GroupMembersCodec.decode(null)).isEmpty()
        assertThat(GroupMembersCodec.decode("[]")).isEmpty()
        assertThat(GroupMembersCodec.decode("not a list")).isEmpty()
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
