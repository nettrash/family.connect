/*
 * BirthdayRosterTest.kt
 * Family Connect (Android)
 *
 * A birthday reaches the roster down TWO paths — the
 * `GET /families/mine` refresh and an approved join request — and
 * FamilyRepository writes the DTO-to-entity mapping out longhand in each
 * of them. Miss one and the roster shows a birthday that vanishes on the
 * next frame, which is exactly the shape of the bug attachments were
 * bitten by: the send path is the one you remember and the inbound path
 * is the one that matters.
 *
 * The `member_joined` frame is the THIRD writer of that row and carries
 * no birthday at all — it is a `UserBrief` — so what it owes is the
 * opposite promise: it must leave the stored one alone. An absent field
 * never wipes a stored one, on either platform.
 *
 * The writes are here too, for the same reason: a birthday change raises
 * no WebSocket frame and no push (docs/protocol.md, "Birthdays"), so the
 * device that made the change has only its own response — if that is not
 * mirrored onto the row, the list it came from does not move.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.ApproveResponse
import me.nettrash.familyconnect.data.net.dto.BirthdayDto
import me.nettrash.familyconnect.data.net.dto.BirthdayResponse
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.FamilyResponse
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
class BirthdayRosterTest {

    private companion object {
        const val ME = 7L
        const val PEER = 9L
    }

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private val authApi = FakeAuthApi()
    private val familyApi = FakeFamilyApi()
    private val socket = FakeChatSocket()
    private val settings = FakeSettingsRepository(
        SettingsState(
            serverUrl = "https://chat.example.com",
            familyStatus = FamilyStatus.OWNER,
            myUserId = ME,
        ),
    )

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        authApi.birthdayUserId = ME
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        db.close()
    }

    private fun TestScope.repository(): FamilyRepository {
        val sessionRepository = SessionRepository(
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
        // Let the frame collector subscribe before anything is emitted.
        runCurrent()
        return repository
    }

    private suspend fun birthdayOf(userId: Long): BirthdayDto? =
        db.memberDao().observeMembers().first().single { it.userId == userId }.birthday

    @Test
    fun `the roster refresh carries every birthday the server reports`() = runTest(dispatcher) {
        familyApi.mineResult = ApiResult.Ok(
            FamilyMineResponse(
                family = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open"),
                members = listOf(
                    memberDto(ME, "anna", role = "owner")
                        .copy(birthday = BirthdayDto(month = 3, day = 14)),
                    // Nobody has to have one — absent means unset.
                    memberDto(PEER, "ben"),
                ),
            ),
        )
        val repository = repository()

        repository.refreshMine()

        assertThat(birthdayOf(ME)).isEqualTo(BirthdayDto(month = 3, day = 14))
        assertThat(birthdayOf(PEER)).isNull()
    }

    /**
     * The frame the server ACTUALLY sends, which is the whole point.
     *
     * `member_joined.user` is a `UserBrief` (docs/protocol.md,
     * "Server → client"): id, username, display_name, avatar_version and
     * nothing else. There is no birthday field on it to carry one, so
     * `UserDto.birthday` decodes to null on every single one of these
     * frames — and a wholesale upsert of the row then writes that null
     * over a birthday the roster already knew.
     *
     * Ben leaves and is re-approved, which is the ordinary way this
     * happens, and every OTHER device blanks his birthday until somebody
     * happens to trigger a roster refresh. An absent field must never
     * wipe a stored one — the same rule iOS spells with a double optional
     * (ChatSyncCoordinator.upsertMember), and the same rule the
     * attachment coordinates were bitten by once already.
     */
    @Test
    fun `a member_joined frame leaves a stored birthday alone`() = runTest(dispatcher) {
        familyApi.mineResult = ApiResult.Ok(
            FamilyMineResponse(
                family = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open"),
                members = listOf(
                    memberDto(ME, "anna", role = "owner"),
                    memberDto(PEER, "ben", role = "member")
                        .copy(birthday = BirthdayDto(month = 3, day = 3), avatarVersion = 4),
                ),
            ),
        )
        val repository = repository()
        repository.refreshMine()
        db.memberDao().markLeft(PEER)

        // Built with no birthday at all, exactly as the wire shape is.
        socket.emit(ServerFrame.MemberJoined(familyId = 3, user = userDto(PEER, "ben")))
        runCurrent()

        assertThat(birthdayOf(PEER)).isEqualTo(BirthdayDto(month = 3, day = 3))
        // What the frame DOES carry still lands — leaving the row alone
        // altogether would be the other way to fail this.
        val row = db.memberDao().observeMembers().first().single { it.userId == PEER }
        assertThat(row.hasLeft).isFalse()
        assertThat(row.displayName).isEqualTo("Ben")
        assertThat(row.avatarVersion).isEqualTo(0)
    }

    @Test
    fun `a first-sight joiner lands with no birthday until the next roster refresh`() =
        runTest(dispatcher) {
            repository()

            socket.emit(ServerFrame.MemberJoined(familyId = 3, user = userDto(PEER, "ben")))
            runCurrent()

            // Nothing invented: the frame never mentioned a birthday, and
            // there was no row to carry one through from. `GET
            // /families/mine` is where it arrives.
            assertThat(birthdayOf(PEER)).isNull()
            assertThat(db.memberDao().observeMembers().first().single().role).isEqualTo("member")
        }

    @Test
    fun `approving a join request keeps the new member's birthday`() = runTest(dispatcher) {
        val repository = repository()
        familyApi.approveResult = ApiResult.Ok(
            ApproveResponse(
                memberDto(PEER, "ben").copy(birthday = BirthdayDto(month = 2, day = 29)),
            ),
        )

        repository.approve(requestId = 12)

        assertThat(birthdayOf(PEER)).isEqualTo(BirthdayDto(month = 2, day = 29))
    }

    @Test
    fun `setting my own birthday mirrors the server's answer onto my row`() =
        runTest(dispatcher) {
            familyApi.mineResult = ApiResult.Ok(
                FamilyMineResponse(
                    family = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open"),
                    members = listOf(memberDto(ME, "anna", role = "owner")),
                ),
            )
            val repository = repository()
            repository.refreshMine()

            // The response is the authority, not what was sent — nothing
            // else will tell this device, since a birthday raises no frame.
            authApi.setBirthdayHandler = { _, _ ->
                ApiResult.Ok(
                    BirthdayResponse(
                        userDto(ME, "anna").copy(birthday = BirthdayDto(month = 5, day = 2)),
                    ),
                )
            }

            val result = repository.setMyBirthday(month = 5, day = 2)

            assertThat(authApi.birthdaysSet).containsExactly(5 to 2)
            assertThat(result.okOrNull()).isEqualTo(BirthdayDto(month = 5, day = 2))
            assertThat(birthdayOf(ME)).isEqualTo(BirthdayDto(month = 5, day = 2))
        }

    @Test
    fun `clearing my own birthday empties my row`() = runTest(dispatcher) {
        familyApi.mineResult = ApiResult.Ok(
            FamilyMineResponse(
                family = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open"),
                members = listOf(
                    memberDto(ME, "anna", role = "owner")
                        .copy(birthday = BirthdayDto(month = 3, day = 14)),
                ),
            ),
        )
        val repository = repository()
        repository.refreshMine()

        repository.clearMyBirthday()

        // The 204 carries no user, so the id comes from settings — get
        // that wrong and the row keeps a birthday the server has dropped.
        assertThat(authApi.birthdaysCleared).isEqualTo(1)
        assertThat(birthdayOf(ME)).isNull()
    }

    @Test
    fun `the owner writing somebody else's birthday updates that row only`() =
        runTest(dispatcher) {
            familyApi.mineResult = ApiResult.Ok(
                FamilyMineResponse(
                    family = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open"),
                    members = listOf(
                        memberDto(ME, "anna", role = "owner"),
                        memberDto(PEER, "ben"),
                    ),
                ),
            )
            val repository = repository()
            repository.refreshMine()

            repository.setMemberBirthday(PEER, month = 7, day = 4)

            assertThat(familyApi.memberBirthdaysSet).containsExactly(Triple(PEER, 7, 4))
            assertThat(birthdayOf(PEER)).isEqualTo(BirthdayDto(month = 7, day = 4))
            assertThat(birthdayOf(ME)).isNull()

            repository.clearMemberBirthday(PEER)

            assertThat(familyApi.memberBirthdaysCleared).containsExactly(PEER)
            assertThat(birthdayOf(PEER)).isNull()
        }

    @Test
    fun `a birthday write leaves the rest of the row alone`() = runTest(dispatcher) {
        familyApi.mineResult = ApiResult.Ok(
            FamilyMineResponse(
                family = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open"),
                members = listOf(
                    memberDto(ME, "anna", role = "owner"),
                    memberDto(PEER, "ben").copy(avatarVersion = 4),
                ),
            ),
        )
        val repository = repository()
        repository.refreshMine()
        db.memberDao().markLeft(PEER)

        // A targeted UPDATE, not an upsert assembled at the call site:
        // rebuilding the row from a MemberDto would reset `hasLeft` and
        // `avatarVersion` to whatever that shape happened to carry.
        repository.setMemberBirthday(PEER, month = 1, day = 9)

        val row = db.memberDao().observeMembers().first().single { it.userId == PEER }
        assertThat(row.birthday).isEqualTo(BirthdayDto(month = 1, day = 9))
        assertThat(row.avatarVersion).isEqualTo(4)
        assertThat(row.hasLeft).isTrue()
    }

    @Test
    fun `the family settings patches reach the wire as the owner chose them`() =
        runTest(dispatcher) {
            val repository = repository()
            familyApi.createResult = ApiResult.Ok(
                FamilyResponse(
                    FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open", language = "ja"),
                ),
            )

            repository.setLanguage("ja")
            repository.setLanguage(null)
            repository.setAiHistory(false)

            // Null is a CLEAR, not an omission — the encoded body is
            // pinned in FamilySettingsDtoTest.
            assertThat(familyApi.languagesSet).containsExactly("ja", null).inOrder()
            assertThat(familyApi.aiHistorySet).containsExactly(false)
        }

    @Test
    fun `a member with no birthday reads back as unset rather than as zero`() =
        runTest(dispatcher) {
            familyApi.mineResult = ApiResult.Ok(
                FamilyMineResponse(
                    family = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open"),
                    members = listOf(memberDto(PEER, "ben")),
                ),
            )
            val repository = repository()
            repository.refreshMine()

            val row = db.memberDao().observeMembers().first().single()
            assertThat(row.birthdayMonth).isNull()
            assertThat(row.birthdayDay).isNull()
            assertThat(row.birthday).isNull()
        }
}
