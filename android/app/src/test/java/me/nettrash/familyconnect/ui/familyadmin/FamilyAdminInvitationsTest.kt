/*
 * FamilyAdminInvitationsTest.kt
 * Family Connect (Android)
 *
 * The owner's screen with several families (#88; docs/protocol.md,
 * "Several families"): "Invite…" offers the members of the owner's OTHER
 * families who are not in this one and is hidden with no other family;
 * the pending list follows the frames; and a password reset is offered
 * except where `password_reset` is explicitly false — absent is the old
 * rule — with `member_in_other_families` said in the app's own words.
 */

package me.nettrash.familyconnect.ui.familyadmin

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.ApproveResponse
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.InvitationDto
import me.nettrash.familyconnect.data.net.dto.InvitationResponse
import me.nettrash.familyconnect.data.net.dto.InvitationsResponse
import me.nettrash.familyconnect.data.net.dto.UserDto
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeFamilyApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.memberDto
import me.nettrash.familyconnect.testutil.userDto
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class FamilyAdminInvitationsTest {

    private companion object {
        const val ME = 7L
        const val MINE = 5L
        const val PARENTS = 3L
    }

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private val familyApi = FakeFamilyApi()
    private val socket = FakeChatSocket()
    private val mine = FamilyDto(id = MINE, name = "The Joneses", joinPolicy = "open", role = "owner")
    private val parents = FamilyDto(id = PARENTS, name = "The Smiths", joinPolicy = "open", role = "member")
    private val settings = FakeSettingsRepository(
        SettingsState(
            serverUrl = "https://chat.example.com",
            familyStatus = FamilyStatus.OWNER,
            familyName = "The Joneses",
            myUserId = ME,
            families = listOf(parents, mine),
            currentFamilyId = MINE,
            multiFamilyServer = true,
        ),
    )

    private val invitation = InvitationDto(
        id = 4,
        familyId = MINE,
        familyName = "The Joneses",
        invitedBy = UserDto(ME, "anna", "Anna"),
        user = UserDto(11, "ben", "Ben"),
    )

    @Before
    fun setUp() {
        Dispatchers.setMain(dispatcher)
        db = createTestDb(dispatcher)
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        Dispatchers.resetMain()
        db.close()
    }

    private fun viewModel() = FamilyAdminViewModel(
        appContext = RuntimeEnvironment.getApplication(),
        familyRepository = FamilyRepository(
            familyApi = familyApi,
            authApi = FakeAuthApi(),
            memberDao = db.memberDao(),
            settings = settings,
            sessionRepository = SessionRepository(
                authApi = FakeAuthApi(),
                tokenStore = FakeTokenStore("tok"),
                settings = settings,
                wiper = RecordingWiper(),
                unauthorizedEvents = MutableSharedFlow(),
                scope = repoScope,
            ),
            socket = socket,
            scope = repoScope,
        ),
        settings = settings,
    )

    private fun member(familyId: Long, userId: Long, name: String) =
        MemberEntity(userId = userId, username = name, displayName = name, role = "member", familyId = familyId)

    @Test
    fun `Invite offers the members of my other families who are not in this one`() = runTest(dispatcher) {
        db.memberDao().upsertAll(
            listOf(
                member(MINE, ME, "Anna"),
                member(MINE, 9, "Zoe"),
                member(PARENTS, ME, "Anna"),
                member(PARENTS, 9, "Zoe"),
                member(PARENTS, 11, "Ben"),
            ),
        )
        familyApi.mineResult = ApiResult.Ok(
            FamilyMineResponse(family = mine, members = listOf(memberDto(ME), memberDto(9, "Zoe"))),
        )
        familyApi.invitationsResult = ApiResult.Ok(InvitationsResponse(listOf(invitation)))
        familyApi.inviteResult = ApiResult.Ok(InvitationResponse(invitation))
        val vm = viewModel()
        backgroundScope.launch { vm.inviteCandidates.collect {} }
        backgroundScope.launch { vm.canInvite.collect {} }
        runCurrent()

        assertThat(vm.canInvite.value).isTrue()
        assertThat(vm.inviteCandidates.value.map { it.userId }).containsExactly(11L)
        assertThat(vm.state.value.invitations).containsExactly(invitation)

        vm.invite(11)
        runCurrent()
        assertThat(familyApi.invitesSent).containsExactly(MINE to 11L)

        vm.withdrawInvitation(4)
        runCurrent()
        assertThat(familyApi.invitationsWithdrawn).containsExactly(MINE to 4L)
    }

    @Test
    fun `with no other family there is nobody to invite from`() = runTest(dispatcher) {
        settings.setFamilies(listOf(mine), emptyList(), emptyList(), true, 5, true)
        val vm = viewModel()
        backgroundScope.launch { vm.canInvite.collect {} }
        runCurrent()

        assertThat(vm.canInvite.value).isFalse()
    }

    @Test
    fun `the pending list follows the invitation frames`() = runTest(dispatcher) {
        val vm = viewModel()
        runCurrent()
        assertThat(vm.state.value.invitations).isEmpty()

        familyApi.invitationsResult = ApiResult.Ok(InvitationsResponse(listOf(invitation)))
        socket.emit(ServerFrame.Invitation(invitation))
        runCurrent()

        assertThat(vm.state.value.invitations).containsExactly(invitation)
    }

    @Test
    fun `a reset is offered unless the server says false, and absent is the old rule`() = runTest(dispatcher) {
        familyApi.mineResult = ApiResult.Ok(
            FamilyMineResponse(
                family = mine,
                members = listOf(
                    memberDto(ME, role = "owner"),
                    memberDto(9).copy(passwordReset = true),
                    memberDto(11).copy(passwordReset = false),
                    memberDto(12),
                ),
            ),
        )
        val vm = viewModel()
        runCurrent()

        assertThat(vm.state.value.canReset(9)).isTrue()
        assertThat(vm.state.value.canReset(11)).isFalse()
        assertThat(vm.state.value.canReset(12)).isTrue()
    }

    /**
     * A newcomer who arrives while the screen is open — by open join or an
     * invitation, announced by `member_joined`, or approved right here — is
     * drawn with the answer the frame or the approval carried, not offered a
     * reset the server refuses with `member_in_other_families`.
     */
    @Test
    fun `a member who arrives while the screen is open carries their password_reset`() = runTest(dispatcher) {
        familyApi.mineResult = ApiResult.Ok(
            FamilyMineResponse(family = mine, members = listOf(memberDto(ME, role = "owner"))),
        )
        val vm = viewModel()
        runCurrent()

        socket.emit(ServerFrame.MemberJoined(familyId = MINE, user = userDto(12, "dan"), passwordReset = false))
        runCurrent()
        assertThat(vm.state.value.canReset(12)).isFalse()

        familyApi.approveResult = ApiResult.Ok(ApproveResponse(memberDto(13, "eve").copy(passwordReset = false)))
        vm.approve(21)
        runCurrent()
        assertThat(vm.state.value.canReset(13)).isFalse()

        // Absent — the old rule — offers it.
        socket.emit(ServerFrame.MemberJoined(familyId = MINE, user = userDto(14, "fay")))
        runCurrent()
        assertThat(vm.state.value.canReset(14)).isTrue()
    }

    @Test
    fun `a member also in a family I do not own is said so`() = runTest(dispatcher) {
        familyApi.resetPasswordResult = ApiResult.HttpError(403, "member_in_other_families", null)
        val vm = viewModel()
        runCurrent()

        vm.resetMemberPassword(11, "secret12")
        runCurrent()

        assertThat(familyApi.familyCalls).contains("resetMemberPassword" to MINE)
        assertThat(vm.state.value.error)
            .isEqualTo("This member is also in a family you don't own, so only they can change their password.")
    }
}
