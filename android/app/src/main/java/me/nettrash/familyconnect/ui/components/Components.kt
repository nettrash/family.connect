/*
 * Components.kt
 * Family Connect (Android)
 *
 * Small shared building blocks:
 *
 *   OfflineBanner         — "No connection" / "Connecting…" strip under the
 *                           app bar; animates in/out and renders nothing
 *                           when the socket is open.
 *   Avatar                — initials in a circle; hue derived from the user
 *                           id hash so a person keeps their color everywhere
 *                           without storing anything.
 *   EmptyState            — icon + title + supporting line, centered.
 *   ErrorCard             — inline error surface with optional action.
 *   BusyButtonContent     — button label that cross-fades to a spinner
 *                           without letting the button resize.
 *   DestructiveTextButton — error-tinted TextButton for confirm dialogs.
 *
 * One file, not six: each is a single composable with no private
 * helpers; separate files would be ceremony.
 *
 * iOS counterpart: ios/FamilyConnect/UI/Components/
 */

package me.nettrash.familyconnect.ui.components

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CloudOff
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import me.nettrash.familyconnect.data.net.ws.SocketState
import kotlin.math.absoluteValue

@Composable
fun OfflineBanner(
    isOnline: Boolean,
    socketState: SocketState,
    modifier: Modifier = Modifier,
) {
    val offline = !isOnline
    val visible = offline || socketState == SocketState.Connecting
    // The exit animation still needs content to draw while shrinking, so
    // remember which variant was last shown rather than early-returning.
    val lastOffline = remember { mutableStateOf(offline) }
    if (visible) lastOffline.value = offline
    AnimatedVisibility(
        visible = visible,
        modifier = modifier,
        enter = expandVertically() + fadeIn(),
        exit = shrinkVertically() + fadeOut(),
    ) {
        val showOffline = lastOffline.value
        val containerColor =
            if (showOffline) MaterialTheme.colorScheme.errorContainer
            else MaterialTheme.colorScheme.surfaceVariant
        val contentColor =
            if (showOffline) MaterialTheme.colorScheme.onErrorContainer
            else MaterialTheme.colorScheme.onSurfaceVariant
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .background(containerColor)
                .padding(vertical = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (showOffline) {
                Icon(
                    imageVector = Icons.Outlined.CloudOff,
                    contentDescription = null,
                    modifier = Modifier.size(16.dp),
                    tint = contentColor,
                )
            } else {
                CircularProgressIndicator(
                    modifier = Modifier.size(14.dp),
                    strokeWidth = 2.dp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Text(
                text = if (showOffline) "No connection" else "Connecting…",
                style = MaterialTheme.typography.labelMedium,
                color = contentColor,
            )
        }
    }
}

@Composable
fun Avatar(
    name: String,
    userId: Long,
    modifier: Modifier = Modifier,
    size: Int = 40,
    containerColor: Color? = null,
    contentColor: Color? = null,
) {
    // Stable per-user hue from the id hash; lightness is split by theme
    // (0.40 light / 0.35 dark) so white SemiBold initials stay >= 4.5:1
    // across the whole hue wheel.
    val hue = (userId.hashCode().absoluteValue % 360).toFloat()
    val lightness = if (isSystemInDarkTheme()) 0.35f else 0.40f
    val background = containerColor ?: Color.hsl(hue, 0.45f, lightness)
    val foreground = contentColor ?: Color.White
    val initials = name
        .split(' ', limit = 3)
        .filter { it.isNotBlank() }
        .take(2)
        .map { it.first().uppercaseChar() }
        .joinToString("")
        .ifEmpty { "?" }
    Box(
        modifier = modifier
            .size(size.dp)
            .background(background, CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = initials,
            color = foreground,
            fontWeight = FontWeight.SemiBold,
            fontSize = (size * 0.4).sp,
        )
    }
}

@Composable
fun EmptyState(
    icon: ImageVector,
    title: String,
    subtitle: String,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier.padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            modifier = Modifier.size(48.dp),
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(16.dp))
        Text(
            text = title,
            style = MaterialTheme.typography.titleMedium,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(6.dp))
        Text(
            text = subtitle,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}

@Composable
fun ErrorCard(
    message: String,
    modifier: Modifier = Modifier,
    onRetry: (() -> Unit)? = null,
    actionLabel: String = "Dismiss",
) {
    Card(
        modifier = modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.errorContainer,
        ),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = Icons.Outlined.ErrorOutline,
                contentDescription = null,
                modifier = Modifier.size(20.dp),
                tint = MaterialTheme.colorScheme.onErrorContainer,
            )
            Text(
                text = message,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onErrorContainer,
                modifier = Modifier.weight(1f),
            )
            if (onRetry != null) {
                TextButton(onClick = onRetry) {
                    Text(actionLabel)
                }
            }
        }
    }
}

@Composable
fun BusyButtonContent(label: String, busy: Boolean) {
    Box(contentAlignment = Alignment.Center) {
        // An invisible copy of the label pins the slot's size so the
        // button never resizes when the spinner swaps in.
        Text(text = label, modifier = Modifier.alpha(0f))
        AnimatedContent(
            targetState = busy,
            transitionSpec = { fadeIn(tween(150)) togetherWith fadeOut(tween(150)) },
            label = "busyButtonContent",
        ) { isBusy ->
            if (isBusy) {
                CircularProgressIndicator(
                    modifier = Modifier.size(20.dp),
                    strokeWidth = 2.dp,
                    // Inherit the button's content color (onPrimary inside
                    // filled buttons) instead of the default primary.
                    color = LocalContentColor.current,
                )
            } else {
                Text(text = label)
            }
        }
    }
}

@Composable
fun DestructiveTextButton(
    label: String,
    onClick: () -> Unit,
    enabled: Boolean = true,
) {
    TextButton(
        onClick = onClick,
        enabled = enabled,
        colors = ButtonDefaults.textButtonColors(
            contentColor = MaterialTheme.colorScheme.error,
        ),
    ) {
        Text(label)
    }
}
