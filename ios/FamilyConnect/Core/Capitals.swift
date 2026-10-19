//
//  Capitals.swift
//  FamilyConnect
//
//  Whether a letter just typed should become a capital (issue #97,
//  docs/capitals-and-rich-markdown-2026-10-17.md, "The rule").
//
//  A phone's keyboard capitalises sentences on its own; a Mac's does not,
//  and neither does Windows' or a browser's on a computer. So the desktop
//  clients carry ONE rule between them — this file, Windows'
//  `Capitals.cs` and the web's `composer::capitalizes` — held to one table
//  of cases that every port's tests repeat verbatim. MacCapitalKeys is the
//  Mac's wiring around it; iOS compiles it so both test runs pin it.
//
//  Pure and nonisolated: it reads two values and answers, so it is tested
//  without a window, a field or a keyboard.
//
//  Everything here is deliberately ASCII-minded where the four languages
//  would otherwise disagree (the abbreviation pieces, the list markers),
//  and Unicode-minded only where all four agree — what a lower-case letter
//  is and what its invariant upper case is.
//

import Foundation

nonisolated enum Capitals {

    /// Marks that open a quotation or an aside. Typed just before a word,
    /// they do not stop that word starting a sentence: `"hello` and
    /// `(see` both capitalise.
    static let openingMarks: Set<Character> = ["\"", "'", "(", "[", "«", "“", "‘"]

    /// Marks that close one. A sentence that ends inside them — `"Stop."`,
    /// `(Done.)` — still ended.
    static let closingMarks: Set<Character> = ["\"", "'", ")", "]", "»", "”", "’"]

    /// Should `typed`, the one character the keyboard just produced, be put
    /// in as its capital — given `before`, the text up to the caret (the
    /// selection's start)?
    ///
    /// YES only when all four hold: it is a lower-case letter with a
    /// one-character capital; a word is starting; a sentence is starting;
    /// and the caret is not in code.
    static func capitalizes(before: String, typed: Character) -> Bool {
        guard upperCase(of: typed) != nil else { return false }
        // Opening marks typed just before the word are set aside for both
        // the word test and the sentence test.
        var rest = Substring(before)
        while let last = rest.last, openingMarks.contains(last) { rest = rest.dropLast() }
        guard wordStarts(after: rest) else { return false }
        guard sentenceStarts(after: rest) else { return false }
        return !isInCode(before)
    }

    /// The capital `typed` becomes, or nil when it is not a lower-case
    /// letter with a capital of exactly one character.
    ///
    /// ONE SCALAR, not one grapheme: `ß` → `SS` is two, and so is `ǰ` →
    /// `J̌` (a J and a combining caron) even though it draws as one. The
    /// other ports count code points (Rust `char`, C# `ToUpperInvariant`
    /// on one `char`), so counting scalars is what makes the four agree.
    /// Swift's case mapping is the Unicode default one, which is the
    /// invariant mapping the rule asks for — never the locale's (Turkish
    /// `i` → `İ` is not this rule's business).
    static func upperCase(of typed: Character) -> String? {
        let scalars = typed.unicodeScalars
        guard scalars.count == 1, let scalar = scalars.first else { return nil }
        guard scalar.properties.isAlphabetic, scalar.properties.isLowercase else { return nil }
        let upper = scalar.properties.uppercaseMapping
        guard upper.unicodeScalars.count == 1, upper != String(typed) else { return nil }
        return upper
    }

    /// Spaces, tabs and line breaks — the only things a word starts after.
    private static func isBlank(_ character: Character) -> Bool {
        character == " " || character == "\t" || isLineBreak(character)
    }

    private static func isLineBreak(_ character: Character) -> Bool {
        // "\r\n" is ONE Character in Swift, and it is a line break too.
        character == "\n" || character == "\r" || character == "\r\n"
    }

    /// Rule 2: nothing yet, or a space, a tab or a line break just before.
    private static func wordStarts(after rest: Substring) -> Bool {
        guard let last = rest.last else { return true }
        return isBlank(last)
    }

    /// Rule 3: the text (opening marks already set aside) ends a sentence,
    /// starts a line, or is nothing but a list, heading or quote marker.
    private static func sentenceStarts(after rest: Substring) -> Bool {
        var text = rest
        while let last = text.last, last == " " || last == "\t" { text = text.dropLast() }
        guard let last = text.last, !isLineBreak(last) else { return true }

        // The current line, as a marker on its own.
        let line = text.split(omittingEmptySubsequences: false, whereSeparator: isLineBreak).last ?? text
        if isMarker(line) { return true }

        // A terminator, perhaps inside closing marks.
        var core = text
        while let last = core.last, closingMarks.contains(last) { core = core.dropLast() }
        guard let terminator = core.last, terminator == "." || terminator == "!" || terminator == "?"
        else { return false }
        guard terminator == "." else { return true }
        let beforeDot = core.dropLast()
        // An ellipsis trails off rather than ending anything.
        if beforeDot.last == "." { return false }
        return !isAbbreviation(core)
    }

    /// `-`, `*`, `+`, `>`, one to six `#`, or digits then `.` or `)` — and
    /// nothing else on the line but an indent of spaces and tabs (a nested
    /// item, `  - `, starts a sentence too; every port reads it so).
    private static func isMarker(_ indented: Substring) -> Bool {
        let line = indented.drop { $0 == " " || $0 == "\t" }
        if line == "-" || line == "*" || line == "+" || line == ">" { return true }
        if !line.isEmpty, line.count <= 6, line.allSatisfy({ $0 == "#" }) { return true }
        guard let last = line.last, last == "." || last == ")" else { return false }
        let digits = line.dropLast()
        return !digits.isEmpty && digits.allSatisfy { $0.isASCII && $0.isNumber }
    }

    /// `e.g.`, `i.e.`, `a.m.`, `U.S.`: the word back to the previous blank
    /// holds another `.`, and every piece between the dots is one or two
    /// ASCII letters. `example.com.` is not one — `example` is too long —
    /// so a sentence ending in a domain still ends.
    private static func isAbbreviation(_ text: Substring) -> Bool {
        let word = text.split(omittingEmptySubsequences: false, whereSeparator: isBlank).last ?? text
        let body = word.dropLast()  // the final `.`
        guard body.contains(".") else { return false }
        return body.split(separator: ".", omittingEmptySubsequences: false).allSatisfy { piece in
            (1...2).contains(piece.count) && piece.allSatisfy { $0.isASCII && $0.isLetter }
        }
    }

    /// Rule 4: inside a fence (an odd number of lines starting with three
    /// backticks before the caret), or inside a code span on the current
    /// line (an odd number of backticks on it, outside a fence). Emoji as
    /// you type (#101, EmojiTyping) skips code by the same test.
    ///
    /// Read by SCALARS, as the Rust's `in_code` reads its chars: a line
    /// break is a "\n" or a "\r", and a backtick is U+0060 itself. Swift's
    /// Character view would disagree twice — a backtick with a combining
    /// mark after it is one Character that is not "`", and U+1FEF GREEK
    /// VARIA compares EQUAL to "`" because the two are canonically the same.
    static func isInCode(_ before: String) -> Bool {
        let scalars = before.unicodeScalars
        let lines = scalars.split(omittingEmptySubsequences: false) { $0 == "\n" || $0 == "\r" }
        let fences = lines.filter { $0.starts(with: "```".unicodeScalars) }.count
        if fences % 2 == 1 { return true }
        let current = lines.last ?? scalars[...]
        return current.filter { $0 == "`" }.count % 2 == 1
    }
}
