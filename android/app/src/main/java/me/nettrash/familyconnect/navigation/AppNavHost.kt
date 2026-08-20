/*
 * AppNavHost.kt
 * Family Connect (Android)
 *
 * All navigation decisions in one file: the route table, the
 * status → start-destination mapping, per-screen forward navigation,
 * the session-event reroutes (Expired → AUTH, RemovedFromFamily →
 * FAMILY_GATE) which clear the entire back stack — after either event
 * the old stack refers to state that no longer exists — and the push-tap
 * deep link (PendingRoute), consumed exactly once and only when the
 * start destination is CHAT_LIST (any other start means there is no
 * family UI to deep-link into).
 *
 * Routes are plain strings (navigation-compose's string API) — typed
 * routes would drag in a serialization plugin dependency on the nav
 * graph for eight constants.
 *
 * iOS counterpart: ios/FamilyConnect/App/AppNavigation.swift
 */

package me.nettrash.familyconnect.navigation

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument
import kotlinx.coroutines.flow.SharedFlow
import me.nettrash.familyconnect.data.push.PendingRoute
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.SessionEvent
import me.nettrash.familyconnect.ui.auth.AuthScreen
import me.nettrash.familyconnect.ui.chat.ChatScreen
import me.nettrash.familyconnect.ui.chatlist.ChatListScreen
import me.nettrash.familyconnect.ui.familyadmin.FamilyAdminScreen
import me.nettrash.familyconnect.ui.familygate.FamilyGateScreen
import me.nettrash.familyconnect.ui.serversetup.ServerSetupScreen
import me.nettrash.familyconnect.ui.settings.SettingsScreen
import me.nettrash.familyconnect.ui.waiting.WaitingScreen

object Routes {
    const val SERVER_SETUP = "server_setup"
    const val AUTH = "auth"
    const val FAMILY_GATE = "family_gate"
    const val WAITING = "waiting"
    const val CHAT_LIST = "chat_list"
    const val CHAT = "chat/{chatId}"
    const val SETTINGS = "settings"
    const val FAMILY_ADMIN = "family_admin"

    fun chat(chatId: Long) = "chat/$chatId"
}

/** Boot-time route for a session status. Pure — pinned by unit tests. */
fun startDestinationFor(status: FamilyStatus): String = when (status) {
    FamilyStatus.NO_SERVER -> Routes.SERVER_SETUP
    FamilyStatus.NO_TOKEN -> Routes.AUTH
    FamilyStatus.NONE -> Routes.FAMILY_GATE
    FamilyStatus.PENDING -> Routes.WAITING
    FamilyStatus.MEMBER, FamilyStatus.OWNER -> Routes.CHAT_LIST
}

@Composable
fun AppNavHost(
    startDestination: String,
    sessionEvents: SharedFlow<SessionEvent>,
    pendingRoute: PendingRoute? = null,
    onPendingRouteConsumed: () -> Unit = {},
    isOwner: Boolean = false,
) {
    val navController = rememberNavController()

    LaunchedEffect(Unit) {
        sessionEvents.collect { event ->
            when (event) {
                SessionEvent.Expired -> navController.navigate(Routes.AUTH) {
                    popUpTo(0) { inclusive = true }
                }
                SessionEvent.RemovedFromFamily -> navController.navigate(Routes.FAMILY_GATE) {
                    popUpTo(0) { inclusive = true }
                }
                // Only meaningful while the waiting screen is showing —
                // WaitingViewModel handles it; no global reroute.
                SessionEvent.JoinRequestRejected -> Unit
            }
        }
    }

    // Push-tap deep link. Routable only when the boot snapshot put us in
    // the family UI (start == CHAT_LIST); otherwise the tap still opened
    // the app, which is all a logged-out / pending user can get. Consumed
    // in every case — replaying a stale tap after a later login would be
    // surprising, not helpful.
    LaunchedEffect(pendingRoute) {
        val route = pendingRoute ?: return@LaunchedEffect
        if (startDestination == Routes.CHAT_LIST) {
            when (route) {
                is PendingRoute.Chat -> navController.navigate(Routes.chat(route.chatId))
                PendingRoute.JoinRequests ->
                    // join_request pushes go to the owner (protocol), but a
                    // stale/forged tap from a non-owner degrades to the list.
                    if (isOwner) {
                        navController.navigate(Routes.FAMILY_ADMIN)
                    } else {
                        navController.popBackStack(Routes.CHAT_LIST, inclusive = false)
                    }
                PendingRoute.ChatList ->
                    // Already the start destination — just unwind to it.
                    navController.popBackStack(Routes.CHAT_LIST, inclusive = false)
            }
        }
        onPendingRouteConsumed()
    }

    NavHost(
        navController = navController,
        startDestination = startDestination,
    ) {
        composable(Routes.SERVER_SETUP) {
            ServerSetupScreen(
                onSaved = {
                    navController.navigate(Routes.AUTH) {
                        popUpTo(0) { inclusive = true }
                    }
                },
            )
        }

        composable(Routes.AUTH) {
            AuthScreen(
                onAuthenticated = { status ->
                    navController.navigate(startDestinationFor(status)) {
                        popUpTo(0) { inclusive = true }
                    }
                },
                onChangeServer = {
                    navController.navigate(Routes.SERVER_SETUP) {
                        popUpTo(0) { inclusive = true }
                    }
                },
            )
        }

        composable(Routes.FAMILY_GATE) {
            FamilyGateScreen(
                onJoined = {
                    navController.navigate(Routes.CHAT_LIST) {
                        popUpTo(0) { inclusive = true }
                    }
                },
                onPending = {
                    navController.navigate(Routes.WAITING) {
                        popUpTo(0) { inclusive = true }
                    }
                },
            )
        }

        composable(Routes.WAITING) {
            WaitingScreen(
                onApproved = {
                    navController.navigate(Routes.CHAT_LIST) {
                        popUpTo(0) { inclusive = true }
                    }
                },
                onBackToGate = {
                    navController.navigate(Routes.FAMILY_GATE) {
                        popUpTo(0) { inclusive = true }
                    }
                },
            )
        }

        composable(Routes.CHAT_LIST) {
            ChatListScreen(
                onOpenChat = { chatId -> navController.navigate(Routes.chat(chatId)) },
                onOpenSettings = { navController.navigate(Routes.SETTINGS) },
            )
        }

        composable(
            route = Routes.CHAT,
            arguments = listOf(navArgument("chatId") { type = NavType.LongType }),
        ) {
            ChatScreen(
                onBack = { navController.popBackStack() },
            )
        }

        composable(Routes.SETTINGS) {
            SettingsScreen(
                onBack = { navController.popBackStack() },
                onManageFamily = { navController.navigate(Routes.FAMILY_ADMIN) },
                onLoggedOut = {
                    navController.navigate(Routes.AUTH) {
                        popUpTo(0) { inclusive = true }
                    }
                },
            )
        }

        composable(Routes.FAMILY_ADMIN) {
            FamilyAdminScreen(
                onBack = { navController.popBackStack() },
            )
        }
    }
}
