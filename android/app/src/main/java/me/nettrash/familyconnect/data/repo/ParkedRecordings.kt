/*
 * ParkedRecordings.kt
 * Family Connect (Android)
 *
 * Voice messages that were not sent (docs/audio-video-messages-2026-10-04.md,
 * S2.8, #79 Phase 0).
 *
 * A recording stopped by something other than the person — a call, the app
 * leaving the screen, leaving the chat, the recorder failing, another app
 * taking the audio — used to be lost, or worse, left running. It is now
 * PARKED here: the file, its length, the reply it was recorded under and its
 * caption, per chat, until the person sends it or deletes it from the "Voice
 * message not sent" row. A recording cannot be made again, which is why this
 * exists for recordings only — a photo can be picked again.
 *
 * Where it lives: the bytes in `filesDir/parked-recordings`, which the system
 * does not reclaim (MediaStaging's reason for the outbox), and the index in
 * SettingsRepository, so both survive the app being closed. Both go at
 * sign-out — the index with the settings, the files with the session wipe
 * (AppModule's LocalDataWiper) — and a park that finishes after a session
 * has ended is dropped, its file deleted ([SessionEpoch]): nothing recorded
 * in one account may surface in the next. Files no entry names, and entries
 * whose file is gone, are swept once per process.
 *
 * iOS counterpart: ios/FamilyConnect/Core/ParkedRecordings.swift
 */

package me.nettrash.familyconnect.data.repo

import android.content.Context
import android.util.Log
import dagger.hilt.android.qualifiers.ApplicationContext
import java.io.File
import java.util.UUID
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.di.AppScope

/** One voice message that was not sent, as the store keeps it. */
@Serializable
data class ParkedRecording(
    val id: String,
    @SerialName("chat_id") val chatId: Long,
    /** The file's NAME inside [ParkedRecordings.directory] — never a path. */
    val file: String,
    /** How long it ran, by the recorder's own clock: what the row shows. */
    @SerialName("duration_ms") val durationMs: Long,
    /** The message it was recorded in answer to; it goes with it, and with nothing else. */
    @SerialName("reply_to") val replyTo: ReplyToDto? = null,
    /** Its words, when it had any; they never leave without it. */
    val caption: String = "",
    // There is no "sending" mark any more (#79, revised 2026-10-06): it was
    // the five-second Undo window's crash-safe entry, and the window went
    // with the hold. An index a test build wrote with `"sending": true`
    // still reads — the codec ignores unknown keys — and such an entry is
    // simply a not-sent row, which is what a launch used to make of it.
    /**
     * Its waveform as the wire spells it (#79; docs/protocol.md, "A voice
     * note's waveform"): the not-sent row draws it, and its Send uploads it.
     * Absent on every entry written before there were waveforms.
     */
    val waveform: String? = null,
) {
    companion object {
        private val json = Json {
            ignoreUnknownKeys = true
            encodeDefaults = false
        }
        private val listSerializer = ListSerializer(serializer())

        fun encode(entries: List<ParkedRecording>): String =
            json.encodeToString(listSerializer, entries)

        /** Never throws: a corrupt index reads as empty, and the sweep then reclaims the files. */
        fun decode(raw: String?): List<ParkedRecording> {
            if (raw.isNullOrBlank()) return emptyList()
            return runCatching { json.decodeFromString(listSerializer, raw) }.getOrDefault(emptyList())
        }
    }
}

@Singleton
class ParkedRecordings internal constructor(
    private val settings: SettingsRepository,
    private val epoch: SessionEpoch,
    private val scope: CoroutineScope,
    /**
     * Where the bytes live: [directory] in the app. A test hands in a folder
     * of its own — Robolectric gives every test in a JVM the same filesDir,
     * and another test's store sweeping it would take this one's files.
     */
    private val root: File,
) {

    @Inject
    constructor(
        @ApplicationContext context: Context,
        settings: SettingsRepository,
        epoch: SessionEpoch,
        @AppScope scope: CoroutineScope,
    ) : this(settings, epoch, scope, directory(context))

    /**
     * Every write here takes it, so a park, a removal and the sweep never
     * interleave: a file moved in is named by its entry before the sweep can
     * look, and an entry is never read half-written.
     */
    private val lock = Mutex()

    /**
     * Once per process, at launch (FamilyConnectApp creates this store): the
     * files a crash or a killed app left behind with no entry, and entries
     * whose file is gone, are swept.
     */
    internal val launchSweep: Job = scope.launch {
        runCatching { sweep() }.onFailure { Log.w(TAG, "sweep failed: ${it.message}") }
    }

    /** This chat's voice messages that were not sent, oldest first. */
    fun forChat(chatId: Long): Flow<List<ParkedRecording>> =
        settings.state
            .map { state ->
                state.parkedRecordings.filter { it.chatId == chatId && file(it).exists() }
            }
            .distinctUntilChanged()
            .flowOn(Dispatchers.IO)

    /** Where an entry's bytes are. */
    fun file(entry: ParkedRecording): File = File(root, entry.file)

    /**
     * The session now. A chat takes it when it opens and hands it back with
     * every park: one that lands after a sign-out is dropped.
     */
    fun session(): Long = epoch.current()

    /**
     * Keep [source] as a voice message that was not sent, MOVING it out of
     * the cache. Null when it could not be kept — the session that recorded
     * it has ended (sign-out deletes everything recorded and not sent), or
     * the disk refused the file; in both cases [source] is gone.
     */
    suspend fun park(
        chatId: Long,
        source: File,
        durationMs: Long,
        replyTo: ReplyToDto?,
        caption: String,
        session: Long,
        /** Its waveform (#79), kept with it so the row and the eventual send both have it. */
        waveform: String? = null,
    ): ParkedRecording? = lock.withLock {
        var kept: ParkedRecording? = null
        epoch.whileCurrent(session) {
            withContext(Dispatchers.IO) {
                val id = UUID.randomUUID().toString()
                val name = "$id.$EXTENSION"
                val target = File(root.apply { mkdirs() }, name)
                val moved = source.renameTo(target) ||
                    runCatching {
                        source.copyTo(target, overwrite = true)
                        true
                    }.getOrDefault(false)
                if (moved) {
                    val entry = ParkedRecording(
                        id = id,
                        chatId = chatId,
                        file = name,
                        durationMs = durationMs,
                        replyTo = replyTo,
                        caption = caption,
                        waveform = waveform,
                    )
                    settings.updateParkedRecordings { it + entry }
                    kept = entry
                } else {
                    target.delete()
                    Log.w(TAG, "could not keep a recording that was not sent")
                }
            }
        }
        // Moved, copied (the original is then a stray in the cache), or not
        // kept at all — the cache copy goes in every case.
        withContext(Dispatchers.IO) { source.delete() }
        kept
    }

    /** Forget one, and its file. Sent or deleted, it is the same removal. */
    suspend fun remove(id: String): Unit = lock.withLock {
        val entry = settings.state.first().parkedRecordings.firstOrNull { it.id == id }
        settings.updateParkedRecordings { entries -> entries.filterNot { it.id == id } }
        if (entry != null) withContext(Dispatchers.IO) { file(entry).delete() }
    }

    /**
     * Delete every file no entry names, and every entry whose file is gone.
     * Returns how many files went.
     */
    suspend fun sweep(): Int = lock.withLock {
        withContext(Dispatchers.IO) {
            val directory = root
            val missing = settings.state.first().parkedRecordings
                .filterNot { File(directory, it.file).exists() }
                .map { it.id }
                .toSet()
            if (missing.isNotEmpty()) {
                settings.updateParkedRecordings { entries -> entries.filterNot { it.id in missing } }
                Log.i(TAG, "dropped ${missing.size} entries whose file was gone")
            }
            val named = settings.state.first().parkedRecordings.map { it.file }.toSet()
            var removed = 0
            directory.listFiles()?.forEach { candidate ->
                if (candidate.name !in named && candidate.delete()) removed++
            }
            if (removed > 0) Log.i(TAG, "swept $removed recording(s) nothing names")
            removed
        }
    }

    companion object {
        private const val TAG = "ParkedRecordings"
        private const val DIRECTORY = "parked-recordings"
        private const val EXTENSION = "m4a"

        /** Where the bytes live — also what the session wipe deletes. */
        fun directory(context: Context): File = File(context.filesDir, DIRECTORY)
    }
}
