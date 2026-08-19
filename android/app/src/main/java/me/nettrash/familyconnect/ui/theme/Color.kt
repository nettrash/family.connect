/*
 * Color.kt
 * Family Connect (Android)
 *
 * Brand palette — the NETTRASH blue shared across the product family:
 * #0D47A1 is the nettrash.me site's brand blue and Scan's dark accent,
 * #0D3A8A the site's hover blue, #1A1A1A the site/Scan dark surface,
 * and amber the site's secondary accent. Hand-derived Material 3 tonal
 * values; only used when dynamic color isn't available (< Android 12).
 *
 * iOS counterpart: ios/FamilyConnect/Assets.xcassets/AccentColor.colorset
 */

package me.nettrash.familyconnect.ui.theme

import androidx.compose.ui.graphics.Color

// Light scheme
val BluePrimary = Color(0xFF0D47A1)
val BlueOnPrimary = Color(0xFFFFFFFF)
val BluePrimaryContainer = Color(0xFFD6E3FF)
val BlueOnPrimaryContainer = Color(0xFF001B3F)
val AmberSecondary = Color(0xFF7A5900)
val AmberOnSecondary = Color(0xFFFFFFFF)
val AmberSecondaryContainer = Color(0xFFFFDF9E)
val AmberOnSecondaryContainer = Color(0xFF261A00)
val LightBackground = Color(0xFFFAFAFA)
val LightOnBackground = Color(0xFF1A1C1E)
val LightSurface = Color(0xFFFAFAFA)
val LightOnSurface = Color(0xFF1A1C1E)
val LightSurfaceVariant = Color(0xFFE0E2EC)
val LightOnSurfaceVariant = Color(0xFF44474E)

// Dark scheme
val BluePrimaryDark = Color(0xFFA8C8FF)
val BlueOnPrimaryDark = Color(0xFF002F64)
val BluePrimaryContainerDark = Color(0xFF0D3A8A)
val BlueOnPrimaryContainerDark = Color(0xFFD6E3FF)
val AmberSecondaryDark = Color(0xFFF5BE48)
val AmberOnSecondaryDark = Color(0xFF402D00)
val AmberSecondaryContainerDark = Color(0xFF5C4200)
val AmberOnSecondaryContainerDark = Color(0xFFFFDF9E)
val DarkBackground = Color(0xFF1A1A1A)
val DarkOnBackground = Color(0xFFE3E2E6)
val DarkSurface = Color(0xFF1A1A1A)
val DarkOnSurface = Color(0xFFE3E2E6)
val DarkSurfaceVariant = Color(0xFF44474E)
val DarkOnSurfaceVariant = Color(0xFFC4C6CF)
