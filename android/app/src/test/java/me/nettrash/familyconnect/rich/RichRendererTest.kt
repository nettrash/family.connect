package me.nettrash.familyconnect.rich

import android.graphics.Bitmap
import android.graphics.Color
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.GraphicsMode
import java.util.concurrent.atomic.AtomicInteger

/**
 * The renderer's rules, with a fake engine in place of the web view (which
 * Robolectric cannot run): a drawing is made once and kept in memory and on
 * disk, a failure is kept in memory only, one key is one job however many
 * bubbles ask, and a blank source never wakes the engine.
 *
 * NATIVE graphics, so the disk tier really writes and decodes a PNG — and
 * so its alpha is really checked, which is the point of PNG here.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class RichRendererTest {

    @get:Rule
    val folder = TemporaryFolder()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    @After
    fun tearDown() = scope.cancel()

    private val job = RichJob(
        kind = "math-inline", source = "x^2", dark = false,
        color = "#000000", background = "transparent", fontSize = 14f,
    )

    /** A 4 × 3 drawing: one opaque pixel on a transparent ground. */
    private fun drawing(): RichResult.Drawn {
        val bitmap = Bitmap.createBitmap(4, 3, Bitmap.Config.ARGB_8888)
        bitmap.setPixel(1, 1, Color.RED)
        return RichResult.Drawn(bitmap, width = 4f, height = 3f, baseline = 2.5f)
    }

    private class FakeEngine(private val answer: suspend () -> RichResult) : RichEngine {
        val calls = AtomicInteger()
        override suspend fun render(job: RichJob): RichResult {
            calls.incrementAndGet()
            return answer()
        }
    }

    private fun renderer(engine: RichEngine, version: String = "v1") = RichRenderer(
        engine = engine,
        disk = RichDiskCache(folder.root),
        versionSource = { version },
        scope = scope,
    )

    @Test
    fun aDrawingIsMadeOnceAndKept() = runBlocking {
        val engine = FakeEngine { drawing() }
        val renderer = renderer(engine)
        assertNull("nothing is known before the first draw", renderer.cached(job))
        val first = renderer.draw(job)
        assertTrue(first is RichResult.Drawn)
        assertSame(first, renderer.draw(job))
        assertSame(first, renderer.cached(job))
        assertEquals(1, engine.calls.get())
    }

    @Test
    fun theDiskKeepsTheDrawingAcrossLaunches() = runBlocking {
        renderer(FakeEngine { drawing() }).draw(job)
        // A new launch: an empty memory, the same cache folder.
        val engine = FakeEngine { error("must not draw again") }
        val again = renderer(engine).draw(job) as RichResult.Drawn
        assertEquals(0, engine.calls.get())
        assertEquals(4f, again.width)
        assertEquals(3f, again.height)
        assertEquals(2.5f, again.baseline)
        // The PNG kept the alpha: the ground is still transparent and the
        // ink still opaque.
        assertEquals(0, Color.alpha(again.bitmap.getPixel(0, 0)))
        assertEquals(Color.RED, again.bitmap.getPixel(1, 1))
    }

    @Test
    fun aFailureIsRememberedInMemoryOnly() = runBlocking {
        val engine = FakeEngine { RichResult.Failed }
        val renderer = renderer(engine)
        assertSame(RichResult.Failed, renderer.draw(job))
        assertSame(RichResult.Failed, renderer.draw(job))
        assertSame(RichResult.Failed, renderer.cached(job))
        assertEquals(1, engine.calls.get())
        // The next launch tries again — a newer engine, or a renderer that
        // is not out of memory this time, may manage it.
        val next = FakeEngine { drawing() }
        assertTrue(renderer(next).draw(job) is RichResult.Drawn)
        assertEquals(1, next.calls.get())
        assertTrue(folder.root.listFiles().orEmpty().any { it.name.endsWith(".png") })
    }

    @Test
    fun oneKeyIsOneJobHoweverManyAsk() = runBlocking {
        val gate = CompletableDeferred<Unit>()
        val engine = FakeEngine {
            gate.await()
            drawing()
        }
        val renderer = renderer(engine)
        val askers = List(8) { scope.async { renderer.draw(job) } }
        gate.complete(Unit)
        val results = askers.awaitAll()
        assertEquals(1, engine.calls.get())
        assertTrue(results.all { it === results.first() })
    }

    @Test
    fun aBlankSourceNeverWakesTheEngine() = runBlocking {
        val engine = FakeEngine { drawing() }
        assertSame(RichResult.Failed, renderer(engine).draw(job.copy(source = " \n\t")))
        assertEquals(0, engine.calls.get())
    }

    @Test
    fun aNewEngineVersionDrawsAgain() = runBlocking {
        renderer(FakeEngine { drawing() }, version = "v1").draw(job)
        val engine = FakeEngine { drawing() }
        renderer(engine, version = "v2").draw(job)
        assertEquals(1, engine.calls.get())
    }

    @Test
    fun anEngineThatThrowsIsAFailureNotACrash() = runBlocking {
        val engine = FakeEngine { throw IllegalStateException("boom") }
        assertSame(RichResult.Failed, renderer(engine).draw(job))
    }
}
