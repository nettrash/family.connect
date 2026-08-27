/*
 * AppDatabase.kt
 * Family Connect (Android)
 *
 * Room database, version 18.
 *
 * MIGRATION POLICY: fallbackToDestructiveMigration is FORBIDDEN on this
 * database. It holds the family's message history — the only local copy
 * on this device between resyncs — and silently dropping it on a schema
 * bump is data loss the user notices. Every future version bump ships a
 * real Migration.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Db/AppDatabase.swift
 * (the SwiftData ModelContainer).
 */

package me.nettrash.familyconnect.data.db

import androidx.room.Database
import androidx.room.RoomDatabase
import androidx.room.TypeConverters
import androidx.room.migration.Migration
import androidx.sqlite.db.SupportSQLiteDatabase
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Session-teardown seam: repositories depend on this one-method wipe
 * instead of the whole database, so plain-JVM tests can record the call
 * without Room.
 */
fun interface LocalDataWiper {
    suspend fun wipeAll()
}

@Database(
    entities = [
        ChatEntity::class,
        MessageEntity::class,
        MemberEntity::class,
        NoteEntity::class,
    ],
    version = 18,
    exportSchema = false,
)
@TypeConverters(Converters::class)
abstract class AppDatabase : RoomDatabase() {

    abstract fun chatDao(): ChatDao
    abstract fun messageDao(): MessageDao
    abstract fun memberDao(): MemberDao
    abstract fun noteDao(): NoteDao

    /** Logout / removed-from-family: drop every table, keep the schema. */
    suspend fun wipeAll() = withContext(Dispatchers.IO) {
        clearAllTables()
    }

    companion object {

        /**
         * v2: emoji reactions. The DEFAULTs match the entities'
         * @ColumnInfo(defaultValue) so a migrated schema and a fresh
         * install validate identically.
         */
        val MIGRATION_1_2: Migration = object : Migration(1, 2) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE messages ADD COLUMN reactionsJson TEXT")
                db.execSQL("ALTER TABLE messages ADD COLUMN reactionSeq INTEGER NOT NULL DEFAULT 0")
                db.execSQL("ALTER TABLE chats ADD COLUMN maxReactionSeq INTEGER NOT NULL DEFAULT 0")
            }
        }

        /** v3: profile pictures — the roster's per-member avatar version. */
        val MIGRATION_2_3: Migration = object : Migration(2, 3) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE members ADD COLUMN avatarVersion INTEGER NOT NULL DEFAULT 0")
            }
        }

        /**
         * v4: replies. All three columns are nullable with no default —
         * "not a reply" is the absence of a quote, not a zero — so they
         * match the entity, where they default to null in Kotlin rather
         * than through @ColumnInfo.
         */
        val MIGRATION_3_4: Migration = object : Migration(3, 4) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE messages ADD COLUMN replyToMessageId INTEGER")
                db.execSQL("ALTER TABLE messages ADD COLUMN replySenderId INTEGER")
                db.execSQL("ALTER TABLE messages ADD COLUMN replyExcerpt TEXT")
            }
        }

        /** v6: the family board. */
        val MIGRATION_5_6: Migration = object : Migration(5, 6) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL(
                    """
                    CREATE TABLE IF NOT EXISTS notes (
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
        }

        /**
         * v5: editing. `editSeq` carries a DEFAULT because it is NOT NULL
         * ("never edited" is 0, so every existing row already has an
         * answer); `editedAt` does not, because "never edited" there is
         * the absence of a timestamp. Both match their @ColumnInfo.
         */
        val MIGRATION_4_5: Migration = object : Migration(4, 5) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE messages ADD COLUMN editSeq INTEGER NOT NULL DEFAULT 0")
                db.execSQL("ALTER TABLE messages ADD COLUMN editedAt INTEGER")
                db.execSQL("ALTER TABLE chats ADD COLUMN maxEditSeq INTEGER NOT NULL DEFAULT 0")
            }
        }

        /**
         * v7: photos and videos. The two NOT NULL columns carry DEFAULTs
         * because an existing message has an answer for them ("no bytes",
         * "no preview"); the rest are nullable because their absence is
         * the meaning — a message with no attachment has no width.
         */
        val MIGRATION_6_7: Migration = object : Migration(6, 7) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE messages ADD COLUMN attachmentId INTEGER")
                db.execSQL("ALTER TABLE messages ADD COLUMN attachmentKind TEXT")
                db.execSQL("ALTER TABLE messages ADD COLUMN attachmentMime TEXT")
                db.execSQL("ALTER TABLE messages ADD COLUMN attachmentSize INTEGER NOT NULL DEFAULT 0")
                db.execSQL("ALTER TABLE messages ADD COLUMN attachmentWidth INTEGER")
                db.execSQL("ALTER TABLE messages ADD COLUMN attachmentHeight INTEGER")
                db.execSQL("ALTER TABLE messages ADD COLUMN attachmentDurationMs INTEGER")
                db.execSQL(
                    "ALTER TABLE messages ADD COLUMN attachmentHasPreview INTEGER NOT NULL DEFAULT 0",
                )
            }
        }

        /**
         * v10: the second level of a quote.
         *
         * Three nullable columns with NO default, exactly like the first
         * level's: null IS "there is nothing behind this quote", which is
         * the normal case — the quoted message was not itself a reply, or
         * its own parent has since been swept by retention.
         */
        /**
         * v11: the fifth attachment kind, and the first with no bytes.
         *
         * Three nullable columns with NO default, because null IS the
         * normal case: every attachment that is not a location has no
         * coordinates, and a `0` default would be a claim that every photo
         * ever sent was taken off the coast of Africa.
         */
        val MIGRATION_10_11: Migration = object : Migration(10, 11) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE messages ADD COLUMN attachmentLatitude REAL")
                db.execSQL("ALTER TABLE messages ADD COLUMN attachmentLongitude REAL")
                db.execSQL("ALTER TABLE messages ADD COLUMN attachmentAccuracyM INTEGER")
            }
        }

        /**
         * v12: birthdays on the roster.
         *
         * Two nullable columns with NO default, because absence IS the
         * meaning: nobody who was already in the table has told us a
         * birthday, and there is no month 0 for a default to claim they
         * had. They are written and cleared together — see
         * MemberDao.setBirthday.
         */
        val MIGRATION_11_12: Migration = object : Migration(11, 12) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE members ADD COLUMN birthdayMonth INTEGER")
                db.execSQL("ALTER TABLE members ADD COLUMN birthdayDay INTEGER")
            }
        }

        /**
         * v13: an account that no longer exists.
         *
         * NOT NULL with a DEFAULT because every existing row has an
         * answer — anybody already in the table is somebody whose account
         * was still there — and the default byte-matches
         * MemberEntity.deleted's @ColumnInfo so a migrated schema and a
         * fresh one validate identically.
         *
         * Deliberately a column beside `hasLeft` rather than a new value
         * of it: a deleted account has also left, but "left" is
         * reversible and "deleted" never is (docs/protocol.md, "Deleting
         * an account").
         */
        val MIGRATION_12_13: Migration = object : Migration(12, 13) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE members ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0")
            }
        }

        /**
         * v14: polls.
         *
         * `pollJson` is nullable with NO default, because absence IS the
         * meaning: every message already in the table is not a poll, and
         * "{}" would be a poll with no options. `pollSeq` and the chat's
         * `maxPollSeq` are NOT NULL with a DEFAULT of 0, because an
         * existing row has an answer for both — no poll state has ever
         * been applied — and the defaults byte-match the entities'
         * @ColumnInfo, without which Room's validation rejects every
         * upgraded database on launch.
         */
        val MIGRATION_13_14: Migration = object : Migration(13, 14) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE messages ADD COLUMN pollJson TEXT")
                db.execSQL("ALTER TABLE messages ADD COLUMN pollSeq INTEGER NOT NULL DEFAULT 0")
                db.execSQL("ALTER TABLE chats ADD COLUMN maxPollSeq INTEGER NOT NULL DEFAULT 0")
            }
        }

        /**
         * v15: voice-call records.
         *
         * Two nullable columns with NO default, because absence IS the
         * meaning: every message already in the table is not a call, and a
         * call record is written once (outcome, duration) and never
         * changes. Mirrors v14's `pollJson` reasoning.
         */
        val MIGRATION_14_15: Migration = object : Migration(14, 15) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE messages ADD COLUMN callOutcome TEXT")
                db.execSQL("ALTER TABLE messages ADD COLUMN callDurationSecs INTEGER")
            }
        }

        /**
         * v16: multiple attachments per message.
         *
         * One nullable TEXT column with NO default, exactly like v14's
         * `pollJson` and for the same reason: absence IS the meaning —
         * every message already in the table carries at most one
         * attachment, which stays in the twelve flat columns and is read
         * through the entity's fallback. Nothing is backfilled: a
         * pre-plurality row IS its flat columns.
         */
        val MIGRATION_15_16: Migration = object : Migration(15, 16) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE messages ADD COLUMN attachmentsJson TEXT")
            }
        }

        /**
         * v17: video calls — one flag on the call record
         * (docs/protocol.md, "Video"). NOT NULL DEFAULT 0, because
         * absence means voice, which is what every existing record was.
         */
        val MIGRATION_16_17: Migration = object : Migration(16, 17) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE messages ADD COLUMN callVideo INTEGER NOT NULL DEFAULT 0")
            }
        }

        /**
         * v18: note sizes (docs/protocol.md, "Board"). NOT NULL DEFAULT
         * 'medium', because every existing note IS medium — that is the
         * size a note had before it could have one — and the default
         * byte-matches NoteEntity.size's @ColumnInfo so a migrated schema
         * and a fresh install validate identically.
         */
        val MIGRATION_17_18: Migration = object : Migration(17, 18) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE notes ADD COLUMN size TEXT NOT NULL DEFAULT 'medium'")
            }
        }

        val MIGRATION_9_10: Migration = object : Migration(9, 10) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE messages ADD COLUMN replyParentMessageId INTEGER")
                db.execSQL("ALTER TABLE messages ADD COLUMN replyParentSenderId INTEGER")
                db.execSQL("ALTER TABLE messages ADD COLUMN replyParentExcerpt TEXT")
            }
        }

        /**
         * v9: departed members are kept, not deleted.
         *
         * NOT NULL with a DEFAULT because every existing row has an
         * answer: anybody already in the table is somebody the roster
         * still lists, so they have not left.
         */
        val MIGRATION_8_9: Migration = object : Migration(8, 9) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE members ADD COLUMN hasLeft INTEGER NOT NULL DEFAULT 0")
            }
        }

        /**
         * v8: the third attachment kind. A file's NAME is its identity —
         * nullable with no default, because a photo has none and "" would
         * be a name that renders as nothing.
         */
        val MIGRATION_7_8: Migration = object : Migration(7, 8) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE messages ADD COLUMN attachmentName TEXT")
            }
        }
    }
}
