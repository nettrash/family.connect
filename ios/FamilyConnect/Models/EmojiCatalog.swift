//
//  EmojiCatalog.swift
//  FamilyConnect
//
//  The full-picker emoji catalog behind the floating picker's "+" sheet.
//  Transcribed from the canonical cross-platform catalog (the Android app
//  embeds the same list — same categories, same order — so the picker is
//  identical on both platforms; do not add, remove, or reorder entries
//  here without changing it there too). Every entry is a single-grapheme,
//  skin-tone-neutral emoji well under the server's 32-byte UTF-8 limit —
//  the unit tests pin all of that down.
//
//  One deliberate departure from the source JSON: its Hearts category
//  listed 💢 twice; the second occurrence is dropped here (a duplicate
//  offers the same reaction twice and collides ForEach identity). The
//  Android transcription must drop the same entry.
//
//  `name` is the canonical cross-platform identifier, not a UI string —
//  the section headers render `localizedName`.
//

import Foundation

/// One section of the full emoji picker: a canonical name and its emoji,
/// in canonical order.
nonisolated struct EmojiCategory: Equatable, Sendable, Identifiable {
    var id: String { name }
    /// Canonical category name, shared verbatim with Android.
    let name: String
    let emoji: [String]

    /// The localized section header for `name`. A literal per case so the
    /// string catalog's extractor sees every key.
    var localizedName: String {
        switch name {
        case "Smileys": String(localized: "Smileys")
        case "Gestures": String(localized: "Gestures")
        case "Hearts": String(localized: "Hearts")
        case "Animals & Nature": String(localized: "Animals & Nature")
        case "Food & Drink": String(localized: "Food & Drink")
        case "Activities": String(localized: "Activities")
        case "Travel & Places": String(localized: "Travel & Places")
        case "Objects & Symbols": String(localized: "Objects & Symbols")
        default: name
        }
    }
}

nonisolated enum EmojiCatalog {

    static let categories: [EmojiCategory] = [
        EmojiCategory(
            name: "Smileys",
            emoji: [
                "😀", "😃", "😄", "😁", "😆", "😅", "😂", "🤣", "🙂", "😉", "😊", "😇", "🥰", "😍", "🤩", "😘",
                "😗", "😚", "😙", "😋", "😛", "😜", "🤪", "😝", "🤑", "🤗", "🤭", "🤫", "🤔", "🤐", "🤨", "😐",
                "😑", "😶", "😏", "😒", "🙄", "😬", "🤥", "😌", "😔", "😪", "🤤", "😴", "😷", "🤒", "🤕", "🤢",
                "🤮", "🤧", "🥵", "🥶", "🥴", "😵", "🤯", "🤠", "🥳", "😎", "🤓", "🧐", "😕", "😟", "🙁", "😮",
                "😯", "😲", "😳", "🥺", "😦", "😧", "😨", "😰", "😥", "😢", "😭", "😱", "😖", "😣", "😞", "😓",
                "😩", "😫", "🥱", "😤", "😡", "😠", "🤬", "😈", "👿", "💀", "🤡", "👻", "👽", "🤖", "💩",
            ]),
        EmojiCategory(
            name: "Gestures",
            emoji: [
                "👋", "🤚", "🖐", "✋", "🖖", "👌", "🤏", "✌️", "🤞", "🤟", "🤘", "🤙", "👈", "👉", "👆", "🖕",
                "👇", "☝️", "👍", "👎", "✊", "👊", "🤛", "🤜", "👏", "🙌", "👐", "🤲", "🤝", "🙏", "✍️", "💅",
                "🤳", "💪", "🦾", "👂", "👃", "🧠", "👀", "👁", "👅", "👄", "💋",
            ]),
        EmojiCategory(
            name: "Hearts",
            emoji: [
                "❤️", "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍", "🤎", "💔", "❣️", "💕", "💞", "💓", "💗", "💖",
                "💘", "💝", "💟", "♥️", "💌", "💤", "💢", "💥", "💦", "💨", "💫", "⭐", "🌟", "✨", "⚡", "🔥",
                "💯", "🎉", "🎊", "🎈", "🎂", "🎁", "🏆", "🥇", "🥈", "🥉", "🏅",
            ]),
        EmojiCategory(
            name: "Animals & Nature",
            emoji: [
                "🐶", "🐱", "🐭", "🐹", "🐰", "🦊", "🐻", "🐼", "🐨", "🐯", "🦁", "🐮", "🐷", "🐸", "🐵", "🙈",
                "🙉", "🙊", "🐔", "🐧", "🐦", "🐤", "🦆", "🦅", "🦉", "🦇", "🐺", "🐗", "🐴", "🦄", "🐝", "🐛",
                "🦋", "🐌", "🐞", "🐜", "🕷", "🐢", "🐍", "🦎", "🐙", "🦑", "🦀", "🐡", "🐠", "🐟", "🐬", "🐳",
                "🐋", "🦈", "🐊", "🐘", "🦏", "🐪", "🦒", "🦘", "🐃", "🐎", "🐖", "🐏", "🐑", "🦙", "🐐", "🦌",
                "🐕", "🐩", "🐈", "🐇", "🐿", "🦔", "🌵", "🎄", "🌲", "🌳", "🌴", "🌱", "🌿", "☘️", "🍀", "🎍",
                "🍁", "🍄", "🌾", "💐", "🌷", "🌹", "🥀", "🌺", "🌸", "🌼", "🌻", "🌞", "🌝", "🌚", "🌙", "🌎",
                "🪐", "💫", "🌈", "☀️", "⛅", "☁️", "🌧", "⛈", "❄️", "⛄", "🌊",
            ]),
        EmojiCategory(
            name: "Food & Drink",
            emoji: [
                "🍏", "🍎", "🍐", "🍊", "🍋", "🍌", "🍉", "🍇", "🍓", "🍈", "🍒", "🍑", "🥭", "🍍", "🥥", "🥝",
                "🍅", "🍆", "🥑", "🥦", "🥬", "🥒", "🌶", "🌽", "🥕", "🧄", "🧅", "🥔", "🍠", "🥐", "🥯", "🍞",
                "🥖", "🥨", "🧀", "🥚", "🍳", "🧈", "🥞", "🧇", "🥓", "🥩", "🍗", "🍖", "🌭", "🍔", "🍟", "🍕",
                "🥪", "🥙", "🧆", "🌮", "🌯", "🥗", "🥘", "🍝", "🍜", "🍲", "🍛", "🍣", "🍱", "🥟", "🦪", "🍤",
                "🍙", "🍚", "🍘", "🍥", "🥮", "🍢", "🍡", "🍧", "🍨", "🍦", "🥧", "🧁", "🍰", "🎂", "🍮", "🍭",
                "🍬", "🍫", "🍿", "🍩", "🍪", "🌰", "🥜", "🍯", "🥛", "🍼", "☕", "🍵", "🧃", "🥤", "🍶", "🍺",
                "🍻", "🥂", "🍷", "🥃", "🍸", "🍹", "🧉", "🍾", "🧊",
            ]),
        EmojiCategory(
            name: "Activities",
            emoji: [
                "⚽", "🏀", "🏈", "⚾", "🥎", "🎾", "🏐", "🏉", "🥏", "🎱", "🪀", "🏓", "🏸", "🏒", "🏑", "🥍",
                "🏏", "🥅", "⛳", "🪁", "🏹", "🎣", "🤿", "🥊", "🥋", "🎽", "🛹", "🛷", "⛸", "🥌", "🎿", "⛷",
                "🏂", "🏋️", "🤸", "⛹️", "🤺", "🤾", "🏌️", "🏇", "🧘", "🏄", "🏊", "🤽", "🚣", "🧗", "🚴", "🚵",
                "🎪", "🎭", "🎨", "🎬", "🎤", "🎧", "🎼", "🎹", "🥁", "🎷", "🎺", "🎸", "🪕", "🎻", "🎲", "♟",
                "🎯", "🎳", "🎮", "🎰", "🧩",
            ]),
        EmojiCategory(
            name: "Travel & Places",
            emoji: [
                "🚗", "🚕", "🚙", "🚌", "🚎", "🏎", "🚓", "🚑", "🚒", "🚐", "🚚", "🚛", "🚜", "🛴", "🚲", "🛵",
                "🏍", "🚨", "🚔", "🚍", "🚘", "🚖", "🚡", "🚠", "🚟", "🚃", "🚋", "🚞", "🚝", "🚄", "🚅", "🚈",
                "🚂", "🚆", "🚇", "🚊", "🚉", "✈️", "🛫", "🛬", "🛩", "💺", "🛰", "🚀", "🛸", "🚁", "🛶", "⛵",
                "🚤", "🛥", "🛳", "⛴", "🚢", "⚓", "⛽", "🚧", "🚦", "🚥", "🚏", "🗺", "🗿", "🗽", "🗼", "🏰",
                "🏯", "🏟", "🎡", "🎢", "🎠", "⛲", "⛱", "🏖", "🏝", "🏜", "🌋", "⛰", "🏔", "🗻", "🏕", "⛺",
                "🏠", "🏡", "🏘", "🏚", "🏗", "🏭", "🏢", "🏬", "🏣", "🏤", "🏥", "🏦", "🏨", "🏪", "🏫", "🏩",
                "💒", "🏛", "⛪", "🕌", "🕍", "🛕", "🕋", "⛩", "🌅", "🌄", "🌠", "🎇", "🎆", "🌇", "🌆", "🏙",
                "🌃", "🌌", "🌉", "🌁",
            ]),
        EmojiCategory(
            name: "Objects & Symbols",
            emoji: [
                "⌚", "📱", "💻", "⌨️", "🖥", "🖨", "🖱", "🕹", "🗜", "💽", "💾", "💿", "📀", "📼", "📷", "📸",
                "📹", "🎥", "📽", "🎞", "📞", "☎️", "📟", "📠", "📺", "📻", "🎙", "⏰", "⌛", "⏳", "📡", "🔋",
                "🔌", "💡", "🔦", "🕯", "🧯", "🛢", "💸", "💵", "💰", "💳", "💎", "⚖️", "🧰", "🔧", "🔨", "⚒",
                "🛠", "⛏", "🔩", "⚙️", "🧲", "🔫", "💣", "🧨", "🪓", "🔪", "🗡", "🛡", "🚬", "⚰️", "⚱️", "🏺",
                "🔮", "📿", "🧿", "💈", "⚗️", "🔭", "🔬", "🕳", "💊", "💉", "🧬", "🦠", "🧫", "🧪", "🌡", "🧹",
                "🧺", "🧻", "🚽", "🚰", "🚿", "🛁", "🧼", "🪒", "🧽", "🧴", "🛎", "🔑", "🗝", "🚪", "🪑", "🛋",
                "🛏", "🧸", "🖼", "🛍", "🛒", "🎀", "🎏", "🎗", "📯", "📦", "📫", "📮", "📜", "📃", "📄", "📊",
                "📈", "📉", "🗒", "📆", "📅", "📇", "🗃", "🗳", "🗄", "📋", "📁", "📂", "🗞", "📰", "📓", "📔",
                "📒", "📕", "📗", "📘", "📙", "📚", "📖", "🔖", "🧷", "🔗", "📎", "📐", "📏", "🧮", "📌", "📍",
                "✂️", "🖊", "🖋", "✒️", "🖌", "🖍", "📝", "✏️", "🔍", "🔎", "🔒", "🔓", "❗", "❓", "‼️", "⁉️",
                "✅", "❌", "⭕", "🚫", "💬", "💭", "🗯", "♻️", "🔱", "📣", "📢", "🔔", "🔕", "🎵", "🎶", "➕",
                "➖", "➗", "✖️", "♾", "💲", "™️", "©️", "®️", "🔴", "🟠", "🟡", "🟢", "🔵", "🟣", "⚫", "⚪",
                "🟤",
            ]),
    ]
}
