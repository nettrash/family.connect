/*
 * ReactionRecents.kt
 * Family Connect (Android)
 *
 * The latest picker choices for the current server and account.
 */

package me.nettrash.familyconnect.data.settings

data class ReactionRecents(val scope: Scope? = null, val choices: List<String> = emptyList()) {
    data class Scope(val serverUrl: String, val userId: Long) {
        companion object {
            fun of(serverUrl: String?, userId: Long?): Scope? =
                if (serverUrl.isNullOrBlank() || userId == null || userId <= 0) null
                else Scope(serverUrl, userId)
        }
    }
}
