/*
 * SeveralFamiliesSessionTest.kt
 * Family Connect (Android)
 *
 * The session as a LIST of families (#88; docs/protocol.md, "Several
 * families"): `GET /me`'s `families` stored with a current family that
 * falls back to the first, a server from before read as a one-entry list,
 * rows from before claimed for the home family, and a family that goes
 * taking only ITS data — the last one taking everything, as before.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.InvitationDto
import me.nettrash.familyconnect.data.net.dto.MeResponse
import me.nettrash.familyconnect.data.net.dto.UserDto
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import me.nettrash.familyconnect.testutil.userDto
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class SeveralFamiliesSessionTest {

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private val authApi = FakeAuthApi()
    private val settings = FakeSettingsRepository(SettingsState(serverUrl = "https://chat.example.com"))
    private val wiper = RecordingWiper()

    /** What the session asked of the per-family store, in order. */
    private val familyData = object : FamilyLocalData {
        val claims = mutableListOf<Long>()
        val drops = mutableListOf<Pair<Long, Set<Long>>>()
        override suspend fun claimLegacy(homeFamilyId: Long) {
            claims += homeFamilyId
        }
        override suspend fun dropFamily(familyId: Long, remainingFamilyIds: Set<Long>) {
            drops += familyId to remainingFamilyIds
        }
    }

    private fun TestScope.newRepository(): SessionRepository {
        val repository = SessionRepository(
            authApi = authApi,
            tokenStore = FakeTokenStore("tok"),
            settings = settings,
            wiper = wiper,
            unauthorizedEvents = MutableSharedFlow(),
            scope = repoScope,
            familyData = familyData,
        )
        runCurrent()
        return repository
    }

    private val home = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open", role = "member", aiVision = true)
    private val mine = FamilyDto(id = 5, name = "The Joneses", joinPolicy = "approval", role = "owner")

    private val invitation = InvitationDto(
        id = 4,
        familyId = 8,
        familyName = "The Browns",
        invitedBy = UserDto(11, "carl", "Carl"),
        user = UserDto(7, "anna", "Anna"),
    )

    private fun me(vararg families: FamilyDto, invitations: List<InvitationDto> = emptyList()) = ApiResult.Ok(
        MeResponse(
            user = userDto(7, "anna"),
            family = families.firstOrNull(),
            role = families.firstOrNull()?.role,
            families = families.toList(),
            pendingJoinRequests = emptyList(),
            invitations = invitations,
            memberFamiliesEnabled = true,
            maxFamiliesPerAccount = 5,
        ),
    )

    @Test
    fun `me stores the list and shows the home family first`() = runTest(dispatcher) {
        val repository = newRepository()
        authApi.meResult = me(home, mine, invitations = listOf(invitation))

        repository.refreshMe()

        val state = settings.current
        assertThat(state.families.map { it.id }).containsExactly(3L, 5L).inOrder()
        assertThat(state.currentFamilyId).isEqualTo(3L)
        assertThat(state.familyStatus).isEqualTo(FamilyStatus.MEMBER)
        assertThat(state.familyName).isEqualTo("The Smiths")
        assertThat(state.familyAiVision).isTrue()
        assertThat(state.invitations).containsExactly(invitation)
        assertThat(state.multiFamilyServer).isTrue()
        assertThat(state.maxFamiliesPerAccount).isEqualTo(5)
        // Rows cached before several families are the home family's.
        assertThat(familyData.claims).containsExactly(3L)
        assertThat(settings.legacyClaims).containsExactly(3L)
    }

    @Test
    fun `a server from before several families is a one-entry list`() = runTest(dispatcher) {
        val repository = newRepository()
        authApi.meResult = ApiResult.Ok(
            MeResponse(user = userDto(7, "anna"), family = mine.copy(role = null), role = "owner"),
        )

        repository.refreshMe()

        val state = settings.current
        assertThat(state.families.single().id).isEqualTo(5L)
        assertThat(state.families.single().role).isEqualTo("owner")
        assertThat(state.familyStatus).isEqualTo(FamilyStatus.OWNER)
        assertThat(state.multiFamilyServer).isFalse()
    }

    @Test
    fun `the family shown is kept, and its role is the status`() = runTest(dispatcher) {
        val repository = newRepository()
        settings.setCurrentFamilyId(5)
        authApi.meResult = me(home, mine)

        repository.refreshMe()

        assertThat(settings.current.currentFamilyId).isEqualTo(5L)
        assertThat(settings.current.familyStatus).isEqualTo(FamilyStatus.OWNER)
        assertThat(settings.current.familyName).isEqualTo("The Joneses")
    }

    @Test
    fun `a family gone while not looking takes only its own data, and the shown one falls back`() =
        runTest(dispatcher) {
            val repository = newRepository()
            authApi.meResult = me(home, mine)
            repository.refreshMe()
            repository.switchFamily(5)
            val events = mutableListOf<SessionEvent>()
            val collector = backgroundScope.launch { repository.sessionEvents.collect { events += it } }
            runCurrent()

            authApi.meResult = me(home)
            repository.refreshMe()
            runCurrent()

            assertThat(familyData.drops).containsExactly(5L to setOf(3L))
            assertThat(wiper.wipeCount).isEqualTo(0)
            assertThat(settings.forgottenFamilies).contains(5L)
            assertThat(settings.current.currentFamilyId).isEqualTo(3L)
            assertThat(settings.current.familyStatus).isEqualTo(FamilyStatus.MEMBER)
            assertThat(events).containsExactly(SessionEvent.CurrentFamilyGone)
            collector.cancel()
        }

    @Test
    fun `out of the last family everything goes, as leaving always did`() = runTest(dispatcher) {
        val repository = newRepository()
        authApi.meResult = me(home)
        repository.refreshMe()
        val events = mutableListOf<SessionEvent>()
        val collector = backgroundScope.launch { repository.sessionEvents.collect { events += it } }
        runCurrent()

        authApi.meResult = me()
        repository.refreshMe()
        runCurrent()

        assertThat(wiper.wipeCount).isEqualTo(1)
        assertThat(familyData.drops).isEmpty()
        assertThat(settings.current.familyStatus).isEqualTo(FamilyStatus.NONE)
        assertThat(events).containsExactly(SessionEvent.RemovedFromFamily)
        collector.cancel()
    }

    @Test
    fun `leaving a family not shown drops it quietly`() = runTest(dispatcher) {
        val repository = newRepository()
        authApi.meResult = me(home, mine)
        repository.refreshMe()
        val events = mutableListOf<SessionEvent>()
        val collector = backgroundScope.launch { repository.sessionEvents.collect { events += it } }
        runCurrent()

        repository.leftFamily(5)
        runCurrent()

        assertThat(familyData.drops).containsExactly(5L to setOf(3L))
        assertThat(settings.current.families.map { it.id }).containsExactly(3L)
        assertThat(settings.current.currentFamilyId).isEqualTo(3L)
        assertThat(events).isEmpty()
        collector.cancel()
    }

    @Test
    fun `leaving the family shown switches to the first remaining`() = runTest(dispatcher) {
        val repository = newRepository()
        authApi.meResult = me(home, mine)
        repository.refreshMe()
        val events = mutableListOf<SessionEvent>()
        val collector = backgroundScope.launch { repository.sessionEvents.collect { events += it } }
        runCurrent()

        repository.leftFamily(3)
        runCurrent()

        assertThat(settings.current.currentFamilyId).isEqualTo(5L)
        assertThat(settings.current.familyStatus).isEqualTo(FamilyStatus.OWNER)
        assertThat(wiper.wipeCount).isEqualTo(0)
        assertThat(events).containsExactly(SessionEvent.CurrentFamilyGone)

        // Our own leave already did it: the member_left that confirms it
        // must not tear down twice.
        repository.leftFamily(3)
        runCurrent()
        assertThat(familyData.drops).hasSize(1)
        collector.cancel()
    }

    @Test
    fun `leaving the last family returns to the gate`() = runTest(dispatcher) {
        val repository = newRepository()
        authApi.meResult = me(home)
        repository.refreshMe()
        val events = mutableListOf<SessionEvent>()
        val collector = backgroundScope.launch { repository.sessionEvents.collect { events += it } }
        runCurrent()

        repository.leftFamily(3)
        runCurrent()

        assertThat(wiper.wipeCount).isEqualTo(1)
        assertThat(settings.current.families).isEmpty()
        assertThat(settings.current.currentFamilyId).isNull()
        assertThat(settings.current.familyStatus).isEqualTo(FamilyStatus.NONE)
        assertThat(events).containsExactly(SessionEvent.RemovedFromFamily)
        collector.cancel()
    }

    @Test
    fun `switching mirrors that family's status, name and switches`() = runTest(dispatcher) {
        val repository = newRepository()
        authApi.meResult = me(home, mine)
        repository.refreshMe()

        repository.switchFamily(5)
        assertThat(settings.state.first().familyStatus).isEqualTo(FamilyStatus.OWNER)
        assertThat(settings.current.familyName).isEqualTo("The Joneses")
        assertThat(settings.current.familyAiVision).isFalse()

        // Not one of ours: nothing moves.
        repository.switchFamily(99)
        assertThat(settings.current.currentFamilyId).isEqualTo(5L)
    }

    @Test
    fun `a family added here is shown before any me says so`() = runTest(dispatcher) {
        val repository = newRepository()
        authApi.meResult = me(home, invitations = listOf(invitation))
        repository.refreshMe()

        repository.addFamily(FamilyDto(id = 8, name = "The Browns", joinPolicy = "open", role = "member"))

        assertThat(settings.current.families.map { it.id }).containsExactly(3L, 8L).inOrder()
        assertThat(settings.current.currentFamilyId).isEqualTo(8L)
        assertThat(settings.current.familyName).isEqualTo("The Browns")
        // Its invitation is answered by it.
        assertThat(settings.current.invitations).isEmpty()
    }

    @Test
    fun `family owner hands a family's role over, and the status follows only for the one shown`() =
        runTest(dispatcher) {
            val repository = newRepository()
            authApi.meResult = me(home, mine)
            repository.refreshMe()

            repository.setFamilyRole(5, "member")
            assertThat(settings.current.families.single { it.id == 5L }.role).isEqualTo("member")
            assertThat(settings.current.familyStatus).isEqualTo(FamilyStatus.MEMBER)

            repository.setFamilyRole(3, "owner")
            assertThat(settings.current.familyStatus).isEqualTo(FamilyStatus.OWNER)
        }
}
