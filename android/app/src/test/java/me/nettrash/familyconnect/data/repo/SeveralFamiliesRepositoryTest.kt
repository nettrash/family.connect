/*
 * SeveralFamiliesRepositoryTest.kt
 * Family Connect (Android)
 *
 * FamilyRepository with several families (#88; docs/protocol.md, "Several
 * families"): frames applied to the family they name, never to "my
 * family"; every family-level call aimed at the CURRENT family; the three
 * doors to another family — create, join with a code, an invitation —
 * adding a family and showing it; invitations kept live by their frames;
 * and the owner's side of invitations.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.ApproveResponse
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.FamilyResponse
import me.nettrash.familyconnect.data.net.dto.InvitationDto
import me.nettrash.familyconnect.data.net.dto.InvitationResponse
import me.nettrash.familyconnect.data.net.dto.JoinResponse
import me.nettrash.familyconnect.data.net.dto.MeResponse
import me.nettrash.familyconnect.data.net.dto.UserDto
import me.nettrash.familyconnect.data.net.ws.ServerFrame
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
class SeveralFamiliesRepositoryTest {

    private companion object {
        const val ME = 7L
        const val PEER = 9L
        const val HOME = 3L
        const val OTHER = 5L
        const val NEW = 8L
    }

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private val authApi = FakeAuthApi()
    private val familyApi = FakeFamilyApi()
    private val socket = FakeChatSocket()

    private val home = FamilyDto(id = HOME, name = "The Smiths", joinPolicy = "open", role = "member")
    private val other = FamilyDto(id = OTHER, name = "The Joneses", joinPolicy = "open", role = "owner")
    private val newFamily = FamilyDto(id = NEW, name = "The Browns", joinPolicy = "open")

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

    private val invitation = InvitationDto(
        id = 4,
        familyId = NEW,
        familyName = "The Browns",
        invitedBy = UserDto(11, "carl", "Carl"),
        user = UserDto(ME, "anna", "Anna"),
    )

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        db.close()
    }

    private lateinit var sessionRepository: SessionRepository

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

    private fun roster(family: FamilyDto, vararg ids: Long) = ApiResult.Ok(
        FamilyMineResponse(family = family, members = ids.map { memberDto(it) }),
    )

    private fun me(vararg families: FamilyDto) = ApiResult.Ok(
        MeResponse(
            user = userDto(ME, "anna"),
            family = families.firstOrNull(),
            role = families.firstOrNull()?.role,
            families = families.toList(),
            pendingJoinRequests = emptyList(),
            memberFamiliesEnabled = true,
            maxFamiliesPerAccount = 5,
        ),
    )

    private suspend fun rows(familyId: Long): List<MemberEntity> = db.memberDao().observeActiveMembers(familyId).first()

    @Test
    fun `each family's roster is kept under that family, and read by its own path`() = runTest(dispatcher) {
        val repository = repository()
        familyApi.familyResults[HOME] = roster(home, ME, PEER)
        familyApi.familyResults[OTHER] = roster(other, ME)

        repository.refreshFamily(HOME)
        repository.refreshFamily(OTHER)

        assertThat(familyApi.familiesRead).containsExactly(HOME, OTHER).inOrder()
        assertThat(rows(HOME).map { it.userId }).containsExactly(ME, PEER)
        assertThat(rows(OTHER).map { it.userId }).containsExactly(ME)
        // One person, one name, whichever families they are in.
        assertThat(db.memberDao().observeMembers().first().count { it.userId == ME }).isEqualTo(1)
        // The pickers read the family shown.
        assertThat(repository.observeActiveMembers().first().map { it.userId }).containsExactly(ME, PEER)
    }

    @Test
    fun `member frames are applied to the family they name, never to my family`() = runTest(dispatcher) {
        val repository = repository()
        familyApi.familyResults[HOME] = roster(home, ME, PEER)
        familyApi.familyResults[OTHER] = roster(other, ME, PEER)
        repository.refreshFamily(HOME)
        repository.refreshFamily(OTHER)

        socket.emit(ServerFrame.MemberLeft(familyId = OTHER, userId = PEER))
        socket.emit(ServerFrame.MemberJoined(familyId = OTHER, user = userDto(12, "dan")))
        socket.emit(ServerFrame.FamilyOwner(familyId = OTHER, userId = PEER))
        runCurrent()

        assertThat(rows(HOME).map { it.userId }).containsExactly(ME, PEER)
        assertThat(rows(OTHER).map { it.userId }).containsExactly(ME, 12L)
        // The crown moved in THAT family: my role there is member now, and
        // the family shown — where I was always a member — is untouched.
        assertThat(settings.current.families.single { it.id == OTHER }.role).isEqualTo("member")
        assertThat(settings.current.familyStatus).isEqualTo(FamilyStatus.MEMBER)
    }

    /**
     * The owner's copy of `member_joined` says whether they may reset the
     * newcomer's password, top-level (docs/protocol.md, "Several families").
     * Stored when told; a copy that says nothing — everyone else's, an older
     * server's — never clears what is known, and a member never told about
     * is the old rule.
     */
    @Test
    fun `member joined stores password_reset when told and keeps it when not`() = runTest(dispatcher) {
        val repository = repository()
        familyApi.familyResults[OTHER] = roster(other, ME)
        repository.refreshFamily(OTHER)

        socket.emit(ServerFrame.MemberJoined(familyId = OTHER, user = userDto(12, "dan"), passwordReset = false))
        runCurrent()
        assertThat(rows(OTHER).single { it.userId == 12L }.passwordReset).isFalse()
        assertThat(rows(OTHER).single { it.userId == 12L }.offersPasswordReset).isFalse()

        // Not told: the stored false stands.
        socket.emit(ServerFrame.MemberJoined(familyId = OTHER, user = userDto(12, "dan")))
        runCurrent()
        assertThat(rows(OTHER).single { it.userId == 12L }.passwordReset).isFalse()

        // Told otherwise: the newer answer is written.
        socket.emit(ServerFrame.MemberJoined(familyId = OTHER, user = userDto(12, "dan"), passwordReset = true))
        runCurrent()
        assertThat(rows(OTHER).single { it.userId == 12L }.passwordReset).isTrue()

        // Never told at all: the reset is offered, as on a server from before.
        socket.emit(ServerFrame.MemberJoined(familyId = OTHER, user = userDto(13, "eve")))
        runCurrent()
        assertThat(rows(OTHER).single { it.userId == 13L }.passwordReset).isNull()
        assertThat(rows(OTHER).single { it.userId == 13L }.offersPasswordReset).isTrue()
    }

    /**
     * Creating a family here: `create` reads the new roster with me as its
     * owner, and the server's `member_joined` naming me — the creator —
     * arrives after it. A joiner is a plain member, but this is no joiner:
     * the frame used to write "member" over my live row, and the family I
     * had just made showed no owner until the next roster read.
     */
    @Test
    fun `the member_joined echo of a family created here keeps me its owner`() = runTest(dispatcher) {
        val repository = repository()
        familyApi.createResult = ApiResult.Ok(FamilyResponse(newFamily))
        familyApi.familyResults[NEW] = ApiResult.Ok(
            FamilyMineResponse(family = newFamily, members = listOf(memberDto(ME, "anna", role = "owner"))),
        )

        repository.create("The Browns")
        socket.emit(ServerFrame.MemberJoined(familyId = NEW, user = userDto(ME, "anna"), passwordReset = false))
        runCurrent()

        val mine = rows(NEW).single()
        assertThat(mine.role).isEqualTo("owner")
        // The frame's other columns still land: the COALESCE'd flag, the name.
        assertThat(mine.passwordReset).isFalse()
        assertThat(mine.displayName).isEqualTo("Anna")
        assertThat(repository.observeActiveMembers().first().single().role).isEqualTo("owner")
    }

    /**
     * The same echo winning the race with the roster read (or arriving when
     * that read failed): the row is NEW, and mine, so it takes the role my
     * entry for the family already says rather than "member".
     */
    @Test
    fun `my own member_joined before the roster read takes the role my entry says`() = runTest(dispatcher) {
        settings.setFamilies(
            listOf(home, other, newFamily.copy(role = "owner")), emptyList(), emptyList(), true, 5, multiFamilyServer = true,
        )
        repository()

        socket.emit(ServerFrame.MemberJoined(familyId = NEW, user = userDto(ME, "anna")))
        runCurrent()

        assertThat(rows(NEW).single().role).isEqualTo("owner")
        // Held already, so this is no "you are in a new family": nothing re-read.
        assertThat(familyApi.familiesRead).isEmpty()
    }

    /**
     * `member_joined` never moves a role on a live row — only `family_owner`
     * and a roster read do — but a row that had LEFT is a rejoin, and a
     * joiner comes back a plain member whatever their old row said (an
     * ex-owner whose hand-off this device never heard of).
     */
    @Test
    fun `member_joined keeps a live row's role and rejoins a departed one as a member`() = runTest(dispatcher) {
        repository()
        db.memberDao().upsertAll(
            listOf(
                MemberEntity(familyId = OTHER, userId = PEER, username = "ben", displayName = "Ben", role = "owner"),
                MemberEntity(
                    familyId = OTHER, userId = 12, username = "dan", displayName = "Dan", role = "owner", hasLeft = true,
                ),
            ),
        )

        socket.emit(ServerFrame.MemberJoined(familyId = OTHER, user = userDto(PEER, "ben")))
        socket.emit(ServerFrame.MemberJoined(familyId = OTHER, user = userDto(12, "dan")))
        runCurrent()

        assertThat(rows(OTHER).associate { it.userId to it.role }).containsExactly(PEER, "owner", 12L, "member")
    }

    @Test
    fun `an approval keeps the owner's password_reset for the newcomer`() = runTest(dispatcher) {
        val repository = repository()
        settings.setCurrentFamilyId(OTHER)
        familyApi.approveResult = ApiResult.Ok(ApproveResponse(memberDto(12, "dan").copy(passwordReset = false)))

        repository.approve(21)

        assertThat(rows(OTHER).single { it.userId == 12L }.passwordReset).isFalse()
    }

    @Test
    fun `member left naming me drops that family alone`() = runTest(dispatcher) {
        val repository = repository()
        familyApi.familyResults[HOME] = roster(home, ME, PEER)
        repository.refreshFamily(HOME)

        socket.emit(ServerFrame.MemberLeft(familyId = OTHER, userId = ME))
        runCurrent()

        assertThat(settings.current.families.map { it.id }).containsExactly(HOME)
        assertThat(settings.current.currentFamilyId).isEqualTo(HOME)
        assertThat(settings.current.familyStatus).isEqualTo(FamilyStatus.MEMBER)
    }

    @Test
    fun `member joined naming me for a family I do not hold reads me and that family`() = runTest(dispatcher) {
        val repository = repository()
        authApi.meResult = me(home, other, newFamily.copy(role = "member"))
        familyApi.familyResults[NEW] = roster(newFamily, ME, 11)
        val added = mutableListOf<Long>()
        val collector = backgroundScope.launch { repository.familiesAdded.collect { added += it } }
        runCurrent()

        socket.emit(ServerFrame.MemberJoined(familyId = NEW, user = userDto(ME, "anna")))
        runCurrent()

        assertThat(settings.current.families.map { it.id }).containsExactly(HOME, OTHER, NEW)
        assertThat(familyApi.familiesRead).containsExactly(NEW)
        assertThat(rows(NEW).map { it.userId }).containsExactly(ME, 11L)
        assertThat(added).containsExactly(NEW)
        collector.cancel()
    }

    @Test
    fun `family-level calls name the family shown`() = runTest(dispatcher) {
        val repository = repository()
        settings.setCurrentFamilyId(OTHER)
        familyApi.createResult = ApiResult.Ok(FamilyResponse(other.copy(role = null, aiVision = true)))

        repository.rotateInviteCode()
        repository.setLanguage("ru")
        repository.resetMemberPassword(PEER, "secret12")
        repository.removeMember(PEER)
        repository.stats()

        assertThat(familyApi.familyCalls).containsAtLeast(
            "setLanguage" to OTHER,
            "resetMemberPassword" to OTHER,
            "removeMember" to OTHER,
            "stats" to OTHER,
        ).inOrder()
        // The PATCH answer mirrors onto the shown family's entry, its role kept.
        assertThat(settings.current.families.single { it.id == OTHER }.aiVision).isTrue()
        assertThat(settings.current.families.single { it.id == OTHER }.role).isEqualTo("owner")
        assertThat(settings.current.familyAiVision).isTrue()
    }

    @Test
    fun `a server from before several families keeps the old paths`() = runTest(dispatcher) {
        settings.setFamilies(listOf(home), emptyList(), emptyList(), false, 0, multiFamilyServer = false)
        val repository = repository()

        repository.leave()

        assertThat(familyApi.familyCalls).containsExactly("leave" to null)
    }

    @Test
    fun `leaving leaves the family shown and shows the next`() = runTest(dispatcher) {
        val repository = repository()

        repository.leave()
        runCurrent()

        assertThat(familyApi.familyCalls).containsExactly("leave" to HOME)
        assertThat(settings.current.families.map { it.id }).containsExactly(OTHER)
        assertThat(settings.current.currentFamilyId).isEqualTo(OTHER)
        assertThat(settings.current.familyStatus).isEqualTo(FamilyStatus.OWNER)
    }

    @Test
    fun `a new family is added, owned and shown`() = runTest(dispatcher) {
        val repository = repository()
        familyApi.createResult = ApiResult.Ok(FamilyResponse(newFamily))
        familyApi.familyResults[NEW] = roster(newFamily, ME)

        repository.create("The Browns")

        assertThat(settings.current.currentFamilyId).isEqualTo(NEW)
        assertThat(settings.current.families.single { it.id == NEW }.role).isEqualTo("owner")
        assertThat(settings.current.familyStatus).isEqualTo(FamilyStatus.OWNER)
        assertThat(rows(NEW).map { it.userId }).containsExactly(ME)
    }

    @Test
    fun `joining another family with a code shows the family the answer names`() = runTest(dispatcher) {
        val repository = repository()
        familyApi.joinResult = ApiResult.Ok(JoinResponse(status = "joined", familyId = NEW))
        authApi.meResult = me(newFamily.copy(role = "member"), home, other)
        familyApi.familyResults[NEW] = roster(newFamily, ME)

        repository.join("ABCDEF")

        assertThat(settings.current.currentFamilyId).isEqualTo(NEW)
        assertThat(familyApi.familiesRead).contains(NEW)
    }

    @Test
    fun `invitation frames keep the banner live`() = runTest(dispatcher) {
        repository()
        val forSomebodyElse = invitation.copy(id = 5, user = UserDto(PEER, "ben", "Ben"))

        socket.emit(ServerFrame.Invitation(invitation))
        socket.emit(ServerFrame.Invitation(forSomebodyElse))
        runCurrent()
        assertThat(settings.current.invitations).containsExactly(invitation)

        socket.emit(ServerFrame.InvitationRemoved(invitation.id))
        runCurrent()
        assertThat(settings.current.invitations).isEmpty()
    }

    @Test
    fun `joining an invitation adds the family and shows it`() = runTest(dispatcher) {
        settings.setInvitations(listOf(invitation))
        val repository = repository()
        familyApi.acceptResult = ApiResult.Ok(FamilyResponse(newFamily))
        familyApi.familyResults[NEW] = roster(newFamily, ME, 11)

        val result = repository.acceptInvitation(invitation.id)

        assertThat(result).isInstanceOf(ApiResult.Ok::class.java)
        assertThat(familyApi.invitationsAccepted).containsExactly(4L)
        assertThat(settings.current.invitations).isEmpty()
        assertThat(settings.current.currentFamilyId).isEqualTo(NEW)
        assertThat(settings.current.families.single { it.id == NEW }.role).isEqualTo("member")
        assertThat(rows(NEW).map { it.userId }).containsExactly(ME, 11L)
    }

    @Test
    fun `an invitation no longer pending leaves the banner, and one at the cap stays`() = runTest(dispatcher) {
        settings.setInvitations(listOf(invitation, invitation.copy(id = 6)))
        val repository = repository()

        familyApi.acceptResult = ApiResult.HttpError(409, "invitation_not_pending", null)
        repository.acceptInvitation(4)
        assertThat(settings.current.invitations.map { it.id }).containsExactly(6L)

        familyApi.acceptResult = ApiResult.HttpError(409, "too_many_families", null)
        repository.acceptInvitation(6)
        assertThat(settings.current.invitations.map { it.id }).containsExactly(6L)
        assertThat(settings.current.currentFamilyId).isEqualTo(HOME)
    }

    @Test
    fun `declining takes it off the banner`() = runTest(dispatcher) {
        settings.setInvitations(listOf(invitation))
        val repository = repository()

        repository.declineInvitation(4)

        assertThat(familyApi.invitationsDeclined).containsExactly(4L)
        assertThat(settings.current.invitations).isEmpty()
    }

    @Test
    fun `the owner invites, lists and withdraws in the family shown`() = runTest(dispatcher) {
        settings.setCurrentFamilyId(OTHER)
        val repository = repository()
        familyApi.inviteResult = ApiResult.Ok(InvitationResponse(invitation.copy(familyId = OTHER)))

        repository.invite(11)
        repository.pendingInvitations()
        repository.withdrawInvitation(4)

        assertThat(familyApi.invitesSent).containsExactly(OTHER to 11L)
        assertThat(familyApi.familyCalls).contains("invitations" to OTHER)
        assertThat(familyApi.invitationsWithdrawn).containsExactly(OTHER to 4L)
    }

    @Test
    fun `family not found re-reads me`() = runTest(dispatcher) {
        val repository = repository()
        authApi.meResult = me(home)
        familyApi.familyResults[OTHER] = ApiResult.HttpError(404, "family_not_found", null)

        repository.refreshFamily(OTHER)
        runCurrent()

        assertThat(authApi.meCalls).isEqualTo(1)
        assertThat(settings.current.families.map { it.id }).containsExactly(HOME)
    }
}
