/*
 * FamilySettingsPersistenceTest.kt
 * Family Connect (Android)
 *
 * Several families in the real DataStore (#88; docs/protocol.md, "Several
 * families"): the family list, the current family, the invitations and a
 * board and pack cursor PER FAMILY survive a restart; the single values an
 * app from before kept for "the family" move under the home family once;
 * leaving a family forgets its cursors; and a sign-out forgets it all.
 */

package me.nettrash.familyconnect.data.settings

import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.longPreferencesKey
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.InvitationDto
import me.nettrash.familyconnect.data.net.dto.PendingJoinRequestDto
import me.nettrash.familyconnect.data.net.dto.UserDto
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

class FamilySettingsPersistenceTest {

    @get:Rule
    val folder = TemporaryFolder()

    private fun <T> launch(
        file: File,
        before: suspend (androidx.datastore.core.DataStore<androidx.datastore.preferences.core.Preferences>) -> Unit = {},
        block: suspend (SettingsRepository) -> T,
    ): T = runBlocking {
        val job = Job()
        val store = PreferenceDataStoreFactory.create(scope = CoroutineScope(Dispatchers.IO + job)) { file }
        try {
            before(store)
            block(DataStoreSettingsRepository(store))
        } finally {
            job.cancelAndJoin()
        }
    }

    private val home = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open", role = "member", aiVision = true)
    private val mine = FamilyDto(id = 5, name = "The Joneses", joinPolicy = "approval", role = "owner")
    private val invitation = InvitationDto(
        id = 4,
        familyId = 8,
        familyName = "The Browns",
        invitedBy = UserDto(11, "carl", "Carl"),
        user = UserDto(7, "anna", "Anna"),
        createdAt = "2026-10-22T10:00:00Z",
    )
    private val pending = PendingJoinRequestDto(familyId = 9, familyName = "The Greens", createdAt = "2026-10-22T10:00:00Z")

    @Test
    fun `the family list and its cursors survive a restart`() {
        val file = File(folder.root, "settings.preferences_pb")
        launch(file) { settings ->
            settings.setFamilies(listOf(home, mine), listOf(pending), listOf(invitation), true, 5, true)
            settings.setCurrentFamilyId(5)
            settings.setBoardCursor(3, 88)
            settings.setBoardCursor(5, 12)
            settings.setPackCursor(5, 14)
            settings.setBoardSeenNoteId(5, 40)
            settings.setBoardSeenNoteId(5, 20) // never backwards
        }

        val after = launch(file) { settings -> settings.state.first() }

        assertThat(after.families).containsExactly(home, mine).inOrder()
        assertThat(after.currentFamilyId).isEqualTo(5L)
        assertThat(after.currentFamily).isEqualTo(mine)
        assertThat(after.pendingJoinRequests).containsExactly(pending)
        assertThat(after.invitations).containsExactly(invitation)
        assertThat(after.memberFamiliesEnabled).isTrue()
        assertThat(after.maxFamiliesPerAccount).isEqualTo(5)
        assertThat(after.multiFamilyServer).isTrue()
        assertThat(after.boardCursor(3)).isEqualTo(88)
        assertThat(after.boardCursor(5)).isEqualTo(12)
        assertThat(after.packCursor(5)).isEqualTo(14)
        assertThat(after.packCursor(3)).isEqualTo(0)
        assertThat(after.boardSeenNoteIds[5L]).isEqualTo(40)
        // The path names the family only on a server that speaks several.
        assertThat(after.pathFamily(5)).isEqualTo(5L)
        assertThat(after.copy(multiFamilyServer = false).pathFamily(5)).isNull()
    }

    @Test
    fun `what an app from before kept for the family moves under the home family, once`() {
        val file = File(folder.root, "settings.preferences_pb")
        val after = launch(
            file,
            before = { store ->
                store.edit {
                    it[longPreferencesKey("board_cursor")] = 88
                    it[longPreferencesKey("pack_cursor")] = 14
                    it[longPreferencesKey("board_seen_note_id")] = 12
                    it[longPreferencesKey("board_seen_content_seq")] = 80
                }
            },
        ) { settings ->
            settings.claimLegacyFamilyState(3)
            settings.setBoardCursor(3, 90)
            // A second claim finds nothing left to move.
            settings.claimLegacyFamilyState(5)
            settings.state.first()
        }

        assertThat(after.boardCursor(3)).isEqualTo(90)
        assertThat(after.packCursor(3)).isEqualTo(14)
        assertThat(after.boardSeenNoteIds).containsExactly(3L, 12L)
        assertThat(after.boardSeenContentSeqs).containsExactly(3L, 80L)
        assertThat(after.boardCursors.keys).containsExactly(3L)
    }

    @Test
    fun `leaving a family forgets it and its cursors, and a sign-out forgets everything`() {
        val file = File(folder.root, "settings.preferences_pb")
        val left = launch(file) { settings ->
            settings.setFamilies(listOf(home, mine), emptyList(), emptyList(), true, 5, true)
            settings.setBoardCursor(3, 88)
            settings.setPackCursor(5, 14)
            settings.forgetFamily(5)
            settings.state.first()
        }
        assertThat(left.families).containsExactly(home)
        assertThat(left.packCursors).isEmpty()
        assertThat(left.boardCursor(3)).isEqualTo(88)

        val after = launch(file) { settings ->
            settings.setCurrentFamilyId(3)
            settings.resetKeepingServerUrl()
            settings.state.first()
        }
        assertThat(after.families).isEmpty()
        assertThat(after.currentFamilyId).isNull()
        assertThat(after.boardCursors).isEmpty()
    }

    @Test
    fun `the codecs never throw on what they cannot read`() {
        assertThat(LongMapCodec.decode("3:88,5:12")).containsExactly(3L, 88L, 5L, 12L)
        assertThat(LongMapCodec.encode(mapOf(5L to 12L, 3L to 88L))).isEqualTo("3:88,5:12")
        assertThat(LongMapCodec.decode("garbage,3:x,:4,7:1")).containsExactly(7L, 1L)
        assertThat(LongMapCodec.decode(null)).isEmpty()
        assertThat(FamilyListCodec.decodeFamilies("not json")).isEmpty()
        assertThat(FamilyListCodec.decodeInvitations(FamilyListCodec.encodeInvitations(listOf(invitation))))
            .containsExactly(invitation)
    }
}
