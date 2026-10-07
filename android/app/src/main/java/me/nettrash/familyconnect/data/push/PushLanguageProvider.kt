/*
 * PushLanguageProvider.kt
 * Family Connect (Android)
 *
 * Seam for "what language is this app shown in?" — what POST /devices
 * sends, so the server writes this device's pushes in it (docs/protocol.md,
 * "The words of a push"; issue #82). Read off a string resource that every
 * locale folder defines, so the answer is the folder Android actually picked
 * for the screens, per-app language included, rather than a guess from the
 * system locale.
 */

package me.nettrash.familyconnect.data.push

import android.content.Context
import dagger.hilt.android.qualifiers.ApplicationContext
import me.nettrash.familyconnect.R
import javax.inject.Inject

fun interface PushLanguageProvider {
    /** One of [PushLanguage.ALL]. */
    fun shownLanguage(): String
}

/** The apps' nine localisations, as POST /devices names them. */
object PushLanguage {
    val ALL = listOf("en", "de", "es", "fr", "ja", "ru", "sr", "sr-Latn", "zh-Hans")

    /** A shown language as the wire spells it; anything unknown is English. */
    fun of(shown: String?): String =
        ALL.firstOrNull { it.equals(shown?.trim(), ignoreCase = true) } ?: "en"
}

class ResourcePushLanguageProvider @Inject constructor(
    @ApplicationContext private val context: Context,
) : PushLanguageProvider {
    override fun shownLanguage(): String = PushLanguage.of(context.getString(R.string.push_language))
}
