/*
 * AppDatabase.kt
 * Family Connect (Android)
 *
 * Room database, version 1.
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
    version = 1,
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
}
