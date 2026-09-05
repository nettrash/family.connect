/*
 * PendingAttachmentsMigrationTest.kt
 * Family Connect (Android)
 *
 * v20 → v21: the table that makes a media send survive the app
 * (docs/protocol.md, "Sending on an unreliable network").
 *
 * This migration shipped unregistered, which means it had never run
 * anywhere — not on a device, not in a test — when v21 went out. So the
 * whole of it is unproven ground, and the checks here are the full set
 * rather than the usual new-column spot check: every column AND every
 * index has to byte-match what Room generates for
 * PendingAttachmentEntity, because Room compares both on launch and
 * fallbackToDestructiveMigration is forbidden on this database.
 *
 * Nothing is asserted about existing rows: v21 adds a table and touches
 * no other, and sends in flight at upgrade time were in memory, which
 * does not survive an upgrade either.
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
class PendingAttachmentsMigrationTest {

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
    }

    @After
    fun tearDown() {
        helper.close()
    }

    @Test
    fun aMediaSendCanBeParkedAndReadBackWhole() {
        AppDatabase.MIGRATION_20_21.migrate(db)

        db.execSQL(
            "INSERT INTO pending_attachments (clientMsgId, position, localPath, previewPath, " +
                "mime, kind, sizeBytes, width, height, durationMs, name, latitude, longitude, " +
                "accuracyM, attachmentId) VALUES ('msg-1', 0, '/f/a.jpg', '/f/a-thumb.jpg', " +
                "'image/jpeg', 'photo', 4096, 1920, 1080, NULL, NULL, NULL, NULL, NULL, NULL)",
        )

        db.query(
            "SELECT localId, clientMsgId, position, localPath, previewPath, mime, kind, " +
                "sizeBytes, width, height, attachmentId, posterUploaded, uploadAttempts " +
                "FROM pending_attachments",
        ).use { cursor ->
            cursor.moveToFirst()
            assertThat(cursor.getLong(0)).isGreaterThan(0L)
            assertThat(cursor.getString(1)).isEqualTo("msg-1")
            assertThat(cursor.getInt(2)).isEqualTo(0)
            assertThat(cursor.getString(3)).isEqualTo("/f/a.jpg")
            assertThat(cursor.getString(4)).isEqualTo("/f/a-thumb.jpg")
            assertThat(cursor.getString(5)).isEqualTo("image/jpeg")
            assertThat(cursor.getString(6)).isEqualTo("photo")
            assertThat(cursor.getLong(7)).isEqualTo(4096L)
            assertThat(cursor.getInt(8)).isEqualTo(1920)
            assertThat(cursor.getInt(9)).isEqualTo(1080)
            // The resume key: null = the bytes still have to go up.
            assertThat(cursor.isNull(10)).isTrue()
            assertThat(cursor.getInt(11)).isEqualTo(0)
            assertThat(cursor.getInt(12)).isEqualTo(0)
        }
    }

    @Test
    fun oneAttachmentPerPositionPerMessage() {
        AppDatabase.MIGRATION_20_21.migrate(db)
        val insert =
            "INSERT INTO pending_attachments (clientMsgId, position, localPath, previewPath, " +
                "mime, kind, sizeBytes, width, height, durationMs, name, latitude, longitude, " +
                "accuracyM, attachmentId) VALUES ('msg-1', 0, '/f/a.jpg', NULL, 'image/jpeg', " +
                "'photo', 1, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)"
        db.execSQL(insert)

        // A resumed send that re-parks its items must not double them.
        val duplicated = runCatching { db.execSQL(insert) }

        assertThat(duplicated.isFailure).isTrue()
    }

    @Test
    fun theTableMatchesWhatTheEntityDeclares() {
        AppDatabase.MIGRATION_20_21.migrate(db)

        val fresh = androidx.room.Room.inMemoryDatabaseBuilder(
            RuntimeEnvironment.getApplication(),
            AppDatabase::class.java,
        ).build()
        val expectedColumns: Map<String, String>
        val expectedIndices: Map<String, String>
        try {
            val freshDb = fresh.openHelper.writableDatabase
            expectedColumns = columnsOf(freshDb, "pending_attachments")
            expectedIndices = indicesOf(freshDb, "pending_attachments")
        } finally {
            fresh.close()
        }

        // Room did generate a table to compare against — an empty map on
        // both sides would make this test pass by agreeing about nothing.
        assertThat(expectedColumns).isNotEmpty()
        assertThat(columnsOf(db, "pending_attachments")).isEqualTo(expectedColumns)
        assertThat(indicesOf(db, "pending_attachments")).isEqualTo(expectedIndices)
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

    /** name → "unique|col,col" — Room validates the name and the shape. */
    private fun indicesOf(db: SupportSQLiteDatabase, table: String): Map<String, String> =
        db.query("PRAGMA index_list($table)").use { list ->
            buildMap {
                while (list.moveToNext()) {
                    val name = list.getString(list.getColumnIndexOrThrow("name"))
                    // SQLite's own indices for UNIQUE constraints, which
                    // Room neither creates nor compares.
                    if (name.startsWith("sqlite_autoindex")) continue
                    val unique = list.getInt(list.getColumnIndexOrThrow("unique"))
                    val columns = db.query("PRAGMA index_info($name)").use { info ->
                        buildList {
                            while (info.moveToNext()) {
                                add(info.getString(info.getColumnIndexOrThrow("name")))
                            }
                        }
                    }
                    put(name, "$unique|${columns.joinToString(",")}")
                }
            }
        }
}
