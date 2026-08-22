/*
 * AppDatabase.kt
 * Family Connect (Android)
 *
 * Room database, version 4.
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
    ],
    version = 4,
    exportSchema = false,
)
@TypeConverters(Converters::class)
abstract class AppDatabase : RoomDatabase() {

    abstract fun chatDao(): ChatDao
    abstract fun messageDao(): MessageDao
    abstract fun memberDao(): MemberDao

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
    }
}
