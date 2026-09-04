/*
 * FamilyAdminViewModel.kt
 * Family Connect (Android)
 *
 * Owner console: pending join requests (approve/reject), invite-code
 * rotation, join-policy toggle, the family's language, whether a mention
 * of the assistant sees the chat's recent history, member removal and
 * member birthdays. Every mutation reloads the affected slice from the
 * server — admin actions are rare enough that correctness beats optimism
 * here.
 *
 * iOS counterpart: ios/FamilyConnect/UI/FamilyAdmin/FamilyAdminViewModel.swift
 */

package me.nettrash.familyconnect.ui.familyadmin

import android.content.Context
import dagger.hilt.android.qualifiers.ApplicationContext
import me.nettrash.familyconnect.R
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.JoinRequestDto
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.settings.SettingsRepository
import javax.inject.Inject
import me.nettrash.familyconnect.data.net.dto.ReportDto

@HiltViewModel
class FamilyAdminViewModel @Inject constructor(
    /**
     * The application context, for `getString` only.
     *
     * A ViewModel holding a Context is usually a smell; the APPLICATION
     * context is the exception — it outlives every screen, so there is
     * nothing to leak. The alternative, carrying @StringRes ids through
     * every state field, spreads resource plumbing across code whose job
     * is state. The trade-off: a message is resolved when it is produced
     * rather than when it is drawn, so one already on screen keeps its
     * language if the system locale changes underneath it — and Android
     * recreates the activity then anyway.
     */
    @param:ApplicationContext private val appContext: Context,
    private val familyRepository: FamilyRepository,
    private val settings: SettingsRepository,
) : ViewModel() {

    data class UiState(
        val requests: List<JoinRequestDto> = emptyList(),
        /**
         * The owner's moderation list, oldest first and open only.
         *
         * Never contains a report ABOUT the owner: the server omits those
         * from `GET /families/reports` entirely, so there is nothing to
         * filter here. And it never says who blocked whom — blocking and
         * reporting are independent (docs/protocol.md, "Reporting a
         * member").
         */
        val reports: List<ReportDto> = emptyList(),
        val inviteCode: String? = null,
        val joinPolicy: String = "open",
        /**
         * The family's language tag, or null for UNSET — which is not
         * English (protocol.md, "The family's language"). The screen has
         * to keep the two apart, because the server does.
         */
        val language: String? = null,
        /** Whether a mention carries the chat's recent history. */
        val aiHistory: Boolean = true,
        /**
         * Whether a photograph a member points the assistant at may be
         * shown to the model: in their OWN assistant chat, and — since
         * #56 — on an `@ai` message in the family chat or on the message
         * it replies to (docs/protocol.md, "Pictures" and "Showing the
         * assistant a picture from the family chat"). The sentence under
         * the switch names all three doors.
         *
         * FALSE by default, which is the opposite of [aiHistory] and the
         * point rather than an oversight: `ai_history` widened what a
         * model was already being told, while a photograph is a different
         * kind of disclosure altogether.
         */
        val aiVision: Boolean = false,
        /**
         * The THIRD switch (`ai_history_photos`, docs/protocol.md, "Recent
         * photos from the family chat"): whether an `@ai` mention may
         * also be shown the chat's most recent photographs — pictures
         * nobody pointed the assistant at. FALSE by default, one notch
         * further than [aiVision]: not only a different kind of thing
         * from a sentence, a different kind of ACT — nobody chose these
         * pictures for this question. Only ever true while [aiVision] is;
         * the server turns it off whenever that goes off, so a PATCH
         * answer for either is the truth for both.
         */
        val aiHistoryPhotos: Boolean = false,
        /**
         * Whether this SERVER has a deployment that can look at a picture
         * at all (`assistant.vision`).
         *
         * The switch above is HIDDEN when this is false — not disabled.
         * A greyed-out control on a server that has no vision deployment
         * would be a control that lies about what turning it on would do,
         * and an owner would reasonably read it as "my server could, but
         * something is stopping me".
         */
        val assistantVision: Boolean = false,
        /** The owner's own cap, or null for none of their own. */
        val maxMembers: Int? = null,
        /** The operator's ceiling; null on a server too old to say. */
        val ceiling: Int? = null,
        /** Live members, for the caption and the seed. */
        val memberCount: Int = 0,
        val busy: Boolean = false,
        val error: String? = null,
    ) {
        /**
         * How the third switch is drawn: offered, or DISABLED with its
         * reason. Unlike [aiVision]'s switch, which is hidden on a server
         * that cannot see, this one is present in both withheld states,
         * because the protocol asks for exactly that — one reason is
         * something the owner can act on (turn pictures on first; the
         * server refuses this while they are off) and the other is
         * something they deserve to be told rather than left to find
         * missing (docs/protocol.md, "Recent photos from the family chat"
         * — "What a client shows").
         */
        val historyPhotosSwitch: HistoryPhotosSwitch
            get() = HistoryPhotosSwitch.of(serverCanSee = assistantVision, familyAllowsPhotos = aiVision)
    }

    /**
     * The third switch's state from the two locks under it. The vision
     * deployment is asked first: an owner on a server that cannot see is
     * told that, not "turn pictures on first" — which they could do, to
     * no effect. `ai_history` is deliberately NOT an input: with it off
     * the switch is inert rather than withheld (there is no transcript
     * for it to widen) and the server accepts it all the same; the screen
     * says so beside it instead.
     */
    enum class HistoryPhotosSwitch {
        /** Both locks under it are open; the owner may turn it either way. */
        OFFERED,
        /** `assistant.vision` is false: the assistant here cannot look at pictures at all. */
        WITHHELD_NO_VISION_DEPLOYMENT,
        /** `ai_vision` is off: the server refuses this while that is. */
        WITHHELD_VISION_OFF,
        ;

        val isEnabled: Boolean get() = this == OFFERED

        companion object {
            fun of(serverCanSee: Boolean, familyAllowsPhotos: Boolean): HistoryPhotosSwitch = when {
                !serverCanSee -> WITHHELD_NO_VISION_DEPLOYMENT
                !familyAllowsPhotos -> WITHHELD_VISION_OFF
                else -> OFFERED
            }
        }
    }

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state

    /** Live roster straight from Room (WS frames keep it fresh). */
    // Active only: this list offers Remove and Reset Password, and
    // neither means anything for somebody who already left.
    val members: StateFlow<List<MemberEntity>> = familyRepository.observeActiveMembers()
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    val myUserId: StateFlow<Long?> = settings.state.map { it.myUserId }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    /**
     * Owners get the whole screen; everyone else gets the roster only.
     * The member list itself is not owner-gated on the server — only the
     * invite code, the join policy, the requests and removal are — so a
     * plain member can see who is in the family.
     */
    val isOwner: StateFlow<Boolean> = settings.state
        .map { it.familyStatus == FamilyStatus.OWNER }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), false)

    init {
        load()
    }

    fun load() {
        viewModelScope.launch {
            familyRepository.refreshMine().okOrNull()?.let { mine ->
                _state.update {
                    it.copy(
                        inviteCode = mine.family.inviteCode,
                        joinPolicy = mine.family.joinPolicy,
                        language = mine.family.language,
                        aiHistory = mine.family.aiHistory,
                        aiVision = mine.family.aiVision,
                        aiHistoryPhotos = mine.family.aiHistoryPhotos,
                        assistantVision = mine.assistant?.vision == true,
                        maxMembers = mine.family.maxMembers,
                        memberCount = mine.members.size,
                        ceiling = settings.state.first().maxFamilyMembers,
                    )
                }
            }
            // Join requests are an owner-only endpoint: asking as a plain
            // member is a guaranteed 403, which would paint an error over
            // a screen that is otherwise perfectly useful to them.
            if (settings.state.first().familyStatus == FamilyStatus.OWNER) {
                familyRepository.reports().okOrNull()?.let { response ->
                    _state.update { it.copy(reports = response.reports) }
                }
                loadRequests()
            }
        }
    }

    private suspend fun loadRequests() {
        when (val result = familyRepository.joinRequests()) {
            is ApiResult.Ok -> _state.update { it.copy(requests = result.value.requests) }
            is ApiResult.HttpError ->
                _state.update { it.copy(error = result.message ?: appContext.getString(R.string.e_load_requests_failed)) }
            is ApiResult.NetworkError ->
                _state.update { it.copy(error = appContext.getString(R.string.e_unreachable)) }
        }
    }

    fun approve(requestId: Long, onSuccess: () -> Unit = {}) =
        mutate(onSuccess) { familyRepository.approve(requestId) }

    fun reject(requestId: Long, onSuccess: () -> Unit = {}) =
        mutate(onSuccess) { familyRepository.reject(requestId) }

    fun rotateInviteCode() {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = familyRepository.rotateInviteCode()) {
                is ApiResult.Ok ->
                    _state.update { it.copy(busy = false, inviteCode = result.value.inviteCode) }
                is ApiResult.HttpError ->
                    _state.update { it.copy(busy = false, error = result.message ?: appContext.getString(R.string.e_rotate_failed)) }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(busy = false, error = appContext.getString(R.string.e_unreachable)) }
            }
        }
    }

    /**
     * Set or clear the family's member cap.
     *
     * `null` CLEARS it, which is NOT the same as setting it to the
     * operator's ceiling — the request carries a real JSON null for that
     * (docs/protocol.md, `PATCH /families/mine`).
     */
    fun setMemberCap(cap: Int?) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = familyRepository.setMemberCap(cap)) {
                is ApiResult.Ok ->
                    _state.update { it.copy(busy = false, maxMembers = result.value.family.maxMembers) }
                is ApiResult.HttpError ->
                    _state.update { it.copy(busy = false, error = result.message ?: appContext.getString(R.string.e_member_limit_failed)) }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(busy = false, error = appContext.getString(R.string.e_unreachable)) }
            }
        }
    }

    /**
     * Take one report off the list.
     *
     * Says nothing about what the owner DID: this protocol has removing a
     * member, resetting a password and closing the family; it does not
     * have deleting somebody else's message. Idempotent server-side, so a
     * double tap and a retry after a timeout are the same request twice
     * and neither is an error.
     */
    fun resolveReport(reportId: Long) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = familyRepository.resolveReport(reportId)) {
                is ApiResult.Ok ->
                    _state.update { s -> s.copy(busy = false, reports = s.reports.filterNot { it.id == reportId }) }
                is ApiResult.HttpError ->
                    _state.update { it.copy(busy = false, error = result.message ?: appContext.getString(R.string.e_resolve_report_failed)) }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(busy = false, error = appContext.getString(R.string.e_unreachable)) }
            }
        }
    }

    /** The operator's published contact, for the report sheet. */
    val supportContact: StateFlow<String?> = settings.state.map { it.supportContact }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    /** Everybody this reader has blocked, for the roster's Block/Unblock. */
    val blockedUserIds: StateFlow<Set<Long>> = settings.state.map { it.blockedUserIds }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptySet())

    /**
     * Block or unblock one member from the roster.
     *
     * NOT owner-gated, unlike everything else on this screen: any member
     * may block any other, the owner included (docs/protocol.md, "Blocking
     * a member"). The request first, then the local write — never
     * optimistic.
     */
    fun setBlocked(userId: Long, blocked: Boolean) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            val result = if (blocked) familyRepository.block(userId) else familyRepository.unblock(userId)
            _state.update {
                it.copy(
                    busy = false,
                    error = if (result is ApiResult.Ok) null else appContext.getString(R.string.e_block_failed),
                )
            }
        }
    }

    /**
     * Report a PERSON — `messageId` is null, which is what makes the roster
     * the only way to report somebody without singling out one message.
     */
    fun reportMember(userId: Long, reason: String, onDone: () -> Unit) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            val result = familyRepository.report(userId, reason, messageId = null)
            _state.update {
                it.copy(
                    busy = false,
                    error = if (result is ApiResult.Ok<*>) null else appContext.getString(R.string.e_report_failed),
                )
            }
            if (result is ApiResult.Ok<*>) onDone()
        }
    }

    fun setJoinPolicy(policy: String) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = familyRepository.setJoinPolicy(policy)) {
                is ApiResult.Ok ->
                    _state.update { it.copy(busy = false, joinPolicy = result.value.family.joinPolicy) }
                is ApiResult.HttpError ->
                    _state.update { it.copy(busy = false, error = result.message ?: appContext.getString(R.string.e_change_policy_failed)) }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(busy = false, error = appContext.getString(R.string.e_unreachable)) }
            }
        }
    }

    /**
     * Owner-only. A null tag CLEARS the language; the state that comes
     * back is the server's, so an unchanged one shows the picker was
     * refused rather than pretending it took.
     */
    fun setLanguage(tag: String?) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = familyRepository.setLanguage(tag)) {
                is ApiResult.Ok ->
                    _state.update { it.copy(busy = false, language = result.value.family.language) }
                is ApiResult.HttpError ->
                    _state.update {
                        it.copy(
                            busy = false,
                            error = result.message ?: appContext.getString(R.string.e_change_language_failed),
                        )
                    }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(busy = false, error = appContext.getString(R.string.e_unreachable)) }
            }
        }
    }

    /** Owner-only: what a mention of the assistant is allowed to see. */
    fun setAiHistory(enabled: Boolean) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = familyRepository.setAiHistory(enabled)) {
                is ApiResult.Ok ->
                    _state.update { it.copy(busy = false, aiHistory = result.value.family.aiHistory) }
                is ApiResult.HttpError ->
                    _state.update {
                        it.copy(
                            busy = false,
                            error = result.message ?: appContext.getString(R.string.e_change_assistant_history_failed),
                        )
                    }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(busy = false, error = appContext.getString(R.string.e_unreachable)) }
            }
        }
    }

    /**
     * Owner-only: whether a photograph a member attaches in their own
     * assistant chat may be shown to the model.
     *
     * One of THREE locks, and the only one an owner holds. The operator
     * must also have configured a deployment that can see — without one
     * this screen shows no switch at all — and the member must attach the
     * picture to the question themselves, which is per-picture consent
     * and is deliberately never a remembered setting anywhere in this app
     * (docs/protocol.md, "Pictures").
     */
    fun setAiVision(enabled: Boolean) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = familyRepository.setAiVision(enabled)) {
                is ApiResult.Ok ->
                    _state.update {
                        it.copy(
                            busy = false,
                            aiVision = result.value.family.aiVision,
                            // Turning this OFF turns the third switch off
                            // in the same write on the server, asked or
                            // not — the answer carries both, and both are
                            // drawn from it.
                            aiHistoryPhotos = result.value.family.aiHistoryPhotos,
                        )
                    }
                is ApiResult.HttpError ->
                    _state.update {
                        it.copy(
                            busy = false,
                            error = result.message ?: appContext.getString(R.string.e_change_assistant_vision_failed),
                        )
                    }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(busy = false, error = appContext.getString(R.string.e_unreachable)) }
            }
        }
    }

    /**
     * Owner-only: the third switch (docs/protocol.md, "Recent photos from
     * the family chat"). The screen never calls this while the switch is
     * withheld — the server would answer `validation` to `true` under a
     * shut `ai_vision` — but a refusal that does arrive is shown, and the
     * switch stays where the server says it is.
     */
    fun setAiHistoryPhotos(enabled: Boolean) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = familyRepository.setAiHistoryPhotos(enabled)) {
                is ApiResult.Ok ->
                    _state.update {
                        it.copy(busy = false, aiHistoryPhotos = result.value.family.aiHistoryPhotos)
                    }
                is ApiResult.HttpError ->
                    _state.update {
                        it.copy(
                            busy = false,
                            error = result.message
                                ?: appContext.getString(R.string.e_change_assistant_history_photos_failed),
                        )
                    }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(busy = false, error = appContext.getString(R.string.e_unreachable)) }
            }
        }
    }

    /**
     * Owner-only, and the owner may name themselves — the roster offers
     * this on every row, including their own (protocol.md, "Birthdays").
     * The repository mirrors the answer onto the Room row, so the list
     * redraws without another round trip.
     */
    fun setMemberBirthday(userId: Long, month: Int, day: Int, onSuccess: () -> Unit = {}) =
        mutateBirthday(onSuccess) { familyRepository.setMemberBirthday(userId, month, day) }

    fun clearMemberBirthday(userId: Long, onSuccess: () -> Unit = {}) =
        mutateBirthday(onSuccess) { familyRepository.clearMemberBirthday(userId) }

    /**
     * [mutate] without the join-request reload — a birthday touches
     * neither the requests nor anything else this screen holds, and
     * refetching them on every save would be a request per keystroke of
     * the picker.
     *
     * A `validation` from the server is SHOWN rather than swallowed: the
     * picker cannot offer an impossible date, but the server is the
     * authority on which dates exist and a silent failure would leave the
     * roster claiming a birthday nobody stored.
     */
    private fun mutateBirthday(onSuccess: () -> Unit, block: suspend () -> ApiResult<*>) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = block()) {
                is ApiResult.Ok -> onSuccess()
                is ApiResult.HttpError ->
                    _state.update {
                        it.copy(error = result.message ?: appContext.getString(R.string.e_birthday_failed))
                    }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(error = appContext.getString(R.string.e_unreachable)) }
            }
            _state.update { it.copy(busy = false) }
        }
    }

    fun removeMember(userId: Long, onSuccess: () -> Unit = {}) =
        mutate(onSuccess) { familyRepository.removeMember(userId) }

    /**
     * Owner-only: set a member's password for them. Every session they have
     * is revoked server-side, so their devices return to login.
     */
    fun resetMemberPassword(userId: Long, newPassword: String, onSuccess: () -> Unit = {}) =
        mutate(onSuccess) { familyRepository.resetMemberPassword(userId, newPassword) }

    /** Runs [block]; [onSuccess] fires only when the server confirms. */
    private fun mutate(onSuccess: () -> Unit = {}, block: suspend () -> ApiResult<*>) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = block()) {
                is ApiResult.Ok -> onSuccess()
                is ApiResult.HttpError ->
                    _state.update {
                        it.copy(
                            error = when (result.code) {
                                // The cap is re-checked at approval,
                                // because the roster can fill between a
                                // request and the decision. The request
                                // stays PENDING — a full family is a
                                // temporary condition, not a decision — so
                                // say that rather than something the owner
                                // cannot act on (docs/protocol.md,
                                // `POST /families/join-requests/{id}/approve`).
                                "family_full" -> appContext.getString(R.string.e_family_full_approve)
                                else -> result.message ?: appContext.getString(R.string.e_action_failed)
                            },
                        )
                    }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(error = appContext.getString(R.string.e_unreachable)) }
            }
            loadRequests()
            _state.update { it.copy(busy = false) }
        }
    }
}
