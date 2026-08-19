/*
 * Components.kt
 * Family Connect (Android)
 *
 * Small shared building blocks:
 *
 *   OfflineBanner — "No connection" / "Connecting…" strip under the app
 *                   bar; renders nothing when the socket is open.
 *   Avatar        — initials in a circle; hue derived from the user id
 *                   hash so a person keeps their color everywhere
 *                   without storing anything.
 *   EmptyState    — icon + title + supporting line, centered.
 *   ErrorCard     — inline error surface with optional retry.
 *
 * One file, not four: each is a single composable with no private
 * helpers; separate files would be ceremony.
 *
 * iOS counterpart: ios/FamilyConnect/UI/Components/
 */

package me.nettrash.familyconnect.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
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
    val label = when {
        !isOnline -> "No connection"
        socketState == SocketState.Connecting -> "Connecting…"
        else -> return
    }
    Box(
        modifier = modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .padding(vertical = 4.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
fun Avatar(
    name: String,
    userId: Long,
    modifier: Modifier = Modifier,
    size: Int = 40,
) {
    // Stable per-user hue from the id hash; fixed saturation/lightness
    // keeps every variant readable under white initials.
    val hue = (userId.hashCode().absoluteValue % 360).toFloat()
    val background = Color.hsl(hue, 0.45f, 0.45f)
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
            color = Color.White,
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
        Spacer(Modifier.height(4.dp))
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
) {
    Card(
        modifier = modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.errorContainer,
        ),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = message,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onErrorContainer,
                modifier = Modifier.weight(1f),
            )
            if (onRetry != null) {
                TextButton(onClick = onRetry) {
                    Text("Retry")
                }
            }
        }
    }
}
