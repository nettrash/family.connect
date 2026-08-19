/*
 * ServerSetupViewModel.kt
 * Family Connect (Android)
 *
 * Normalize → probe → save. "Alive" means: unauthenticated GET /me came
 * back 401 *with a parsed protocol error code* — anything else on that
 * path (nginx 404, a random website's 401, connection refused) is not a
 * Family Connect server.
 *
 * The field pre-fills with the URL currently in effect (typed earlier,
 * or the store build's compiled-in default adopted at boot) so "Use a
 * different server" edits the current address instead of a blank field;
 * saving a different URL overrides the default persistently. On a true
 * first run without a default there is nothing stored — stays blank.
 *
 * iOS counterpart: ios/FamilyConnect/UI/ServerSetup/ServerSetupViewModel.swift
 */

package me.nettrash.familyconnect.ui.serversetup

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.AuthApi
import me.nettrash.familyconnect.data.settings.ServerUrlNormalizer
import me.nettrash.familyconnect.data.settings.SettingsRepository
import javax.inject.Inject

@HiltViewModel
class ServerSetupViewModel @Inject constructor(
    private val authApi: AuthApi,
    private val settings: SettingsRepository,
) : ViewModel() {

    data class UiState(
        val url: String = "",
        val probing: Boolean = false,
        val error: String? = null,
        val showSaveAnyway: Boolean = false,
        val isCleartext: Boolean = false,
        val saved: Boolean = false,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state

    init {
        // Pre-fill with the stored URL, if any. Guarded on url.isEmpty()
        // so a user who starts typing before the DataStore read lands
        // doesn't get their input clobbered.
        viewModelScope.launch {
            val existing = settings.state.first().serverUrl ?: return@launch
            _state.update { current ->
                if (current.url.isEmpty()) {
                    current.copy(
                        url = existing,
                        isCleartext = ServerUrlNormalizer.isCleartext(existing),
                    )
                } else {
                    current
                }
            }
        }
    }

    fun onUrlChange(value: String) {
        val normalized = ServerUrlNormalizer.normalize(value)
        _state.update {
            it.copy(
                url = value,
                error = null,
                showSaveAnyway = false,
                isCleartext = normalized != null && ServerUrlNormalizer.isCleartext(normalized),
            )
        }
    }

    fun probeAndSave() {
        val normalized = ServerUrlNormalizer.normalize(_state.value.url)
        if (normalized == null) {
            _state.update { it.copy(error = "That doesn't look like a valid address") }
            return
        }
        viewModelScope.launch {
            _state.update { it.copy(probing = true, error = null, showSaveAnyway = false) }
            val result = authApi.probe(normalized)
            val alive = result is ApiResult.HttpError &&
                result.status == 401 &&
                result.code != null
            if (alive) {
                save(normalized)
            } else {
                _state.update {
                    it.copy(
                        probing = false,
                        error = "No Family Connect server answered at $normalized",
                        showSaveAnyway = true,
                    )
                }
            }
        }
    }

    fun saveAnyway() {
        val normalized = ServerUrlNormalizer.normalize(_state.value.url) ?: return
        viewModelScope.launch { save(normalized) }
    }

    private suspend fun save(normalized: String) {
        settings.setServerUrl(normalized)
        _state.update { it.copy(probing = false, saved = true) }
    }
}
