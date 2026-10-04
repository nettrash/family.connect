/*
 * FamilyAdminGreetingPlacesTest.kt
 * Family Connect (Android)
 *
 * The owner's "Weather in the greeting" places (`greeting_places`) and the
 * capability beside them (`assistant.greeting_weather`), docs/protocol.md,
 * "Today's weather, for places the owner chose": decoded with the
 * compatibility defaults, written as exactly one key with `[]` meaning
 * clear, sent as the server would keep them, and drawn from the server's
 * ANSWER rather than from what was sent.
 */

package me.nettrash.familyconnect.ui.familyadmin

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.AssistantDto
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.FamilyResponse
import me.nettrash.familyconnect.data.net.dto.PatchFamilyRequest
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeFamilyApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.memberDto
import me.nettrash.familyconnect.util.GreetingPlaces
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class FamilyAdminGreetingPlacesTest {

    private companion object {
        const val ME = 7L
    }

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private val familyApi = FakeFamilyApi()
    private val settings = FakeSettingsRepository(
        SettingsState(
            serverUrl = "https://chat.example.com",
            familyStatus = FamilyStatus.OWNER,
            myUserId = ME,
        ),
    )

    private val json = Json { ignoreUnknownKeys = true }
    private val houseJson = Json {
        ignoreUnknownKeys = true
        classDiscriminator = "type"
        encodeDefaults = false
    }

    @Before
    fun setUp() {
        Dispatchers.setMain(dispatcher)
        db = createTestDb(dispatcher)
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        Dispatchers.resetMain()
        db.close()
    }

    private val weatherServer = AssistantDto(
        userId = 1,
        displayName = "Assistant",
        mention = "@ai",
        processor = "Microsoft — Azure OpenAI (Sweden Central)",
        greetingWeather = true,
    )

    private fun family(places: List<String>?) =
        FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open", aiGreeting = true, greetingPlaces = places)

    private fun mine(assistant: AssistantDto?, places: List<String>?) = ApiResult.Ok(
        FamilyMineResponse(
            family = family(places),
            members = listOf(memberDto(ME, "anna", role = "owner")),
            assistant = assistant,
        ),
    )

    private fun answer(places: List<String>?) = ApiResult.Ok(FamilyResponse(family(places)))

    private fun familyRepository() = FamilyRepository(
        familyApi = familyApi,
        authApi = FakeAuthApi(),
        memberDao = db.memberDao(),
        settings = settings,
        sessionRepository = SessionRepository(
            authApi = FakeAuthApi(),
            tokenStore = FakeTokenStore("tok"),
            settings = settings,
            wiper = RecordingWiper(),
            unauthorizedEvents = MutableSharedFlow(),
            scope = repoScope,
        ),
        socket = FakeChatSocket(),
        scope = repoScope,
    )

    private fun viewModel() = FamilyAdminViewModel(
        appContext = RuntimeEnvironment.getApplication(),
        familyRepository = familyRepository(),
        settings = settings,
    )

    // -- The wire ----------------------------------------------------------------

    @Test
    fun `an older server's family and assistant decode as no places and no greeting weather`() {
        val family = json.decodeFromString<FamilyDto>("""{"id": 3, "name": "The Smiths", "join_policy": "open"}""")
        assertThat(family.places).isEmpty()
        val assistant = json.decodeFromString<AssistantDto>(
            """{"user_id": 1, "display_name": "Assistant", "mention": "@ai"}""",
        )
        assertThat(assistant.greetingWeather).isFalse()
        assertThat(GreetingPlaces.isShown(isOwner = true, greetingWeather = assistant.greetingWeather)).isFalse()
    }

    @Test
    fun `a current server's fields decode, in the server's order`() {
        val family = json.decodeFromString<FamilyDto>(
            """{"id": 3, "name": "The Smiths", "join_policy": "open",
               "greeting_places": ["Moscow", "Belgrade"]}""",
        )
        assertThat(family.places).containsExactly("Moscow", "Belgrade").inOrder()
        val assistant = json.decodeFromString<AssistantDto>(
            """{"user_id": 1, "display_name": "Assistant", "mention": "@ai",
               "greeting_weather": true, "some_future_key": 1}""",
        )
        assertThat(assistant.greetingWeather).isTrue()
    }

    @Test
    fun `an unexpected null list decodes as none rather than failing the family`() {
        val family = json.decodeFromString<FamilyDto>(
            """{"id": 3, "name": "The Smiths", "join_policy": "open", "greeting_places": null}""",
        )
        assertThat(family.places).isEmpty()
    }

    @Test
    fun `saving sends exactly that one key, and an empty list clears`() {
        assertThat(houseJson.encodeToString(PatchFamilyRequest.greetingPlaces(listOf("Moscow", "Belgrade"))))
            .isEqualTo("""{"greeting_places":["Moscow","Belgrade"]}""")
        // `[]` is not the Kotlin default, so it is SENT — never dropped,
        // and never a null (the server refuses null).
        assertThat(houseJson.encodeToString(PatchFamilyRequest.greetingPlaces(emptyList())))
            .isEqualTo("""{"greeting_places":[]}""")
        // …and no other write carries it.
        assertThat(houseJson.encodeToString(PatchFamilyRequest.aiGreeting(true)))
            .doesNotContain("greeting_places")
        assertThat(houseJson.encodeToString(PatchFamilyRequest.aiLookups(true)))
            .doesNotContain("greeting_places")
        assertThat(houseJson.encodeToString(PatchFamilyRequest.language(null)))
            .doesNotContain("greeting_places")
    }

    // -- The owner's screen ----------------------------------------------------------

    @Test
    fun `the screen reads the places and whether the server can use them`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = weatherServer, places = listOf("Moscow", "Belgrade"))
        val viewModel = viewModel()
        runCurrent()

        val state = viewModel.state.value
        assertThat(state.greetingWeather).isTrue()
        assertThat(state.greetingPlaces).containsExactly("Moscow", "Belgrade").inOrder()
        assertThat(state.placeFields).containsExactly("Moscow", "Belgrade").inOrder()
        assertThat(state.placesChanged).isFalse()
        assertThat(state.placesSavable).isFalse()
    }

    @Test
    fun `no greeting weather on the server, or no assistant, hides the field`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = weatherServer.copy(greetingWeather = false), places = listOf("Moscow"))
        val viewModel = viewModel()
        runCurrent()
        assertThat(viewModel.state.value.greetingWeather).isFalse()

        familyApi.mineResult = mine(assistant = null, places = listOf("Moscow"))
        viewModel.load()
        runCurrent()
        assertThat(viewModel.state.value.greetingWeather).isFalse()
    }

    @Test
    fun `with no places the editor opens on one empty field`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = weatherServer, places = emptyList())
        val viewModel = viewModel()
        runCurrent()

        assertThat(viewModel.state.value.placeFields).containsExactly("")
        assertThat(viewModel.state.value.placesSavable).isFalse()
    }

    @Test
    fun `fields are added up to three and removed`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = weatherServer, places = listOf("Moscow"))
        val viewModel = viewModel()
        runCurrent()

        viewModel.addPlaceField()
        viewModel.addPlaceField()
        viewModel.addPlaceField() // a fourth is refused
        assertThat(viewModel.state.value.placeFields).containsExactly("Moscow", "", "").inOrder()

        viewModel.removePlaceField(0)
        assertThat(viewModel.state.value.placeFields).containsExactly("", "").inOrder()
        viewModel.removePlaceField(5) // out of range: nothing
        assertThat(viewModel.state.value.placeFields).hasSize(2)
        // Removing Moscow is a change worth saving.
        assertThat(viewModel.state.value.placesSavable).isTrue()
    }

    @Test
    fun `typing is held to the server's rules`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = weatherServer, places = emptyList())
        val viewModel = viewModel()
        runCurrent()

        viewModel.editPlaceField(0, "Bel\u0000grade" + "x".repeat(100))
        val typed = viewModel.state.value.placeFields.single()
        assertThat(typed).startsWith("Belgrade")
        assertThat(GreetingPlaces.charCount(typed)).isEqualTo(GreetingPlaces.MAX_NAME_CHARS)
    }

    @Test
    fun `save sends the places as the server would keep them, and draws the answer`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = weatherServer, places = emptyList())
        val viewModel = viewModel()
        runCurrent()
        viewModel.editPlaceField(0, "  Moscow ")
        viewModel.addPlaceField()
        viewModel.editPlaceField(1, "moscow")
        viewModel.addPlaceField()
        viewModel.editPlaceField(2, "Novi   Sad")
        // The server answers with its own spelling of the list.
        familyApi.createResult = answer(listOf("Moscow", "Novi Sad"))

        assertThat(viewModel.state.value.placesSavable).isTrue()
        viewModel.saveGreetingPlaces()
        runCurrent()

        assertThat(familyApi.greetingPlacesSet).containsExactly(listOf("Moscow", "Novi Sad"))
        val state = viewModel.state.value
        assertThat(state.busy).isFalse()
        assertThat(state.error).isNull()
        assertThat(state.greetingPlaces).containsExactly("Moscow", "Novi Sad").inOrder()
        assertThat(state.placeFields).containsExactly("Moscow", "Novi Sad").inOrder()
        assertThat(state.placesChanged).isFalse()
    }

    @Test
    fun `clearing every field sends an empty list`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = weatherServer, places = listOf("Moscow"))
        val viewModel = viewModel()
        runCurrent()
        viewModel.editPlaceField(0, "")
        familyApi.createResult = answer(emptyList())

        viewModel.saveGreetingPlaces()
        runCurrent()

        assertThat(familyApi.greetingPlacesSet).containsExactly(emptyList<String>())
        assertThat(viewModel.state.value.greetingPlaces).isEmpty()
        assertThat(viewModel.state.value.placeFields).containsExactly("")
    }

    @Test
    fun `a server that ignores the key is believed, not what was sent`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = weatherServer, places = emptyList())
        val viewModel = viewModel()
        runCurrent()
        viewModel.editPlaceField(0, "Moscow")
        // An older server answers without the field at all.
        familyApi.createResult = answer(null)

        viewModel.saveGreetingPlaces()
        runCurrent()

        assertThat(viewModel.state.value.greetingPlaces).isEmpty()
        assertThat(viewModel.state.value.placeFields).containsExactly("")
    }

    @Test
    fun `a refused write keeps the stored list and the owner's typing`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = weatherServer, places = listOf("Moscow"))
        val viewModel = viewModel()
        runCurrent()
        viewModel.editPlaceField(0, "Belgrade")
        familyApi.createResult = ApiResult.HttpError(400, "validation", "greeting place 1 is longer than 80 characters")

        viewModel.saveGreetingPlaces()
        runCurrent()

        val state = viewModel.state.value
        assertThat(state.greetingPlaces).containsExactly("Moscow")
        assertThat(state.placeFields).containsExactly("Belgrade")
        assertThat(state.error).isEqualTo("greeting place 1 is longer than 80 characters")
        assertThat(state.busy).isFalse()
    }

    @Test
    fun `an unreachable server says so and changes nothing`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = weatherServer, places = listOf("Moscow"))
        val viewModel = viewModel()
        runCurrent()
        viewModel.editPlaceField(0, "Belgrade")
        familyApi.createResult = ApiResult.NetworkError(java.io.IOException("offline"))

        viewModel.saveGreetingPlaces()
        runCurrent()

        assertThat(viewModel.state.value.greetingPlaces).containsExactly("Moscow")
        assertThat(viewModel.state.value.placeFields).containsExactly("Belgrade")
        assertThat(viewModel.state.value.error).isNotNull()
    }

    @Test
    fun `a reload keeps unsaved typing but follows the server otherwise`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = weatherServer, places = listOf("Moscow"))
        val viewModel = viewModel()
        runCurrent()

        // Not edited: a reload brings the other device's change.
        familyApi.mineResult = mine(assistant = weatherServer, places = listOf("Paris"))
        viewModel.load()
        runCurrent()
        assertThat(viewModel.state.value.placeFields).containsExactly("Paris")

        // Edited: a reload updates what is stored but keeps the typing.
        viewModel.editPlaceField(0, "Belgr")
        familyApi.mineResult = mine(assistant = weatherServer, places = listOf("Tokyo"))
        viewModel.load()
        runCurrent()
        assertThat(viewModel.state.value.greetingPlaces).containsExactly("Tokyo")
        assertThat(viewModel.state.value.placeFields).containsExactly("Belgr")
    }

    @Test
    fun `the owner is shown the field and a member is not`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = weatherServer, places = listOf("Moscow"))
        val viewModel = viewModel()
        // isOwner is shared while subscribed; the screen subscribes, so must this.
        backgroundScope.launch { viewModel.isOwner.collect {} }
        runCurrent()
        assertThat(
            GreetingPlaces.isShown(isOwner = viewModel.isOwner.value, greetingWeather = viewModel.state.value.greetingWeather),
        ).isTrue()

        settings.setFamilyStatus(FamilyStatus.MEMBER)
        runCurrent()
        assertThat(viewModel.isOwner.value).isFalse()
        assertThat(
            GreetingPlaces.isShown(isOwner = viewModel.isOwner.value, greetingWeather = viewModel.state.value.greetingWeather),
        ).isFalse()
    }
}
