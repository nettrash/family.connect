/*
 * MediaUploadScheduler.kt
 * Family Connect (Android)
 *
 * Who to ask when an upload has to outlive the app (docs/protocol.md,
 * "Sending on an unreliable network").
 *
 * An interface rather than a call to `WorkManager` from the repository,
 * for the reason every seam here exists: a unit test that sends media
 * would otherwise have to stand a whole WorkManager up, and the thing
 * worth asserting — that a send ASKS for the work — is a recorded id, not
 * a scheduler's internals. [None] is what the tests that do not care pass.
 */

package me.nettrash.familyconnect.data.repo

import android.content.Context
import dagger.hilt.android.qualifiers.ApplicationContext
import javax.inject.Inject
import javax.inject.Singleton

fun interface MediaUploadScheduler {

    /** Finish this send's uploads, whatever happens to the app. */
    fun schedule(clientMsgId: String)

    companion object {
        /** Ask nobody: the app's own coroutine is the only uploader. */
        val None = MediaUploadScheduler { }
    }
}

/** The production one: a unique [MediaUploadWorker] per send. */
@Singleton
class WorkManagerUploads @Inject constructor(
    @param:ApplicationContext private val appContext: Context,
) : MediaUploadScheduler {

    override fun schedule(clientMsgId: String) {
        MediaUploadWorker.enqueue(appContext, clientMsgId)
    }
}
