/*
 * DeletedMemberRosterTest.kt
 * Family Connect (Android)
 *
 * A deleted account reaches this client down two paths — the
 * `former_members` array on `GET /families/mine` and the
 * `member_deleted` frame — and both land in the ONE roster table, because
 * a message, a note or a reaction they left behind still has to be given
 * a name (docs/protocol.md, "Deleting an account").
 *
 * What that costs, and what these tests pin:
 *
 *   - the tombstone is written DELIBERATELY. `member_deleted` is the one
 *     frame in this protocol whose job is to WIPE fields — the picture
 *     and the birthday are gone, and saying so is the point. Everywhere
 *     else an absent field must never clear a stored one, which is why
 *     this must not go through the ordinary member upsert, and why the
 *     test checks that the tombstone clears the deleted member's birthday
 *     and NOBODY else's.
 *   - no roster shows them. They hold no role, they are offered to
 *     nobody, and nothing counts them.
 *   - `family_owner` moves the crown, locally and in the stored status,
 *     so a client that has just become the owner gains the owner-only
 *     screens without waiting for its next GET /me.
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
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.BirthdayDto
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.MemberDto
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
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class DeletedMemberRosterTest {

    private companion object {
        const val ME = 7L
        const val PEER = 9L
        const val GONE = 11L
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
            familyStatus = FamilyStatus.MEMBER,
            myUserId = ME,
        ),
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

    private suspend fun rowFor(userId: Long): MemberEntity =
        db.memberDao().observeMembers().first().single { it.userId == userId }

    private fun tombstone(id: Long) = MemberDto(
        id = id,
        username = "junior",
        // The server's ENGLISH placeholder — what a client that knows the
        // flag replaces with its own translation.
        displayName = "Deleted account",
        avatarVersion = 0,
        deleted = true,
    )

    // -- GET /families/mine ------------------------------------------------------

    @Test
    fun `former members are stored beside the roster and kept out of it`() = runTest(dispatcher) {
        familyApi.mineResult = ApiResult.Ok(
            FamilyMineResponse(
                family = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open"),
                members = listOf(
                    memberDto(ME, "anna", role = "owner"),
                    memberDto(PEER, "ben"),
                ),
                formerMembers = listOf(tombstone(GONE)),
            ),
        )

        repository().refreshMine()

        // One table: the tombstone is there, so a message from GONE can
        // still be named.
        val gone = rowFor(GONE)
        assertThat(gone.deleted).isTrue()
        assertThat(gone.avatarVersion).isEqualTo(0)
        assertThat(gone.birthday).isNull()
        // And also flagged as departed, because they are: the roster
        // filter must not depend on which of the two facts arrived.
        assertThat(gone.hasLeft).isTrue()

        // But no roster, picker, admin list or statistic includes them.
        val active = db.memberDao().observeActiveMembers().first()
        assertThat(active.map { it.userId }).containsExactly(ME, PEER)
    }

    @Test
    fun `a server that reports nobody deleted leaves the roster alone`() = runTest(dispatcher) {
        familyApi.mineResult = ApiResult.Ok(
            FamilyMineResponse(
                family = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open"),
                members = listOf(memberDto(ME, "anna", role = "owner")),
            ),
        )

        repository().refreshMine()

        assertThat(rowFor(ME).deleted).isFalse()
        assertThat(db.memberDao().observeActiveMembers().first()).hasSize(1)
    }

    @Test
    fun `an in-flight roster response never resurrects a tombstone`() = runTest(dispatcher) {
        // The interleaving this pins: the server computes GET /families/mine
        // while GONE still exists, so `members` lists them live and
        // `former_members` does not. The member_deleted frame overtakes the
        // response. Applying the response afterwards would put the real
        // name, the real picture and `deleted = 0` back on the tombstone —
        // and GONE would be offered in the new-chat picker (where POST
        // /chats/direct answers user_not_found), listed in the family admin
        // screen and counted in every poll's "3 of 5 voted" denominator.
        val repository = repository()
        familyApi.mineResult = ApiResult.Ok(
            FamilyMineResponse(
                family = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open"),
                // Stale: GONE is still listed as a LIVE member here.
                members = listOf(
                    memberDto(ME, "anna", role = "owner"),
                    MemberDto(GONE, "junior", "Junior", "member", avatarVersion = 7),
                ),
            ),
        )

        socket.emit(ServerFrame.MemberDeleted(familyId = 3, member = tombstone(GONE)))
        runCurrent()
        repository.refreshMine()

        // Deletion is one-way and ids are never reused, so a live entry for
        // a tombstoned id can only be stale — and is ignored.
        val gone = rowFor(GONE)
        assertThat(gone.deleted).isTrue()
        assertThat(gone.avatarVersion).isEqualTo(0)
        assertThat(gone.displayName).isEqualTo("Deleted account")
        assertThat(db.memberDao().observeActiveMembers().first().map { it.userId })
            .containsExactly(ME)
    }

    // -- The member_deleted frame ---------------------------------------------------

    @Test
    fun `the tombstone frame wipes the account's own fields and nobody else's`() =
        runTest(dispatcher) {
            repository()
            db.memberDao().upsertAll(
                listOf(
                    MemberEntity(
                        userId = PEER,
                        username = "ben",
                        displayName = "Ben",
                        role = "member",
                        avatarVersion = 4,
                        birthdayMonth = 5,
                        birthdayDay = 6,
                    ),
                    MemberEntity(
                        userId = GONE,
                        username = "junior",
                        displayName = "Junior",
                        role = "member",
                        avatarVersion = 7,
                        birthdayMonth = 3,
                        birthdayDay = 14,
                    ),
                ),
            )

            socket.emit(ServerFrame.MemberDeleted(familyId = 3, member = tombstone(GONE)))
            runCurrent()

            val gone = rowFor(GONE)
            assertThat(gone.deleted).isTrue()
            assertThat(gone.hasLeft).isTrue()
            // Everything that identified the account is gone — the
            // picture and the birthday included. This is the ONE write
            // that is allowed to clear a stored field, and it does.
            assertThat(gone.avatarVersion).isEqualTo(0)
            assertThat(gone.birthday).isNull()
            assertThat(gone.displayName).isEqualTo("Deleted account")

            // The frame said nothing about anybody else, so nothing about
            // anybody else moved — the bug member_joined was once bitten
            // by, in the one place a wipe is legitimate.
            val ben = rowFor(PEER)
            assertThat(ben.deleted).isFalse()
            assertThat(ben.avatarVersion).isEqualTo(4)
            assertThat(ben.birthday).isEqualTo(BirthdayDto(month = 5, day = 6))
        }

    @Test
    fun `a tombstone for somebody this device never saw is still stored`() = runTest(dispatcher) {
        repository()

        socket.emit(ServerFrame.MemberDeleted(familyId = 3, member = tombstone(GONE)))
        runCurrent()

        // They joined and deleted between two resyncs of this device — and
        // their messages are still in the family chat, so the row has to
        // exist for them to be named at all.
        val gone = rowFor(GONE)
        assertThat(gone.deleted).isTrue()
        assertThat(db.memberDao().observeActiveMembers().first()).isEmpty()
    }

    @Test
    fun `a deleted owner stops being the owner locally`() = runTest(dispatcher) {
        repository()
        db.memberDao().upsertAll(
            listOf(
                MemberEntity(userId = GONE, username = "olive", displayName = "Olive", role = "owner"),
                MemberEntity(userId = ME, username = "anna", displayName = "Anna", role = "member"),
            ),
        )

        socket.emit(ServerFrame.MemberDeleted(familyId = 3, member = tombstone(GONE)))
        runCurrent()

        // A tombstone holds no role, and an ex-owner who no longer exists
        // must not go on outranking the member the family was handed to.
        assertThat(rowFor(GONE).role).isEqualTo("member")
    }

    // -- The family_owner frame -------------------------------------------------------

    @Test
    fun `family owner moves the crown and gains me the owner screens at once`() =
        runTest(dispatcher) {
            repository()
            db.memberDao().upsertAll(
                listOf(
                    MemberEntity(userId = GONE, username = "olive", displayName = "Olive", role = "owner"),
                    MemberEntity(userId = ME, username = "anna", displayName = "Anna", role = "member"),
                    MemberEntity(userId = PEER, username = "ben", displayName = "Ben", role = "member"),
                ),
            )

            socket.emit(ServerFrame.FamilyOwner(familyId = 3, userId = ME))
            runCurrent()

            assertThat(rowFor(ME).role).isEqualTo("owner")
            // Exactly one owner, always: the old one is demoted in the
            // same transaction.
            assertThat(rowFor(GONE).role).isEqualTo("member")
            assertThat(rowFor(PEER).role).isEqualTo("member")
            // The point of the frame: the owner-only screens appear now,
            // not at the next GET /me.
            assertThat(settings.current.familyStatus).isEqualTo(FamilyStatus.OWNER)
        }

    @Test
    fun `family owner naming somebody else leaves me a plain member`() = runTest(dispatcher) {
        repository()
        db.memberDao().upsertAll(
            listOf(
                MemberEntity(userId = ME, username = "anna", displayName = "Anna", role = "member"),
                MemberEntity(userId = PEER, username = "ben", displayName = "Ben", role = "member"),
            ),
        )

        socket.emit(ServerFrame.FamilyOwner(familyId = 3, userId = PEER))
        runCurrent()

        assertThat(rowFor(PEER).role).isEqualTo("owner")
        assertThat(settings.current.familyStatus).isEqualTo(FamilyStatus.MEMBER)
    }
}
