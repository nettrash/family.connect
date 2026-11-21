package me.nettrash.familyconnect.ui.chat

import me.nettrash.familyconnect.util.EmojiTyping
import me.nettrash.familyconnect.util.reactionEmojiKey
import java.util.Locale

/** Android's implementation of the shared Web search rules in emoji_search.rs. */
internal object ReactionSearch {
    private val catalogue = EMOJI_CATALOG.flatMap { it.emoji }.distinct()
    private val canonical = catalogue.associateBy(::reactionEmojiKey)
    private val aliases by lazy {
        EmojiTyping.shortcodes.entries
            .mapNotNull { (name, emoji) ->
                canonical[reactionEmojiKey(emoji)]?.let { it to normalize(name) }
            }
            .groupBy({ it.first }, { it.second })
    }
    private val whitespace = Regex("[\\s\\p{Z}\\u0085]+")
    private val separators = Regex("[_\\-:|]")

    fun normalize(query: String): String = query.lowercase(Locale.ROOT)
        .replace("ß", "ss")
        .replace(separators, " ")
        .replace(whitespace, " ")
        .trim()

    fun localeTag(locales: Iterable<Locale>): String {
        for (locale in locales) {
            when (locale.language) {
                "en", "de", "es", "fr", "ja", "ru" -> return locale.language
                "sr", "sh" -> return if (locale.script == "Latn") "sr-Latn" else "sr"
                "hr", "bs" -> return "sr-Latn"
                "zh" -> return "zh-Hans"
            }
        }
        return "en"
    }

    private data class Annotation(val name: String, val normalizedName: String, val terms: String)

    private fun readAnnotations(tag: String): Map<String, Annotation>? {
        val stream = ReactionSearch::class.java.getResourceAsStream("/emoji-search/$tag.tsv") ?: return null
        return stream.bufferedReader(Charsets.UTF_8).useLines { lines ->
            val annotations = mutableMapOf<String, Annotation>()
            for (line in lines.filterNot { it.startsWith('#') }) {
                val fields = line.split('\t')
                if (fields.size != 4) return@useLines null
                val (locale, emoji, name, keywords) = fields
                if (locale != tag || canonical[reactionEmojiKey(emoji)] != emoji ||
                    name.isEmpty() || keywords.isEmpty() || emoji in annotations
                ) return@useLines null
                annotations[emoji] = Annotation(name, normalize(name), normalize("$name $keywords"))
            }
            annotations.takeIf { it.size == catalogue.size }
        }
    }

    private val english by lazy { readAnnotations("en").orEmpty() }

    class Index(tag: String) {
        private val local = if (tag == "en") english else readAnnotations(tag) ?: english

        fun name(emoji: String): String = local[emoji]?.name ?: emoji

        fun search(rawQuery: String): List<String> {
            val query = normalize(rawQuery)
            if (query.isEmpty()) return emptyList()
            val words = query.split(' ')
            val queryKey = reactionEmojiKey(query)
            // One bucket per rank, so ties stay in catalogue order.
            val exact = mutableListOf<String>()
            val prefixes = mutableListOf<String>()
            val keywords = mutableListOf<String>()
            for (emoji in catalogue) {
                val localized = local[emoji]
                val fallback = english[emoji]
                val shortcodes = aliases[emoji].orEmpty()
                when {
                    emoji == canonical[queryKey] || localized?.normalizedName == query ||
                        fallback?.normalizedName == query || query in shortcodes -> exact.add(emoji)
                    localized?.normalizedName?.startsWith(query) == true ||
                        fallback?.normalizedName?.startsWith(query) == true -> prefixes.add(emoji)
                    words.all { word ->
                        localized?.terms?.contains(word) == true ||
                            fallback?.terms?.contains(word) == true ||
                            shortcodes.any { it.contains(word) }
                    } -> keywords.add(emoji)
                }
            }
            return exact + prefixes + keywords
        }
    }

    fun recents(choices: List<String>): List<String> =
        choices.mapNotNull { canonical[reactionEmojiKey(it)] }.distinct()
}
