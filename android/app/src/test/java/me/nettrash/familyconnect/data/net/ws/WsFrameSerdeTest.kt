/*
 * WsFrameSerdeTest.kt
 * Family Connect (Android)
 *
 * Round-trips every frame against the EXACT JSON in docs/protocol.md's
 * "WebSocket protocol" section — the doc is authoritative, so these
 * literals are transcribed, not derived. Comparison is by JsonElement:
 * exact content, order-insensitive.
 */

package me.nettrash.familyconnect.data.net.ws

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.CallDto
import me.nettrash.familyconnect.data.net.dto.IceCandidateDto
import kotlinx.serialization.json.Json
import org.junit.Test

class WsFrameSerdeTest {

    // Same configuration as AppModule.provideJson.
    private val json = Json {
        ignoreUnknownKeys = true
        classDiscriminator = "type"
        encodeDefaults = false
    }

    private fun assertEncodesTo(frame: ClientFrame, expected: String) {
        val encoded = json.encodeToString<ClientFrame>(frame)
        assertThat(json.parseToJsonElement(encoded))
            .isEqualTo(json.parseToJsonElement(expected))
    }

    private fun assertRoundTrips(literal: String, expected: ServerFrame) {
        val decoded = parseServerFrame(json, literal)
        assertThat(decoded).isEqualTo(expected)
        // And back: re-encoding produces the same JSON content.
        val encoded = json.encodeToString<ServerFrame>(decoded!!)
        assertThat(json.parseToJsonElement(encoded))
            .isEqualTo(json.parseToJsonElement(literal))
    }

    // -- Client → server (protocol.md lines, verbatim values) ---------------

    @Test
    fun sendFrameEncodesExactlyAsProtocol() {
        assertEncodesTo(
            ClientFrame.Send(
                chatId = 42,
                clientMsgId = "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
                body = "Dinner at 7?",
            ),
            """{"type": "send", "chat_id": 42, "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01", "body": "Dinner at 7?"}""",
        )
    }

    @Test
    fun readFrameEncodesExactlyAsProtocol() {
        assertEncodesTo(
            ClientFrame.Read(chatId = 42, lastReadMessageId = 1337),
            """{"type": "read", "chat_id": 42, "last_read_message_id": 1337}""",
        )
    }

    @Test
    fun typingFrameEncodesExactlyAsProtocol() {
        assertEncodesTo(
            ClientFrame.Typing(chatId = 42),
            """{"type": "typing", "chat_id": 42}""",
        )
    }

    @Test
    fun pingFrameEncodesExactlyAsProtocol() {
        assertEncodesTo(ClientFrame.Ping, """{"type": "ping"}""")
    }

    // -- Server → client -------------------------------------------------------

    private val messageJson =
        """{"id": 1338, "chat_id": 42, "sender_id": 7,
           "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
           "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:12Z"}"""

    private val messageDto = me.nettrash.familyconnect.data.net.dto.MessageDto(
        id = 1338,
        chatId = 42,
        senderId = 7,
        clientMsgId = "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
        body = "Dinner at 7?",
        createdAt = "2026-08-19T17:03:12Z",
    )

    @Test
    fun ackFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "ack", "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01", "message": $messageJson}""",
            ServerFrame.Ack(
                clientMsgId = "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
                message = messageDto,
            ),
        )
    }

    @Test
    fun messageFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "message", "message": $messageJson}""",
            ServerFrame.Message(message = messageDto),
        )
    }

    @Test
    fun messageFrameWithEmbeddedReactionsRoundTrips() {
        // Message objects grow OPTIONAL reactions + reaction_seq once
        // the message has ever been reacted to (protocol, "Objects") —
        // and encodeDefaults=false keeps them absent otherwise, which
        // the plain messageFrameRoundTrips above already pins.
        val reactedJson =
            """{"id": 1338, "chat_id": 42, "sender_id": 7,
               "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
               "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:12Z",
               "reactions": [{"user_id": 9, "emoji": "❤️"}], "reaction_seq": 123}"""
        assertRoundTrips(
            """{"type": "message", "message": $reactedJson}""",
            ServerFrame.Message(
                message = messageDto.copy(
                    reactions = listOf(
                        me.nettrash.familyconnect.data.net.dto.ReactionDto(userId = 9, emoji = "❤️"),
                    ),
                    reactionSeq = 123,
                ),
            ),
        )
    }

    @Test
    fun readFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "read", "chat_id": 42, "user_id": 9, "last_read_message_id": 1338}""",
            ServerFrame.Read(chatId = 42, userId = 9, lastReadMessageId = 1338),
        )
    }

    @Test
    fun typingFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "typing", "chat_id": 42, "user_id": 9}""",
            ServerFrame.Typing(chatId = 42, userId = 9),
        )
    }

    /**
     * protocol.md's second `send` example. The field is optional, so a
     * client that predates replies keeps working — and, because the house
     * Json sets encodeDefaults=false, an ordinary send frame stays
     * byte-identical to what it was before replies existed.
     */
    @Test
    fun sendFrameCarriesAReplyTarget() {
        assertEncodesTo(
            ClientFrame.Send(
                chatId = 42,
                clientMsgId = "1c4a9b02",
                body = "Six works",
                replyToMessageId = 1337,
            ),
            """{"type": "send", "chat_id": 42, "client_msg_id": "1c4a9b02", "body": "Six works", "reply_to_message_id": 1337}""",
        )
    }

    @Test
    fun memberJoinedFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "member_joined", "family_id": 3, "user": {"id": 11, "username": "junior", "display_name": "Junior"}}""",
            ServerFrame.MemberJoined(
                familyId = 3,
                user = me.nettrash.familyconnect.data.net.dto.UserDto(
                    id = 11,
                    username = "junior",
                    displayName = "Junior",
                ),
            ),
        )
    }

    /**
     * The frame is the one place a picture change reaches a client
     * without a roster refresh (protocol: a frame carries at most the
     * avatar_version, never the bytes), so the field has to survive the
     * frame decoder — and stay absent-tolerant for older servers.
     */
    @Test
    fun memberJoinedFrameCarriesTheAvatarVersion() {
        assertRoundTrips(
            """{"type": "member_joined", "family_id": 3, "user": {"id": 11, "username": "junior", "display_name": "Junior", "avatar_version": 7}}""",
            ServerFrame.MemberJoined(
                familyId = 3,
                user = me.nettrash.familyconnect.data.net.dto.UserDto(
                    id = 11,
                    username = "junior",
                    displayName = "Junior",
                    avatarVersion = 7,
                ),
            ),
        )
    }

    @Test
    fun memberLeftFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "member_left", "family_id": 3, "user_id": 11}""",
            ServerFrame.MemberLeft(familyId = 3, userId = 11),
        )
    }

    /**
     * The tombstone `Member`, exactly as protocol.md writes it: the flag,
     * the placeholder name, `avatar_version: 0` — and NO `role`, which is
     * why that field had to become optional on MemberDto.
     */
    @Test
    fun memberDeletedFrameDecodes() {
        // Decode-only rather than a round trip: `avatar_version` is 0 on a
        // tombstone and the house Json omits defaults on the way out, so
        // re-encoding this literal cannot reproduce it byte for byte. The
        // direction that matters is the inbound one.
        val decoded = parseServerFrame(
            json,
            """{"type": "member_deleted", "family_id": 3,
                "member": {"id": 11, "username": "junior",
                           "display_name": "Deleted account",
                           "avatar_version": 0, "deleted": true}}""",
        )
        assertThat(decoded).isEqualTo(
            ServerFrame.MemberDeleted(
                familyId = 3,
                member = me.nettrash.familyconnect.data.net.dto.MemberDto(
                    id = 11,
                    username = "junior",
                    displayName = "Deleted account",
                    deleted = true,
                ),
            ),
        )
        // No role on the wire, and none invented on the way in — the
        // roster maps that to a stored value deliberately, it is not the
        // decoder's to guess.
        assertThat((decoded as ServerFrame.MemberDeleted).member.role).isNull()
        assertThat(decoded.member.isDeleted).isTrue()
    }

    @Test
    fun familyOwnerFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "family_owner", "family_id": 3, "user_id": 9}""",
            ServerFrame.FamilyOwner(familyId = 3, userId = 9),
        )
    }

    @Test
    fun memberBlockedFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "member_blocked", "user_id": 11, "blocked": true}""",
            ServerFrame.MemberBlocked(userId = 11, blocked = true),
        )
    }

    /**
     * The `false` case is the one that proves neither field was given a
     * Kotlin default: `encodeDefaults = false` would drop `"blocked":
     * false` from the re-encoded form and this round-trip would fail. An
     * unblock is the same frame carrying full current state, not a
     * different frame (docs/protocol.md, "Blocking a member").
     */
    @Test
    fun anUnblockIsTheSameFrameCarryingFalse() {
        assertRoundTrips(
            """{"type": "member_blocked", "user_id": 11, "blocked": false}""",
            ServerFrame.MemberBlocked(userId = 11, blocked = false),
        )
    }

    @Test
    fun reactionFrameRoundTrips() {
        // protocol.md, "Server → client" — the frame carries the FULL
        // current reaction state, never a delta.
        assertRoundTrips(
            """{"type": "reaction", "chat_id": 42, "message_id": 1338, "reaction_seq": 124,
                "reactions": [{"user_id": 9, "emoji": "❤️"}]}""",
            ServerFrame.Reaction(
                chatId = 42,
                messageId = 1338,
                reactionSeq = 124,
                reactions = listOf(
                    me.nettrash.familyconnect.data.net.dto.ReactionDto(userId = 9, emoji = "❤️"),
                ),
            ),
        )
    }

    @Test
    fun reactionFrameWithEmptyReactionsRoundTrips() {
        // "Cleared" state: the last reaction was removed — reactions is
        // [] with the seq still present.
        assertRoundTrips(
            """{"type": "reaction", "chat_id": 42, "message_id": 1338, "reaction_seq": 125, "reactions": []}""",
            ServerFrame.Reaction(
                chatId = 42,
                messageId = 1338,
                reactionSeq = 125,
                reactions = emptyList(),
            ),
        )
    }

    @Test
    fun pollFrameRoundTrips() {
        // protocol.md, "Server -> client", transcribed verbatim: the frame
        // carries a poll's FULL current state, never a delta.
        assertRoundTrips(
            """{"type": "poll", "chat_id": 42, "message_id": 1340,
                "poll": {"poll_seq": 89, "closed": false,
                         "options": [{"id": 5, "text": "Pizza", "votes": [7, 9]},
                                     {"id": 6, "text": "Pasta", "votes": []}]}}""",
            ServerFrame.Poll(
                chatId = 42,
                messageId = 1340,
                poll = me.nettrash.familyconnect.data.net.dto.PollDto(
                    pollSeq = 89,
                    closed = false,
                    options = listOf(
                        me.nettrash.familyconnect.data.net.dto.PollOptionDto(
                            id = 5,
                            text = "Pizza",
                            votes = listOf(7, 9),
                        ),
                        me.nettrash.familyconnect.data.net.dto.PollOptionDto(
                            id = 6,
                            text = "Pasta",
                            votes = emptyList(),
                        ),
                    ),
                ),
            ),
        )
    }

    @Test
    fun aClosedPollRoundTripsWithItsResult() {
        // `closed` is ALWAYS on the wire, and it survives the re-encode:
        // it carries no Kotlin default, so encodeDefaults=false has
        // nothing to drop. Without that a closed poll would come back out
        // of this client looking open.
        assertRoundTrips(
            """{"type": "poll", "chat_id": 42, "message_id": 1340,
                "poll": {"poll_seq": 91, "closed": true,
                         "options": [{"id": 5, "text": "Pizza", "votes": [7]}]}}""",
            ServerFrame.Poll(
                chatId = 42,
                messageId = 1340,
                poll = me.nettrash.familyconnect.data.net.dto.PollDto(
                    pollSeq = 91,
                    closed = true,
                    options = listOf(
                        me.nettrash.familyconnect.data.net.dto.PollOptionDto(
                            id = 5,
                            text = "Pizza",
                            votes = listOf(7),
                        ),
                    ),
                ),
            ),
        )
    }

    @Test
    fun messageFrameWithAnEmbeddedPollRoundTrips() {
        // A poll IS an ordinary message — its question is the body — so
        // it arrives on the `message` frame like everything else, and a
        // client that knows nothing of polls loses only the buttons.
        val polledJson =
            """{"id": 1340, "chat_id": 42, "sender_id": 7,
               "client_msg_id": "5b2e0c14-ceea-4e17-a91c-0d9f8e7b2a01",
               "body": "Pizza or pasta?", "created_at": "2026-08-19T17:03:12Z",
               "poll": {"poll_seq": 88, "closed": false,
                        "options": [{"id": 5, "text": "Pizza", "votes": []},
                                    {"id": 6, "text": "Pasta", "votes": []}]}}"""
        assertRoundTrips(
            """{"type": "message", "message": $polledJson}""",
            ServerFrame.Message(
                message = me.nettrash.familyconnect.data.net.dto.MessageDto(
                    id = 1340,
                    chatId = 42,
                    senderId = 7,
                    clientMsgId = "5b2e0c14-ceea-4e17-a91c-0d9f8e7b2a01",
                    body = "Pizza or pasta?",
                    createdAt = "2026-08-19T17:03:12Z",
                    poll = me.nettrash.familyconnect.data.net.dto.PollDto(
                        pollSeq = 88,
                        closed = false,
                        options = listOf(
                            me.nettrash.familyconnect.data.net.dto.PollOptionDto(
                                id = 5,
                                text = "Pizza",
                                votes = emptyList(),
                            ),
                            me.nettrash.familyconnect.data.net.dto.PollOptionDto(
                                id = 6,
                                text = "Pasta",
                                votes = emptyList(),
                            ),
                        ),
                    ),
                ),
            ),
        )
    }

    @Test
    fun sendFrameWithAPollEncodesExactlyAsProtocol() {
        assertEncodesTo(
            ClientFrame.Send(
                chatId = 42,
                clientMsgId = "5b2e0c14-ceea-4e17-a91c-0d9f8e7b2a01",
                body = "Pizza or pasta?",
                poll = me.nettrash.familyconnect.data.net.dto.NewPollDto(
                    options = listOf("Pizza", "Pasta"),
                ),
            ),
            """{"type": "send", "chat_id": 42, "client_msg_id": "5b2e0c14-ceea-4e17-a91c-0d9f8e7b2a01",
                "body": "Pizza or pasta?", "poll": {"options": ["Pizza", "Pasta"]}}""",
        )
    }

    /**
     * protocol.md's third `send` example, verbatim: an album claim. The
     * PLURAL spelling always — this client stopped sending the legacy
     * `attachment_id` when messages learned to carry more than one.
     */
    @Test
    fun sendFrameWithAttachmentIdsEncodesExactlyAsProtocol() {
        assertEncodesTo(
            ClientFrame.Send(
                chatId = 42,
                clientMsgId = "9d3f1e77-ceea-4e17-a91c-0d9f8e7b2a01",
                body = "",
                attachmentIds = listOf(34, 35, 36),
            ),
            """{"type": "send", "chat_id": 42, "client_msg_id": "9d3f1e77-ceea-4e17-a91c-0d9f8e7b2a01",
                "body": "", "attachment_ids": [34, 35, 36]}""",
        )
    }

    /**
     * A message carrying attachments per protocol.md's "Objects": the
     * plural array in the sender's order, plus the legacy `attachment` —
     * its FIRST element, which a reader that knows `attachments` ignores.
     */
    @Test
    fun aMessageWithAttachmentsReadsThePluralAndIgnoresTheLegacy() {
        val decoded = parseServerFrame(
            json,
            """{"type": "message", "message": {"id": 1341, "chat_id": 42, "sender_id": 7,
                "client_msg_id": "9d3f1e77-ceea-4e17-a91c-0d9f8e7b2a01",
                "body": "", "created_at": "2026-08-19T17:03:12Z",
                "attachments": [{"id": 34, "kind": "photo", "mime": "image/jpeg", "size": 182734},
                                {"id": 35, "kind": "video", "mime": "video/mp4", "size": 999999,
                                 "duration_ms": 8400}],
                "attachment": {"id": 34, "kind": "photo", "mime": "image/jpeg", "size": 182734}}}""",
        ) as ServerFrame.Message
        assertThat(decoded.message.resolvedAttachments.map { it.id })
            .containsExactly(34L, 35L)
            .inOrder()
    }

    /** A legacy-only server: the singular is still the fallback read. */
    @Test
    fun aLegacyOnlyAttachmentStillResolves() {
        val decoded = parseServerFrame(
            json,
            """{"type": "message", "message": {"id": 1342, "chat_id": 42, "sender_id": 7,
                "client_msg_id": "9d3f1e77-ceea-4e17-a91c-0d9f8e7b2a01",
                "body": "", "created_at": "2026-08-19T17:03:12Z",
                "attachment": {"id": 34, "kind": "photo", "mime": "image/jpeg", "size": 182734}}}""",
        ) as ServerFrame.Message
        assertThat(decoded.message.resolvedAttachments.map { it.id }).containsExactly(34L)
    }

    @Test
    fun pongFrameRoundTrips() {
        assertRoundTrips("""{"type": "pong"}""", ServerFrame.Pong)
    }

    @Test
    fun errorFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "error", "code": "not_chat_member", "message": "you are not a member of this chat", "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01"}""",
            ServerFrame.Error(
                code = "not_chat_member",
                message = "you are not a member of this chat",
                clientMsgId = "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
            ),
        )
    }

    @Test
    fun errorFrameWithoutClientMsgIdDecodes() {
        val decoded = parseServerFrame(json, """{"type": "error", "code": "internal", "message": "boom"}""")
        assertThat(decoded).isEqualTo(ServerFrame.Error(code = "internal", message = "boom"))
    }

    // -- Compatibility rules --------------------------------------------------------

    @Test
    fun unknownFrameTypeIsDroppedNotThrown() {
        // Whatever comes after voice calls must not kill the socket —
        // exactly as call_offer did not, before this client knew it.
        val decoded = parseServerFrame(
            json,
            """{"type": "video_offer", "chat_id": 42, "sdp": "v=0..."}""",
        )
        assertThat(decoded).isNull()
    }

    // -- Voice calls (protocol.md, "Voice calls" — verbatim) -------------------

    @Test
    fun callOfferFrameEncodesExactlyAsProtocol() {
        assertEncodesTo(
            ClientFrame.CallOffer(callId = "6a1f0c3e-0000-4000-8000-000000000001", chatId = 42, sdp = "v=0\r\n…"),
            """{"type": "call_offer", "call_id": "6a1f0c3e-0000-4000-8000-000000000001", "chat_id": 42, "sdp": "v=0\r\n…"}""",
        )
    }

    @Test
    fun callAnswerFrameEncodesExactlyAsProtocol() {
        assertEncodesTo(
            ClientFrame.CallAnswer(callId = "6a1f0c3e-0000-4000-8000-000000000001", sdp = "v=0\r\n…"),
            """{"type": "call_answer", "call_id": "6a1f0c3e-0000-4000-8000-000000000001", "sdp": "v=0\r\n…"}""",
        )
    }

    @Test
    fun callIceFrameEncodesExactlyAsProtocol() {
        assertEncodesTo(
            ClientFrame.CallIce(
                callId = "6a1f0c3e-0000-4000-8000-000000000001",
                candidate = IceCandidateDto(candidate = "candidate:…", sdpMid = "0", sdpMlineIndex = 0),
            ),
            """{"type": "call_ice", "call_id": "6a1f0c3e-0000-4000-8000-000000000001",
               "candidate": {"candidate": "candidate:…", "sdp_mid": "0", "sdp_mline_index": 0}}""",
        )
    }

    @Test
    fun aCandidateOmitsWhatTheStackDidNotSupply() {
        // sdp_mid and sdp_mline_index are each optional and ABSENT — never
        // null — when absent, like every optional field on this wire.
        assertEncodesTo(
            ClientFrame.CallIce(
                callId = "6a1f0c3e-0000-4000-8000-000000000001",
                candidate = IceCandidateDto(candidate = "candidate:…"),
            ),
            """{"type": "call_ice", "call_id": "6a1f0c3e-0000-4000-8000-000000000001",
               "candidate": {"candidate": "candidate:…"}}""",
        )
    }

    @Test
    fun callEndFrameEncodesExactlyAsProtocol() {
        assertEncodesTo(
            ClientFrame.CallEnd(callId = "6a1f0c3e-0000-4000-8000-000000000001", reason = CallEndReason.HANGUP),
            """{"type": "call_end", "call_id": "6a1f0c3e-0000-4000-8000-000000000001", "reason": "hangup"}""",
        )
    }

    @Test
    fun serverCallOfferFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "call_offer", "call_id": "6a1f0c3e-0000-4000-8000-000000000001", "chat_id": 42, "from_user_id": 7, "sdp": "v=0\r\n…"}""",
            ServerFrame.CallOffer(
                callId = "6a1f0c3e-0000-4000-8000-000000000001",
                chatId = 42,
                fromUserId = 7,
                sdp = "v=0\r\n…",
            ),
        )
    }

    @Test
    fun serverCallRingingFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "call_ringing", "call_id": "6a1f0c3e-0000-4000-8000-000000000001"}""",
            ServerFrame.CallRinging(callId = "6a1f0c3e-0000-4000-8000-000000000001"),
        )
    }

    @Test
    fun serverCallAnswerFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "call_answer", "call_id": "6a1f0c3e-0000-4000-8000-000000000001", "sdp": "v=0\r\n…"}""",
            ServerFrame.CallAnswer(callId = "6a1f0c3e-0000-4000-8000-000000000001", sdp = "v=0\r\n…"),
        )
    }

    @Test
    fun serverCallIceFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "call_ice", "call_id": "6a1f0c3e-0000-4000-8000-000000000001",
               "candidate": {"candidate": "candidate:…", "sdp_mid": "0", "sdp_mline_index": 0}}""",
            ServerFrame.CallIce(
                callId = "6a1f0c3e-0000-4000-8000-000000000001",
                candidate = IceCandidateDto(candidate = "candidate:…", sdpMid = "0", sdpMlineIndex = 0),
            ),
        )
    }

    @Test
    fun serverCallEndFrameRoundTrips() {
        assertRoundTrips(
            """{"type": "call_end", "call_id": "6a1f0c3e-0000-4000-8000-000000000001", "reason": "answered_elsewhere"}""",
            ServerFrame.CallEnd(callId = "6a1f0c3e-0000-4000-8000-000000000001", reason = "answered_elsewhere"),
        )
    }

    @Test
    fun errorFrameAnsweringACallCarriesTheCallId() {
        assertRoundTrips(
            """{"type": "error", "code": "peer_busy", "message": "…", "call_id": "6a1f0c3e-0000-4000-8000-000000000001"}""",
            ServerFrame.Error(code = "peer_busy", message = "…", callId = "6a1f0c3e-0000-4000-8000-000000000001"),
        )
    }

    @Test
    fun aMessageCarryingACallRecordDecodesIt() {
        val decoded = parseServerFrame(
            json,
            """{"type": "message", "message": {"id": 1338, "chat_id": 42, "sender_id": 7,
               "client_msg_id": "6a1f0c3e-0000-4000-8000-000000000001",
               "body": "Voice call", "created_at": "2026-08-19T17:03:12Z",
               "call": {"outcome": "completed", "duration_secs": 222}}}""",
        ) as ServerFrame.Message
        assertThat(decoded.message.call).isEqualTo(CallDto(outcome = "completed", durationSecs = 222))
        // Absent on an ordinary message — and absent means "not a call".
        val plain = parseServerFrame(json, """{"type": "message", "message": $messageJson}""") as ServerFrame.Message
        assertThat(plain.message.call).isNull()
    }

    // -- Video (protocol.md, "Video" — literals verbatim) --------------------

    @Test
    fun aVideoCallOfferEncodesItsKindExactlyAsProtocol() {
        assertEncodesTo(
            ClientFrame.CallOffer(
                callId = "7b2e1d4f-0000-4000-8000-000000000002",
                chatId = 42,
                sdp = "v=0\r\n…",
                video = true,
            ),
            """{"type": "call_offer", "call_id": "7b2e1d4f-0000-4000-8000-000000000002", "chat_id": 42, "sdp": "v=0\r\n…", "video": true}""",
        )
    }

    @Test
    fun aVoiceCallOfferStaysByteIdenticalCarryingNoVideoKey() {
        // encodeDefaults=false is what PINS this: a voice offer is EXACTLY
        // what it was before video existed — the key is ABSENT, not false.
        val encoded = json.encodeToString<ClientFrame>(
            ClientFrame.CallOffer(callId = "6a1f0c3e-0000-4000-8000-000000000001", chatId = 42, sdp = "v=0\r\n…"),
        )
        assertThat(encoded).doesNotContain("video")
        assertThat(json.parseToJsonElement(encoded)).isEqualTo(
            json.parseToJsonElement(
                """{"type": "call_offer", "call_id": "6a1f0c3e-0000-4000-8000-000000000001", "chat_id": 42, "sdp": "v=0\r\n…"}""",
            ),
        )
    }

    @Test
    fun serverVideoCallOfferRoundTripsExactlyAsProtocol() {
        assertRoundTrips(
            """{"type": "call_offer", "call_id": "7b2e1d4f-0000-4000-8000-000000000002", "chat_id": 42, "from_user_id": 7, "sdp": "v=0\r\n…", "video": true}""",
            ServerFrame.CallOffer(
                callId = "7b2e1d4f-0000-4000-8000-000000000002",
                chatId = 42,
                fromUserId = 7,
                sdp = "v=0\r\n…",
                video = true,
            ),
        )
    }

    @Test
    fun aMessageCarryingAVideoCallRecordDecodesItsKind() {
        val decoded = parseServerFrame(
            json,
            """{"type": "message", "message": {"id": 1339, "chat_id": 42, "sender_id": 7,
               "client_msg_id": "7b2e1d4f-0000-4000-8000-000000000002",
               "body": "Video call", "created_at": "2026-08-26T17:03:12Z",
               "call": {"outcome": "completed", "duration_secs": 222, "video": true}}}""",
        ) as ServerFrame.Message
        assertThat(decoded.message.call).isEqualTo(CallDto(outcome = "completed", durationSecs = 222, video = true))
    }

    @Test
    fun malformedJsonIsDroppedNotThrown() {
        assertThat(parseServerFrame(json, "{not json")).isNull()
    }

    @Test
    fun unknownKeysInKnownFramesAreIgnored() {
        val decoded = parseServerFrame(
            json,
            """{"type": "pong", "server_time": "2026-08-19T17:03:12Z", "shard": 4}""",
        )
        assertThat(decoded).isEqualTo(ServerFrame.Pong)

        val read = parseServerFrame(
            json,
            """{"type": "read", "chat_id": 42, "user_id": 9, "last_read_message_id": 1338, "extra": true}""",
        )
        assertThat(read).isEqualTo(ServerFrame.Read(chatId = 42, userId = 9, lastReadMessageId = 1338))
    }
}
