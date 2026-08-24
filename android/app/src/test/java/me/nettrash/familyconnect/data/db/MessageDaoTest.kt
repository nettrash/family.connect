/*
 * MessageDaoTest.kt
 * Family Connect (Android)
 *
 * The DAO carries the ordering contract the chat screen renders by and
 * the dedup rules the sync pipeline leans on — Robolectric + in-memory
 * Room, real SQLite semantics.
 */

package me.nettrash.familyconnect.data.db

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.testutil.createTestDb
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class MessageDaoTest {

    // One scheduler for the tests AND for Room — see TestDb.kt.
    private val dispatcher = StandardTestDispatcher()
    private lateinit var db: AppDatabase
    private lateinit var dao: MessageDao

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        dao = db.messageDao()
    }

    @After
    fun tearDown() {
        db.close()
    }

    private fun message(
        clientMsgId: String,
        serverId: Long?,
        createdAt: Long,
        chatId: Long = 1L,
        senderId: Long = 7L,
        status: MessageStatus = if (serverId == null) MessageStatus.SENDING else MessageStatus.SENT,
    ) = MessageEntity(
        clientMsgId = clientMsgId,
        serverId = serverId,
        chatId = chatId,
        senderId = senderId,
        body = "body $clientMsgId",
        createdAt = createdAt,
        status = status,
    )

    @Test
    fun observeMessagesOrdersPendingFirstThenByServerIdDescending() = runTest(dispatcher) {
        dao.insert(message("s10", serverId = 10, createdAt = 100))
        dao.insert(message("s30", serverId = 30, createdAt = 300))
        dao.insert(message("pending-b", serverId = null, createdAt = 250))
        dao.insert(message("s20", serverId = 20, createdAt = 200))
        dao.insert(message("pending-a", serverId = null, createdAt = 400))

        val ordered = dao.observeMessages(chatId = 1L, limit = 50).first()

        // Pending rows lead (they're the newest side under reverseLayout),
        // newest local first; acked rows follow by server id descending.
        assertThat(ordered.map { it.clientMsgId }).containsExactly(
            "pending-a", "pending-b", "s30", "s20", "s10",
        ).inOrder()
    }

    @Test
    fun observeMessagesHonorsLimitFromTheNewestSide() = runTest(dispatcher) {
        dao.insert(message("s1", serverId = 1, createdAt = 1))
        dao.insert(message("s2", serverId = 2, createdAt = 2))
        dao.insert(message("s3", serverId = 3, createdAt = 3))

        val window = dao.observeMessages(chatId = 1L, limit = 2).first()

        assertThat(window.map { it.serverId }).containsExactly(3L, 2L).inOrder()
    }

    @Test
    fun observeMessagesIsScopedToTheChat() = runTest(dispatcher) {
        dao.insert(message("mine", serverId = 1, createdAt = 1, chatId = 1L))
        dao.insert(message("other", serverId = 2, createdAt = 2, chatId = 2L))

        val ordered = dao.observeMessages(chatId = 1L, limit = 50).first()

        assertThat(ordered.map { it.clientMsgId }).containsExactly("mine")
    }

    @Test
    fun insertIgnoreDedupsBothByPrimaryKeyAndByServerId() = runTest(dispatcher) {
        dao.insert(message("original", serverId = 5, createdAt = 100))

        dao.insertIgnore(
            listOf(
                // Same PK — dropped.
                message("original", serverId = 99, createdAt = 999),
                // Different PK, same serverId (resync echo) — dropped.
                message("s5", serverId = 5, createdAt = 999),
                // Genuinely new — inserted.
                message("s6", serverId = 6, createdAt = 200),
            ),
        )

        val all = dao.observeMessages(chatId = 1L, limit = 50).first()
        assertThat(all.map { it.clientMsgId }).containsExactly("s6", "original").inOrder()
        assertThat(dao.findByClientMsgId("original")!!.serverId).isEqualTo(5L)
    }

    @Test
    fun markAckedPreservesThePrimaryKeyAndUpdatesInPlace() = runTest(dispatcher) {
        dao.insert(message("uuid-1", serverId = null, createdAt = 100))

        dao.markAcked("uuid-1", serverId = 77, createdAt = 12345)

        val row = dao.findByClientMsgId("uuid-1")!!
        assertThat(row.clientMsgId).isEqualTo("uuid-1") // same row, same key
        assertThat(row.serverId).isEqualTo(77L)
        assertThat(row.createdAt).isEqualTo(12345L) // server clock wins
        assertThat(row.status).isEqualTo(MessageStatus.SENT)
        assertThat(dao.findByServerId(77L)!!.clientMsgId).isEqualTo("uuid-1")
    }

    @Test
    fun multiplePendingRowsCoexistDespiteUniqueServerIdIndex() = runTest(dispatcher) {
        // SQLite unique indexes permit any number of NULLs.
        dao.insert(message("p1", serverId = null, createdAt = 1))
        dao.insert(message("p2", serverId = null, createdAt = 2))

        assertThat(dao.pendingSending().map { it.clientMsgId })
            .containsExactly("p1", "p2").inOrder()
    }

    @Test
    fun cursorsReturnMaxAndMinServerIdPerChat() = runTest(dispatcher) {
        dao.insert(message("a", serverId = 10, createdAt = 1, chatId = 1L))
        dao.insert(message("b", serverId = 25, createdAt = 2, chatId = 1L))
        dao.insert(message("c", serverId = 99, createdAt = 3, chatId = 2L))
        dao.insert(message("p", serverId = null, createdAt = 4, chatId = 1L))

        assertThat(dao.maxServerId(1L)).isEqualTo(25L)
        assertThat(dao.oldestServerId(1L)).isEqualTo(10L)
        assertThat(dao.maxServerId(3L)).isNull()
        assertThat(dao.oldestServerId(3L)).isNull()
    }

    @Test
    fun setStatusAndExistsByServerIdBehave() = runTest(dispatcher) {
        dao.insert(message("uuid-2", serverId = null, createdAt = 1))

        dao.setStatus("uuid-2", MessageStatus.FAILED)
        assertThat(dao.findByClientMsgId("uuid-2")!!.status).isEqualTo(MessageStatus.FAILED)

        assertThat(dao.existsByServerId(123L)).isFalse()
        dao.markAcked("uuid-2", serverId = 123, createdAt = 5)
        assertThat(dao.existsByServerId(123L)).isTrue()
    }

    // -- Reaction state (seq guard) -----------------------------------------

    @Test
    fun applyReactionStateAppliesOnlyWhenSeqIsStrictlyGreater() = runTest(dispatcher) {
        dao.insert(message("s5", serverId = 5, createdAt = 1))

        // Fresh row (seq 0): any positive seq applies.
        assertThat(dao.applyReactionState(5L, """[{"user_id":9,"emoji":"x"}]""", 10L)).isEqualTo(1)
        var row = dao.findByServerId(5L)!!
        assertThat(row.reactionSeq).isEqualTo(10L)
        assertThat(row.reactionsJson).isEqualTo("""[{"user_id":9,"emoji":"x"}]""")

        // Same seq re-delivered — strictly-greater guard drops it.
        assertThat(dao.applyReactionState(5L, "[]", 10L)).isEqualTo(0)
        // Stale seq — dropped too.
        assertThat(dao.applyReactionState(5L, "[]", 3L)).isEqualTo(0)
        row = dao.findByServerId(5L)!!
        assertThat(row.reactionSeq).isEqualTo(10L)
        assertThat(row.reactionsJson).isEqualTo("""[{"user_id":9,"emoji":"x"}]""")

        // Newer seq wins — including the "cleared" [] state.
        assertThat(dao.applyReactionState(5L, "[]", 11L)).isEqualTo(1)
        row = dao.findByServerId(5L)!!
        assertThat(row.reactionSeq).isEqualTo(11L)
        assertThat(row.reactionsJson).isEqualTo("[]")
    }

    @Test
    fun applyReactionStateForUnknownServerIdMatchesNothing() = runTest(dispatcher) {
        assertThat(dao.applyReactionState(404L, "[]", 99L)).isEqualTo(0)
    }

    @Test
    fun setReactionsJsonRewritesStateButNeverTheSeq() = runTest(dispatcher) {
        dao.insert(message("s5", serverId = 5, createdAt = 1))
        dao.applyReactionState(5L, "[]", 7L)

        // Optimistic rewrite: json changes, guard seq stays 7 — so the
        // authoritative response (seq 8) still passes the guard.
        dao.setReactionsJson(5L, """[{"user_id":7,"emoji":"y"}]""", expectedSeq = 7L)
        var row = dao.findByServerId(5L)!!
        assertThat(row.reactionsJson).isEqualTo("""[{"user_id":7,"emoji":"y"}]""")
        assertThat(row.reactionSeq).isEqualTo(7L)

        // Revert path: back to null works too.
        dao.setReactionsJson(5L, null, expectedSeq = 7L)
        row = dao.findByServerId(5L)!!
        assertThat(row.reactionsJson).isNull()
        assertThat(row.reactionSeq).isEqualTo(7L)
    }

    @Test
    fun setReactionsJsonIsSkippedWhenTheSeqMovedSinceTheRead() = runTest(dispatcher) {
        dao.insert(message("s5", serverId = 5, createdAt = 1))
        dao.applyReactionState(5L, "[]", 7L)

        // A WS frame lands mid-toggle and bumps the seq to 8: the stale
        // optimistic write (or revert) read the row at seq 7 and must
        // now match zero rows instead of clobbering the frame's state.
        dao.applyReactionState(5L, """[{"user_id":9,"emoji":"x"}]""", 8L)
        dao.setReactionsJson(5L, null, expectedSeq = 7L)

        val row = dao.findByServerId(5L)!!
        assertThat(row.reactionsJson).isEqualTo("""[{"user_id":9,"emoji":"x"}]""")
        assertThat(row.reactionSeq).isEqualTo(8L)
    }

    @Test
    fun deleteByClientMsgIdRemovesExactlyThatRow() = runTest(dispatcher) {
        dao.insert(message("keep", serverId = 1, createdAt = 1))
        dao.insert(message("drop", serverId = null, createdAt = 2))

        dao.deleteByClientMsgId("drop")

        assertThat(dao.observeMessages(1L, 50).first().map { it.clientMsgId })
            .containsExactly("keep")
    }

    /**
     * A location that ARRIVES rather than one this device sent.
     *
     * A location has no bytes to fall back on — the coordinates ARE the
     * attachment — so an ack that forgets to carry them does not degrade
     * the bubble, it empties it. The bug this pins was invisible to the
     * sender (whose optimistic row already held them) and total for
     * everybody else, which is exactly why it reached three platforms.
     */
    @Test
    fun setAttachmentCarriesTheCoordinatesOfALocation() = runTest(dispatcher) {
        dao.insert(message("a", serverId = null, createdAt = 1_000))

        dao.setAttachment(
            clientMsgId = "a",
            attachmentId = 61L,
            kind = "location",
            mime = "application/vnd.family-connect.location",
            size = 0L,
            width = null,
            height = null,
            durationMs = null,
            hasPreview = false,
            name = "Home",
            latitude = 55.7558,
            longitude = 37.6173,
            accuracyM = 12,
        )

        val stored = dao.findByClientMsgId("a")
        assertThat(stored).isNotNull()
        assertThat(stored!!.attachmentLatitude).isEqualTo(55.7558)
        assertThat(stored.attachmentLongitude).isEqualTo(37.6173)
        assertThat(stored.attachmentAccuracyM).isEqualTo(12)
        // And the wire shape the bubble actually reads is complete, which
        // is the property that matters: `LocationRow` returns early on a
        // null coordinate and draws nothing at all.
        val attachment = stored.attachment
        assertThat(attachment).isNotNull()
        assertThat(attachment!!.isLocation).isTrue()
        assertThat(attachment.latitude).isEqualTo(55.7558)
        assertThat(attachment.longitude).isEqualTo(37.6173)
    }

    /**
     * The repair query has to FIND the broken rows and leave everything
     * else alone — it is what heals a location that a build with the
     * inbound bug already stored, and catch-up never revisits those.
     */
    @Test
    fun locationsMissingCoordinatesFindsOnlyTheBrokenOnes() = runTest(dispatcher) {
        // Broken: a location with no pin. This is the empty bubble.
        dao.insert(
            message("broken", serverId = 10, createdAt = 1_000).copy(
                attachmentId = 61,
                attachmentKind = "location",
                attachmentMime = "application/vnd.family-connect.location",
            ),
        )
        // Whole: the same kind, with its coordinates.
        dao.insert(
            message("whole", serverId = 11, createdAt = 2_000).copy(
                attachmentId = 62,
                attachmentKind = "location",
                attachmentMime = "application/vnd.family-connect.location",
                attachmentLatitude = 55.7558,
                attachmentLongitude = 37.6173,
            ),
        )
        // A photo has no coordinates and never should — it must not be
        // dragged into a repair loop on every single resync.
        dao.insert(
            message("photo", serverId = 12, createdAt = 3_000).copy(
                attachmentId = 63,
                attachmentKind = "photo",
                attachmentMime = "image/jpeg",
            ),
        )
        // Not yet acked, so there is no server id to re-fetch by.
        dao.insert(
            message("pending", serverId = null, createdAt = 4_000).copy(
                attachmentId = 64,
                attachmentKind = "location",
                attachmentMime = "application/vnd.family-connect.location",
            ),
        )

        val found = dao.locationsMissingCoordinates(limit = 25)
        assertThat(found.map { it.clientMsgId }).containsExactly("broken")
    }
}
