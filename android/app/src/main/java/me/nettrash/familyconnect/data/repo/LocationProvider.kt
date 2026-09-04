package me.nettrash.familyconnect.data.repo

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Looper
import androidx.core.content.ContextCompat
import dagger.hilt.android.qualifiers.ApplicationContext
import javax.inject.Inject
import javax.inject.Singleton
import kotlin.coroutines.resume
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull

/**
 * One fix, on demand — the whole of what "share my location" needs.
 *
 * Deliberately NOT a running location service. This app shares a place
 * once, at the moment somebody asks it to (docs/protocol.md, "Locations": a
 * location is decided at send time and never changes), so the manager
 * starts, waits for a fix, and stops. Nothing runs in the background and
 * nothing subscribes.
 *
 * **The platform's own `LocationManager`, not Play Services.** The
 * dependency catalog states the posture plainly — "no analytics, no
 * crashlytics… nothing else may phone Google", with firebase-messaging as
 * the single sanctioned exception — and `FusedLocationProviderClient` would
 * be a second one. `LocationManager` is in the framework, needs no
 * dependency and no API key, and for a one-shot street-level fix it is
 * enough. It is slower to first fix than the fused client, which is what
 * the generous timeout below is for.
 */
@Singleton
class LocationProvider @Inject constructor(
    @param:ApplicationContext private val context: Context,
) {
    /** A place, as the wire wants it. */
    data class Fix(
        val latitude: Double,
        val longitude: Double,
        /**
         * Metres, or null when the device did not report a usable one — in
         * which case a bubble draws a plain pin rather than claiming a
         * precision nobody measured.
         */
        val accuracyM: Int?,
    )

    sealed interface Result {
        data class Found(val fix: Fix) : Result
        /** No permission — the caller asks for it and tries again. */
        data object Denied : Result
        /** Permission held, but no fix arrived (or location is switched off). */
        data object Unavailable : Result
    }

    /** Has the person already allowed this? */
    fun hasPermission(): Boolean =
        PERMISSIONS.any {
            ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED
        }

    /**
     * Ask for one fix.
     *
     * A LAST KNOWN location is used when it is recent enough, because it is
     * instant and a place somebody was two minutes ago is the place they
     * are: waiting twenty seconds for a fresh fix that says the same thing
     * makes sharing feel broken. Anything older asks for a live one.
     */
    @SuppressLint("MissingPermission")
    suspend fun currentFix(): Result {
        if (!hasPermission()) return Result.Denied
        val manager = context.getSystemService(Context.LOCATION_SERVICE) as? LocationManager
            ?: return Result.Unavailable

        lastKnown(manager)?.let { return Result.Found(it.toFix()) }

        val live = withTimeoutOrNull(TIMEOUT_MS) {
            suspendCancellableCoroutine { continuation ->
                val listener = object : LocationListener {
                    override fun onLocationChanged(location: Location) {
                        manager.removeUpdates(this)
                        if (continuation.isActive) continuation.resume(location)
                    }

                    // Present because the pre-API-30 interface declares
                    // them abstract; a one-shot cares about none of them.
                    @Deprecated("Required by the pre-API-30 interface")
                    override fun onStatusChanged(provider: String?, status: Int, extras: android.os.Bundle?) = Unit

                    override fun onProviderEnabled(provider: String) = Unit
                    override fun onProviderDisabled(provider: String) = Unit
                }
                // Whichever providers exist: GPS is precise and slow
                // indoors, network is fast and coarse, and "where are you?"
                // is answered well enough by either.
                val providers = manager.getProviders(true).filter { it in USABLE_PROVIDERS }
                if (providers.isEmpty()) {
                    continuation.resume(null)
                    return@suspendCancellableCoroutine
                }
                for (provider in providers) {
                    manager.requestLocationUpdates(provider, 0L, 0f, listener, Looper.getMainLooper())
                }
                continuation.invokeOnCancellation { manager.removeUpdates(listener) }
            }
        }
        return live?.let { Result.Found(it.toFix()) } ?: Result.Unavailable
    }

    @SuppressLint("MissingPermission")
    private fun lastKnown(manager: LocationManager): Location? =
        USABLE_PROVIDERS
            .mapNotNull { provider -> runCatching { manager.getLastKnownLocation(provider) }.getOrNull() }
            .filter { System.currentTimeMillis() - it.time <= FRESH_ENOUGH_MS }
            .maxByOrNull { it.time }

    private fun Location.toFix() = Fix(
        latitude = latitude,
        longitude = longitude,
        // `hasAccuracy` is the guard: `accuracy` is 0f when unknown, and
        // reporting ±0 m would claim a precision no phone has.
        accuracyM = if (hasAccuracy()) accuracy.toInt() else null,
    )

    companion object {
        /**
         * Either is enough. COARSE alone names a street, which is what
         * "where are you?" asks — so a family member who grants only the
         * approximate permission still gets the feature.
         */
        val PERMISSIONS = arrayOf(
            Manifest.permission.ACCESS_FINE_LOCATION,
            Manifest.permission.ACCESS_COARSE_LOCATION,
        )

        private val USABLE_PROVIDERS =
            listOf(LocationManager.GPS_PROVIDER, LocationManager.NETWORK_PROVIDER)

        /** A fix this recent is the answer; anything older asks afresh. */
        private const val FRESH_ENOUGH_MS = 2 * 60 * 1000L

        /**
         * Generous, because `LocationManager` has no fused fast path: a
         * first GPS fix indoors genuinely takes this long, and giving up
         * early would report "unavailable" for a phone that was about to
         * answer.
         */
        private const val TIMEOUT_MS = 25_000L
    }
}
