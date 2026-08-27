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

import androidx.compose.animation.AnimatedContentTransitionScope
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.navigation.NavBackStackEntry
import androidx.compose.runtime.LaunchedEffect
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.runtime.getValue
import me.nettrash.familyconnect.data.push.PendingRoute
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.SessionEvent
import me.nettrash.familyconnect.ui.stats.StatisticsScreen
import me.nettrash.familyconnect.ui.auth.AuthScreen
import me.nettrash.familyconnect.ui.board.BoardScreen
import me.nettrash.familyconnect.ui.chat.ChatScreen
import me.nettrash.familyconnect.ui.chatlist.ChatListScreen
import me.nettrash.familyconnect.ui.familyadmin.FamilyAdminScreen
import me.nettrash.familyconnect.ui.familygate.FamilyGateScreen
import me.nettrash.familyconnect.MainViewModel
import me.nettrash.familyconnect.calls.CallState
import me.nettrash.familyconnect.ui.call.CallScreen
import me.nettrash.familyconnect.ui.share.SharePreparingSheet
import me.nettrash.familyconnect.ui.share.ShareTargetSheet
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
    const val BOARD = "board"
    const val STATISTICS = "statistics"
    const val CALL = "call"

    fun chat(chatId: Long) = "chat/$chatId"
}

// Shared-axis X (M3): forward pushes slide in from the right, pops slide
// back out — quarter-width offset so the motion reads as depth, not a
// full-page swipe. Phase resets use a fade-through instead: after a
// popUpTo(0) stack wipe there is no spatial "back", so a slide would
// imply a hierarchy that no longer exists. Whether a transition is a
// phase reset is a property of the transition — either endpoint being a
// phase-gate route — so it is decided here in the NavHost-level
// defaults, not by per-destination overrides (which would leave the
// other half of the transition sliding).
private val phaseGateRoutes = setOf(
    Routes.SERVER_SETUP,
    Routes.AUTH,
    Routes.FAMILY_GATE,
    Routes.WAITING,
)

private fun AnimatedContentTransitionScope<NavBackStackEntry>.isPhaseReset(): Boolean =
    initialState.destination.route in phaseGateRoutes ||
        targetState.destination.route in phaseGateRoutes

private fun sharedAxisEnter(): EnterTransition =
    slideInHorizontally(
        initialOffsetX = { it / 4 },
        animationSpec = tween(300, easing = FastOutSlowInEasing),
    ) + fadeIn(tween(300))

private fun sharedAxisExit(): ExitTransition =
    slideOutHorizontally(
        targetOffsetX = { -it / 4 },
        animationSpec = tween(300, easing = FastOutSlowInEasing),
    ) + fadeOut(tween(300))

private fun sharedAxisPopEnter(): EnterTransition =
    slideInHorizontally(
        initialOffsetX = { -it / 4 },
        animationSpec = tween(300, easing = FastOutSlowInEasing),
    ) + fadeIn(tween(300))

private fun sharedAxisPopExit(): ExitTransition =
    slideOutHorizontally(
        targetOffsetX = { it / 4 },
        animationSpec = tween(300, easing = FastOutSlowInEasing),
    ) + fadeOut(tween(300))

// Fade-through halves for phase-reset transitions: outgoing screen
// fades fully before the incoming one starts (90ms overlap gap).
private fun fadeThroughEnter(): EnterTransition = fadeIn(tween(200, delayMillis = 90))

private fun fadeThroughExit(): ExitTransition = fadeOut(tween(150))

/** Boot-time route for a session status. Pure — pinned by unit tests. */
fun startDestinationFor(status: FamilyStatus): String = when (status) {
    FamilyStatus.NO_SERVER -> Routes.SERVER_SETUP
    FamilyStatus.NO_TOKEN -> Routes.AUTH
    FamilyStatus.NONE -> Routes.FAMILY_GATE
    FamilyStatus.PENDING -> Routes.WAITING
    FamilyStatus.MEMBER, FamilyStatus.OWNER -> Routes.CHAT_LIST
}

/**
 * Whether picking a share target may navigate to that chat NOW.
 *
 * The same status → route rule [startDestinationFor] pins, evaluated on
 * the CURRENT session status — never the frozen boot start destination:
 * someone who logged in or joined a family after boot has a live chat UI
 * even though the NavHost was composed with a gate route as its start,
 * and [MainViewModel.onShared] admits their share off a FRESH snapshot.
 * Gating on the boot route stranded that share's targeted stash with no
 * visible outcome. Null (no emission yet) never navigates. Pure —
 * pinned by unit tests.
 */
fun shareNavigatesToChat(status: FamilyStatus?): Boolean =
    status != null && startDestinationFor(status) == Routes.CHAT_LIST

@Composable
fun AppNavHost(
    startDestination: String,
    sessionEvents: SharedFlow<SessionEvent>,
    pendingRoute: PendingRoute? = null,
    onPendingRouteConsumed: () -> Unit = {},
    isOwner: Boolean = false,
    /** The one voice call this device can be on; the call screen follows it. */
    callState: StateFlow<CallState>? = null,
    /** The OS-share flow — null when no share is in progress (MainViewModel). */
    shareFlow: StateFlow<MainViewModel.ShareFlow?>? = null,
    /** The picker chose a chat: the stash is aimed, then this navigates. */
    onShareChatChosen: (Long) -> Unit = {},
    /** The picker was dismissed: the share (and its cache files) is discarded. */
    onShareCancelled: () -> Unit = {},
    /** LIVE session status — the share picker's navigation gate (see [shareNavigatesToChat]). */
    sessionStatus: StateFlow<FamilyStatus?>? = null,
) {
    val navController = rememberNavController()

    // The call screen is pushed whenever a call exists and popped when it
    // is over — from wherever the person was, and only inside the family
    // UI: a call cannot reach a signed-out device, and the start
    // destination is what says whether this is that UI.
    if (callState != null) {
        val call by callState.collectAsStateWithLifecycle()
        LaunchedEffect(call is CallState.Idle) {
            if (startDestination != Routes.CHAT_LIST) return@LaunchedEffect
            val onCallScreen = navController.currentBackStackEntry?.destination?.route == Routes.CALL
            if (call !is CallState.Idle && !onCallScreen) {
                navController.navigate(Routes.CALL) { launchSingleTop = true }
            } else if (call is CallState.Idle && onCallScreen) {
                navController.popBackStack(Routes.CALL, inclusive = true)
            }
        }
    }

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
                PendingRoute.Board -> navController.navigate(Routes.BOARD)
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
                is PendingRoute.Call ->
                    // The call state collector above shows the screen; a
                    // tap on a call notification only needs the app up.
                    if (navController.currentBackStackEntry?.destination?.route != Routes.CALL) {
                        navController.navigate(Routes.CALL) { launchSingleTop = true }
                    }
            }
        }
        onPendingRouteConsumed()
    }

    // The OS-share overlay: a holding sheet while the bytes copy, then
    // the chat picker. Sheets rather than destinations, because a share
    // is a question layered over wherever the person already is — and
    // dismissing it must leave them exactly there.
    if (shareFlow != null) {
        val share by shareFlow.collectAsStateWithLifecycle()
        val currentStatus = sessionStatus?.collectAsStateWithLifecycle()
        when (share) {
            MainViewModel.ShareFlow.Preparing ->
                SharePreparingSheet(onDismiss = onShareCancelled)
            is MainViewModel.ShareFlow.ChooseChat -> ShareTargetSheet(
                onPick = { chatId ->
                    onShareChatChosen(chatId)
                    // Gated on the CURRENT status, not the frozen boot
                    // start destination: onShared admits a share off a
                    // FRESH snapshot, so a login or family join after
                    // boot must not strand the targeted stash without
                    // navigation (see shareNavigatesToChat).
                    if (shareNavigatesToChat(currentStatus?.value)) {
                        navController.navigate(Routes.chat(chatId))
                    }
                },
                onDismiss = onShareCancelled,
            )
            null -> Unit
        }
    }

    // The NavHost paints its own ground. Every transition below offsets
    // AND cross-fades two screens at once, so for ~300ms neither covers
    // the window opaquely; without a background here the platform
    // window shows through (and under dynamic color it would not match
    // the scheme even when its XML colour does).
    NavHost(
        modifier = Modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.background),
        navController = navController,
        startDestination = startDestination,
        enterTransition = { if (isPhaseReset()) fadeThroughEnter() else sharedAxisEnter() },
        exitTransition = { if (isPhaseReset()) fadeThroughExit() else sharedAxisExit() },
        popEnterTransition = { if (isPhaseReset()) fadeThroughEnter() else sharedAxisPopEnter() },
        popExitTransition = { if (isPhaseReset()) fadeThroughExit() else sharedAxisPopExit() },
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
                onOpenBoard = { navController.navigate(Routes.BOARD) },
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

        composable(Routes.BOARD) {
            BoardScreen(onBack = { navController.popBackStack() })
        }

        composable(Routes.STATISTICS) {
            StatisticsScreen(onBack = { navController.popBackStack() })
        }

        composable(Routes.CALL) {
            CallScreen()
        }

        composable(Routes.SETTINGS) {
            SettingsScreen(
                onBack = { navController.popBackStack() },
                onManageFamily = { navController.navigate(Routes.FAMILY_ADMIN) },
                onOpenStatistics = { navController.navigate(Routes.STATISTICS) },
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
