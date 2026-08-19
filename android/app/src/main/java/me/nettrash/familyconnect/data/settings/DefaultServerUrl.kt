/*
 * DefaultServerUrl.kt
 * Family Connect (Android)
 *
 * Nominal seam for the build's compiled-in server URL. The Play Store
 * build is compiled with -PdefaultServerUrl=https://fc.nettrash.me
 * (app/build.gradle.kts → BuildConfig.DEFAULT_SERVER_URL) so first run
 * skips server setup and boots straight to Auth; generic source builds
 * compile in an empty string and keep asking for the URL on first run.
 *
 * A fun interface instead of a direct BuildConfig read keeps
 * SessionRepository unit-testable on the plain JVM — tests hand in
 * `DefaultServerUrl { null }` or a fixed URL, no generated code needed.
 *
 * iOS counterpart: none yet — iOS still always shows server setup on
 * first run; port as an Info.plist-provided default read by AppSettings.
 */

package me.nettrash.familyconnect.data.settings

fun interface DefaultServerUrl {
    /**
     * The URL this build ships pre-pointed at, or null when the build has
     * none (a blank BuildConfig value maps to null in the DI provider).
     * SessionRepository normalizes it before adopting — implementations
     * don't have to return canonical form.
     */
    fun get(): String?
}
