/*
 * StatisticsViewModel.kt
 * Family Connect (Android)
 *
 * One fetch, three states. Nothing is cached: statistics are a page opened
 * occasionally, and a stale count would be worse than a moment's spinner.
 */

package me.nettrash.familyconnect.ui.stats

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.FamilyApi
import me.nettrash.familyconnect.data.net.dto.FamilyStatsDto
import javax.inject.Inject

@HiltViewModel
class StatisticsViewModel @Inject constructor(
    private val familyApi: FamilyApi,
) : ViewModel() {

    sealed interface State {
        data object Loading : State
        data class Loaded(val stats: FamilyStatsDto) : State
        data object Failed : State
    }

    private val _state = MutableStateFlow<State>(State.Loading)
    val state: StateFlow<State> = _state

    init {
        load()
    }

    fun load() {
        _state.value = State.Loading
        viewModelScope.launch {
            _state.value = when (val result = familyApi.stats()) {
                is ApiResult.Ok -> State.Loaded(result.value)
                // A failure of either kind reads the same to the person
                // looking: the numbers are not here.
                is ApiResult.HttpError, is ApiResult.NetworkError -> State.Failed
            }
        }
    }
}
