/*
 * MainViewModel.kt
 * Family Connect (Android)
 *
 * Boot gate: loads ONE SessionSnapshot so MainActivity knows the start
 * destination, then gets out of the way. Deliberately not a live flow —
 * the NavHost must be composed exactly once with a stable start
 * destination; everything after boot navigates via events, never by
 * re-seeding the graph.
 *
 * Also holds the push-tap deep link: MainActivity parses notification
 * extras (cold start AND onNewIntent) into a PendingRoute which sits
 * here as state until AppNavHost consumes it exactly once — StateFlow
 * rather than an event flow because the NavHost may not be composed yet
 * when a cold-start tap arrives (bootState is still loading).
 *
 * And the OS-share flow (ACTION_SEND / ACTION_SEND_MULTIPLE): decide
 * what arrived (ShareIn), copy the bytes to cache IMMEDIATELY (the read
 * grants are transient — ShareImporter), park the result in ShareStash,
 * and drive the chat picker through [shareFlow]. Nothing auto-sends:
 * the chosen chat's composer drains the stash into its staging.
 *
 * iOS counterpart: ios/FamilyConnect/App/RootViewModel.swift
 */

package me.nettrash.familyconnect

import android.net.Uri
import androidx.annotation.StringRes
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.push.PendingRoute
import me.nettrash.familyconnect.calls.CallState
import me.nettrash.familyconnect.calls.CallStarter
import me.nettrash.familyconnect.calls.CallStateSource
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.SessionEvent
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.data.repo.SessionSnapshot
import me.nettrash.familyconnect.data.repo.ShareImporter
import me.nettrash.familyconnect.data.repo.ShareIn
import me.nettrash.familyconnect.data.repo.ShareStash
import javax.inject.Inject

@HiltViewModel
class MainViewModel @Inject constructor(
    private val sessionRepository: SessionRepository,
    /** Defaulted for the tests; Dagger ignores the default and injects CallManager. */
    private val calls: CallStateSource = CallStateSource.NONE,
    /** The call log's call-back (PendingRoute.CallBack); same trick. */
    private val callStarter: CallStarter = CallStarter { _, _, _ -> false },
    /** Same trick: the tests never share; Dagger injects DefaultShareImporter. */
    private val shareImporter: ShareImporter = ShareImporter.NONE,
    /** And the app-wide stash the chosen chat's composer drains. */
    private val shareStash: ShareStash = ShareStash(),
) : ViewModel() {

    /** The one voice call this device can be on — AppNavHost shows the call screen off it. */
    val callState: StateFlow<CallState> = calls.state

    /** The notification's Answer button: remembered until the call screen can act on it. */
    fun requestAnswer() = calls.requestAnswer()

    private val _bootState = MutableStateFlow<SessionSnapshot?>(null)

    /** null while loading → spinner; then the one-shot boot snapshot. */
    val bootState: StateFlow<SessionSnapshot?> = _bootState

    /** Expired / removed-from-family reroutes, relayed to the NavHost. */
    val sessionEvents: SharedFlow<SessionEvent> = sessionRepository.sessionEvents

    /**
     * LIVE session status, for the decisions made long after boot — the
     * share picker's navigation gate. [bootState] stays a one-shot on
     * purpose (the NavHost's start destination must never be re-seeded);
     * this flow is for rules that must see the CURRENT session instead,
     * exactly as [onShared] reads a fresh snapshot rather than the boot
     * one. Null until the first emission.
     */
    val sessionStatus: StateFlow<FamilyStatus?> = sessionRepository.sessionFlow
        .map { it.status }
        .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    private val _pendingRoute = MutableStateFlow<PendingRoute?>(null)

    /** Notification-tap deep link; AppNavHost consumes it exactly once. */
    val pendingRoute: StateFlow<PendingRoute?> = _pendingRoute

    init {
        viewModelScope.launch {
            _bootState.value = sessionRepository.snapshot()
        }
    }

    /** A newer tap wins — the user tapped it last, it's what they want. */
    fun onPendingRoute(route: PendingRoute) {
        _pendingRoute.value = route
    }

    fun consumePendingRoute() {
        _pendingRoute.value = null
    }

    /** The Phone app's call log: ring them again. False when a call is already up. */
    fun callBack(route: PendingRoute.CallBack): Boolean =
        callStarter.startCall(route.chatId, route.peerUserId, route.video)

    // -- The OS share target --------------------------------------------------

    /** Where the share flow is, for AppNavHost's overlay. Null = no share. */
    sealed interface ShareFlow {
        /** The bytes are still being copied to cache; a sheet says so. */
        data object Preparing : ShareFlow

        /** Everything is stashed; the chat picker is up. */
        data class ChooseChat(val itemCount: Int, val hasText: Boolean) : ShareFlow
    }

    /** One sentence the activity shows as a toast, then consumes. */
    data class ShareNotice(@param:StringRes val resId: Int, val arg: Int? = null)

    private val _shareFlow = MutableStateFlow<ShareFlow?>(null)
    val shareFlow: StateFlow<ShareFlow?> = _shareFlow

    private val _shareNotice = MutableStateFlow<ShareNotice?>(null)
    val shareNotice: StateFlow<ShareNotice?> = _shareNotice

    fun consumeShareNotice() {
        _shareNotice.value = null
    }

    /** The in-flight share import; cancelling the flow must kill it too. */
    private var shareJob: Job? = null

    /**
     * Something was shared at this app. [streams] describe [uris] item by
     * item (scheme + media type), built by the activity because that is
     * where the ContentResolver is; [text] is EXTRA_TEXT, if any.
     *
     * Signed out, or in no family: the app shows itself normally and the
     * share is dropped with a notice — there is no composer to land in,
     * and holding bytes hostage until a login would surprise more than
     * it would help. The status is read FRESH from the repository, not
     * from the boot snapshot, because a share can arrive long after boot.
     */
    fun onShared(uris: List<Uri>, streams: List<ShareIn.Stream>, text: String?) {
        // A newer share supersedes an in-flight one: the older import must
        // not run to completion and clobber the newer share's stash.
        shareJob?.cancel()
        shareJob = viewModelScope.launch {
            val snapshot = sessionRepository.snapshot()
            if (!snapshot.canChat) {
                if (uris.isNotEmpty() || !text.isNullOrBlank()) {
                    _shareNotice.value = ShareNotice(R.string.e_share_unavailable)
                }
                return@launch
            }
            when (val verdict = ShareIn.decide(streams, text)) {
                is ShareIn.Verdict.Words -> {
                    // Shared TEXT becomes composer text, never an
                    // attachment — the PastedMedia-style rule, one level up.
                    shareStash.deposit(emptyList(), verdict.text)
                    _shareFlow.value = ShareFlow.ChooseChat(itemCount = 0, hasText = true)
                }
                is ShareIn.Verdict.Attach -> {
                    _shareFlow.value = ShareFlow.Preparing
                    if (verdict.dropped > 0) {
                        _shareNotice.value = ShareNotice(
                            R.string.e_share_too_many,
                            AttachmentDto.MAX_PER_MESSAGE,
                        )
                    }
                    val prepared = shareImporter.prepare(verdict.indices.map { uris[it] })
                    // Dismissing the preparing sheet cancels this job. A
                    // cooperative importer dies inside prepare(); one that
                    // finished the copy anyway must not resurrect the
                    // dismissed flow — deposit nothing, and delete what it
                    // prepared, because nobody will ever claim it.
                    if (!isActive) {
                        prepared.forEach { it.file.delete() }
                        return@launch
                    }
                    if (prepared.isEmpty()) {
                        _shareNotice.value = ShareNotice(R.string.e_share_failed)
                        _shareFlow.value = null
                    } else {
                        shareStash.deposit(prepared, text = null)
                        _shareFlow.value =
                            ShareFlow.ChooseChat(itemCount = prepared.size, hasText = false)
                    }
                }
                ShareIn.Verdict.Nothing -> Unit
            }
        }
    }

    /** The picker chose: aim the stash and close the flow; the NavHost navigates. */
    fun shareChatChosen(chatId: Long) {
        shareStash.target(chatId)
        _shareFlow.value = null
    }

    /** The picker was dismissed: the share is over, cache files and all. */
    fun cancelShare() {
        // Kill the import FIRST: a prepare still running would otherwise
        // deposit into the stash after this discard and resurrect the
        // flow (or delete a newer share's files out from under it).
        shareJob?.cancel()
        shareJob = null
        shareStash.discard()
        _shareFlow.value = null
    }
}
