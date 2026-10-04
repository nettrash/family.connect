/*
 * GreetingPlaces.kt
 * Family Connect (Android)
 *
 * The owner's "Weather in the greeting" places, as rules rather than as
 * checks spread through a screen (docs/protocol.md, "Today's weather, for
 * places the owner chose"): when the field is drawn, how a typed name is
 * kept, and what a Save may send.
 *
 * The server is the authority — it trims, folds, de-duplicates and counts
 * again, and its answer is what the screen shows. These rules MIRROR
 * `validate_greeting_places` in server/src/handlers_family.rs so that a
 * Save this client offers is one the server accepts, and so the screen can
 * stop a name at 80 characters instead of letting the owner type a refusal.
 * Mirrored exactly where "the same" has more than one answer between Rust
 * and the JVM:
 *
 * - **whitespace** is Unicode `White_Space`, Rust's `split_whitespace`.
 *   Not Kotlin's `isWhitespace`, which also counts U+001C..U+001F and
 *   misses U+0085 — four control characters the server would refuse that
 *   this would have folded away, and one whitespace it would have kept;
 * - **a control character** is general category Cc, Rust's
 *   `char::is_control` and the JVM's `isISOControl` alike;
 * - **a character** is a code point, as Rust counts `chars()`: a name of
 *   80 emoji is 160 UTF-16 units and still 80 characters;
 * - **a repeat** is the same name once both are lower-cased, keeping the
 *   first spelling.
 *
 * Kept free of Compose and Android so a plain JUnit test pins it.
 */

package me.nettrash.familyconnect.util

object GreetingPlaces {

    /** The most places a family keeps, counted after repeats are dropped. */
    const val MAX_PLACES = 3

    /** The longest name, in characters (code points), after folding. */
    const val MAX_NAME_CHARS = 80

    /**
     * Is the places field drawn? Only for the OWNER — nobody else can set
     * it, and the screen shows no other member the owner's AI settings —
     * and only where the server says greeting weather is available
     * (`assistant.greeting_weather`; absent on an older server, read as
     * false). Elsewhere the list would be kept and do nothing, and the
     * footnote's promise about what is sent would be about nothing.
     *
     * Deliberately NOT gated on the family's own greeting switch: the
     * server accepts the list while the greeting is off, and an owner may
     * well choose the places before turning the greeting on.
     */
    fun isShown(isOwner: Boolean, greetingWeather: Boolean): Boolean = isOwner && greetingWeather

    /** May another place field be added? */
    fun canAdd(fieldCount: Int): Boolean = fieldCount < MAX_PLACES

    /**
     * The fields the editor opens with: the stored places, or one empty
     * field to type into when there are none.
     */
    fun fieldsFor(stored: List<String>): List<String> = stored.take(MAX_PLACES).ifEmpty { listOf("") }

    /** Unicode `White_Space` — exactly the set Rust's `char::is_whitespace` uses. */
    fun isWhiteSpace(codePoint: Int): Boolean = when (codePoint) {
        in 0x09..0x0D, 0x20, 0x85, 0xA0, 0x1680, in 0x2000..0x200A,
        0x2028, 0x2029, 0x202F, 0x205F, 0x3000,
        -> true
        else -> false
    }

    /** General category Cc, as Rust's `char::is_control`. */
    fun isControl(codePoint: Int): Boolean = Character.isISOControl(codePoint)

    /** Trimmed, with every run of whitespace inside folded to one space. */
    fun normalize(raw: String): String {
        val out = StringBuilder(raw.length)
        var pendingSpace = false
        var i = 0
        while (i < raw.length) {
            val cp = raw.codePointAt(i)
            i += Character.charCount(cp)
            if (isWhiteSpace(cp)) {
                if (out.isNotEmpty()) pendingSpace = true
            } else {
                if (pendingSpace) out.append(' ')
                pendingSpace = false
                out.appendCodePoint(cp)
            }
        }
        return out.toString()
    }

    /** Characters as the server counts them: code points. */
    fun charCount(name: String): Int = name.codePointCount(0, name.length)

    /**
     * What a place field keeps of what was typed or pasted: control
     * characters that are not whitespace are dropped (the server refuses
     * them, and nobody means to type one), and the text stops where its
     * FOLDED form would pass [MAX_NAME_CHARS] — so leading, trailing and
     * doubled spaces, which the server folds away, never cost a character.
     */
    fun limitInput(raw: String): String {
        val out = StringBuilder(raw.length)
        var count = 0
        var pendingSpace = false
        var i = 0
        while (i < raw.length) {
            val cp = raw.codePointAt(i)
            i += Character.charCount(cp)
            when {
                isWhiteSpace(cp) -> {
                    if (count > 0) pendingSpace = true
                    out.appendCodePoint(cp)
                }
                isControl(cp) -> Unit
                else -> {
                    val needed = if (pendingSpace) 2 else 1
                    if (count + needed > MAX_NAME_CHARS) break
                    count += needed
                    pendingSpace = false
                    out.appendCodePoint(cp)
                }
            }
        }
        return out.toString()
    }

    /** Why a set of fields cannot be saved as it stands. */
    enum class Problem {
        /** A name holds a control character. */
        CONTROL_CHARACTER,

        /** A name is longer than [MAX_NAME_CHARS] characters once folded. */
        TOO_LONG,

        /** More than [MAX_PLACES] different names. */
        TOO_MANY,
    }

    /** The outcome of [prepare]. */
    sealed interface Prepared {
        /** The list to send, already as the server would keep it. */
        data class Ok(val places: List<String>) : Prepared

        /** Nothing can be sent; [position] is the field's 1-based index, or 0 for the count. */
        data class Refused(val problem: Problem, val position: Int) : Prepared
    }

    /**
     * The list a Save sends for these [fields], or why there is none.
     *
     * A field left blank is not a place — it is a field the owner opened
     * and did not fill — so it is dropped rather than sent: the server
     * would refuse an empty name, and refusing the whole Save over an
     * empty row would be a nuisance. Every other rule is the server's, in
     * its order: fold, refuse a control character, refuse a long name,
     * drop a repeat, then count.
     */
    fun prepare(fields: List<String>): Prepared {
        val kept = mutableListOf<String>()
        val seen = mutableSetOf<String>()
        fields.forEachIndexed { index, raw ->
            val name = normalize(raw)
            if (name.isEmpty()) return@forEachIndexed
            if (name.codePoints().anyMatch(::isControl)) {
                return Prepared.Refused(Problem.CONTROL_CHARACTER, index + 1)
            }
            if (charCount(name) > MAX_NAME_CHARS) return Prepared.Refused(Problem.TOO_LONG, index + 1)
            if (seen.add(name.lowercase())) kept += name
        }
        if (kept.size > MAX_PLACES) return Prepared.Refused(Problem.TOO_MANY, 0)
        return Prepared.Ok(kept)
    }

    /**
     * Would a Save change anything? True when [fields] cannot be saved at
     * all (the owner must see that something is pending) or when what they
     * would send differs from what the server [stored].
     */
    fun isChanged(fields: List<String>, stored: List<String>): Boolean =
        when (val prepared = prepare(fields)) {
            is Prepared.Ok -> prepared.places != stored
            is Prepared.Refused -> true
        }
}
