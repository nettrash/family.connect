/*
 * Avatars.kt
 * Family Connect (Android)
 *
 * The seam between the avatar cache and composition.
 *
 * LocalAvatars carries the app-scoped AvatarRepository down the tree so
 * every Avatar() can resolve a picture without each screen's ViewModel
 * having to carry one. It defaults to null so a @Preview — and any test
 * that composes a screen without Hilt — renders initials instead of
 * crashing.
 *
 * rememberAvatar is where the fetch is kicked off: keyed on (userId,
 * version) so a changed picture re-requests, and only ever from a
 * LaunchedEffect, never from composition itself.
 *
 * iOS counterpart: the @Environment(AvatarStore.self) lookup inside
 * ios/FamilyConnect/Views/ChatListView.swift's InitialsAvatar.
 */

package me.nettrash.familyconnect.ui.components

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.ImageBitmap
import me.nettrash.familyconnect.data.repo.AvatarRepository

val LocalAvatars = staticCompositionLocalOf<AvatarRepository?> { null }

/**
 * This user's profile picture, or null while it loads — and forever if
 * they have none, if the cache isn't provided, or if we may not see it.
 */
@Composable
fun rememberAvatar(userId: Long, version: Long): ImageBitmap? {
    val avatars = LocalAvatars.current ?: return null
    if (version <= 0) return null
    // retryToken is in the key set on purpose: a fetch that failed while
    // the phone was offline has nothing else to restart it — a row that
    // stays on screen never re-runs its effect on its own.
    LaunchedEffect(userId, version, avatars.retryToken) { avatars.load(userId, version) }
    return avatars.cached(userId, version)
}
