package me.nettrash.familyconnect.ui.chat

import android.graphics.Bitmap
import android.graphics.Color
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import me.nettrash.familyconnect.rich.RichEngine
import me.nettrash.familyconnect.rich.RichJob
import me.nettrash.familyconnect.rich.RichRenderer
import me.nettrash.familyconnect.rich.RichResult
import org.junit.After
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.GraphicsMode
import java.util.concurrent.CopyOnWriteArrayList

/**
 * A rich block in the bubble (#97), with a fake engine behind the real
 * renderer: the source shows, monospaced, until there is a drawing and for
 * good when there is none; the drawing replaces it with the source as its
 * label; and the job the bubble sends carries the bubble's ink for a
 * formula and the fixed card for a diagram.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class RichBlocksTest {

    @get:Rule
    val compose = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    @After
    fun tearDown() = scope.cancel()

    private class Engine(private val draws: Boolean) : RichEngine {
        val jobs = CopyOnWriteArrayList<RichJob>()
        override suspend fun render(job: RichJob): RichResult {
            jobs += job
            if (!draws) return RichResult.Failed
            val bitmap = Bitmap.createBitmap(40, 20, Bitmap.Config.ARGB_8888)
            bitmap.eraseColor(Color.BLACK)
            return RichResult.Drawn(bitmap, width = 40f, height = 20f, baseline = 15f)
        }
    }

    private fun renderer(engine: RichEngine) = RichRenderer(
        engine = engine,
        disk = null,
        versionSource = { "test" },
        scope = scope,
    )

    private val display = MessageMarkdown.blocks("\$\$\nx^2\n\$\$").single() as MessageMarkdown.Block.Rich
    private val diagram = MessageMarkdown.blocks("```mermaid\ngraph TD\nA-->B\n```").single() as MessageMarkdown.Block.Rich

    @Test
    fun aBlockThatCannotBeDrawnShowsItsSourceAsTyped() {
        val engine = Engine(draws = false)
        compose.setContent {
            CompositionLocalProvider(LocalRichRenderer provides renderer(engine)) {
                RichBlockView(block = display, onDoubleTap = null, onLongPress = {})
            }
        }
        compose.waitUntil(5_000) { engine.jobs.isNotEmpty() }
        compose.waitForIdle()
        // Delimiters and all.
        compose.onNodeWithText("\$\$\nx^2\n\$\$").assertExists()
        compose.onAllNodesWithContentDescription("x^2").assertCountEquals(0)
    }

    @Test
    fun aDrawnBlockIsAPictureLabelledWithItsSource() {
        val engine = Engine(draws = true)
        compose.setContent {
            CompositionLocalProvider(LocalRichRenderer provides renderer(engine)) {
                RichBlockView(block = diagram, onDoubleTap = null, onLongPress = {})
            }
        }
        compose.waitUntil(5_000) {
            compose.onAllNodesWithContentDescription("graph TD\nA-->B").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithContentDescription("graph TD\nA-->B").assertExists()
        compose.onAllNodesWithText("graph TD\nA-->B").assertCountEquals(0)
        // A diagram goes on the fixed card, whatever the bubble's ink.
        val job = engine.jobs.single()
        assertThat(job.kind).isEqualTo("mermaid")
        assertThat(job.background).isEqualTo(RichJob.CARD_LIGHT)
        assertThat(job.color).isEqualTo(RichJob.CARD_INK_LIGHT)
    }

    @Test
    fun withNoRendererEveryBlockShowsItsSource() {
        compose.setContent { RichBlockView(block = diagram, onDoubleTap = null, onLongPress = {}) }
        compose.onNodeWithText("graph TD\nA-->B").assertExists()
    }

    @Test
    fun inlineFormulasGainContentOnceDrawn() {
        val engine = Engine(draws = true)
        val rendered = (MessageMarkdown.blocks("area \$r^2\$ and \$r^2\$ again").single() as MessageMarkdown.Block.Text)
            .rendered
        var map: Map<String, InlineTextContent> = emptyMap()
        compose.setContent {
            CompositionLocalProvider(LocalRichRenderer provides renderer(engine)) {
                map = rememberInlineMath(rendered.annotated)
                BasicText(text = rendered.annotated, inlineContent = map)
            }
        }
        compose.waitUntil(5_000) { map.isNotEmpty() }
        assertThat(map.keys).containsExactly(MessageMarkdown.MATH_ID_PREFIX + "r^2")
        // One formula written twice is one job.
        assertThat(engine.jobs.map { it.source }).containsExactly("r^2")
        assertThat(engine.jobs.single().kind).isEqualTo("math-inline")
        assertThat(engine.jobs.single().background).isEqualTo("transparent")
    }

    @Test
    fun textWithNoFormulaAsksForNothing() {
        val engine = Engine(draws = true)
        val rendered = (MessageMarkdown.blocks("it costs \$5").single() as MessageMarkdown.Block.Text).rendered
        var map: Map<String, InlineTextContent>? = null
        compose.setContent {
            CompositionLocalProvider(LocalRichRenderer provides renderer(engine)) {
                map = rememberInlineMath(rendered.annotated)
            }
        }
        compose.waitForIdle()
        assertThat(map).isEmpty()
        assertThat(engine.jobs).isEmpty()
    }
}
