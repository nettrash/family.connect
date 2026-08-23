/*
 * RosterParityTest.kt
 * Family Connect (Android)
 *
 * A member who leaves is KEPT and flagged, never deleted.
 *
 * The bug this pins: Android used to delete the row, so every message
 * that member had ever sent lost its name and face and fell back to
 * "Member 11" with initials — while iOS, which has always flagged, went
 * on showing them correctly. Two devices in the same family disagreed
 * about the same history.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.testutil.createTestDb
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class RosterParityTest {

    private val dispatcher = StandardTestDispatcher()
    private lateinit var db: AppDatabase
    private lateinit var members: MemberDao

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        members = db.memberDao()
    }

    @After
    fun tearDown() = db.close()

    private fun member(id: Long, name: String, role: String = "member") =
        MemberEntity(
            userId = id,
            username = name.lowercase(),
            displayName = name,
            role = role,
            avatarVersion = 3,
        )

    @Test
    fun `a member who leaves keeps their row so old messages keep a name`() =
        runTest(dispatcher) {
            members.upsertAll(listOf(member(1, "Olive", "owner"), member(2, "Junior")))

            members.markLeft(2)

            // Still there, still named, still with an avatar version — the
            // whole point, since their bubbles are still in the thread.
            val all = members.observeMembers().first()
            val junior = all.single { it.userId == 2L }
            assertThat(junior.displayName).isEqualTo("Junior")
            assertThat(junior.avatarVersion).isEqualTo(3)
            assertThat(junior.hasLeft).isTrue()

            // But gone from the lists that offer to do something with them.
            val active = members.observeActiveMembers().first()
            assertThat(active.map { it.userId }).containsExactly(1L)
        }

    @Test
    fun `a roster refresh flags whoever the server no longer lists`() = runTest(dispatcher) {
        members.upsertAll(
            listOf(member(1, "Olive", "owner"), member(2, "Junior"), member(3, "Robin")),
        )

        // What refreshMine does: upsert who is present, flag the rest.
        val present = listOf(member(1, "Olive", "owner"), member(2, "Junior"))
        members.upsertAll(present)
        members.markLeftExcept(present.map { it.userId })

        val all = members.observeMembers().first().associateBy { it.userId }
        assertThat(all[3L]!!.hasLeft).isTrue()
        assertThat(all[3L]!!.displayName).isEqualTo("Robin")
        assertThat(all[1L]!!.hasLeft).isFalse()
        assertThat(all[2L]!!.hasLeft).isFalse()
        assertThat(members.observeActiveMembers().first()).hasSize(2)
    }

    /** History survives leave/rejoin on the server, so the flag must clear. */
    @Test
    fun `rejoining clears the flag`() = runTest(dispatcher) {
        members.upsertAll(listOf(member(2, "Junior")))
        members.markLeft(2)
        assertThat(members.observeActiveMembers().first()).isEmpty()

        // The upsert replaces the row wholesale, which is what resets it.
        members.upsertAll(listOf(member(2, "Junior")))
        members.markLeftExcept(listOf(2L))

        assertThat(members.observeMembers().first().single().hasLeft).isFalse()
        assertThat(members.observeActiveMembers().first()).hasSize(1)
    }
}
