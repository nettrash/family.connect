/*
 * Color.kt
 * Family Connect (Android)
 *
 * Brand palette — the NETTRASH blue shared across the product family:
 * #0D47A1 is the nettrash.me site's brand blue and Scan's dark accent,
 * #0D3A8A the site's hover blue, and amber the site's secondary accent.
 * Hand-derived Material 3 tonal values seeded from the brand blue —
 * neutrals carry a faint blue cast (not the site's flat grays) so the
 * surface-container ladder layers tonally instead of going muddy.
 * Only used when dynamic color isn't available (< Android 12).
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
// Tertiary: calm blue-teal, halfway between the brand blue and the
// amber's warmth-free complement — used sparingly (links, info accents).
val TealTertiary = Color(0xFF156683)
val TealOnTertiary = Color(0xFFFFFFFF)
val TealTertiaryContainer = Color(0xFFBFE9FF)
val TealOnTertiaryContainer = Color(0xFF001F2A)
val LightError = Color(0xFFBA1A1A)
val LightOnError = Color(0xFFFFFFFF)
val LightErrorContainer = Color(0xFFFFDAD6)
val LightOnErrorContainer = Color(0xFF410002)
// background sits a hair below surface so full-bleed screens still read
// as "behind" cards/sheets even before the container ladder kicks in.
val LightBackground = Color(0xFFFAFAFC)
val LightOnBackground = Color(0xFF1A1C1E)
val LightSurface = Color(0xFFF8F9FE)
val LightOnSurface = Color(0xFF1A1C1E)
val LightSurfaceVariant = Color(0xFFE0E2EC)
val LightOnSurfaceVariant = Color(0xFF44474E)
val LightSurfaceDim = Color(0xFFD9DAE0)
val LightSurfaceBright = Color(0xFFFDFDFF)
val LightSurfaceContainerLowest = Color(0xFFFFFFFF)
val LightSurfaceContainerLow = Color(0xFFF4F5FA)
val LightSurfaceContainer = Color(0xFFEEEFF5)
val LightSurfaceContainerHigh = Color(0xFFE8EAF0)
val LightSurfaceContainerHighest = Color(0xFFE2E4EA)
val LightOutline = Color(0xFF74777F)
val LightOutlineVariant = Color(0xFFC4C6CF)
val LightInverseSurface = Color(0xFF2E3036)
val LightInverseOnSurface = Color(0xFFF0F0F7)
val LightScrim = Color(0xFF000000)

// Dark scheme
val BluePrimaryDark = Color(0xFFA8C8FF)
val BlueOnPrimaryDark = Color(0xFF002F64)
val BluePrimaryContainerDark = Color(0xFF0D3A8A)
val BlueOnPrimaryContainerDark = Color(0xFFD6E3FF)
val AmberSecondaryDark = Color(0xFFF5BE48)
val AmberOnSecondaryDark = Color(0xFF402D00)
val AmberSecondaryContainerDark = Color(0xFF5C4200)
val AmberOnSecondaryContainerDark = Color(0xFFFFDF9E)
val TealTertiaryDark = Color(0xFF8CCFF0)
val TealOnTertiaryDark = Color(0xFF003547)
val TealTertiaryContainerDark = Color(0xFF004D64)
val TealOnTertiaryContainerDark = Color(0xFFBFE9FF)
val DarkError = Color(0xFFFFB4AB)
val DarkOnError = Color(0xFF690005)
val DarkErrorContainer = Color(0xFF93000A)
val DarkOnErrorContainer = Color(0xFFFFDAD6)
val DarkBackground = Color(0xFF111316)
val DarkOnBackground = Color(0xFFE3E2E6)
val DarkSurface = Color(0xFF141619)
val DarkOnSurface = Color(0xFFE3E2E6)
val DarkSurfaceVariant = Color(0xFF44474E)
val DarkOnSurfaceVariant = Color(0xFFC4C6CF)
val DarkSurfaceDim = Color(0xFF0F1113)
val DarkSurfaceBright = Color(0xFF383A40)
val DarkSurfaceContainerLowest = Color(0xFF0F1113)
val DarkSurfaceContainerLow = Color(0xFF181A1E)
val DarkSurfaceContainer = Color(0xFF1C1F24)
val DarkSurfaceContainerHigh = Color(0xFF26292F)
val DarkSurfaceContainerHighest = Color(0xFF31343A)
val DarkOutline = Color(0xFF8E9099)
val DarkOutlineVariant = Color(0xFF44474E)
val DarkInverseSurface = Color(0xFFE2E2E9)
val DarkInverseOnSurface = Color(0xFF2E3036)
val DarkScrim = Color(0xFF000000)
