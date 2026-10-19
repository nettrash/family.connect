//
//  EmojiTyping.swift
//  FamilyConnect
//
//  Emoji as you type (issue #101, docs/emoji-as-you-type-2026-10-18.md):
//  Telegram's emoticons — `:-)`, `<3`, `;-P` — with their no-nose forms,
//  and `:name:` shortcodes, become emoji the moment the last character
//  goes in. A Backspace straight after takes the replacement back.
//
//  A PORT, NOT A READING: `web/text/src/emoji_typing.rs` is the original,
//  and this answers every one of its vectors (Fixtures/emoji-typing-
//  vectors.json, printed by board-oracle) exactly. The Mac's wiring is
//  MacCapitalKeys, iOS's is EmojiTypingField; this file is the rule alone,
//  pure and nonisolated, so it is tested without a window or a keyboard.
//
//  Offsets are UTF-16 units — what NSTextView and NSString count — where
//  the Rust counts UTF-8 bytes; the two agree on every boundary, because
//  both only ever cut between whole scalars.
//
//  Text is COMPARED BY ITS CODE UNITS, never with String's `==` or
//  `hasSuffix`: those compare canonically, and `;` has a canonical twin
//  (U+037E GREEK QUESTION MARK), as has the backtick (U+1FEF) and `K`
//  (U+212A KELVIN SIGN). The Rust compares bytes, so this does too.
//

import Foundation

nonisolated enum EmojiTyping {

    /// The emoticons, as typed, and the emoji each becomes — the Rust's
    /// `EMOTICONS`, in its order.
    ///
    /// The first 27 are Telegram Desktop's own table; the rest are the
    /// owner's addition (2026-10-18): the no-nose forms of the same faces,
    /// so `:)` works as the issue asks, plus the case twins Telegram's
    /// case-sensitive table misses. `8)` and `B)` are left out on purpose.
    ///
    /// An entry ending in a space is replaced when that space is typed, and
    /// the space stays: `:o` and `:p` would otherwise eat the start of
    /// `:ok_hand:` and `:pizza:`, and `xD` of `xDDD`.
    static let emoticons: [(text: String, emoji: String)] = [
        // Telegram's.
        (":-)", "\u{1F60A}"),
        ("8-)", "\u{1F60D}"),
        ("<3", "\u{2764}\u{FE0F}"),
        (":-*", "\u{1F61A}"),
        ("B-)", "\u{1F60E}"),
        (":-D", "\u{1F603}"),
        (";-)", "\u{1F609}"),
        (";-P", "\u{1F61C}"),
        (":-p", "\u{1F60B}"),
        ("3(", "\u{1F614}"),
        (":-(", "\u{1F61E}"),
        (":]", "\u{1F60F}"),
        (":'(", "\u{1F622}"),
        (":_(", "\u{1F62D}"),
        (":((", "\u{1F629}"),
        (":|", "\u{1F610}"),
        ("3-)", "\u{1F60C}"),
        (">(", "\u{1F620}"),
        (">((", "\u{1F621}"),
        ("O:)", "\u{1F607}"),
        (";o", "\u{1F630}"),
        ("8|", "\u{1F633}"),
        ("8o", "\u{1F632}"),
        (":X", "\u{1F637}"),
        ("}:)", "\u{1F608}"),
        (":o ", "\u{1F628}"),
        ("xD ", "\u{1F606}"),
        // The no-nose forms and the case twins.
        (":)", "\u{1F60A}"),
        (":D", "\u{1F603}"),
        (";)", "\u{1F609}"),
        (":(", "\u{1F61E}"),
        (":*", "\u{1F61A}"),
        (":P", "\u{1F60B}"),
        (":p ", "\u{1F60B}"),
        (":-P", "\u{1F60B}"),
        (";P", "\u{1F61C}"),
        (";p", "\u{1F61C}"),
        (";-p", "\u{1F61C}"),
        ("O:-)", "\u{1F607}"),
        ("}:-)", "\u{1F608}"),
        ("XD ", "\u{1F606}"),
    ]

    /// The emoticons as code units, built once: the rule compares these.
    private static let emoticonUnits: [(text: [UInt16], emoji: String)] =
        emoticons.map { (Array($0.text.utf16), $0.emoji) }

    // MARK: - Shortcodes

    /// The emoji `name` (no colons) stands for, if any. Names are matched
    /// exactly — byte for byte, so `:Fire:` is not `:fire:`, and a name
    /// typed with a decomposed `ñ` is not the table's composed one.
    static func emoji(named name: String) -> String? {
        names[Array(name.utf8)]
    }

    /// How many names the table holds (a test's sanity check).
    static var nameCount: Int { names.count }

    /// The shortcode table, read from the app bundle's `emoji-names.tsv`
    /// the first time a name is looked up, and kept. tools/emoji/
    /// make_names.py writes the file; every client ships a copy of it.
    ///
    /// Keyed by the name's UTF-8 bytes rather than a String, whose hashing
    /// and equality are canonical — see the file's header.
    private static let names: [[UInt8]: String] = loadNames()

    private static func loadNames() -> [[UInt8]: String] {
        guard let url = Bundle.main.url(forResource: "emoji-names", withExtension: "tsv"),
              let data = try? Data(contentsOf: url)
        else { return [:] }
        return parseNames(data)
    }

    /// One `name<TAB>emoji` per line; empty lines and lines starting with
    /// `#` are skipped. Split on the BYTES: Swift's Character view reads
    /// "\r\n" as one character, and Rust's `lines()` splits at "\n" and
    /// drops a "\r" before it — so this does exactly that. A name listed
    /// twice keeps its last emoji, as the Rust's `collect` into a map does.
    static func parseNames(_ data: Data) -> [[UInt8]: String] {
        let tab = UInt8(ascii: "\t")
        var table: [[UInt8]: String] = [:]
        table.reserveCapacity(7000)
        for raw in data.split(separator: UInt8(ascii: "\n"), omittingEmptySubsequences: false) {
            var line = raw[...]
            if line.last == UInt8(ascii: "\r") { line = line.dropLast() }
            guard let first = line.first, first != UInt8(ascii: "#"),
                  let split = line.firstIndex(of: tab)
            else { continue }
            let name = Array(line[line.startIndex..<split])
            table[name] = String(decoding: line[line.index(after: split)...], as: UTF8.self)
        }
        return table
    }

    // MARK: - The swap

    /// A replacement the rule just made, kept so the Backspace pressed
    /// straight after it can take it back — and so the next key can grow
    /// it, `:(` and another `(` being `:((`.
    ///
    /// The UI keeps one from the moment the rule replaces until anything
    /// else happens — a key the rule does not act on, a click, a paste, a
    /// selection — and then drops it, exactly like #97's capital.
    struct Swap: Equatable, Sendable {
        /// Where the replacement starts, in UTF-16 units.
        let at: Int
        /// What was put in: the emoji, and the space after it for the
        /// emoticons that end in one.
        let put: String
        /// What was typed, all of it: what a Backspace puts back.
        let typed: String

        static func == (lhs: Swap, rhs: Swap) -> Bool {
            lhs.at == rhs.at && lhs.put.utf16.elementsEqual(rhs.put.utf16)
                && lhs.typed.utf16.elementsEqual(rhs.typed.utf16)
        }

        /// The field's text and caret (UTF-16) after the swap, given what
        /// was before the caret (without the typed character) and what is
        /// after it.
        func apply(before: String, after: String) -> (text: String, caret: Int) {
            let head = Array(before.utf16)
            let put16 = Array(put.utf16)
            precondition(at <= head.count, "a swap past the end of its text")
            let front = head[..<at] + put16
            let text = String(decoding: front + Array(after.utf16), as: UTF16.self)
            return (text, front.count)
        }

        /// A Backspace with the caret at `caret` (UTF-16) and nothing
        /// selected: the text with what was typed put back, and the caret
        /// right behind it — or nil when the caret is not right behind the
        /// replacement or the replacement is no longer there, and the
        /// Backspace is an ordinary one.
        func backspace(text: String, caret: Int) -> (text: String, caret: Int)? {
            let units = Array(text.utf16)
            let put16 = Array(put.utf16)
            let end = at + put16.count
            guard caret == end, end <= units.count, units[at..<end].elementsEqual(put16) else {
                return nil
            }
            let typed16 = Array(typed.utf16)
            let undone = String(decoding: units[..<at] + typed16 + units[end...], as: UTF16.self)
            return (undone, at + typed16.count)
        }
    }

    // MARK: - The rule

    /// What typing `typed` turns into: `before` is the field's text before
    /// the caret, without `typed`; `live` is the swap the UI still holds, if
    /// any. nil when the character goes in as it is.
    ///
    /// In order:
    ///
    /// 1. nothing in code (the test #97's capitals use);
    /// 2. GROWING: the caret is right behind `live`'s replacement, and what
    ///    was typed then plus `typed` is a longer emoticon starting at the
    ///    same place — `:(` then `(` is `:((`, one swap whose Backspace puts
    ///    back all three characters;
    /// 3. a SHORTCODE, when `typed` is `:` — `before` ends in `:name` with
    ///    no whitespace or colon in the name, the opening colon is at the
    ///    start or after something that is not an ASCII letter or digit (so
    ///    `10:30:` and `a:b:` are left alone), and the table knows the name;
    /// 4. an EMOTICON: `before` plus `typed` ends in one, the longest whose
    ///    guard passes — one starting with `:` or `;` must not follow an
    ///    ASCII digit (`1:]`), any other must not follow an ASCII letter or
    ///    digit (`x<3`, `item3(`). `HELLO:)` is `:)` after `HELL`'s `O`,
    ///    because the longer `O:)` fails its guard.
    static func replace(before: String, typed: Unicode.Scalar, live: Swap?) -> Swap? {
        // 1. Code is typed as it is.
        if Capitals.isInCode(before) { return nil }
        let head = Array(before.utf16)

        // 2. Growing the swap just made.
        if let live {
            let put16 = Array(live.put.utf16)
            if live.at + put16.count == head.count, head[live.at...].elementsEqual(put16) {
                var grown = live.typed
                grown.unicodeScalars.append(typed)
                let grown16 = Array(grown.utf16)
                if let match = emoticonUnits.first(where: { $0.text == grown16 }) {
                    return Swap(at: live.at, put: put(grown16, match.emoji), typed: grown)
                }
            }
        }

        // 3. `:name:`.
        let colonUnit = unit(":")
        if typed == ":", let colon = head.lastIndex(of: colonUnit) {
            let name16 = head[(colon + 1)...]
            let name = String(decoding: name16, as: UTF16.self)
            if !name16.isEmpty,
               !name.unicodeScalars.contains(where: { $0.properties.isWhitespace }),
               !(colon > 0 && isASCIILetterOrDigit(head[colon - 1])),
               let emoji = emoji(named: name) {
                return Swap(at: colon, put: emoji, typed: ":" + name + ":")
            }
        }

        // 4. An emoticon, the longest that may stand here.
        let all = head + Array(String(typed).utf16)
        var best: (text: [UInt16], emoji: String)?
        for entry in emoticonUnits where entry.text.count <= all.count {
            guard all.suffix(entry.text.count).elementsEqual(entry.text) else { continue }
            let rest = all.count - entry.text.count
            guard guardPasses(previous: rest > 0 ? all[rest - 1] : nil, emoticon: entry.text) else {
                continue
            }
            if entry.text.count > best?.text.count ?? 0 { best = entry }
        }
        guard let best else { return nil }
        return Swap(
            at: all.count - best.text.count,
            put: put(best.text, best.emoji),
            typed: String(decoding: best.text, as: UTF16.self))
    }

    /// The emoji, and the space after it for an emoticon that ends in one.
    private static func put(_ text: [UInt16], _ emoji: String) -> String {
        text.last == unit(" ") ? emoji + " " : emoji
    }

    /// Rule 4's guard: what may stand before `emoticon`. `previous` is the
    /// code unit just before it — a surrogate is never ASCII, so looking at
    /// one unit answers for the whole scalar.
    private static func guardPasses(previous: UInt16?, emoticon: [UInt16]) -> Bool {
        guard let previous else { return true }
        if emoticon.first == unit(":") || emoticon.first == unit(";") {
            return !isASCIIDigit(previous)
        }
        return !isASCIILetterOrDigit(previous)
    }

    /// The UTF-16 unit of a scalar in the Basic Multilingual Plane.
    private static func unit(_ scalar: Unicode.Scalar) -> UInt16 {
        UInt16(scalar.value)
    }

    private static func isASCIIDigit(_ code: UInt16) -> Bool {
        (unit("0")...unit("9")).contains(code)
    }

    private static func isASCIILetterOrDigit(_ code: UInt16) -> Bool {
        isASCIIDigit(code)
            || (unit("a")...unit("z")).contains(code)
            || (unit("A")...unit("Z")).contains(code)
    }
}
