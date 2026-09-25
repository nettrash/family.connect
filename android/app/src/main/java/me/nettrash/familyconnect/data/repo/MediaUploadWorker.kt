/*
 * MediaUploadWorker.kt
 * Family Connect (Android)
 *
 * Finishing an attachment upload after somebody has left the app
 * (docs/protocol.md, "Sending on an unreliable network": an upload may be
 * handed to the system and land while the app is not running).
 *
 * WHY A WORKER AND NOT THE COROUTINE WE ALREADY HAD. A media send is
 * already durable: `MessageRepository.sendMedia` stages the bytes and
 * writes the row before the first byte leaves, and `flushPending` resumes
 * it on the socket connecting, the foreground edge, the network returning
 * and its own wake timer. What none of those cover is the case the issue
 * is about — the person presses Send on a 90 MB video and leaves. The
 * upload runs on an application-scoped coroutine, and Android kills a
 * backgrounded process whenever it likes: the send then waits for the next
 * launch, which may be hours. WorkManager is the one thing on this
 * platform that survives process death, waits for a network, backs off on
 * its own and is still there after a reboot.
 *
 * WHAT IT DOES NOT DO. It does not upload anything itself. The work is
 * exactly `uploadPending`, whose own in-flight guard makes a worker that
 * races the app's own coroutine a no-op rather than a second pass over the
 * same row — which matters, because two passes would each upload the
 * remainder and leave a copy for the sweep.
 *
 * iOS counterpart: ios/FamilyConnect/Core/BackgroundUploads.swift, which
 * hands the same work to a background `URLSession`.
 */

package me.nettrash.familyconnect.data.repo

import android.content.Context
import androidx.hilt.work.HiltWorker
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import dagger.assisted.Assisted
import dagger.assisted.AssistedInject
import me.nettrash.familyconnect.data.db.PendingAttachmentDao
import java.util.concurrent.TimeUnit

@HiltWorker
class MediaUploadWorker @AssistedInject constructor(
    @Assisted context: Context,
    @Assisted params: WorkerParameters,
    private val messages: MessageRepository,
    private val pendingAttachments: PendingAttachmentDao,
) : CoroutineWorker(context, params) {

    override suspend fun doWork(): Result {
        val one = inputData.getString(KEY_CLIENT_MSG_ID)
        val owing = pendingAttachments.sendsWorthUploading()
        // A send that no longer owes anything — landed while the app was
        // still up, or was deleted — is done, not failed.
        val sends = if (one != null) owing.filter { it == one } else owing
        if (sends.isEmpty()) return Result.success()

        for (clientMsgId in sends) {
            runCatching { messages.uploadPending(clientMsgId) }
        }

        // Whether to come back is decided by the ROWS, not by what the
        // upload legs returned: `uploadPending` is deliberately quiet about
        // a refusal it has already written onto the row as failed, and a
        // worker that retried those would retry them for ever.
        val left = pendingAttachments.sendsWorthUploading()
        val stillOwed = if (one != null) left.contains(one) else left.isNotEmpty()
        return if (stillOwed) Result.retry() else Result.success()
    }

    companion object {
        private const val KEY_CLIENT_MSG_ID = "client_msg_id"

        /** What [cancelAll] takes: every upload job this app ever asked for. */
        private const val TAG = "media-upload"

        /** The name one send's work is unique under. */
        private fun nameFor(clientMsgId: String) = "media-upload:$clientMsgId"

        /**
         * Ask the system to finish this send's uploads, whatever happens to
         * the app.
         *
         * `KEEP`, so pressing Send twice in a chat does not stack two jobs
         * on one row; `CONNECTED`, because an upload with no network is a
         * wasted wake-up — WorkManager is what waits for the network here,
         * which is the whole reason it is worth a dependency.
         *
         * NOT expedited, deliberately. This app supports Android 8, and
         * below API 31 WorkManager runs expedited work as a foreground
         * service: it calls `getForegroundInfo()` and a worker that has
         * not implemented one THROWS at run time. Buying immediacy would
         * therefore cost every old phone a "Sending…" notification — and
         * immediacy is not what this job is for. The app's own coroutine is
         * what starts now, while somebody is looking; this is what finishes
         * after they have gone, and a minute either way is invisible.
         */
        /** Every one of these jobs, dropped — what a sign-out owes. */
        fun cancelAll(context: Context) {
            WorkManager.getInstance(context).cancelAllWorkByTag(TAG)
        }

        fun enqueue(context: Context, clientMsgId: String) {
            val request = OneTimeWorkRequestBuilder<MediaUploadWorker>()
                .addTag(TAG)
                .setInputData(workDataOf(KEY_CLIENT_MSG_ID to clientMsgId))
                .setConstraints(
                    Constraints.Builder()
                        .setRequiredNetworkType(NetworkType.CONNECTED)
                        .build(),
                )
                .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS)
                .build()
            WorkManager.getInstance(context)
                .enqueueUniqueWork(nameFor(clientMsgId), ExistingWorkPolicy.KEEP, request)
        }
    }
}
