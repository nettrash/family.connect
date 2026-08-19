/*
 * MainActivity.kt
 * Family Connect (Android)
 *
 * The single Activity. Edge-to-edge; waits for MainViewModel's boot
 * snapshot (spinner while null), then composes the theme + NavHost
 * exactly once with the start destination the snapshot dictates. All
 * later routing is event-driven inside AppNavHost.
 *
 * iOS counterpart: ios/FamilyConnect/App/RootView.swift
 */

package me.nettrash.familyconnect

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dagger.hilt.android.AndroidEntryPoint
import me.nettrash.familyconnect.navigation.AppNavHost
import me.nettrash.familyconnect.navigation.startDestinationFor
import me.nettrash.familyconnect.ui.theme.FamilyConnectTheme

@AndroidEntryPoint
class MainActivity : ComponentActivity() {

    private val viewModel: MainViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            val boot by viewModel.bootState.collectAsStateWithLifecycle()
            val snapshot = boot
            if (snapshot == null) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center,
                ) {
                    CircularProgressIndicator()
                }
            } else {
                FamilyConnectTheme {
                    // remember → the NavHost keeps its original start
                    // destination even if recomposition delivers a newer
                    // snapshot; reroutes go through session events.
                    val start = remember { startDestinationFor(snapshot.status) }
                    AppNavHost(
                        startDestination = start,
                        sessionEvents = viewModel.sessionEvents,
                    )
                }
            }
        }
    }
}
