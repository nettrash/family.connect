//
//  MessageMarkdown.swift
//  FamilyConnect
//
//  Markdown in a chat bubble: the small subset people actually type.
//
//  **bold**, *italic*, `code`, ~~strike~~, [label](url), \escapes and
//  ```fenced``` blocks inline; `# ` through `### ` headings, `- `/`* `/`+ `
//  bullets and GFM pipe tables at the start of a line. Nothing else.
//
//  HEADINGS AND BULLETS ARE RUNS, NOT BLOCKS, and that is the constraint
//  the whole bubble rests on: a heading changes a font and a bullet
//  replaces two characters, so both live inside the one attributed string
//  the balloon already draws. A message with no table therefore renders
//  through EXACTLY the path it always has — one `Text`, one laid-out
//  string, one offset space — which is what keeps working:
//
//    - the `\.openURL` arbitration that lets a double-tap over a link land
//      the quick heart (a second Text would be a second arbiter),
//    - the link detector, the `@ai` highlight and the accessibility label,
//      all of which index the RENDERED string,
//    - the bounded non-lazy 60/300-row scroll window on both clients.
//
//  ONLY A BODY THAT ACTUALLY CONTAINS A TABLE becomes a stack of blocks —
//  see `blocks(_:)`, and `MessageBodyBlocks` for how they are drawn. A
//  fenced block stays a monospaced RUN rather than a scrolling box for the
//  same reason: it reads as code, it wraps like text, and it costs the
//  bubble nothing structurally.
//
//  This is a RENDERING convention, not a wire format (docs/protocol.md
//  states it explicitly): `body` is plain text on the wire, the server
//  neither parses nor validates any of it, the 4000-character limit counts
//  what was typed, and a client that renders none of this shows the source
//  — which is still exactly what was written. Reply excerpts, the chat-list
//  preview, push bodies, copy and share all stay raw source for that
//  reason.
//
//  The inline half leans on Foundation's own parser, exactly as the `md`
//  apps do (md/md/MarkdownInline.swift): it is correct, ships in the SDK,
//  and hand-rolling a tokenizer to match CommonMark's emphasis rules is a
//  well-known way to get `snake_case` wrong. Android has no equivalent and
//  carries a hand-written one; the SUBSET is what the two share, not the
//  implementation.
//

import Foundation
import SwiftUI

nonisolated enum MessageMarkdown {

    /// One piece of a laid-out message body.
    ///
    /// A body with no table is exactly ONE `.text` block — pinned by a
    /// test, because it is the invariant every note above is made of.
    enum Block: Equatable {
        case text(AttributedString)
        case table(Table)

        var isTable: Bool {
            if case .table = self { return true }
            return false
        }
    }

    /// A GFM pipe table, already inline-rendered cell by cell.
    ///
    /// Header and body are separate because the header is drawn bold above
    /// a rule; `alignments` comes from the delimiter row and has one entry
    /// per column, which is also what makes it the authority on how many
    /// columns there are.
    struct Table: Equatable {
        var alignments: [ColumnAlignment]
        var header: [AttributedString]
        var rows: [[AttributedString]]

        var columnCount: Int { alignments.count }

        func alignment(_ column: Int) -> ColumnAlignment {
            column < alignments.count ? alignments[column] : .leading
        }
    }

    /// `:---` / `:--:` / `---:` / `---`, in the vocabulary of a layout
    /// rather than SwiftUI's: a cell needs both a column alignment and a
    /// text alignment, and neither type is worth pinning a parser to.
    enum ColumnAlignment: Equatable {
        case leading
        case center
        case trailing
    }

    /// Render a message body as the blocks it lays out in.
    ///
    /// The ONLY thing that splits a body is a table; everything else is a
    /// run inside a text block.
    static func blocks(_ body: String) -> [Block] {
        parse(body, tables: true)
    }

    /// Render a message body as one attributed string.
    ///
    /// Tables are deliberately NOT recognised here — a caller holding one
    /// string has nowhere to put a grid, so a pipe table renders as the
    /// rows that were typed. This is what the link detector, the preview
    /// card and the tests index; `blocks(_:)` is what the balloon draws.
    static func render(_ body: String) -> AttributedString {
        var out = AttributedString()
        for block in parse(body, tables: false) {
            if case .text(let text) = block { out.append(text) }
        }
        return out
    }

    // MARK: - Blocks

    /// A run of lines that renders as text, or a table that does not.
    private enum Piece {
        case lines(AttributedString)
        case table(Table)
    }

    /// Fences first, then line structure inside each plain segment: the
    /// whole point of a code fence is that what is inside it is not markup,
    /// so nothing below ever sees a fenced line.
    private static func parse(_ body: String, tables: Bool) -> [Block] {
        var out: [Block] = []
        var current = AttributedString()

        func flush() {
            guard !current.characters.isEmpty else { return }
            out.append(.text(current))
            current = AttributedString()
        }

        for segment in segments(of: body) {
            switch segment {
            case .code(let code):
                var run = AttributedString(code)
                run.font = .system(.body, design: .monospaced)
                current.append(run)
            case .text(let text):
                // Newlines BETWEEN pieces, never around a table: the lines
                // a table swallowed take their separators with them, or a
                // grid would leave a blank line above and below itself.
                var needsSeparator = false
                for piece in pieces(of: text, tables: tables) {
                    switch piece {
                    case .lines(let rendered):
                        if needsSeparator { current.append(AttributedString("\n")) }
                        current.append(rendered)
                        needsSeparator = true
                    case .table(let table):
                        flush()
                        out.append(.table(table))
                        needsSeparator = false
                    }
                }
            }
        }
        flush()
        // An empty body is still one text block: callers switch on the
        // block count, and "no blocks at all" is a shape none of them
        // expects.
        return out.isEmpty ? [.text(AttributedString())] : out
    }

    /// Split one fence-free segment into the pieces it lays out in.
    ///
    /// Consecutive ORDINARY lines are parsed together rather than one at a
    /// time, and that is deliberate: Foundation's inline parser carries
    /// emphasis across a newline (`**over\ntwo lines**` is bold on both),
    /// and a message with no block markers at all must come back exactly
    /// as it was typed. Only a line that IS a heading or a bullet is
    /// parsed on its own, because only those need their own attributes.
    private static func pieces(of segment: String, tables: Bool) -> [Piece] {
        let lines = segment.components(separatedBy: "\n")
        var pieces: [Piece] = []
        var plain: [String] = []
        var index = 0

        func flushPlain() {
            guard !plain.isEmpty else { return }
            pieces.append(.lines(inline(plain.joined(separator: "\n"))))
            plain = []
        }

        while index < lines.count {
            let line = lines[index]
            if let heading = heading(in: line) {
                flushPlain()
                var run = inline(heading.content)
                run.font = font(forHeadingLevel: heading.level)
                pieces.append(.lines(run))
                index += 1
                continue
            }
            if let bullet = bullet(in: line) {
                flushPlain()
                // The indent is copied verbatim, which is what gives
                // visually nested lists for free without a parser that can
                // mis-nest one.
                var run = AttributedString(bullet.indent + "• ")
                run.append(inline(bullet.content))
                pieces.append(.lines(run))
                index += 1
                continue
            }
            if tables, let found = table(at: index, in: lines) {
                flushPlain()
                pieces.append(.table(found.table))
                index = found.end
                continue
            }
            plain.append(line)
            index += 1
        }
        flushPlain()
        return pieces
    }

    // MARK: - Headings

    private struct Heading {
        let level: Int
        let content: String
    }

    /// One to three `#`, then AT LEAST ONE SPACE, then something.
    ///
    /// `#Heading`, `#### X` and a bare `# ` are all left exactly as typed:
    /// a heading has to be something somebody meant, or every message
    /// starting with a hash tag silently grows a font.
    private static func heading(in line: String) -> Heading? {
        var level = 0
        var index = line.startIndex
        while index < line.endIndex, line[index] == "#", level < 3 {
            level += 1
            index = line.index(after: index)
        }
        guard level > 0, index < line.endIndex else { return nil }
        // A fourth `#` is not a deeper heading — it is not a heading.
        guard line[index] == " " else { return nil }
        let content = String(line[line.index(after: index)...])
        // No closing-sequence stripping: `# Done #` says "Done #".
        //
        // Whitespace is not content either. `"#  "` — hash, space, space —
        // is somebody mid-sentence exactly as `"# "` is, and treating the
        // second space as a heading made the hashes vanish and the line grow
        // a font. Android has always rejected it (`text.isBlank()`); this is
        // the contract's answer for both.
        guard !content.trimmingCharacters(in: .whitespaces).isEmpty else { return nil }
        return Heading(level: level, content: content)
    }

    /// The three steps, relative to the body font so they scale with
    /// Dynamic Type for free. Android spells the same ladder 1.29 / 1.18 /
    /// 1.00 em; the exact points are not part of the contract.
    private static func font(forHeadingLevel level: Int) -> Font {
        switch level {
        case 1: return .title2.bold()
        case 2: return .title3.bold()
        default: return .headline
        }
    }

    // MARK: - Lists

    private struct Bullet {
        let indent: String
        let content: String
    }

    /// `- `, `* ` or `+ ` after any leading whitespace, then content.
    ///
    /// The marker never reaches the emphasis parser, which is what keeps
    /// `* X` a bullet and `2 * 3 * 4 = 24` arithmetic. A line of only `- `
    /// is not an item, and `---` is not one either — it is a table
    /// delimiter candidate.
    ///
    /// Ordered items are left exactly as typed: they already read as a
    /// list, and re-numbering somebody's message is worse than leaving it.
    private static func bullet(in line: String) -> Bullet? {
        var index = line.startIndex
        while index < line.endIndex, line[index] == " " || line[index] == "\t" {
            index = line.index(after: index)
        }
        guard index < line.endIndex, line[index] == "-" || line[index] == "*" || line[index] == "+"
        else { return nil }
        let afterMarker = line.index(after: index)
        guard afterMarker < line.endIndex, line[afterMarker] == " " else { return nil }
        let content = String(line[line.index(after: afterMarker)...])
        // Whitespace is not content, the same rule the heading above keeps:
        // `"-  "` is a marker somebody has not finished typing after, not a
        // bullet whose text is a space.
        guard !content.trimmingCharacters(in: .whitespaces).isEmpty else { return nil }
        return Bullet(indent: String(line[line.startIndex..<index]), content: content)
    }

    // MARK: - Tables

    /// A header row, a delimiter row with the SAME number of cells, then
    /// rows until the first line that is not one.
    ///
    /// The count has to match or there is no table at all: `some | thing`
    /// followed by a `---` rule is two ordinary lines, and turning it into
    /// a one-column grid would eat half of what was written.
    private static func table(at start: Int, in lines: [String]) -> (table: Table, end: Int)? {
        guard start + 1 < lines.count else { return nil }
        guard containsUnescapedPipe(lines[start]) else { return nil }
        let header = cells(of: lines[start])
        guard !header.isEmpty else { return nil }
        guard let alignments = delimiterRow(lines[start + 1]), alignments.count == header.count
        else { return nil }

        var rows: [[AttributedString]] = []
        var index = start + 2
        while index < lines.count {
            let line = lines[index]
            // Ends at a blank line or anything that is not a row — and a
            // heading or a bullet is what it says it is, not a cell.
            guard containsUnescapedPipe(line),
                !line.trimmingCharacters(in: .whitespaces).isEmpty,
                heading(in: line) == nil, bullet(in: line) == nil
            else { break }
            // A row that parses to NO cells at all — a lone `|` — ends the
            // table and is left exactly as typed. Padding it produced a
            // phantom all-empty row and swallowed the character somebody
            // wrote, which is worse than stopping; the header path has
            // guarded this since the beginning, and the body path forgot.
            let parsed = cells(of: line)
            guard !parsed.isEmpty else { break }
            var row = parsed.map(cell)
            // Ragged rows are the common case in a message somebody typed
            // by hand, and dropping the whole table over one missing pipe
            // would be the worst possible answer.
            if row.count < header.count {
                row.append(
                    contentsOf: repeatElement(
                        AttributedString(), count: header.count - row.count))
            } else if row.count > header.count {
                row.removeLast(row.count - header.count)
            }
            rows.append(row)
            index += 1
        }
        return (Table(alignments: alignments, header: header.map(cell), rows: rows), index)
    }

    /// The delimiter row's alignments, or nil when the line is not one.
    ///
    /// A PIPE IS REQUIRED, so a one-column table has to be written
    /// `| --- |`. A bare `---` under a line with a pipe in it is far more
    /// often a rule or a signature separator than somebody's one-column
    /// grid, and reading it as a delimiter turned two ordinary lines into a
    /// table. Android has always required the pipe (`rowCells` refuses a
    /// pipe-less line); this is the contract's answer for both.
    private static func delimiterRow(_ line: String) -> [ColumnAlignment]? {
        guard containsUnescapedPipe(line) else { return nil }
        let parts = cells(of: line)
        guard !parts.isEmpty else { return nil }
        var alignments: [ColumnAlignment] = []
        for part in parts {
            var dashes = part[...]
            let left = dashes.hasPrefix(":")
            if left { dashes = dashes.dropFirst() }
            let right = dashes.hasSuffix(":")
            if right { dashes = dashes.dropLast() }
            guard !dashes.isEmpty, dashes.allSatisfy({ $0 == "-" }) else { return nil }
            alignments.append(left && right ? .center : right ? .trailing : .leading)
        }
        return alignments
    }

    /// Split a row on unescaped `|`, dropping the optional leading and
    /// trailing ones. `\|` stays as it was written — the inline parser
    /// turns it into a literal pipe further down, exactly as it does
    /// everywhere else in a body.
    private static func cells(of line: String) -> [String] {
        let text = line.trimmingCharacters(in: .whitespaces)
        var parts: [String] = []
        var current = ""
        var escaped = false
        for character in text {
            if escaped {
                current.append(character)
                escaped = false
                continue
            }
            if character == "\\" {
                current.append(character)
                escaped = true
                continue
            }
            if character == "|" {
                parts.append(current)
                current = ""
                continue
            }
            current.append(character)
        }
        parts.append(current)
        if text.hasPrefix("|"), !parts.isEmpty { parts.removeFirst() }
        if endsWithUnescapedPipe(text), !parts.isEmpty { parts.removeLast() }
        return parts.map { $0.trimmingCharacters(in: .whitespaces) }
    }

    /// One cell: emphasis, code, strikethrough and escapes — but NEVER a
    /// link.
    ///
    /// `[label](url)` is left exactly as typed, and that is what makes a
    /// table cost nothing structurally: no per-cell hit test, no per-cell
    /// offset space, no clickable child fighting the balloon's gestures,
    /// and no second place where a label and a destination can disagree —
    /// the phishing shape MessageLinks already has an explicit rule for.
    ///
    /// A cell is parsed normally FIRST and re-parsed with the link syntax
    /// escaped only when a link actually formed, so the ordinary cell —
    /// which is nearly every cell — goes through the same one pass as any
    /// other text. Whatever survives (an `<autolink>` has no brackets to
    /// escape) loses its destination outright: a run that cannot be tapped
    /// is the promise, however it got here.
    private static func cell(_ source: String) -> AttributedString {
        let rendered = inline(source)
        guard rendered.runs.contains(where: { $0.link != nil }) else { return rendered }
        var literal = inline(
            source
                .replacingOccurrences(of: "[", with: "\\[")
                .replacingOccurrences(of: "]", with: "\\]"))
        for run in literal.runs where run.link != nil {
            literal[run.range].link = nil
        }
        return literal
    }

    private static func containsUnescapedPipe(_ line: String) -> Bool {
        var escaped = false
        for character in line {
            if escaped {
                escaped = false
                continue
            }
            if character == "\\" {
                escaped = true
                continue
            }
            if character == "|" { return true }
        }
        return false
    }

    /// A trailing `|` that is a cell boundary rather than a `\|` somebody
    /// typed — an odd number of backslashes before it means it is escaped.
    private static func endsWithUnescapedPipe(_ text: String) -> Bool {
        guard text.hasSuffix("|") else { return false }
        var backslashes = 0
        for character in text.dropLast().reversed() {
            guard character == "\\" else { break }
            backslashes += 1
        }
        return backslashes % 2 == 0
    }

    // MARK: - Fences

    private enum Segment {
        case text(String)
        case code(String)
    }

    /// The fence marker. Three backticks, at the start of a line.
    private static let fence = "```"

    /// Split a body into alternating plain and fenced-code segments.
    ///
    /// A fence must OPEN at the start of a line and CLOSE at the start of a
    /// line, which is what stops a stray triple-backtick in the middle of a
    /// sentence swallowing the rest of the message. An unclosed fence is
    /// not a fence at all: the text stays as it was typed, because somebody
    /// mid-way through writing one should not watch their message turn into
    /// a code block as they type the third backtick.
    private static func segments(of body: String) -> [Segment] {
        guard body.contains(fence) else { return [.text(body)] }
        let lines = body.components(separatedBy: "\n")
        var result: [Segment] = []
        var plain: [String] = []
        var code: [String] = []
        var inFence = false
        for line in lines {
            let opensOrCloses = line.hasPrefix(fence)
            if !inFence, opensOrCloses {
                // The language tag on an opening fence is dropped: nothing
                // here highlights syntax, so it would only be noise.
                inFence = true
                // Keep the newline that ENDED the line before the fence,
                // the mirror of the empty push below. Without it the code
                // welds onto the last word above it — "try this:" followed
                // by a block came out as "try this:let x = 1".
                plain.append("")
                continue
            }
            if inFence, opensOrCloses {
                inFence = false
                result.append(.text(plain.joined(separator: "\n")))
                plain = []
                result.append(.code(code.joined(separator: "\n")))
                code = []
                // Keep the newline that ended the fence, so text after a
                // block does not run onto its last line.
                plain.append("")
                continue
            }
            if inFence {
                code.append(line)
            } else {
                plain.append(line)
            }
        }
        if inFence {
            // Never closed — put it back exactly as it was typed.
            return [.text(body)]
        }
        result.append(.text(plain.joined(separator: "\n")))
        return result.filter { segment in
            if case .text(let text) = segment { return !text.isEmpty }
            return true
        }
    }

    // MARK: - Inline

    /// Foundation's parser, held to inline syntax only.
    ///
    /// `.inlineOnlyPreservingWhitespace` is what keeps line breaks and stops
    /// it re-discovering block structure — without it a blank line becomes a
    /// paragraph break, a multi-line message comes back collapsed, and the
    /// heading and bullet rules above would be applied twice by two parsers
    /// that disagree.
    private static func inline(_ text: String) -> AttributedString {
        var options = AttributedString.MarkdownParsingOptions()
        options.interpretedSyntax = .inlineOnlyPreservingWhitespace
        options.allowsExtendedAttributes = true
        // A half-typed `[label](` must render as the characters it is, not
        // vanish.
        options.failurePolicy = .returnPartiallyParsedIfPossible
        guard let parsed = try? AttributedString(markdown: text, options: options) else {
            return AttributedString(text)
        }
        return parsed
    }
}
