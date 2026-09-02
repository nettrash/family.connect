/*
 * LinkPreviewFixtures.kt
 * Family Connect (Android)
 *
 * Real pages, kept as bytes, for the link-preview tests.
 *
 * resources/fixtures/youtube-watch.html is a genuine capture of
 * https://www.youtube.com/watch?v=dQw4w9WgXcQ, fetched with exactly the
 * headers LinkPreviewRepository sends (its own User-Agent, Accept:
 * text/html) — markup, entities, Cyrillic locale attributes and all.
 *
 * ONE thing was changed, and it is the reason the file is 12KB rather
 * than 1.3MB: the ~700KB of inline player JSON that YouTube puts BEFORE
 * its og: tags is replaced by a marker comment naming its exact byte
 * count. `youTubeWatchPage()` swaps that marker back for a filler script
 * of precisely that size, so every tag in the page ends up at the offset
 * it really has — <title> at 704,923, og:title at 706,842, </head> at
 * 715,108. Those offsets ARE the bug in #50: they sit far past the 256K
 * the fetcher used to read and the 200K the parser used to scan, so the
 * page yielded no card at all.
 *
 * iOS mirror: FamilyConnectTests/Fixtures/youtube-watch.html with
 * LinkPreviewFixtures.swift. The two fixture files are byte-identical.
 */

package me.nettrash.familyconnect.data.net

object LinkPreviewFixtures {

    /** Real offsets in the captured page, as measured on the live one. */
    const val TITLE_TAG_OFFSET = 704_923
    const val OG_TITLE_OFFSET = 706_842
    const val HEAD_END_OFFSET = 715_108
    const val TITLE = "Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)"
    const val IMAGE_URL = "https://i.ytimg.com/vi/dQw4w9WgXcQ/maxresdefault.jpg"

    private const val MARKER_PREFIX = "<!--FAMILY-CONNECT-FIXTURE: "

    /** The captured YouTube watch page, restored to its real size. */
    fun youTubeWatchPage(): String = reinflated("fixtures/youtube-watch.html")

    private fun reinflated(path: String): String {
        val raw = requireNotNull(javaClass.classLoader?.getResourceAsStream(path)) {
            "fixture $path is missing from the test resources"
        }.use { it.readBytes().toString(Charsets.UTF_8) }

        val open = raw.indexOf(MARKER_PREFIX)
        require(open >= 0) { "fixture $path has no elision marker" }
        val close = raw.indexOf("-->", open)
        require(close >= 0) { "fixture $path has an unterminated elision marker" }
        val count = raw.substring(open + MARKER_PREFIX.length)
            .takeWhile { it.isDigit() }
            .toInt()
        // A <script> rather than bare filler, because that is what really
        // sits there — and the parser must ignore it either way. The
        // wrapper is 31 bytes, so the padding makes up the rest.
        val script = "<script>var elided=\"" + "a".repeat(count - 31) + "\";</script>"
        require(script.length == count) { "filler must restore the exact elided length" }
        return raw.substring(0, open) + script + raw.substring(close + 3)
    }
}
