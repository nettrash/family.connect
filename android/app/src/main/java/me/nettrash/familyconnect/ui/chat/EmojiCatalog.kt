/*
 * EmojiCatalog.kt
 * Family Connect (Android)
 *
 * The canonical full-picker emoji catalog behind the reaction capsule's
 * "+" button. Transcribed from the shared cross-platform catalog — the
 * iOS app embeds the IDENTICAL list (same categories, same order) so the
 * "+" picker looks the same on both platforms. Do not add, remove, or
 * reorder entries here without changing iOS in the same breath.
 *
 * Only widely-supported, skin-tone-neutral, single-grapheme emoji
 * (renders on Android 8 / iOS 17); every entry is well under the
 * server's 32-byte UTF-8 reaction limit — EmojiCatalogTest pins that.
 *
 * Kept free of Compose and Android so the sanity test runs as a plain
 * unit test. Category names are UI strings (the app's UI copy is inline
 * English throughout — see ChatScreen.kt).
 *
 * iOS counterpart: ios/FamilyConnect/Views/ConversationView.swift
 */

package me.nettrash.familyconnect.ui.chat

import androidx.annotation.StringRes
import me.nettrash.familyconnect.R

/** One section of the full emoji picker. */
data class EmojiCategory(
    /** The category's stable English name — the grid's key and the tests' handle, never drawn. */
    val name: String,
    /** The section header, through the catalogue (nine languages). */
    @param:StringRes val titleRes: Int,
    val emoji: List<String>,
)

/** Every category the "+" picker offers, in display order. */
val EMOJI_CATALOG: List<EmojiCategory> = listOf(
    EmojiCategory(
        name = "Smileys",
        titleRes = R.string.s_emoji_smileys,
        emoji = listOf(
            "😀", "😃", "😄", "😁", "😆", "😅", "😂", "🤣", "🙂", "😉", "😊", "😇",
            "🥰", "😍", "🤩", "😘", "😗", "😚", "😙", "😋", "😛", "😜", "🤪", "😝",
            "🤑", "🤗", "🤭", "🤫", "🤔", "🤐", "🤨", "😐", "😑", "😶", "😏", "😒",
            "🙄", "😬", "🤥", "😌", "😔", "😪", "🤤", "😴", "😷", "🤒", "🤕", "🤢",
            "🤮", "🤧", "🥵", "🥶", "🥴", "😵", "🤯", "🤠", "🥳", "😎", "🤓", "🧐",
            "😕", "😟", "🙁", "😮", "😯", "😲", "😳", "🥺", "😦", "😧", "😨", "😰",
            "😥", "😢", "😭", "😱", "😖", "😣", "😞", "😓", "😩", "😫", "🥱", "😤",
            "😡", "😠", "🤬", "😈", "👿", "💀", "🤡", "👻", "👽", "🤖", "💩",
        ),
    ),
    EmojiCategory(
        name = "Gestures",
        titleRes = R.string.s_emoji_gestures,
        emoji = listOf(
            "👋", "🤚", "🖐", "✋", "🖖", "👌", "🤏", "✌️", "🤞", "🤟", "🤘", "🤙",
            "👈", "👉", "👆", "🖕", "👇", "☝️", "👍", "👎", "✊", "👊", "🤛", "🤜",
            "👏", "🙌", "👐", "🤲", "🤝", "🙏", "✍️", "💅", "🤳", "💪", "🦾", "👂",
            "👃", "🧠", "👀", "👁", "👅", "👄", "💋",
        ),
    ),
    EmojiCategory(
        name = "Hearts",
        titleRes = R.string.s_emoji_hearts,
        emoji = listOf(
            "❤️", "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍", "🤎", "💔", "❣️", "💕",
            "💞", "💓", "💗", "💖", "💘", "💝", "💟", "♥️", "💌", "💤", "💢", "💥",
            "💦", "💨", "💫", "⭐", "🌟", "✨", "⚡", "🔥", "💯", "🎉", "🎊", "🎈",
            "🎂", "🎁", "🏆", "🥇", "🥈", "🥉", "🏅",
        ),
    ),
    EmojiCategory(
        name = "Animals & Nature",
        titleRes = R.string.s_emoji_animals,
        emoji = listOf(
            "🐶", "🐱", "🐭", "🐹", "🐰", "🦊", "🐻", "🐼", "🐨", "🐯", "🦁", "🐮",
            "🐷", "🐸", "🐵", "🙈", "🙉", "🙊", "🐔", "🐧", "🐦", "🐤", "🦆", "🦅",
            "🦉", "🦇", "🐺", "🐗", "🐴", "🦄", "🐝", "🐛", "🦋", "🐌", "🐞", "🐜",
            "🕷", "🐢", "🐍", "🦎", "🐙", "🦑", "🦀", "🐡", "🐠", "🐟", "🐬", "🐳",
            "🐋", "🦈", "🐊", "🐘", "🦏", "🐪", "🦒", "🦘", "🐃", "🐎", "🐖", "🐏",
            "🐑", "🦙", "🐐", "🦌", "🐕", "🐩", "🐈", "🐇", "🐿", "🦔", "🌵", "🎄",
            "🌲", "🌳", "🌴", "🌱", "🌿", "☘️", "🍀", "🎍", "🍁", "🍄", "🌾", "💐",
            "🌷", "🌹", "🥀", "🌺", "🌸", "🌼", "🌻", "🌞", "🌝", "🌚", "🌙", "🌎",
            "🪐", "💫", "🌈", "☀️", "⛅", "☁️", "🌧", "⛈", "❄️", "⛄", "🌊",
        ),
    ),
    EmojiCategory(
        name = "Food & Drink",
        titleRes = R.string.s_emoji_food,
        emoji = listOf(
            "🍏", "🍎", "🍐", "🍊", "🍋", "🍌", "🍉", "🍇", "🍓", "🍈", "🍒", "🍑",
            "🥭", "🍍", "🥥", "🥝", "🍅", "🍆", "🥑", "🥦", "🥬", "🥒", "🌶", "🌽",
            "🥕", "🧄", "🧅", "🥔", "🍠", "🥐", "🥯", "🍞", "🥖", "🥨", "🧀", "🥚",
            "🍳", "🧈", "🥞", "🧇", "🥓", "🥩", "🍗", "🍖", "🌭", "🍔", "🍟", "🍕",
            "🥪", "🥙", "🧆", "🌮", "🌯", "🥗", "🥘", "🍝", "🍜", "🍲", "🍛", "🍣",
            "🍱", "🥟", "🦪", "🍤", "🍙", "🍚", "🍘", "🍥", "🥮", "🍢", "🍡", "🍧",
            "🍨", "🍦", "🥧", "🧁", "🍰", "🎂", "🍮", "🍭", "🍬", "🍫", "🍿", "🍩",
            "🍪", "🌰", "🥜", "🍯", "🥛", "🍼", "☕", "🍵", "🧃", "🥤", "🍶", "🍺",
            "🍻", "🥂", "🍷", "🥃", "🍸", "🍹", "🧉", "🍾", "🧊",
        ),
    ),
    EmojiCategory(
        name = "Activities",
        titleRes = R.string.s_emoji_activities,
        emoji = listOf(
            "⚽", "🏀", "🏈", "⚾", "🥎", "🎾", "🏐", "🏉", "🥏", "🎱", "🪀", "🏓",
            "🏸", "🏒", "🏑", "🥍", "🏏", "🥅", "⛳", "🪁", "🏹", "🎣", "🤿", "🥊",
            "🥋", "🎽", "🛹", "🛷", "⛸", "🥌", "🎿", "⛷", "🏂", "🏋️", "🤸", "⛹️",
            "🤺", "🤾", "🏌️", "🏇", "🧘", "🏄", "🏊", "🤽", "🚣", "🧗", "🚴", "🚵",
            "🎪", "🎭", "🎨", "🎬", "🎤", "🎧", "🎼", "🎹", "🥁", "🎷", "🎺", "🎸",
            "🪕", "🎻", "🎲", "♟", "🎯", "🎳", "🎮", "🎰", "🧩",
        ),
    ),
    EmojiCategory(
        name = "Travel & Places",
        titleRes = R.string.s_emoji_travel,
        emoji = listOf(
            "🚗", "🚕", "🚙", "🚌", "🚎", "🏎", "🚓", "🚑", "🚒", "🚐", "🚚", "🚛",
            "🚜", "🛴", "🚲", "🛵", "🏍", "🚨", "🚔", "🚍", "🚘", "🚖", "🚡", "🚠",
            "🚟", "🚃", "🚋", "🚞", "🚝", "🚄", "🚅", "🚈", "🚂", "🚆", "🚇", "🚊",
            "🚉", "✈️", "🛫", "🛬", "🛩", "💺", "🛰", "🚀", "🛸", "🚁", "🛶", "⛵",
            "🚤", "🛥", "🛳", "⛴", "🚢", "⚓", "⛽", "🚧", "🚦", "🚥", "🚏", "🗺",
            "🗿", "🗽", "🗼", "🏰", "🏯", "🏟", "🎡", "🎢", "🎠", "⛲", "⛱", "🏖",
            "🏝", "🏜", "🌋", "⛰", "🏔", "🗻", "🏕", "⛺", "🏠", "🏡", "🏘", "🏚",
            "🏗", "🏭", "🏢", "🏬", "🏣", "🏤", "🏥", "🏦", "🏨", "🏪", "🏫", "🏩",
            "💒", "🏛", "⛪", "🕌", "🕍", "🛕", "🕋", "⛩", "🌅", "🌄", "🌠", "🎇",
            "🎆", "🌇", "🌆", "🏙", "🌃", "🌌", "🌉", "🌁",
        ),
    ),
    EmojiCategory(
        name = "Objects & Symbols",
        titleRes = R.string.s_emoji_objects,
        emoji = listOf(
            "⌚", "📱", "💻", "⌨️", "🖥", "🖨", "🖱", "🕹", "🗜", "💽", "💾", "💿",
            "📀", "📼", "📷", "📸", "📹", "🎥", "📽", "🎞", "📞", "☎️", "📟", "📠",
            "📺", "📻", "🎙", "⏰", "⌛", "⏳", "📡", "🔋", "🔌", "💡", "🔦", "🕯",
            "🧯", "🛢", "💸", "💵", "💰", "💳", "💎", "⚖️", "🧰", "🔧", "🔨", "⚒",
            "🛠", "⛏", "🔩", "⚙️", "🧲", "🔫", "💣", "🧨", "🪓", "🔪", "🗡", "🛡",
            "🚬", "⚰️", "⚱️", "🏺", "🔮", "📿", "🧿", "💈", "⚗️", "🔭", "🔬", "🕳",
            "💊", "💉", "🧬", "🦠", "🧫", "🧪", "🌡", "🧹", "🧺", "🧻", "🚽", "🚰",
            "🚿", "🛁", "🧼", "🪒", "🧽", "🧴", "🛎", "🔑", "🗝", "🚪", "🪑", "🛋",
            "🛏", "🧸", "🖼", "🛍", "🛒", "🎀", "🎏", "🎗", "📯", "📦", "📫", "📮",
            "📜", "📃", "📄", "📊", "📈", "📉", "🗒", "📆", "📅", "📇", "🗃", "🗳",
            "🗄", "📋", "📁", "📂", "🗞", "📰", "📓", "📔", "📒", "📕", "📗", "📘",
            "📙", "📚", "📖", "🔖", "🧷", "🔗", "📎", "📐", "📏", "🧮", "📌", "📍",
            "✂️", "🖊", "🖋", "✒️", "🖌", "🖍", "📝", "✏️", "🔍", "🔎", "🔒", "🔓",
            "❗", "❓", "‼️", "⁉️", "✅", "❌", "⭕", "🚫", "💬", "💭", "🗯", "♻️",
            "🔱", "📣", "📢", "🔔", "🔕", "🎵", "🎶", "➕", "➖", "➗", "✖️", "♾",
            "💲", "™️", "©️", "®️", "🔴", "🟠", "🟡", "🟢", "🔵", "🟣", "⚫", "⚪",
            "🟤",
        ),
    ),
)
