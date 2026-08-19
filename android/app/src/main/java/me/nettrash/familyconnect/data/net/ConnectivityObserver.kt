/*
 * ConnectivityObserver.kt
 * Family Connect (Android)
 *
 * Wraps ConnectivityManager's default-network callback in two flows:
 * `isOnline` (StateFlow for the offline banner) and `onAvailable`
 * (SharedFlow of edges — ChatSocketManager uses it to short-circuit a
 * pending backoff delay the instant a network returns).
 *
 * Interface + impl split so JVM tests can feed connectivity transitions
 * without a ConnectivityManager.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Net/ConnectivityObserver.swift
 * (NWPathMonitor there).
 */

package me.nettrash.familyconnect.data.net

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import javax.inject.Inject
import javax.inject.Singleton

interface ConnectivityObserver {
    val isOnline: StateFlow<Boolean>
    val onAvailable: SharedFlow<Unit>
}

@Singleton
class AndroidConnectivityObserver @Inject constructor(
    @param:ApplicationContext private val context: Context,
) : ConnectivityObserver {

    private val _isOnline = MutableStateFlow(currentlyOnline())
    override val isOnline: StateFlow<Boolean> = _isOnline

    private val _onAvailable = MutableSharedFlow<Unit>(extraBufferCapacity = 1)
    override val onAvailable: SharedFlow<Unit> = _onAvailable

    init {
        val cm = context.getSystemService(ConnectivityManager::class.java)
        cm?.registerDefaultNetworkCallback(object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                _isOnline.value = true
                _onAvailable.tryEmit(Unit)
            }

            override fun onLost(network: Network) {
                // The *default* network went away. A replacement (wifi →
                // cellular) arrives as a fresh onAvailable.
                _isOnline.value = false
            }
        })
    }

    private fun currentlyOnline(): Boolean {
        val cm = context.getSystemService(ConnectivityManager::class.java) ?: return false
        val caps = cm.getNetworkCapabilities(cm.activeNetwork) ?: return false
        return caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
    }
}
