/*
 * SeveralFamiliesApiTest.kt
 * Family Connect (Android)
 *
 * Several families on the wire (#88; docs/protocol.md, "Several
 * families"): every request says `X-FC-Families: 1`, the socket asks with
 * `families=1`, a family-level call names its family in the path (and keeps
 * the old form, which means the home family, for null), uploads and new
 * groups name theirs, and the invitation endpoints are spelled as the
 * protocol's table spells them.
 */

package me.nettrash.familyconnect.data.net

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.data.settings.ServerUrlNormalizer
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.di.AppModule
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import okhttp3.Interceptor
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Protocol
import okhttp3.Response
import okhttp3.ResponseBody.Companion.toResponseBody
import okio.Buffer
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import java.io.File

@RunWith(RobolectricTestRunner::class)
class SeveralFamiliesApiTest {

    private data class Seen(val method: String, val path: String, val query: String?, val body: String, val families: String?)

    private val seen = mutableListOf<Seen>()

    private var reply: Pair<Int, String> = 200 to "{}"

    private val server = Interceptor { chain ->
        val request = chain.request()
        val buffer = Buffer().also { request.body?.writeTo(it) }
        seen += Seen(
            request.method,
            request.url.encodedPath,
            request.url.encodedQuery,
            buffer.readUtf8(),
            request.header("X-FC-Families"),
        )
        val (code, text) = reply
        Response.Builder()
            .request(request)
            .protocol(Protocol.HTTP_1_1)
            .code(code)
            .message("x")
            .body(text.toResponseBody("application/json".toMediaType()))
            .build()
    }

    private fun client() = ApiClient(
        context = RuntimeEnvironment.getApplication(),
        client = AppModule.provideOkHttpClient().newBuilder().addInterceptor(server).build(),
        json = Json {
            ignoreUnknownKeys = true
            encodeDefaults = false
        },
        tokenStore = FakeTokenStore("tok"),
        settings = FakeSettingsRepository(SettingsState(serverUrl = "https://chat.example.com")),
    )

    private val family = """{"family":{"id":5,"name":"The Joneses","join_policy":"open"}}"""

    @Test
    fun `every request says the client knows about several families`() = runTest {
        DefaultFamilyApi(client()).family(null)
        DefaultAuthApi(client()).me()

        assertThat(seen.map { it.families }).containsExactly("1", "1")
    }

    @Test
    fun `the socket asks for every family too`() {
        assertThat(ServerUrlNormalizer.wsUrl("https://chat.example.com")).endsWith("/api/v1/ws?families=1")
    }

    @Test
    fun `a family-level call names its family, and null keeps the home family's old form`() = runTest {
        reply = 200 to family
        val api = DefaultFamilyApi(client())

        api.family(5)
        api.family(null)
        api.leave(5)
        api.leave(null)
        api.rotateInviteCode(5)
        api.setJoinPolicy(5, "approval")
        api.resetMemberPassword(5, 11, "secret12")
        api.removeMember(5, 11)
        api.stats(5)

        assertThat(seen.map { it.method to it.path }).containsExactly(
            "GET" to "/api/v1/families/5",
            "GET" to "/api/v1/families/mine",
            "POST" to "/api/v1/families/5/leave",
            "POST" to "/api/v1/families/leave",
            "POST" to "/api/v1/families/5/invite-code/rotate",
            "PATCH" to "/api/v1/families/5",
            "POST" to "/api/v1/families/5/members/11/password",
            "DELETE" to "/api/v1/families/5/members/11",
            "GET" to "/api/v1/families/5/stats",
        ).inOrder()
    }

    @Test
    fun `blocking keeps the form without a family — a block holds in every family`() = runTest {
        reply = 204 to ""
        DefaultFamilyApi(client()).blockMember(11)

        assertThat(seen.single().path).isEqualTo("/api/v1/families/members/11/block")
    }

    @Test
    fun `the board and the pack of one family`() = runTest {
        reply = 200 to """{"notes":[],"max_board_seq":0,"items":[],"max_pack_seq":0}"""
        DefaultBoardApi(client()).getBoard(5)
        DefaultBoardApi(client()).getBoardChanges(5, 10, 200)
        DefaultPackApi(client()).getPack(5)
        DefaultPackApi(client()).getPack(null)

        assertThat(seen.map { it.path }).containsExactly(
            "/api/v1/families/5/board",
            "/api/v1/families/5/board/changes",
            "/api/v1/families/5/pack",
            "/api/v1/families/mine/pack",
        ).inOrder()
    }

    @Test
    fun `the invitation endpoints, as the protocol spells them`() = runTest {
        val api = DefaultFamilyApi(client())
        reply = 201 to """{"invitation":{"id":4,"family_id":5,"family_name":"The Joneses",
            "invited_by":{"id":7,"username":"anna","display_name":"Anna"},
            "user":{"id":11,"username":"ben","display_name":"Ben"},"created_at":"2026-10-22T10:00:00Z"}}"""

        val invited = api.invite(5, 11)
        reply = 200 to """{"invitations":[]}"""
        api.invitations(5)
        reply = 204 to ""
        api.withdrawInvitation(5, 4)
        reply = 200 to family
        val accepted = api.acceptInvitation(4)
        reply = 204 to ""
        api.declineInvitation(4)

        assertThat(seen.map { it.method to it.path }).containsExactly(
            "POST" to "/api/v1/families/5/invitations",
            "GET" to "/api/v1/families/5/invitations",
            "DELETE" to "/api/v1/families/5/invitations/4",
            "POST" to "/api/v1/invitations/4/accept",
            "POST" to "/api/v1/invitations/4/decline",
        ).inOrder()
        assertThat(seen.first().body).isEqualTo("""{"user_id":11}""")
        assertThat((invited as ApiResult.Ok).value.invitation.invitedBy.displayName).isEqualTo("Anna")
        assertThat((accepted as ApiResult.Ok).value.family.id).isEqualTo(5L)
    }

    @Test
    fun `an upload names the family it is for, and none leaves it off`() = runTest {
        reply = 201 to """{"attachment":{"id":9,"kind":"location","mime":"x","size":0}}"""
        val api = DefaultAttachmentApi(client())

        api.uploadLocation(55.7, 37.6, null, null, familyId = 5)
        api.uploadLocation(55.7, 37.6, null, null)
        val file = File.createTempFile("pic", ".jpg").apply { writeBytes(byteArrayOf(1, 2, 3)) }
        api.upload(file, "image/jpeg", "photo", 1, 1, null, familyId = 5)
        file.delete()

        assertThat(seen[0].query).contains("family_id=5")
        assertThat(seen[1].query).doesNotContain("family_id")
        assertThat(seen[2].query).contains("family_id=5")
    }

    @Test
    fun `a new group names its family, and the home family's sends nothing`() = runTest {
        reply = 201 to """{"chat":{"id":77,"kind":"group","title":"Trip"}}"""
        val api = DefaultGroupApi(client())

        api.create("Trip", listOf(9), familyId = 5)
        api.create("Trip", listOf(9))

        assertThat(seen[0].body).isEqualTo("""{"title":"Trip","member_ids":[9],"family_id":5}""")
        assertThat(seen[1].body).isEqualTo("""{"title":"Trip","member_ids":[9]}""")
    }
}
