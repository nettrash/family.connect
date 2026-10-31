/*
 * PaidFamiliesRepositoryTest.kt
 * Family Connect (Android)
 *
 * Paid families in the session (#89; docs/protocol.md, "Paid families"):
 * `GET /me` says whether the server charges and whether the caller pays,
 * and the `family_plan` frame moves a family's deadline on the family it
 * names — an absent `delete_at` clearing it — so the chat list's banner
 * appears and goes live.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.MeResponse
import me.nettrash.familyconnect.data.net.dto.SubscriptionDto
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.settings.FamilyListCodec
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

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class PaidFamiliesRepositoryTest {

    private companion object {
        const val ME = 7L
        const val HOME = 3L
        const val OTHER = 5L
        const val DEADLINE = "2026-11-14T09:00:00Z"
    }

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private val authApi = FakeAuthApi()
    private val familyApi = FakeFamilyApi()
    private val socket = FakeChatSocket()

    private val home = FamilyDto(id = HOME, name = "The Smiths", joinPolicy = "open", role = "member", plan = "paid")
    private val other = FamilyDto(id = OTHER, name = "The Joneses", joinPolicy = "open", role = "owner", plan = "free")

    private val settings = FakeSettingsRepository(
        SettingsState(
            serverUrl = "https://chat.example.com",
            familyStatus = FamilyStatus.MEMBER,
            myUserId = ME,
            families = listOf(home, other),
            currentFamilyId = HOME,
            multiFamilyServer = true,
            maxFamiliesPerAccount = 5,
        ),
    )

    private lateinit var sessionRepository: SessionRepository

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        db.close()
    }

    private fun TestScope.repository(): FamilyRepository {
        sessionRepository = SessionRepository(
            authApi = authApi,
            tokenStore = FakeTokenStore("tok"),
            settings = settings,
            wiper = RecordingWiper(),
            unauthorizedEvents = MutableSharedFlow(),
            scope = repoScope,
        )
        val repository = FamilyRepository(
            familyApi = familyApi,
            authApi = authApi,
            memberDao = db.memberDao(),
            settings = settings,
            sessionRepository = sessionRepository,
            socket = socket,
            scope = repoScope,
        )
        runCurrent()
        return repository
    }

    private fun me(
        subscriptionsEnabled: Boolean,
        subscription: SubscriptionDto?,
        vararg families: FamilyDto = arrayOf(home, other),
    ) = ApiResult.Ok(
        MeResponse(
            user = userDto(ME, "anna"),
            family = families.firstOrNull(),
            role = families.firstOrNull()?.role,
            families = families.toList(),
            pendingJoinRequests = emptyList(),
            memberFamiliesEnabled = true,
            maxFamiliesPerAccount = 5,
            subscriptionsEnabled = subscriptionsEnabled,
            subscription = subscription,
        ),
    )

    private fun entry(id: Long): FamilyDto = settings.current.families.single { it.id == id }

    @Test
    fun `me says whether the server charges and whether the caller pays`() = runTest(dispatcher) {
        repository()

        authApi.meResult = me(true, SubscriptionDto(productId = "monthly", active = true))
        sessionRepository.refreshMe()
        assertThat(settings.current.subscriptionsEnabled).isTrue()
        assertThat(settings.current.subscriptionActive).isTrue()

        // A subscription that ended: still on the wire, no longer active.
        authApi.meResult = me(true, SubscriptionDto(productId = "monthly", active = false))
        sessionRepository.refreshMe()
        assertThat(settings.current.subscriptionsEnabled).isTrue()
        assertThat(settings.current.subscriptionActive).isFalse()

        authApi.meResult = me(true, null)
        sessionRepository.refreshMe()
        assertThat(settings.current.subscriptionActive).isFalse()
    }

    /**
     * With the switch off nothing about paying applies — even a stale
     * active subscription on the wire does not count, and an older server
     * that never says reads as off.
     */
    @Test
    fun `a server that does not charge is off whatever else it says`() = runTest(dispatcher) {
        repository()
        authApi.meResult = me(true, SubscriptionDto(active = true))
        sessionRepository.refreshMe()

        authApi.meResult = me(false, SubscriptionDto(active = true))
        sessionRepository.refreshMe()

        assertThat(settings.current.subscriptionsEnabled).isFalse()
        assertThat(settings.current.subscriptionActive).isFalse()
        assertThat(PaidFamilies.mustPayToStart(settings.current.subscriptionsEnabled, settings.current.subscriptionActive))
            .isFalse()
    }

    @Test
    fun `the family_plan frame sets the deadline on the family it names, and leaving it out clears it`() =
        runTest(dispatcher) {
            repository()

            socket.emit(ServerFrame.FamilyPlan(familyId = HOME, plan = "paid", deleteAt = DEADLINE))
            runCurrent()
            assertThat(entry(HOME).deleteAt).isEqualTo(DEADLINE)
            assertThat(entry(HOME).plan).isEqualTo("paid")
            // The other family is not this frame's business.
            assertThat(entry(OTHER).deleteAt).isNull()
            // The role on the entry survives: the frame carries none.
            assertThat(entry(HOME).role).isEqualTo("member")

            // Renewed: the frame again, with no `delete_at` at all.
            socket.emit(ServerFrame.FamilyPlan(familyId = HOME, plan = "paid"))
            runCurrent()
            assertThat(entry(HOME).deleteAt).isNull()
            assertThat(entry(HOME).plan).isEqualTo("paid")
        }

    /** A family the app is NOT showing still takes its own frame — it is shown the moment it is switched to. */
    @Test
    fun `a frame for a family not shown lands on that family`() = runTest(dispatcher) {
        repository()

        socket.emit(ServerFrame.FamilyPlan(familyId = OTHER, plan = "paid", deleteAt = DEADLINE))
        runCurrent()

        assertThat(entry(OTHER).deleteAt).isEqualTo(DEADLINE)
        assertThat(entry(OTHER).plan).isEqualTo("paid")
        assertThat(entry(HOME).deleteAt).isNull()
        assertThat(settings.current.currentFamilyId).isEqualTo(HOME)
    }

    @Test
    fun `a frame for a family this account is not in changes nothing`() = runTest(dispatcher) {
        repository()
        val before = settings.current.families

        socket.emit(ServerFrame.FamilyPlan(familyId = 99, plan = "paid", deleteAt = DEADLINE))
        runCurrent()

        assertThat(settings.current.families).isEqualTo(before)
    }

    /** The roster read carries the whole Family, so a deadline missed while offline arrives there too. */
    @Test
    fun `the roster read brings the deadline and the ceiling`() = runTest(dispatcher) {
        val repository = repository()
        familyApi.familyResults[HOME] = ApiResult.Ok(
            FamilyMineResponse(
                family = home.copy(role = null, deleteAt = DEADLINE, memberCeiling = 25),
                members = listOf(memberDto(ME), memberDto(11, "olga", role = "owner")),
            ),
        )

        repository.refreshFamily(HOME)

        assertThat(entry(HOME).deleteAt).isEqualTo(DEADLINE)
        assertThat(entry(HOME).memberCeiling).isEqualTo(25)
        assertThat(entry(HOME).role).isEqualTo("member")
    }

    /** The family list is kept as JSON between launches; the new fields must survive it. */
    @Test
    fun `the deadline, the plan and the ceiling survive the stored family list`() {
        val stored = listOf(home.copy(deleteAt = DEADLINE, memberCeiling = 25), other)

        val read = FamilyListCodec.decodeFamilies(FamilyListCodec.encodeFamilies(stored))

        assertThat(read).isEqualTo(stored)
        // …and a list stored before the fields existed reads them as absent.
        val older = FamilyListCodec.decodeFamilies("""[{"id": 3, "name": "The Smiths", "join_policy": "open"}]""")
        assertThat(older.single().deleteAt).isNull()
        assertThat(older.single().plan).isNull()
        assertThat(older.single().memberCeiling).isNull()
    }
}
