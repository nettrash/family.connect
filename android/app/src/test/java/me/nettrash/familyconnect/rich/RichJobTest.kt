package me.nettrash.familyconnect.rich

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.float
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import me.nettrash.familyconnect.ui.chat.MessageMarkdown
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The pure half of drawing a rich block (#97): the job the page is sent,
 * the key the drawing is kept under, and the snapshot arithmetic. None of
 * it needs a WebView, and all of it is what decides whether a drawing is
 * shown for the right message — a key that left a field out would show one
 * bubble's formula in another's colour, silently.
 */
class RichJobTest {

    private val version = "1;katex-0.17.0;mermaid-11.16.0;viz-3.24.0;plot-md-1.6"

    private fun rich(kind: MessageMarkdown.RichKind, source: String, engine: String? = null) =
        MessageMarkdown.Block.Rich(kind, engine, source, shown = source)

    private val inkMine = 0xFF0B1D3A.toInt()
    private val inkTheirs = 0xFF1B1B1F.toInt()

    @Test
    fun `a formula is the bubble's ink on nothing, a diagram the fixed card`() {
        val formula = RichJob.forBlock(rich(MessageMarkdown.RichKind.MATH, "x^2"), dark = false, ink = inkMine, fontSize = 14f)
        assertEquals("math", formula.kind)
        assertEquals("#0B1D3A", formula.color)
        assertEquals("transparent", formula.background)
        assertTrue(formula.isMath)
        assertFalse(formula.isCard)

        val light = RichJob.forBlock(rich(MessageMarkdown.RichKind.MERMAID, "graph TD"), false, inkMine, 14f)
        assertEquals("#1C1C1E", light.color)
        assertEquals("#FFFFFF", light.background)
        assertTrue(light.isCard)
        val dark = RichJob.forBlock(rich(MessageMarkdown.RichKind.PLOT, "sin(x)"), true, inkMine, 14f)
        assertEquals("#F2F2F7", dark.color)
        assertEquals("#1C1C1E", dark.background)
        // The bubble's tint never reaches a card: mine and theirs draw the
        // same diagram the same way, and share one drawing.
        assertEquals(
            RichJob.forBlock(rich(MessageMarkdown.RichKind.MERMAID, "A"), false, inkMine, 14f),
            RichJob.forBlock(rich(MessageMarkdown.RichKind.MERMAID, "A"), false, inkTheirs, 14f),
        )

        val graph = RichJob.forBlock(rich(MessageMarkdown.RichKind.GRAPHVIZ, "graph{a--b}", "neato"), false, inkMine, 14f)
        assertEquals("graphviz", graph.kind)
        assertEquals("neato", graph.engine)

        val inline = RichJob.forInline("a*b", dark = true, ink = inkTheirs, fontSize = 16.1f)
        assertEquals("math-inline", inline.kind)
        assertEquals("transparent", inline.background)
        assertTrue(inline.isMath)
    }

    @Test
    fun `the job's JSON is fc-rich's JOB`() {
        val job = RichJob.forBlock(
            rich(MessageMarkdown.RichKind.GRAPHVIZ, "digraph { \"a\" -> b }\n</script>", "circo"),
            dark = true,
            ink = inkMine,
            fontSize = 15.5f,
        )
        val json = Json.parseToJsonElement(job.toJson()).jsonObject
        assertEquals("graphviz", json["kind"]!!.jsonPrimitive.content)
        // The source survives exactly — quotes, newlines and all.
        assertEquals("digraph { \"a\" -> b }\n</script>", json["source"]!!.jsonPrimitive.content)
        assertEquals("circo", json["engine"]!!.jsonPrimitive.content)
        assertTrue(json["dark"]!!.jsonPrimitive.boolean)
        assertEquals("#F2F2F7", json["color"]!!.jsonPrimitive.content)
        assertEquals("#1C1C1E", json["background"]!!.jsonPrimitive.content)
        assertEquals(15.5f, json["fontSize"]!!.jsonPrimitive.float)
        // No engine is no field, rather than a null the page must handle.
        val math = Json.parseToJsonElement(
            RichJob.forInline("x", false, inkMine, 14f).toJson(),
        ).jsonObject
        assertNull(math["engine"])
        assertEquals(setOf("kind", "source", "dark", "color", "background", "fontSize"), math.keys)
    }

    @Test
    fun `the key covers every field and the version`() {
        val base = RichJob(
            kind = "mermaid", source = "graph TD\nA-->B", engine = null, dark = false,
            color = "#1C1C1E", background = "#FFFFFF", fontSize = 14f,
        )
        val key = base.cacheKey(version)
        assertEquals(64, key.length)
        assertTrue(key.all { it in '0'..'9' || it in 'a'..'f' })
        // Stable: the same job is the same key, today and on the next launch.
        assertEquals(key, base.copy().cacheKey(version))
        val variants = listOf(
            base.copy(kind = "plot"),
            base.copy(engine = "dot"),
            base.copy(dark = true),
            base.copy(color = "#F2F2F7"),
            base.copy(background = "transparent"),
            base.copy(fontSize = 14.5f),
            base.copy(source = "graph TD\nA-->C"),
        )
        for (variant in variants) assertNotEquals(variant.toString(), key, variant.cacheKey(version))
        // A new engine version makes every old drawing unreachable.
        assertNotEquals(key, base.cacheKey("2;katex-0.18.0"))
        // NUL-joined with the source last: moving characters between fields
        // cannot make two jobs collide.
        assertNotEquals(
            base.copy(color = "a", background = "b").cacheKey(version),
            base.copy(color = "a\u0000b", background = "").cacheKey(version),
        )
        // Non-ASCII sources key by their UTF-8 bytes, distinctly.
        assertNotEquals(
            base.copy(source = "Привет").cacheKey(version),
            base.copy(source = "Привёт").cacheKey(version),
        )
    }

    @Test
    fun `colours are CSS, and font sizes round to a hundredth`() {
        assertEquals("#FFFFFF", RichJob.cssColor(0xFFFFFFFF.toInt()))
        assertEquals("#000000", RichJob.cssColor(0xFF000000.toInt()))
        assertEquals("#1C1C1E", RichJob.cssColor(0xFF1C1C1E.toInt()))
        assertEquals("rgba(255,0,0,0.502)", RichJob.cssColor(0x80FF0000.toInt()))
        assertEquals(16.1f, RichJob.roundedFontSize(14f * 1.15f))
        assertEquals(
            RichJob.forInline("x", false, inkMine, 16.099999f).cacheKey(version),
            RichJob.forInline("x", false, inkMine, 16.1f).cacheKey(version),
        )
    }

    @Test
    fun `a snapshot is at the screen's density, capped at 4096 on the longest side`() {
        // Small: exactly the density, rounded up so no edge is lost.
        assertEquals(RichCapture.Size(2.75f, 83, 55), RichCapture.size(30f, 20f, 2.75f))
        // Exactly at the cap stays at the density.
        assertEquals(RichCapture.Size(2f, 4096, 1024), RichCapture.size(2048f, 512f, 2f))
        // Tall: the longest side becomes 4096 and the other keeps the ratio.
        val tall = RichCapture.size(1200f, 3000f, 3f)
        assertEquals(4096, tall.height)
        assertEquals(1639, tall.width)
        assertEquals(4096f / 3000f, tall.scale, 1e-6f)
        // Wide, too, and never past the cap from rounding.
        val wide = RichCapture.size(4097.3f, 10f, 1f)
        assertEquals(4096, wide.width)
        assertTrue(wide.height in 1..10)
        // Never zero: a hairline still gets a pixel.
        assertEquals(1, RichCapture.size(10f, 0.01f, 1f).height)
    }

    @Test
    fun `the version is read from fc-rich's own constant`() {
        val header = "(function () {\n  'use strict';\n\n  var VERSION = '$version';\n"
        assertEquals(version, RichAssets.versionIn(header))
        assertNull(RichAssets.versionIn("var VERSIONS = 1"))
    }
}
