/*
 * FamilyConnectApp.kt
 * Family Connect (Android)
 *
 * Application singleton. Hilt owns the object graph; the only manual
 * wiring is attaching ChatSocketManager to ProcessLifecycleOwner so the
 * WebSocket follows the *process* foreground state (Activity lifecycles
 * churn on rotation; the process lifecycle doesn't).
 *
 * iOS counterpart: ios/FamilyConnect/App/FamilyConnectApp.swift
 */

package me.nettrash.familyconnect

import android.app.Application
import androidx.lifecycle.ProcessLifecycleOwner
import dagger.hilt.android.HiltAndroidApp
import me.nettrash.familyconnect.data.net.ws.ChatSocketManager
import javax.inject.Inject

@HiltAndroidApp
class FamilyConnectApp : Application() {

    // Injecting the manager here also instantiates the repository graph
    // eagerly, so the WS frame collectors (messages, roster) are live
    // before the first frame can possibly arrive.
    @Inject
    lateinit var chatSocketManager: ChatSocketManager

    override fun onCreate() {
        super.onCreate()
        ProcessLifecycleOwner.get().lifecycle.addObserver(chatSocketManager)
    }
}
