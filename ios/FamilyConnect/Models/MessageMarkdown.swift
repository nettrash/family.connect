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
//  ONLY A BODY THAT ACTUALLY CONTAINS A TABLE (or a rich block, below)
//  becomes a stack of blocks —
//  see `blocks(_:)`, and `MessageBodyBlocks` for how they are drawn. A
//  fenced block stays a monospaced RUN rather than a scrolling box for the
//  same reason: it reads as code, it wraps like text, and it costs the
//  bubble nothing structurally.
//
//  FORMULAS, DIAGRAMS AND GRAPHS (issue #97) are the one other thing that
//  splits a body. A fence tagged `math`, `mermaid`, `dot` (and the rest of
//  docs/protocol.md's table) or `plot`, and display math (`$$…$$` or
//  `\[…\]` on lines of their own), become a `.rich` block the bubble DRAWS
//  — through RichRenderer's hidden web view — rather than sets in type; it
//  splits the text around it exactly as a table does. An INLINE formula
//  (`$…$`, `$$…$$`, `\(…\)` inside a line) stays a run: its characters are
//  exactly what was typed, delimiters and all, and it carries the formula in
//  `InlineMathAttribute`, which is what the bubble swaps for a drawing. The
//  parsing rules are docs/protocol.md's ("What counts, exactly"), and four
//  parsers — this, Kotlin, C# and Rust — are held to them by one table.
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
        /// A formula, a diagram or a graph standing on its own (#97).
        case rich(RichBlock)

        var isTable: Bool {
            if case .table = self { return true }
            return false
        }

        var isRich: Bool {
            if case .rich = self { return true }
            return false
        }
    }

    /// A block a bubble DRAWS rather than sets in type: what to draw it
    /// with, and what to draw (docs/protocol.md, "Rich blocks").
    ///
    /// Until it has been drawn — and for good when it cannot be — the
    /// bubble shows `fallback` in the monospaced face, as it shows a code
    /// block.
    struct RichBlock: Hashable, Sendable {
        /// The job's `kind` word, too (`math` here; an inline formula's job
        /// is `math-inline`).
        enum Kind: String, Hashable, Sendable {
            case math
            case mermaid
            case graphviz
            case plot
        }

        let kind: Kind
        /// Graphviz's layout: `dot` unless the fence named another. Nil for
        /// every other kind.
        let engine: String?
        /// The fence's content exactly as a code block's would be, or a
        /// display formula's content trimmed of spaces, tabs and newlines.
        let source: String
        /// What the bubble shows until the block is drawn: a fence's
        /// content (its fences and tag are not shown, as a code block's are
        /// not), or a display formula AS TYPED, delimiters and all.
        let fallback: String

        init(kind: Kind, engine: String? = nil, source: String, fallback: String? = nil) {
            self.kind = kind
            self.engine = engine
            self.source = source
            self.fallback = fallback ?? source
        }
    }

    /// An inline formula's run: the formula, and which one it is.
    ///
    /// The ordinal is not decoration. `AttributedString` merges adjacent
    /// runs whose attributes are equal, so `\(a\)\(a\)` would come back as
    /// ONE run holding both formulas' characters — and be drawn once. Each
    /// formula's ordinal (its place in the string it was parsed from) keeps
    /// every run its own.
    struct InlineMath: Hashable, Sendable {
        let source: String
        let ordinal: Int
    }

    /// The attribute an inline formula's run carries (see `InlineMath`).
    /// Custom, so SwiftUI's `Text` ignores it and draws the characters —
    /// the source, delimiters and all — until a drawing replaces them.
    enum InlineMathAttribute: AttributedStringKey {
        typealias Value = InlineMath
        static let name = "FamilyConnect.inlineMath"
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
    /// The ONLY things that split a body are a table and a rich block;
    /// everything else is a run inside a text block.
    static func blocks(_ body: String) -> [Block] {
        parse(body, structured: true)
    }

    /// Render a message body as one attributed string.
    ///
    /// Tables are deliberately NOT recognised here — a caller holding one
    /// string has nowhere to put a grid, so a pipe table renders as the
    /// rows that were typed. Nor are rich blocks: a rich fence is the code
    /// block an older client draws, and display math is the text it was
    /// typed as (the same answer as the Rust port's `render`). Inline
    /// formulas ARE runs here, carrying `InlineMathAttribute`, so the link
    /// detector can keep out of them. This is what the link detector, the
    /// preview card and the tests index; `blocks(_:)` is what the balloon
    /// draws.
    static func render(_ body: String) -> AttributedString {
        var out = AttributedString()
        for block in parse(body, structured: false) {
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

    /// Fences first, then display math, then line structure inside each
    /// plain segment: the whole point of a code fence is that what is
    /// inside it is not markup, so nothing below ever sees a fenced line —
    /// and display math is taken BEFORE tables (docs/protocol.md, rule 2),
    /// so a formula's lines are never a table's.
    ///
    /// `structured` is false for the flat `render`, which recognises
    /// neither tables nor rich blocks.
    private static func parse(_ body: String, structured: Bool) -> [Block] {
        var out: [Block] = []
        var current = AttributedString()

        func flush() {
            guard !current.characters.isEmpty else { return }
            out.append(.text(current))
            current = AttributedString()
        }

        func appendText(_ text: String) {
            // Newlines BETWEEN pieces, never around a table: the lines
            // a table swallowed take their separators with them, or a
            // grid would leave a blank line above and below itself.
            var needsSeparator = false
            for piece in pieces(of: text, tables: structured) {
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

        for segment in segments(of: body, rich: structured) {
            switch segment {
            case .code(let code):
                var run = AttributedString(code)
                run.font = .system(.body, design: .monospaced)
                current.append(run)
            case .rich(let rich):
                // Stands alone, exactly as a table does (rule 3).
                flush()
                out.append(.rich(rich))
            case .text(let text):
                guard structured else {
                    appendText(text)
                    continue
                }
                for part in displayMath(in: text) {
                    switch part {
                    case .text(let plain):
                        appendText(plain)
                    case .rich(let rich):
                        flush()
                        out.append(.rich(rich))
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

    // MARK: - Display math

    private enum DisplayPart {
        case text(String)
        case rich(RichBlock)
    }

    /// Split one fence-free segment around its display formulas
    /// (docs/protocol.md, rule 2).
    ///
    /// A line whose first characters after spaces and tabs are `$$` or
    /// `\[` opens one; it closes at the FIRST matching `$$` or `\]` after
    /// the opener, on that line or a later one — and only if nothing but
    /// spaces and tabs follows it. The first closer decides: one with words
    /// after it is not searched past (`$$x$$ is it` is a line of text with
    /// an inline formula in it, and so is anything that reads like one).
    /// Without such a closer the lines are text, as typed.
    private static func displayMath(in segment: String) -> [DisplayPart] {
        guard segment.contains("$$") || segment.contains("\\[") else { return [.text(segment)] }
        let lines = segment.components(separatedBy: "\n")
        var parts: [DisplayPart] = []
        var plain: [String] = []
        var index = 0
        while index < lines.count {
            if let found = displayFormula(at: index, in: lines) {
                if !plain.isEmpty {
                    let text = plain.joined(separator: "\n")
                    if !text.isEmpty { parts.append(.text(text)) }
                    plain = []
                }
                parts.append(.rich(found.block))
                index = found.end
                continue
            }
            plain.append(lines[index])
            index += 1
        }
        if !plain.isEmpty {
            let text = plain.joined(separator: "\n")
            if !text.isEmpty { parts.append(.text(text)) }
        }
        return parts
    }

    /// The display formula opening on `lines[start]`, and the index of the
    /// first line after it — or nil when that line opens none.
    ///
    /// Everything is measured in unicode SCALARS, never characters: a
    /// combining mark typed straight after a `$` would otherwise weld onto
    /// it as one Character and hide it, where every other port (bytes,
    /// UTF-16) still sees the dollar.
    private static func displayFormula(at start: Int, in lines: [String]) -> (block: RichBlock, end: Int)? {
        let first = Array(lines[start].unicodeScalars)
        var lead = 0
        while lead < first.count, first[lead] == " " || first[lead] == "\t" { lead += 1 }
        let closer: [Unicode.Scalar]
        if hasPrefix(first, ["$", "$"], at: lead) {
            closer = ["$", "$"]
        } else if hasPrefix(first, ["\\", "["], at: lead) {
            closer = ["\\", "]"]
        } else {
            return nil
        }
        var row = start
        var from = lead + 2
        var source = String.UnicodeScalarView()
        var shown: [String] = []
        while true {
            let line = row == start ? first : Array(lines[row].unicodeScalars)
            if let close = find(closer, in: line, from: from) {
                let after = line[(close + closer.count)...]
                guard after.allSatisfy({ $0 == " " || $0 == "\t" }) else { return nil }
                source.append(contentsOf: line[from..<close])
                shown.append(String(String.UnicodeScalarView(line[(row == start ? lead : 0)..<(close + closer.count)])))
                break
            }
            source.append(contentsOf: line[from...])
            source.append("\n")
            shown.append(String(String.UnicodeScalarView(line[(row == start ? lead : 0)...])))
            row += 1
            guard row < lines.count else { return nil }
            from = 0
        }
        let trimmed = String(source).trimmingCharacters(in: CharacterSet(charactersIn: " \t\n"))
        guard !trimmed.isEmpty else { return nil }
        return (RichBlock(kind: .math, source: trimmed, fallback: shown.joined(separator: "\n")), row + 1)
    }

    private static func hasPrefix(_ scalars: [Unicode.Scalar], _ prefix: [Unicode.Scalar], at index: Int) -> Bool {
        guard index + prefix.count <= scalars.count else { return false }
        for offset in 0..<prefix.count where scalars[index + offset] != prefix[offset] { return false }
        return true
    }

    private static func find(_ needle: [Unicode.Scalar], in scalars: [Unicode.Scalar], from: Int) -> Int? {
        var index = from
        while index + needle.count <= scalars.count {
            if hasPrefix(scalars, needle, at: index) { return index }
            index += 1
        }
        return nil
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
        case rich(RichBlock)
    }

    /// The fence marker. Three backticks, at the start of a line.
    private static let fence = "```"

    /// Split a body into alternating plain, fenced-code and (when `rich`)
    /// rich segments.
    ///
    /// A fence must OPEN at the start of a line and CLOSE at the start of a
    /// line, which is what stops a stray triple-backtick in the middle of a
    /// sentence swallowing the rest of the message. An unclosed fence is
    /// not a fence at all: the text stays as it was typed, because somebody
    /// mid-way through writing one should not watch their message turn into
    /// a code block as they type the third backtick.
    ///
    /// A RICH fence (docs/protocol.md, rule 1) is the same fence with a tag
    /// from the protocol's table. It stands alone, as a table does: the
    /// newlines around it go with it, where a code block's are kept so its
    /// code does not weld onto the words around it.
    private static func segments(of body: String, rich: Bool) -> [Segment] {
        guard body.contains(fence) else { return [.text(body)] }
        let lines = body.components(separatedBy: "\n")
        var result: [Segment] = []
        var plain: [String] = []
        var code: [String] = []
        var inFence = false
        /// What the open fence draws, when it is a rich one.
        var richTag: (kind: RichBlock.Kind, engine: String?)?
        for line in lines {
            let opensOrCloses = line.hasPrefix(fence)
            if !inFence, opensOrCloses {
                inFence = true
                richTag = rich ? Self.richTag(info: fenceInfo(line)) : nil
                if richTag == nil {
                    // The language tag on an ordinary fence is dropped:
                    // nothing here highlights syntax, so it would only be
                    // noise.
                    //
                    // Keep the newline that ENDED the line before the
                    // fence, the mirror of the empty push below. Without it
                    // the code welds onto the last word above it — "try
                    // this:" followed by a block came out as "try this:let
                    // x = 1".
                    plain.append("")
                }
                continue
            }
            if inFence, opensOrCloses {
                inFence = false
                result.append(.text(plain.joined(separator: "\n")))
                plain = []
                let content = code.joined(separator: "\n")
                code = []
                if let tag = richTag {
                    result.append(.rich(RichBlock(kind: tag.kind, engine: tag.engine, source: content)))
                    richTag = nil
                } else {
                    result.append(.code(content))
                    // Keep the newline that ended the fence, so text after
                    // a block does not run onto its last line.
                    plain.append("")
                }
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

    /// The info string of a fence's opening line: what follows ALL its
    /// backticks (a fence opened with four still has a tag after the
    /// fourth), trimmed of spaces and tabs.
    private static func fenceInfo(_ line: String) -> String {
        String(line.drop { $0 == "`" }).trimmingCharacters(in: CharacterSet(charactersIn: " \t"))
    }

    /// What a fence's tag draws, if anything: the info string's first word
    /// — up to a space or a tab — lower-cased in ASCII, against
    /// docs/protocol.md's table.
    ///
    /// ASCII lower-casing on purpose: Swift's `lowercased()` is Unicode's,
    /// which would also fold a Kelvin sign (U+212A) to `k` — four ports,
    /// four ideas of case, unless the one rule is the ASCII one.
    static func richTag(info: String) -> (kind: RichBlock.Kind, engine: String?)? {
        let word = info.prefix { $0 != " " && $0 != "\t" }
        let lowered = String(String.UnicodeScalarView(word.unicodeScalars.map { scalar in
            ("A"..."Z").contains(scalar) ? Unicode.Scalar(scalar.value + 32)! : scalar
        }))
        switch lowered {
        case "math", "latex", "tex": return (.math, nil)
        case "mermaid": return (.mermaid, nil)
        case "dot", "graphviz", "gv": return (.graphviz, "dot")
        case "neato", "circo", "fdp", "sfdp", "twopi", "osage", "patchwork": return (.graphviz, lowered)
        case "plot": return (.plot, nil)
        default: return nil
        }
    }

    // MARK: - Inline

    /// Foundation's parser, held to inline syntax only — with inline
    /// formulas taken out first and put back after.
    ///
    /// `.inlineOnlyPreservingWhitespace` is what keeps line breaks and stops
    /// it re-discovering block structure — without it a blank line becomes a
    /// paragraph break, a multi-line message comes back collapsed, and the
    /// heading and bullet rules above would be applied twice by two parsers
    /// that disagree.
    ///
    /// FORMULAS. Foundation's parser cannot be taught a new rule, and a
    /// formula's insides are not markdown (`*see $a*b$*`: the formula is
    /// `a*b`, inside the emphasis). So each formula `inlineMath(in:)` finds
    /// is swapped for a PLACEHOLDER the parser passes through untouched,
    /// and swapped back afterwards — keeping whatever emphasis, link or
    /// strikethrough the placeholder picked up, adding the formula
    /// attribute and the code face. The placeholder is bracketed by
    /// Unicode punctuation (`⸨…⸩`) because a formula's delimiters, `$` and
    /// `\`, are punctuation to CommonMark's flanking rules, and a delimiter
    /// run next to a formula must flank exactly as it would next to a `$`.
    private static func inline(_ text: String) -> AttributedString {
        let formulas = inlineMath(in: text)
        guard !formulas.isEmpty else { return foundationInline(text) }

        var substituted = ""
        var cursor = text.startIndex
        for (ordinal, formula) in formulas.enumerated() {
            substituted.unicodeScalars.append(contentsOf: text.unicodeScalars[cursor..<formula.whole.lowerBound])
            substituted += placeholder(ordinal)
            cursor = formula.whole.upperBound
        }
        substituted.unicodeScalars.append(contentsOf: text.unicodeScalars[cursor...])

        var parsed = foundationInline(substituted)
        for (ordinal, formula) in formulas.enumerated() {
            guard let found = parsed.range(of: placeholder(ordinal)) else { continue }
            let picked = parsed[found].runs.first?.attributes ?? AttributeContainer()
            let raw = String(String.UnicodeScalarView(text.unicodeScalars[formula.whole]))
            var run = AttributedString(raw, attributes: picked)
            // In a link's LABEL a formula is flattened to the text it was
            // typed as: a drawing inside a tappable run is a second thing
            // to tap that goes somewhere the reader cannot see, and every
            // port answers it the same way (the coordinator's reading of
            // docs/protocol.md, rule 4).
            guard picked.link == nil else {
                parsed.replaceSubrange(found, with: run)
                continue
            }
            run[InlineMathAttribute.self] = InlineMath(source: formula.source, ordinal: ordinal)
            // The code face, until it is drawn — and for good if it cannot
            // be (docs/protocol.md, "How it is drawn").
            run.inlinePresentationIntent = (picked.inlinePresentationIntent ?? []).union(.code)
            parsed.replaceSubrange(found, with: run)
        }
        return parsed
    }

    /// Placeholder `ordinal`: `⸨`, the ordinal in Private Use digits, `⸩`.
    /// Private Use code points never come from a keyboard; a body that
    /// somehow holds the digits used here is parsed without formulas
    /// rather than risk one placeholder matching another's text
    /// (`inlineMath(in:)` checks).
    private static func placeholder(_ ordinal: Int) -> String {
        var out = "\u{2E28}"
        for digit in String(ordinal).unicodeScalars {
            out.unicodeScalars.append(Unicode.Scalar(placeholderDigitBase + digit.value - 48)!)
        }
        out += "\u{2E29}"
        return out
    }

    private static let placeholderDigitBase: UInt32 = 0xE000

    private static func foundationInline(_ text: String) -> AttributedString {
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

    // MARK: - Inline math

    /// One inline formula: where it is in the text (delimiters included)
    /// and what it says.
    struct InlineFormula: Equatable {
        let whole: Range<String.Index>
        let source: String
    }

    /// Every inline formula in `text`, in order (docs/protocol.md, rule 4).
    ///
    /// One pass, left to right, over unicode SCALARS (see
    /// `displayFormula`), shaped like the CommonMark inline parser it runs
    /// ahead of, so it agrees with it about what is code and what is a
    /// link:
    ///
    /// - a backslash escapes the next character, so an escaped `$` or `\(`
    ///   is never an opener ("preceded by an odd number of backslashes");
    /// - a code span is skipped whole — formulas come AFTER code spans —
    ///   and a formula whose search would cross the start of one is not a
    ///   formula;
    /// - a link's destination (after `](`) and an autolink are skipped
    ///   raw, as CommonMark scans them, so a `$` in a URL is never math;
    /// - at each position the three forms are tried in the protocol's
    ///   order of precedence: `$$…$$`, `\(…\)`, `$…$`.
    static func inlineMath(in text: String) -> [InlineFormula] {
        guard text.contains("$") || text.contains("\\(") else { return [] }
        let scalars = Array(text.unicodeScalars)
        // See `placeholder`.
        if scalars.contains(where: { (placeholderDigitBase..<placeholderDigitBase + 10).contains($0.value) }) {
            return []
        }
        var indices = Array(text.unicodeScalars.indices)
        indices.append(text.endIndex)
        let scanner = MathScanner(scalars: scalars)
        return scanner.formulas().map { found in
            InlineFormula(
                whole: indices[found.start]..<indices[found.end],
                source: String(String.UnicodeScalarView(scalars[found.inner..<found.close])))
        }
    }

    /// The scanner behind `inlineMath(in:)`, over scalar offsets.
    private struct MathScanner {
        let scalars: [Unicode.Scalar]

        struct Found {
            let start: Int
            let inner: Int
            let close: Int
            let end: Int
        }

        func formulas() -> [Found] {
            var found: [Found] = []
            var brackets = 0
            var index = 0
            let count = scalars.count
            while index < count {
                switch scalars[index] {
                case "\\":
                    if index + 1 < count, scalars[index + 1] == "(",
                       let close = closer(["\\", ")"], from: index + 2) {
                        found.append(Found(start: index, inner: index + 2, close: close, end: close + 2))
                        index = close + 2
                    } else {
                        // An escape: this and the next character are text.
                        index += 2
                    }
                case "`":
                    let run = backtickRun(at: index)
                    if let end = codeSpanEnd(at: index, run: run) {
                        index = end
                    } else {
                        index += run
                    }
                case "[":
                    brackets += 1
                    index += 1
                case "]":
                    index += 1
                    if brackets > 0 {
                        brackets -= 1
                        if index < count, scalars[index] == "(", let end = linkTail(from: index + 1) {
                            index = end
                        }
                    }
                case "<":
                    index = autolinkEnd(at: index) ?? index + 1
                case "$":
                    if index + 1 < count, scalars[index + 1] == "$",
                       let close = closer(["$", "$"], from: index + 2) {
                        found.append(Found(start: index, inner: index + 2, close: close, end: close + 2))
                        index = close + 2
                    } else if let close = singleDollarCloser(opener: index) {
                        found.append(Found(start: index, inner: index + 1, close: close, end: close + 1))
                        index = close + 1
                    } else {
                        index += 1
                    }
                default:
                    index += 1
                }
            }
            return found
        }

        /// An ASCII letter, an ASCII digit or `_` — ASCII on purpose, so four
        /// languages with four ideas of a letter give one answer.
        private func isWord(_ scalar: Unicode.Scalar) -> Bool {
            scalar == "_" || ("a"..."z").contains(scalar) || ("A"..."Z").contains(scalar)
                || ("0"..."9").contains(scalar)
        }

        /// Where the first `closer` (`$$` or `\)`) at or after `from` starts,
        /// on the same line, with something other than spaces and tabs
        /// before it — or nil. The FIRST one decides: `$$ $$` is not a
        /// formula, and is not searched past.
        private func closer(_ closer: [Unicode.Scalar], from: Int) -> Int? {
            var index = from
            while index < scalars.count {
                if hasPrefix(scalars, closer, at: index) {
                    let content = scalars[from..<index]
                    return content.contains { $0 != " " && $0 != "\t" } ? index : nil
                }
                switch scalars[index] {
                case "\n":
                    return nil
                case "`":
                    if opensCodeSpan(at: index) { return nil }
                    index += backtickRun(at: index)
                default:
                    index += 1
                }
            }
            return nil
        }

        /// The closer of a `$…$` formula opening at `opener`, or nil.
        ///
        /// The opener must not follow a word character or a `$`, and must
        /// be followed by something other than a space, a tab or a `$`. A
        /// closer is a later `$` on the same line with neither a space, a
        /// tab nor a backslash before it, and neither a word character nor
        /// a `$` after it; a candidate that fails is skipped and the search
        /// goes on. That guard is what keeps money as money: in
        /// `$5 and $10` the only candidate has a space before it.
        private func singleDollarCloser(opener: Int) -> Int? {
            if opener > 0, isWord(scalars[opener - 1]) || scalars[opener - 1] == "$" { return nil }
            guard opener + 1 < scalars.count else { return nil }
            let next = scalars[opener + 1]
            if next == " " || next == "\t" || next == "$" { return nil }
            var index = opener + 1
            while index < scalars.count {
                switch scalars[index] {
                case "\n":
                    return nil
                case "`":
                    if opensCodeSpan(at: index) { return nil }
                    index += backtickRun(at: index)
                    continue
                case "$":
                    let before = scalars[index - 1]
                    let after: Unicode.Scalar? = index + 1 < scalars.count ? scalars[index + 1] : nil
                    if before != " ", before != "\t", before != "\\",
                       !(after.map { isWord($0) || $0 == "$" } ?? false) {
                        return index
                    }
                default:
                    break
                }
                index += 1
            }
            return nil
        }

        private func backtickRun(at index: Int) -> Int {
            var end = index
            while end < scalars.count, scalars[end] == "`" { end += 1 }
            return end - index
        }

        /// The end of the code span opened by the `run` backticks at
        /// `index`: CommonMark closes it at the next run of EXACTLY that
        /// length, and a run that has none is just backticks.
        private func codeSpanEnd(at index: Int, run: Int) -> Int? {
            var at = index + run
            while at < scalars.count {
                if scalars[at] == "`" {
                    let length = backtickRun(at: at)
                    if length == run { return at + length }
                    at += length
                } else {
                    at += 1
                }
            }
            return nil
        }

        /// Whether the backtick run at `index`, met while a formula looks
        /// for its closer, would open a code span — which the formula must
        /// not cross, because code spans come first. After an odd number of
        /// backslashes the run's first backtick is escaped, and the rest is
        /// what might open.
        private func opensCodeSpan(at index: Int) -> Bool {
            var backslashes = 0
            var back = index - 1
            while back >= 0, scalars[back] == "\\" {
                backslashes += 1
                back -= 1
            }
            var start = index
            var run = backtickRun(at: index)
            if backslashes % 2 == 1 {
                start += 1
                run -= 1
            }
            return run > 0 && codeSpanEnd(at: start, run: run) != nil
        }

        /// The end of an inline link's tail — `destination "title")` — that
        /// starts at `from` (just past `](`), or nil when it is not one.
        /// Scanned raw, as CommonMark scans it: whatever is in a
        /// destination is never math.
        private func linkTail(from: Int) -> Int? {
            var index = skipSpace(from)
            if index < scalars.count, scalars[index] == "<" {
                index += 1
                while index < scalars.count, scalars[index] != ">" {
                    let scalar = scalars[index]
                    if scalar == "\n" || scalar == "\r" || scalar == "<" { return nil }
                    index += scalar == "\\" ? 2 : 1
                }
                guard index < scalars.count else { return nil }
                index += 1
            } else {
                var depth = 0
                while index < scalars.count {
                    let scalar = scalars[index]
                    if scalar == "\\" {
                        index += 2
                        continue
                    }
                    if scalar.value <= 0x20 || scalar.value == 0x7F { break }
                    if scalar == "(" { depth += 1 }
                    if scalar == ")" {
                        if depth == 0 { break }
                        depth -= 1
                    }
                    index += 1
                }
                guard depth == 0 else { return nil }
            }
            index = skipSpace(index)
            guard index < scalars.count else { return nil }
            if scalars[index] == "\"" || scalars[index] == "'" || scalars[index] == "(" {
                let close: Unicode.Scalar = scalars[index] == "(" ? ")" : scalars[index]
                index += 1
                while index < scalars.count, scalars[index] != close {
                    index += scalars[index] == "\\" ? 2 : 1
                }
                guard index < scalars.count else { return nil }
                index = skipSpace(index + 1)
            }
            guard index < scalars.count, scalars[index] == ")" else { return nil }
            return index + 1
        }

        /// Spaces and tabs, and at most one line break, as between a link's
        /// parts.
        private func skipSpace(_ from: Int) -> Int {
            var index = from
            var breaks = 0
            while index < scalars.count {
                let scalar = scalars[index]
                if scalar == " " || scalar == "\t" {
                    index += 1
                } else if scalar == "\n", breaks == 0 {
                    breaks += 1
                    index += 1
                } else {
                    break
                }
            }
            return index
        }

        /// The end of an autolink `<scheme:…>` at `index`, or nil.
        private func autolinkEnd(at index: Int) -> Int? {
            var at = index + 1
            guard at < scalars.count, ("a"..."z").contains(scalars[at]) || ("A"..."Z").contains(scalars[at])
            else { return nil }
            let schemeStart = at
            while at < scalars.count, isWord(scalars[at]) && scalars[at] != "_"
                || scalars[at] == "+" || scalars[at] == "." || scalars[at] == "-" {
                at += 1
            }
            let schemeLength = at - schemeStart
            guard (2...32).contains(schemeLength), at < scalars.count, scalars[at] == ":" else { return nil }
            at += 1
            while at < scalars.count, scalars[at] != ">" {
                let scalar = scalars[at]
                if scalar.value <= 0x20 || scalar == "<" { return nil }
                at += 1
            }
            return at < scalars.count ? at + 1 : nil
        }
    }
}
