//! Markdown in a chat bubble: the small subset people actually type.
//!
//! A port of the Apple client's `MessageMarkdown.swift`, which is the ORACLE:
//! the same message has to read the same on iPhone, Mac, Android and the web,
//! so every rule here is the Swift rule, including the ones that look odd.
//!
//! `**bold**`, `*italic*`, `` `code` ``, `~~strike~~`, `[label](url)`,
//! `\escapes` and ```` ```fenced``` ```` blocks inline; `# ` through `### `
//! headings, `- `/`* `/`+ ` bullets and GFM pipe tables at the start of a
//! line.
//!
//! HEADINGS AND BULLETS ARE RUNS, NOT BLOCKS, exactly as on Apple: a heading
//! changes a font and a bullet replaces two characters, so both live inside
//! the one [`Text`] a bubble draws. Only a body that actually contains a table
//! becomes more than one [`Block`]. That is what keeps every offset-based pass
//! downstream — link detection, the `@ai` highlight, member mentions — working
//! over ONE string per text block, which is the order the Apple client runs
//! them in (see "Composition" below).
//!
//! This is a RENDERING convention, not a wire format (docs/protocol.md, "A body
//! is plain text on the wire"): the server never parses any of it, and a
//! client that renders none of it shows the source.
//!
//! # Where the rules come from
//!
//! The structure (fences, headings, bullets, tables) is MessageMarkdown.swift,
//! line for line. The inline half on Apple is Foundation's
//! `AttributedString(markdown:)` with `.inlineOnlyPreservingWhitespace` — which
//! is Apple's fork of cmark-gfm, so this file carries a port of that parser
//! (emphasis with CommonMark's delimiter stack, code spans, links, images,
//! autolinks, raw HTML, entities, GFM strikethrough and the GFM autolink
//! extension) plus the handful of things Foundation does on top of it. Every
//! one of those was measured against Foundation on macOS 26 rather than taken
//! from a spec, because the fork is not any one published cmark version. The
//! surprising ones, each pinned by a test:
//!
//! - bare URLs, `www.` hosts and email addresses become links IN THE PARSER
//!   (GFM autolink), before any link detector runs;
//! - a link's label is FLATTENED to plain text: `[*a*](b)` is an unstyled "a";
//! - `[label]: destination` lines at the start of a run of ordinary lines (the
//!   body's start, or right after a heading or bullet line) are reference
//!   definitions and VANISH when a newline follows them;
//! - `^[text](anything)` is Apple's attribute syntax: the markers disappear,
//!   and a `[label]` straight after it is swallowed;
//! - a span nested in one of its own kind switches the style off for the rest
//!   of the outer one (`*a *b* c*` draws " c" upright), and `~` is invisible
//!   to the `*`/`_` flanking rules;
//! - `\r\n` and `\r` become `\n` before parsing; NUL becomes U+FFFD;
//! - raw HTML is kept as the characters it is, never interpreted;
//! - `![alt](src)` draws its alt text (or U+FFFC when there is none).
//!
//! What the VIEW adds, and so is not in this AST (MessageBodyView.swift): a
//! table's header row is drawn bold above a one-pixel rule (119, 103-106),
//! columns share the width evenly and cells wrap (125, 131), and a body that
//! ends in a table gives the streaming cursor a line of its own (56-61).
//!
//! # Composition (what runs after this, and on what)
//!
//! MessageLinks.swift decorates the output in this order, PER TEXT BLOCK, over
//! that block's rendered string ([`Text::plain`]) — never over the raw body:
//!
//! 1. markdown destinations without a scheme gain `https://`
//!    (`MessageLinks.normalized`, MessageLinks.swift:243-247, 402-406);
//! 2. the platform link/phone detector runs over the whole rendered string —
//!    inline code, fenced code, headings and bullets included — and a match
//!    that overlaps ANY existing link is dropped (MessageLinks.swift:249-271);
//! 3. every link run is underlined (and white on an own bubble), 273-278;
//! 4. `@ai` and a leading `/draw` are marked by REPLACING the run's inline
//!    style with bold — so `*@ai*` loses its italic and `` `@ai` `` its code
//!    face on those three characters — inside links and code too; `/draw` is
//!    decided from the RAW body and only positioned in the rendered one, and
//!    only the first block may carry it (279, 374-394, 94-101);
//! 5. member mentions last, the same bold replacement plus a private
//!    `fcmember://` link, skipped wherever any link already covers the range
//!    (280, 290-309).
//!
//! Table cells are never decorated: no links (the cell renderer strips them),
//! no detector, no `@ai` (MessageLinks.swift:30-34, 102-103). The link preview
//! card is taken from [`render`] — the FLAT render, where a table is still its
//! typed rows — so a URL typed only in a cell still gets a card
//! (MessageLinks.swift:135-138, 154).

use std::collections::HashMap;

use unicode_segmentation::UnicodeSegmentation;

// MARK: - The shape a bubble draws

/// One piece of a laid-out message body.
///
/// A body with no table is exactly ONE [`Block::Text`] — the invariant the
/// whole bubble rests on (one laid-out string, one offset space). Only a
/// table splits a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// A run of text laid out as one string: headings, bullets and fenced
    /// code included. Line breaks are `\n` characters inside it.
    Text(Text),
    /// A GFM pipe table.
    Table(Table),
}

impl Block {
    /// Swift's `Block.isTable`.
    pub fn is_table(&self) -> bool {
        matches!(self, Block::Table(_))
    }
}

/// A laid-out string: its characters and what is drawn over them.
///
/// Spans are maximal — two neighbours never share a style — which is the
/// same shape `AttributedString.runs` has.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Text {
    pub spans: Vec<Span>,
}

impl Text {
    /// The characters as drawn, markup removed. THIS is what every
    /// offset-based pass indexes — never the raw body.
    pub fn plain(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }

    /// No characters at all.
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Append `text` drawn with `style`, merging into the last span when the
    /// style is the same, the way `AttributedString` coalesces runs.
    fn push(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.spans.last_mut() {
            if last.style == style {
                last.text.push_str(text);
                return;
            }
        }
        self.spans.push(Span {
            text: text.to_string(),
            style,
        });
    }

    fn append(&mut self, other: Text) {
        for span in other.spans {
            self.push(&span.text, span.style);
        }
    }

    /// Swift's `run.font = …` over the whole string.
    fn set_font(&mut self, font: Font) {
        let spans = std::mem::take(&mut self.spans);
        for mut span in spans {
            span.style.font = font;
            self.push(&span.text, span.style);
        }
    }

    /// Swift's `literal[run.range].link = nil` over every run.
    fn strip_links(&mut self) {
        let spans = std::mem::take(&mut self.spans);
        for mut span in spans {
            span.style.link = None;
            self.push(&span.text, span.style);
        }
    }
}

/// Some characters and what is drawn over them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

/// Everything Apple draws over a run, in Foundation's own vocabulary.
///
/// The booleans are the bits of `InlinePresentationIntent`; `link` and
/// `image` are the `.link` and `.imageURL` attributes; `font` is the one
/// attribute MessageMarkdown.swift sets itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Style {
    /// `*x*` / `_x_` — `.emphasized`, drawn italic.
    pub emphasis: bool,
    /// `**x**` / `__x__` — `.stronglyEmphasized`, drawn bold.
    pub strong: bool,
    /// `` `x` `` — `.code`, drawn monospaced.
    pub code: bool,
    /// `~~x~~` or `~x~` — `.strikethrough`.
    pub strikethrough: bool,
    /// The `\n` of a backslash hard break — `.lineBreak`. It changes nothing
    /// that is drawn; it is here so a run boundary Apple has is not lost.
    pub line_break: bool,
    /// Raw HTML such as `<b>` — `.inlineHTML`. DRAWN AS ITS OWN CHARACTERS:
    /// Apple never interprets it, so a web renderer must never either.
    pub html: bool,
    /// A tappable destination. See [`Link`].
    pub link: Option<Link>,
    /// `![alt](src)`: Apple carries the source but draws only the alt text.
    pub image: Option<Link>,
    /// The face the whole run is set in.
    pub font: Font,
}

/// A destination exactly as the parser produced it: entities and backslash
/// escapes resolved, NOT percent-encoded and NOT given a scheme.
///
/// Apple turns it into a `URL` (percent-encoding spaces and non-ASCII, IDNA
/// for hosts); a destination `URL(string:)` would refuse is never a link
/// here either — see `foundation_accepts_url`. Adding `https://` to a
/// scheme-less one is the next pass's job (`MessageLinks.normalized`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub destination: String,
    /// `[a](b "title")` — Foundation's `.alternateDescription`, not drawn.
    pub title: Option<String>,
}

/// The face a run is set in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Font {
    /// The bubble's own body font.
    #[default]
    Body,
    /// A `#`, `##` or `###` line: 1 is the largest. Apple draws `.title2`
    /// bold, `.title3` bold and `.headline`; Android 1.29 / 1.18 / 1.00 em.
    /// The ladder is the contract, not the points.
    Heading(u8),
    /// The contents of a fenced block: body size, monospaced. A fence is a
    /// RUN, not a box — it wraps like text (MessageMarkdown.swift:24-28).
    Monospaced,
}

/// A GFM pipe table, rendered cell by cell.
///
/// Every row has exactly `alignments.len()` cells: short rows are padded
/// with empty cells and long ones cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// One per column, from the delimiter row — the authority on how many
    /// columns there are.
    pub alignments: Vec<ColumnAlignment>,
    /// Drawn bold above a rule.
    pub header: Vec<Text>,
    pub rows: Vec<Vec<Text>>,
}

impl Table {
    /// Swift's `columnCount`.
    pub fn column_count(&self) -> usize {
        self.alignments.len()
    }

    /// Swift's `alignment(_:)`: leading for a column past the last one.
    pub fn alignment(&self, column: usize) -> ColumnAlignment {
        self.alignments
            .get(column)
            .copied()
            .unwrap_or(ColumnAlignment::Leading)
    }
}

/// `:---` / `:--:` / `---:` / `---`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnAlignment {
    Leading,
    Center,
    Trailing,
}

// MARK: - Blocks (MessageMarkdown.swift)

/// A message body as the blocks it lays out in — what a bubble draws.
///
/// The ONLY thing that splits a body is a table; everything else is a run
/// inside a text block. Never empty: an empty body is one empty text block.
pub fn blocks(body: &str) -> Vec<Block> {
    parse(body, true)
}

/// A message body as ONE string, tables NOT recognised — a pipe table comes
/// back as the rows that were typed.
///
/// This is what the link preview card and the Swift tests index; [`blocks`]
/// is what the balloon draws.
pub fn render(body: &str) -> Text {
    let mut out = Text::default();
    for block in parse(body, false) {
        if let Block::Text(text) = block {
            out.append(text);
        }
    }
    out
}

/// A run of lines that renders as text, or a table that does not.
enum Piece {
    Lines(Text),
    Table(Table),
}

/// Fences first, then line structure inside each plain segment: the whole
/// point of a fence is that what is inside it is not markup.
fn parse(body: &str, tables: bool) -> Vec<Block> {
    let mut out = Vec::new();
    let mut current = Text::default();
    for segment in segments(body) {
        match segment {
            Segment::Code(code) => {
                let style = Style {
                    font: Font::Monospaced,
                    ..Style::default()
                };
                current.push(&code, style);
            }
            Segment::Text(text) => {
                // Newlines BETWEEN pieces, never around a table: the lines a
                // table swallowed take their separators with them.
                let mut needs_separator = false;
                for piece in pieces(&text, tables) {
                    match piece {
                        Piece::Lines(rendered) => {
                            if needs_separator {
                                current.push("\n", Style::default());
                            }
                            current.append(rendered);
                            needs_separator = true;
                        }
                        Piece::Table(table) => {
                            if !current.is_empty() {
                                out.push(Block::Text(std::mem::take(&mut current)));
                            }
                            out.push(Block::Table(table));
                            needs_separator = false;
                        }
                    }
                }
            }
        }
    }
    if !current.is_empty() {
        out.push(Block::Text(current));
    }
    // An empty body is still one text block: callers switch on the count.
    if out.is_empty() {
        out.push(Block::Text(Text::default()));
    }
    out
}

/// Split one fence-free segment into the pieces it lays out in.
///
/// Consecutive ORDINARY lines are inline-parsed together, so emphasis can
/// span a line break and a message with no markers comes back as typed. Only
/// a heading or a bullet line is parsed on its own.
///
/// The split is on U+000A only — Foundation's `components(separatedBy:)`
/// splits a `\r\n` in two, leaving the `\r` on the line, where the inline
/// parser later turns it into a second `\n`. That is Apple's behaviour, so it
/// is this one's.
fn pieces(segment: &str, tables: bool) -> Vec<Piece> {
    let lines: Vec<&str> = segment.split('\n').collect();
    let mut pieces = Vec::new();
    let mut plain: Vec<&str> = Vec::new();
    let mut index = 0;

    fn flush_plain(plain: &mut Vec<&str>, pieces: &mut Vec<Piece>) {
        if plain.is_empty() {
            return;
        }
        pieces.push(Piece::Lines(inline(&plain.join("\n"))));
        plain.clear();
    }

    while index < lines.len() {
        let line = lines[index];
        if let Some(heading) = heading(line) {
            flush_plain(&mut plain, &mut pieces);
            let mut run = inline(heading.content);
            run.set_font(Font::Heading(heading.level));
            pieces.push(Piece::Lines(run));
            index += 1;
            continue;
        }
        if let Some(bullet) = bullet(line) {
            flush_plain(&mut plain, &mut pieces);
            // The indent is copied verbatim, which gives visually nested
            // lists without a parser that can mis-nest one.
            let mut run = Text::default();
            run.push(&format!("{}• ", bullet.indent), Style::default());
            run.append(inline(bullet.content));
            pieces.push(Piece::Lines(run));
            index += 1;
            continue;
        }
        if tables {
            if let Some((table, end)) = table(index, &lines) {
                flush_plain(&mut plain, &mut pieces);
                pieces.push(Piece::Table(table));
                index = end;
                continue;
            }
        }
        plain.push(line);
        index += 1;
    }
    flush_plain(&mut plain, &mut pieces);
    pieces
}

// MARK: Swift's string semantics

/// Swift's `Character == "x"` for one of the ASCII markup characters.
///
/// A Swift `Character` is an extended grapheme CLUSTER, compared by
/// canonical equivalence. So `#` followed by a combining acute is one
/// Character that is not `"#"`, and U+1FEF GREEK VARIA — whose canonical
/// decomposition is U+0060 — IS a backtick. Of the characters this file
/// compares, the backtick is the only one with a canonical twin (checked over
/// every scalar against Swift 6.3).
fn cluster_is(cluster: &str, ascii: char) -> bool {
    let mut chars = cluster.chars();
    match (chars.next(), chars.next()) {
        (Some(only), None) => only == ascii || (ascii == '`' && only == '\u{1FEF}'),
        _ => false,
    }
}

/// Swift's `hasPrefix("x")` for one markup character: the FIRST CLUSTER.
fn starts_with_cluster(text: &str, ascii: char) -> bool {
    text.graphemes(true)
        .next()
        .is_some_and(|first| cluster_is(first, ascii))
}

/// Foundation's `CharacterSet.whitespaces`, scalar by scalar: Unicode `Zs`,
/// TAB, and U+200B ZERO WIDTH SPACE (which is not `Zs` and not Unicode
/// `White_Space`, but Foundation includes it). Measured over every scalar.
///
/// Deliberately NOT `char::is_whitespace`: that is `White_Space`, which adds
/// the newlines (`\n`, U+0085, U+2028…) and drops U+200B. The heading and
/// bullet rules trim with Foundation's set, so `"# \u{200B}"` is not a
/// heading and `"- \u{2028}"` is a bullet whose text is a line separator.
fn is_foundation_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | ' ' | '\u{A0}' | '\u{1680}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    ) || ('\u{2000}'..='\u{200B}').contains(&c)
}

/// `trimmingCharacters(in: .whitespaces)`: scalar by scalar, so a combining
/// mark riding a space keeps the mark and loses the space.
fn trim_foundation_whitespace(text: &str) -> &str {
    text.trim_matches(is_foundation_whitespace)
}

// MARK: Headings

struct Heading<'a> {
    level: u8,
    content: &'a str,
}

/// One to three `#`, then AT LEAST ONE SPACE, then something that is not
/// whitespace. `#Heading`, `#### X` and `# ` are left exactly as typed.
/// No closing-sequence stripping: `# Done #` says "Done #".
fn heading(line: &str) -> Option<Heading<'_>> {
    let mut level = 0u8;
    for (offset, cluster) in line.grapheme_indices(true) {
        if level < 3 && cluster_is(cluster, '#') {
            level += 1;
            continue;
        }
        // A fourth `#` is not a deeper heading — it is not a heading.
        if level == 0 || !cluster_is(cluster, ' ') {
            return None;
        }
        let content = &line[offset + cluster.len()..];
        // Whitespace is not content: `"#  "` is somebody mid-sentence.
        if trim_foundation_whitespace(content).is_empty() {
            return None;
        }
        return Some(Heading { level, content });
    }
    None
}

// MARK: Lists

struct Bullet<'a> {
    indent: &'a str,
    content: &'a str,
}

/// `- `, `* ` or `+ ` after any leading spaces or tabs, then content.
///
/// The marker never reaches the emphasis parser, which is what keeps `* X` a
/// bullet and `2 * 3 * 4 = 24` arithmetic. Ordered items are left as typed.
fn bullet(line: &str) -> Option<Bullet<'_>> {
    let mut clusters = line.grapheme_indices(true);
    let mut marker = None;
    for (offset, cluster) in clusters.by_ref() {
        if cluster_is(cluster, ' ') || cluster_is(cluster, '\t') {
            continue;
        }
        if cluster_is(cluster, '-') || cluster_is(cluster, '*') || cluster_is(cluster, '+') {
            marker = Some(offset);
        }
        break;
    }
    let marker = marker?;
    let (space, cluster) = clusters.next()?;
    if !cluster_is(cluster, ' ') {
        return None;
    }
    let content = &line[space + cluster.len()..];
    if trim_foundation_whitespace(content).is_empty() {
        return None;
    }
    Some(Bullet {
        indent: &line[..marker],
        content,
    })
}

// MARK: Tables

/// A header row, a delimiter row with the SAME number of cells, then rows
/// until the first line that is not one. Returns the table and the index of
/// the first line after it.
fn table(start: usize, lines: &[&str]) -> Option<(Table, usize)> {
    if start + 1 >= lines.len() {
        return None;
    }
    if !contains_unescaped_pipe(lines[start]) {
        return None;
    }
    let header = cells(lines[start]);
    if header.is_empty() {
        return None;
    }
    let alignments = delimiter_row(lines[start + 1])?;
    if alignments.len() != header.len() {
        return None;
    }

    let mut rows = Vec::new();
    let mut index = start + 2;
    while index < lines.len() {
        let line = lines[index];
        // Ends at a blank line or anything that is not a row — and a heading
        // or a bullet is what it says it is, not a cell.
        if !contains_unescaped_pipe(line)
            || trim_foundation_whitespace(line).is_empty()
            || heading(line).is_some()
            || bullet(line).is_some()
        {
            break;
        }
        // A row with NO cells — a lone `|` — ends the table as typed.
        let parsed = cells(line);
        if parsed.is_empty() {
            break;
        }
        let mut row: Vec<Text> = parsed.iter().map(|source| cell(source)).collect();
        // Ragged rows are padded or cut rather than dropping the table.
        row.resize(header.len(), Text::default());
        rows.push(row);
        index += 1;
    }
    let header = header.iter().map(|source| cell(source)).collect();
    Some((
        Table {
            alignments,
            header,
            rows,
        },
        index,
    ))
}

/// The delimiter row's alignments, or `None` when the line is not one. A
/// PIPE IS REQUIRED: a bare `---` is a rule far more often than a
/// one-column table.
fn delimiter_row(line: &str) -> Option<Vec<ColumnAlignment>> {
    if !contains_unescaped_pipe(line) {
        return None;
    }
    let parts = cells(line);
    if parts.is_empty() {
        return None;
    }
    let mut alignments = Vec::new();
    for part in &parts {
        let clusters: Vec<&str> = part.graphemes(true).collect();
        let mut dashes = &clusters[..];
        let left = dashes.first().is_some_and(|first| cluster_is(first, ':'));
        if left {
            dashes = &dashes[1..];
        }
        let right = dashes.last().is_some_and(|last| cluster_is(last, ':'));
        if right {
            dashes = &dashes[..dashes.len() - 1];
        }
        if dashes.is_empty() || !dashes.iter().all(|cluster| cluster_is(cluster, '-')) {
            return None;
        }
        alignments.push(match (left, right) {
            (true, true) => ColumnAlignment::Center,
            (_, true) => ColumnAlignment::Trailing,
            _ => ColumnAlignment::Leading,
        });
    }
    Some(alignments)
}

/// Split a row on unescaped `|`, dropping the optional edge pipes. `\|` stays
/// as written; the inline parser turns it into a literal pipe later.
fn cells(line: &str) -> Vec<String> {
    let text = trim_foundation_whitespace(line);
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for cluster in text.graphemes(true) {
        if escaped {
            current.push_str(cluster);
            escaped = false;
            continue;
        }
        if cluster_is(cluster, '\\') {
            current.push_str(cluster);
            escaped = true;
            continue;
        }
        if cluster_is(cluster, '|') {
            parts.push(std::mem::take(&mut current));
            continue;
        }
        current.push_str(cluster);
    }
    parts.push(current);
    if starts_with_cluster(text, '|') && !parts.is_empty() {
        parts.remove(0);
    }
    if ends_with_unescaped_pipe(text) && !parts.is_empty() {
        parts.pop();
    }
    parts
        .iter()
        .map(|part| trim_foundation_whitespace(part).to_string())
        .collect()
}

/// One cell: emphasis, code, strikethrough and escapes — but NEVER a link.
///
/// Parsed normally first, and re-parsed with every `[` and `]` escaped only
/// when a link actually formed; whatever link survives that (an autolink has
/// no brackets to escape) loses its destination outright.
///
/// Swift's `replacingOccurrences` finds a bracket inside a grapheme cluster,
/// so this is a plain scalar replace. The re-parse escapes brackets
/// EVERYWHERE in the cell, code spans and URLs included — see the test
/// `cell_escaping_shows_backslashes_in_code` for what that does.
fn cell(source: &str) -> Text {
    let rendered = inline(source);
    if !rendered.spans.iter().any(|span| span.style.link.is_some()) {
        return rendered;
    }
    let mut literal = inline(&source.replace('[', "\\[").replace(']', "\\]"));
    literal.strip_links();
    literal
}

fn contains_unescaped_pipe(line: &str) -> bool {
    let mut escaped = false;
    for cluster in line.graphemes(true) {
        if escaped {
            escaped = false;
            continue;
        }
        if cluster_is(cluster, '\\') {
            escaped = true;
            continue;
        }
        if cluster_is(cluster, '|') {
            return true;
        }
    }
    false
}

/// A trailing `|` that is a cell boundary: an odd number of backslashes
/// before it means it is escaped.
fn ends_with_unescaped_pipe(text: &str) -> bool {
    let clusters: Vec<&str> = text.graphemes(true).collect();
    let Some((last, rest)) = clusters.split_last() else {
        return false;
    };
    if !cluster_is(last, '|') {
        return false;
    }
    let backslashes = rest
        .iter()
        .rev()
        .take_while(|cluster| cluster_is(cluster, '\\'))
        .count();
    backslashes.is_multiple_of(2)
}

// MARK: Fences

enum Segment {
    Text(String),
    Code(String),
}

/// A line that opens or closes a fence: its first three clusters are
/// backticks (by Swift's `hasPrefix`, so a combining mark on the third one
/// defeats it and U+1FEF counts as one).
fn has_fence_prefix(line: &str) -> bool {
    let mut clusters = line.graphemes(true);
    (0..3).all(|_| {
        clusters
            .next()
            .is_some_and(|cluster| cluster_is(cluster, '`'))
    })
}

/// Split a body into alternating plain and fenced-code segments.
///
/// A fence must OPEN and CLOSE at the start of a line, and an unclosed fence
/// is not a fence at all — the text stays as typed. The language tag on an
/// opening fence is dropped.
fn segments(body: &str) -> Vec<Segment> {
    // Swift's `body.contains("```")` shortcut. Any fence line needs a
    // backtick (or its twin), so this is only ever a faster way to the same
    // answer.
    if !body.contains('`') && !body.contains('\u{1FEF}') {
        return vec![Segment::Text(body.to_string())];
    }
    let mut result = Vec::new();
    let mut plain: Vec<&str> = Vec::new();
    let mut code: Vec<&str> = Vec::new();
    let mut in_fence = false;
    for line in body.split('\n') {
        let opens_or_closes = has_fence_prefix(line);
        if !in_fence && opens_or_closes {
            in_fence = true;
            // Keep the newline that ENDED the line before the fence, or the
            // code welds onto the last word above it.
            plain.push("");
            continue;
        }
        if in_fence && opens_or_closes {
            in_fence = false;
            result.push(Segment::Text(plain.join("\n")));
            plain.clear();
            result.push(Segment::Code(code.join("\n")));
            code.clear();
            // Keep the newline that ended the fence.
            plain.push("");
            continue;
        }
        if in_fence {
            code.push(line);
        } else {
            plain.push(line);
        }
    }
    if in_fence {
        // Never closed — put it back exactly as it was typed.
        return vec![Segment::Text(body.to_string())];
    }
    result.push(Segment::Text(plain.join("\n")));
    result.retain(|segment| !matches!(segment, Segment::Text(text) if text.is_empty()));
    result
}

// MARK: - Inline: Foundation's `AttributedString(markdown:)`

/// Foundation's parser held to inline syntax, preserving whitespace — the
/// Swift `inline(_:)`. What Foundation does, in order:
///
/// 1. `\r\n` and `\r` become `\n`, and NUL becomes U+FFFD;
/// 2. the whole text is ONE paragraph: nothing block-level is recognised,
///    no line is trimmed, a newline is kept as `\n` and never a hard break;
/// 3. reference definitions at its very start are taken out (cmark's
///    paragraph rule — and because no newline is appended, a definition
///    whose destination runs to the end of the text is not one);
/// 4. cmark-gfm's inline parser and its autolink extension;
/// 5. the tree is flattened into styled runs.
fn inline(text: &str) -> Text {
    let mut source = text.replace("\r\n", "\n").replace('\r', "\n");
    if source.contains('\0') {
        source = source.replace('\0', "\u{FFFD}");
    }
    let mut references = References::default();
    let mut start = 0;
    while source.as_bytes().get(start) == Some(&b'[') {
        match parse_reference_definition(&source[start..], &mut references) {
            Some(length) => start += length,
            None => break,
        }
    }
    let mut subject = Subject::new(&source[start..], &references);
    subject.parse();
    subject.autolink_emails();
    let mut out = Text::default();
    subject.render(ROOT, &mut Style::default(), &mut out);
    out
}

// MARK: The tree cmark builds

const ROOT: usize = 0;

enum NodeKind {
    Root,
    Text(String),
    SoftBreak,
    LineBreak,
    Code(String),
    Html(String),
    Emphasis,
    Strong,
    Strikethrough,
    Link {
        url: String,
        title: String,
    },
    Image {
        url: String,
        title: String,
    },
    /// Apple's `^[text](attributes)`: the text, with attributes Foundation
    /// keeps and SwiftUI does not draw.
    Attributes,
}

struct Node {
    kind: NodeKind,
    parent: Option<usize>,
    prev: Option<usize>,
    next: Option<usize>,
    first_child: Option<usize>,
    last_child: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BracketKind {
    Link,
    Image,
    Attributes,
}

/// A `[`, `![` or `^[` waiting for its `]`.
struct Bracket {
    kind: BracketKind,
    node: usize,
    /// Just past the opener — the stack bottom for the emphasis inside it.
    position: usize,
    /// Another bracket opened after this one (a shortcut reference then
    /// cannot use this one's text as its label).
    bracket_after: bool,
}

/// A run of `*`, `_` or `~` that may open or close.
struct Delimiter {
    character: u8,
    can_open: bool,
    can_close: bool,
    /// The run's ORIGINAL length: the rule of three counts what was typed.
    length: usize,
    node: usize,
    /// Just past the run.
    position: usize,
    prev: Option<usize>,
    next: Option<usize>,
}

/// `[label]: destination "title"` definitions, first one wins.
#[derive(Default)]
struct References {
    map: HashMap<String, (String, String)>,
}

impl References {
    fn insert(&mut self, label: &str, url: String, title: String) {
        if let Some(key) = normalize_label(label) {
            self.map.entry(key).or_insert((url, title));
        }
    }

    fn get(&self, label: &str) -> Option<&(String, String)> {
        self.map.get(&normalize_label(label)?)
    }
}

/// One cmark inline parse: the subject text, the node tree, and the two
/// stacks (delimiters and brackets) cmark keeps while it reads.
struct Subject<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
    references: &'a References,
    nodes: Vec<Node>,
    delimiters: Vec<Delimiter>,
    last_delimiter: Option<usize>,
    brackets: Vec<Bracket>,
    /// cmark 0.30's `no_link_openers`: set when a link closes, so an older
    /// `[` cannot close around it — and cleared by the next `[` or `^[`
    /// pushed, which lets EVERY older one close again. That is how
    /// `[a [b](c) [x] d](e)` becomes one link to `e` on Apple (measured;
    /// the older per-bracket "active" flag would leave it text).
    no_link_openers: bool,
    /// cmark's memo of the last position of a backtick run of each length.
    backticks: Vec<usize>,
    scanned_for_backticks: bool,
}

/// cmark's `MAXBACKTICKS`: longer runs never open a code span.
const MAX_BACKTICKS: usize = 1000;

impl<'a> Subject<'a> {
    fn new(text: &'a str, references: &'a References) -> Self {
        let root = Node {
            kind: NodeKind::Root,
            parent: None,
            prev: None,
            next: None,
            first_child: None,
            last_child: None,
        };
        Subject {
            text,
            bytes: text.as_bytes(),
            pos: 0,
            references,
            nodes: vec![root],
            delimiters: Vec::new(),
            last_delimiter: None,
            brackets: Vec::new(),
            no_link_openers: true,
            backticks: vec![0; MAX_BACKTICKS + 1],
            scanned_for_backticks: false,
        }
    }

    // MARK: Tree plumbing (cmark's node.c)

    fn add(&mut self, kind: NodeKind) -> usize {
        self.nodes.push(Node {
            kind,
            parent: None,
            prev: None,
            next: None,
            first_child: None,
            last_child: None,
        });
        self.nodes.len() - 1
    }

    fn text_node(&mut self, text: &str) -> usize {
        self.add(NodeKind::Text(text.to_string()))
    }

    fn unlink(&mut self, node: usize) {
        let (parent, prev, next) = {
            let n = &self.nodes[node];
            (n.parent, n.prev, n.next)
        };
        if let Some(prev) = prev {
            self.nodes[prev].next = next;
        }
        if let Some(next) = next {
            self.nodes[next].prev = prev;
        }
        if let Some(parent) = parent {
            if self.nodes[parent].first_child == Some(node) {
                self.nodes[parent].first_child = next;
            }
            if self.nodes[parent].last_child == Some(node) {
                self.nodes[parent].last_child = prev;
            }
        }
        let n = &mut self.nodes[node];
        n.parent = None;
        n.prev = None;
        n.next = None;
    }

    fn append_child(&mut self, parent: usize, child: usize) {
        self.unlink(child);
        let last = self.nodes[parent].last_child;
        self.nodes[child].parent = Some(parent);
        self.nodes[child].prev = last;
        match last {
            Some(last) => self.nodes[last].next = Some(child),
            None => self.nodes[parent].first_child = Some(child),
        }
        self.nodes[parent].last_child = Some(child);
    }

    fn insert_before(&mut self, node: usize, sibling: usize) {
        self.unlink(sibling);
        let parent = self.nodes[node].parent;
        let prev = self.nodes[node].prev;
        self.nodes[sibling].parent = parent;
        self.nodes[sibling].prev = prev;
        self.nodes[sibling].next = Some(node);
        self.nodes[node].prev = Some(sibling);
        match prev {
            Some(prev) => self.nodes[prev].next = Some(sibling),
            None => {
                if let Some(parent) = parent {
                    self.nodes[parent].first_child = Some(sibling);
                }
            }
        }
    }

    fn insert_after(&mut self, node: usize, sibling: usize) {
        self.unlink(sibling);
        let parent = self.nodes[node].parent;
        let next = self.nodes[node].next;
        self.nodes[sibling].parent = parent;
        self.nodes[sibling].prev = Some(node);
        self.nodes[sibling].next = next;
        self.nodes[node].next = Some(sibling);
        match next {
            Some(next) => self.nodes[next].prev = Some(sibling),
            None => {
                if let Some(parent) = parent {
                    self.nodes[parent].last_child = Some(sibling);
                }
            }
        }
    }

    /// Move every sibling after `from` (up to, not including, `until`) into
    /// `container`.
    fn adopt_siblings(&mut self, from: usize, until: Option<usize>, container: usize) {
        let mut current = self.nodes[from].next;
        while let Some(node) = current {
            if Some(node) == until {
                break;
            }
            let next = self.nodes[node].next;
            self.append_child(container, node);
            current = next;
        }
    }

    fn text_len(&self, node: usize) -> usize {
        match &self.nodes[node].kind {
            NodeKind::Text(text) => text.len(),
            _ => 0,
        }
    }

    /// Shorten a delimiter run's text node. Runs are ASCII, so any length is
    /// a character boundary.
    fn set_text_len(&mut self, node: usize, len: usize) {
        if let NodeKind::Text(text) = &mut self.nodes[node].kind {
            text.truncate(len);
        }
    }

    // MARK: Reading the subject

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// The character before `pos`, or `\n` at the start — cmark's rule for
    /// what a delimiter run is flanked by.
    fn char_before(&self, pos: usize) -> char {
        if pos == 0 {
            '\n'
        } else {
            self.text[..pos].chars().next_back().unwrap_or('\n')
        }
    }

    /// The character at `pos`, or `\n` at the end.
    fn char_at(&self, pos: usize) -> char {
        self.text[pos..].chars().next().unwrap_or('\n')
    }

    /// cmark's `parse_inlines`: read everything, then resolve emphasis.
    fn parse(&mut self) {
        while self.pos < self.bytes.len() {
            if let Some(node) = self.parse_inline() {
                self.append_child(ROOT, node);
            }
        }
        self.process_emphasis(0);
        self.brackets.clear();
        self.last_delimiter = None;
    }

    /// One step of cmark's `parse_inline`: whatever starts at `pos`.
    fn parse_inline(&mut self) -> Option<usize> {
        let c = self.bytes[self.pos];
        match c {
            b'\n' => Some(self.handle_newline()),
            b'`' => Some(self.handle_backticks()),
            b'\\' => Some(self.handle_backslash()),
            b'&' => Some(self.handle_entity()),
            b'<' => Some(self.handle_pointy_brace()),
            b'*' | b'_' => Some(self.handle_delimiter(c)),
            b'~' => Some(self.handle_tilde()),
            b'[' => {
                self.pos += 1;
                let node = self.text_node("[");
                self.push_bracket(BracketKind::Link, node);
                Some(node)
            }
            b']' => self.handle_close_bracket(),
            b'!' => {
                self.pos += 1;
                // cmark-gfm keeps `![^` from opening an image (it would be a
                // footnote there): `![^a](x)` is "!" and a link.
                if self.peek() == Some(b'[') && self.bytes.get(self.pos + 1) != Some(&b'^') {
                    self.pos += 1;
                    let node = self.text_node("![");
                    self.push_bracket(BracketKind::Image, node);
                    Some(node)
                } else {
                    Some(self.text_node("!"))
                }
            }
            b'^' => {
                if self.bytes.get(self.pos + 1) == Some(&b'[') {
                    self.pos += 2;
                    let node = self.text_node("^[");
                    self.push_bracket(BracketKind::Attributes, node);
                    Some(node)
                } else {
                    self.pos += 1;
                    Some(self.text_node("^"))
                }
            }
            b':' => Some(match self.url_match() {
                Some(node) => node,
                None => self.text_run(),
            }),
            b'w' => Some(match self.www_match() {
                Some(node) => node,
                None => self.text_run(),
            }),
            _ => Some(self.text_run()),
        }
    }

    /// Plain text up to the next character something else might claim.
    fn text_run(&mut self) -> usize {
        let start = self.pos;
        let mut end = start + 1;
        // Every special byte is ASCII, so `end` always lands on a character
        // boundary.
        while end < self.bytes.len() && !is_special(self.bytes[end]) {
            end += 1;
        }
        self.pos = end;
        let text = &self.text[start..end];
        self.text_node(text)
    }

    /// A newline is a soft break drawn as `\n`, whatever is around it: in
    /// Foundation's whitespace-preserving mode no space is trimmed and two
    /// trailing spaces are not a hard break.
    fn handle_newline(&mut self) -> usize {
        self.pos += 1;
        self.add(NodeKind::SoftBreak)
    }

    fn handle_backticks(&mut self) -> usize {
        let ticks_start = self.pos;
        while self.peek() == Some(b'`') {
            self.pos += 1;
        }
        let open = self.pos - ticks_start;
        let start = self.pos;
        match self.scan_to_closing_backticks(open) {
            None => {
                self.pos = start;
                let ticks = &self.text[ticks_start..start];
                self.text_node(ticks)
            }
            Some(end) => {
                let code = normalize_code(&self.text[start..end - open]);
                self.pos = end;
                self.add(NodeKind::Code(code))
            }
        }
    }

    /// The position after a backtick run exactly `open` long, or `None`.
    fn scan_to_closing_backticks(&mut self, open: usize) -> Option<usize> {
        if open > MAX_BACKTICKS {
            return None;
        }
        if self.scanned_for_backticks && self.backticks[open] <= self.pos {
            return None;
        }
        loop {
            while let Some(c) = self.peek() {
                if c == b'`' {
                    break;
                }
                self.pos += 1;
            }
            if self.pos >= self.bytes.len() {
                break;
            }
            let mut count = 0;
            while self.peek() == Some(b'`') {
                self.pos += 1;
                count += 1;
            }
            if count <= MAX_BACKTICKS {
                self.backticks[count] = self.pos - count;
            }
            if count == open {
                return Some(self.pos);
            }
        }
        self.scanned_for_backticks = true;
        None
    }

    /// `\` escapes ASCII punctuation, makes a hard break before a newline,
    /// and is itself anywhere else.
    fn handle_backslash(&mut self) -> usize {
        self.pos += 1;
        match self.peek() {
            Some(c) if c.is_ascii_punctuation() => {
                self.pos += 1;
                let escaped = &self.text[self.pos - 1..self.pos];
                self.text_node(escaped)
            }
            Some(b'\n') => {
                self.pos += 1;
                self.add(NodeKind::LineBreak)
            }
            _ => self.text_node("\\"),
        }
    }

    fn handle_entity(&mut self) -> usize {
        self.pos += 1;
        match unescape_entity(&self.bytes[self.pos..]) {
            Some((decoded, length)) => {
                self.pos += length;
                self.text_node(&decoded)
            }
            None => self.text_node("&"),
        }
    }

    /// `<`: an autolink, an email autolink, raw HTML, or the character.
    fn handle_pointy_brace(&mut self) -> usize {
        self.pos += 1;
        if let Some(length) = scan_autolink_uri(self.bytes, self.pos) {
            let contents = &self.text[self.pos..self.pos + length - 1];
            self.pos += length;
            return self.make_autolink(contents, false);
        }
        if let Some(length) = scan_autolink_email(self.bytes, self.pos) {
            let contents = &self.text[self.pos..self.pos + length - 1];
            self.pos += length;
            return self.make_autolink(contents, true);
        }
        if let Some(length) = scan_html_tag(self.bytes, self.pos) {
            let contents = self.text[self.pos - 1..self.pos + length].to_string();
            self.pos += length;
            return self.add(NodeKind::Html(contents));
        }
        self.text_node("<")
    }

    fn make_autolink(&mut self, contents: &str, is_email: bool) -> usize {
        let trimmed = contents.trim_matches(|c: char| c.is_ascii() && is_cmark_space_byte(c as u8));
        let mut url = String::new();
        if is_email && !trimmed.is_empty() {
            url.push_str("mailto:");
        }
        url.push_str(&unescape_html(trimmed));
        let link = self.add(NodeKind::Link {
            url,
            title: String::new(),
        });
        let text = self.text_node(&unescape_html(contents));
        self.append_child(link, text);
        link
    }

    /// `*` and `_` runs (cmark's `handle_delim` and `scan_delims`).
    ///
    /// Tildes are TRANSPARENT here: cmark-gfm registers the strikethrough
    /// `~` as an emphasis-like character, and `scan_delims` steps over such
    /// characters when it looks for what flanks a run — so `**~~a~~**`
    /// works, and `*(x)*~a` does not close (it sees `a`, not `~`). When the
    /// walk back runs into a tilde at position 0, what is before the run is
    /// the start of the text. Measured both ways against Foundation.
    fn handle_delimiter(&mut self, c: u8) -> usize {
        let start = self.pos;
        let before = if start == 0 {
            '\n'
        } else {
            let mut at = start - 1;
            while at > 0 && (is_utf8_continuation(self.bytes[at]) || self.bytes[at] == b'~') {
                at -= 1;
            }
            if at == 0 && self.bytes[0] == b'~' {
                '\n'
            } else {
                decode_at(self.bytes, at).unwrap_or('\n')
            }
        };
        while self.peek() == Some(c) {
            self.pos += 1;
        }
        let mut at = self.pos;
        while self.bytes.get(at) == Some(&b'~') {
            at += 1;
        }
        let after = if at >= self.bytes.len() {
            '\n'
        } else {
            decode_at(self.bytes, at).unwrap_or('\n')
        };
        let (left, right) = flanking(before, after);
        let (can_open, can_close) = if c == b'_' {
            (
                left && (!right || is_cmark_punctuation(before)),
                right && (!left || is_cmark_punctuation(after)),
            )
        } else {
            (left, right)
        };
        let run = &self.text[start..self.pos];
        let node = self.text_node(run);
        if can_open || can_close {
            self.push_delimiter(c, can_open, can_close, node);
        }
        node
    }

    /// `~` runs — cmark-gfm's strikethrough extension. One or two tildes can
    /// open and close (Foundation does not require the double form); three
    /// or more are just tildes.
    fn handle_tilde(&mut self) -> usize {
        let before = self.char_before(self.pos);
        let start = self.pos;
        // The extension reads into a 100-byte buffer and stops at 101.
        while self.peek() == Some(b'~') && self.pos - start <= 100 {
            self.pos += 1;
        }
        let count = self.pos - start;
        let after = self.char_at(self.pos);
        let (left, right) = flanking(before, after);
        let run = &self.text[start..self.pos];
        let node = self.text_node(run);
        if (left || right) && (count == 1 || count == 2) {
            self.push_delimiter(b'~', left, right, node);
        }
        node
    }

    // MARK: Delimiters and emphasis (cmark's inlines.c)

    fn push_delimiter(&mut self, character: u8, can_open: bool, can_close: bool, node: usize) {
        let index = self.delimiters.len();
        self.delimiters.push(Delimiter {
            character,
            can_open,
            can_close,
            length: self.text_len(node),
            node,
            position: self.pos,
            prev: self.last_delimiter,
            next: None,
        });
        if let Some(last) = self.last_delimiter {
            self.delimiters[last].next = Some(index);
        }
        self.last_delimiter = Some(index);
    }

    fn remove_delimiter(&mut self, index: usize) {
        let (prev, next) = (self.delimiters[index].prev, self.delimiters[index].next);
        match next {
            Some(next) => self.delimiters[next].prev = prev,
            None => self.last_delimiter = prev,
        }
        if let Some(prev) = prev {
            self.delimiters[prev].next = next;
        }
    }

    /// CommonMark's "process emphasis", as cmark writes it: walk the closers
    /// forward, look back for an opener of the same kind, and build the
    /// emphasis between them.
    fn process_emphasis(&mut self, stack_bottom: usize) {
        // cmark's shortcut: once a closer found no opener, later closers of
        // the same character and length-mod-3 do not look below it.
        let mut openers_bottom = [[stack_bottom; 3]; 3];

        let mut closer = None;
        let mut candidate = self.last_delimiter;
        while let Some(index) = candidate {
            if self.delimiters[index].position < stack_bottom {
                break;
            }
            closer = Some(index);
            candidate = self.delimiters[index].prev;
        }

        while let Some(closer_index) = closer {
            let (character, can_close, can_open, length) = {
                let d = &self.delimiters[closer_index];
                (d.character, d.can_close, d.can_open, d.length)
            };
            if !can_close {
                closer = self.delimiters[closer_index].next;
                continue;
            }
            let slot = match character {
                b'*' => 0,
                b'_' => 1,
                _ => 2,
            };
            let bottom = openers_bottom[length % 3][slot];
            let mut opener = self.delimiters[closer_index].prev;
            let mut found = false;
            while let Some(index) = opener {
                let d = &self.delimiters[index];
                if d.position < stack_bottom || d.position < bottom {
                    break;
                }
                if d.can_open && d.character == character {
                    // The rule of three: an interior closer of length 2 does
                    // not match an opener of length 1, and so on.
                    if !(can_open || d.can_close)
                        || length.is_multiple_of(3)
                        || !(d.length + length).is_multiple_of(3)
                    {
                        found = true;
                        break;
                    }
                }
                opener = d.prev;
            }
            let old_closer = closer_index;
            closer = match (found, opener) {
                (true, Some(opener)) if character == b'~' => {
                    self.insert_strikethrough(opener, closer_index)
                }
                (true, Some(opener)) => self.insert_emphasis(opener, closer_index),
                _ => self.delimiters[closer_index].next,
            };
            if !found {
                openers_bottom[length % 3][slot] = self.delimiters[old_closer].position;
                if !self.delimiters[old_closer].can_open {
                    self.remove_delimiter(old_closer);
                }
            }
        }

        while let Some(last) = self.last_delimiter {
            if self.delimiters[last].position < stack_bottom {
                break;
            }
            self.remove_delimiter(last);
        }
    }

    /// cmark's `S_insert_emph`: two characters from each side make strong,
    /// one makes emphasis; returns the next closer to look at.
    fn insert_emphasis(&mut self, opener: usize, closer: usize) -> Option<usize> {
        let opener_node = self.delimiters[opener].node;
        let closer_node = self.delimiters[closer].node;
        let opener_chars = self.text_len(opener_node);
        let closer_chars = self.text_len(closer_node);
        let used = if closer_chars >= 2 && opener_chars >= 2 {
            2
        } else {
            1
        };
        self.set_text_len(opener_node, opener_chars - used);
        self.set_text_len(closer_node, closer_chars - used);

        let mut between = self.delimiters[closer].prev;
        while let Some(index) = between {
            if index == opener {
                break;
            }
            let prev = self.delimiters[index].prev;
            self.remove_delimiter(index);
            between = prev;
        }

        let emphasis = self.add(if used == 1 {
            NodeKind::Emphasis
        } else {
            NodeKind::Strong
        });
        self.adopt_siblings(opener_node, Some(closer_node), emphasis);
        self.insert_after(opener_node, emphasis);

        if opener_chars == used {
            self.unlink(opener_node);
            self.remove_delimiter(opener);
        }
        if closer_chars == used {
            self.unlink(closer_node);
            let next = self.delimiters[closer].next;
            self.remove_delimiter(closer);
            return next;
        }
        Some(closer)
    }

    /// The strikethrough extension's `insert`: only runs of the SAME length
    /// pair up; either way the delimiters between are spent.
    fn insert_strikethrough(&mut self, opener: usize, closer: usize) -> Option<usize> {
        let next = self.delimiters[closer].next;
        let opener_node = self.delimiters[opener].node;
        let closer_node = self.delimiters[closer].node;
        if self.text_len(opener_node) == self.text_len(closer_node) {
            self.nodes[opener_node].kind = NodeKind::Strikethrough;
            self.adopt_siblings(opener_node, Some(closer_node), opener_node);
            self.unlink(closer_node);
        }
        let mut current = Some(closer);
        while let Some(index) = current {
            if index == opener {
                break;
            }
            let prev = self.delimiters[index].prev;
            self.remove_delimiter(index);
            current = prev;
        }
        self.remove_delimiter(opener);
        next
    }

    // MARK: Brackets, links and images

    fn push_bracket(&mut self, kind: BracketKind, node: usize) {
        if let Some(last) = self.brackets.last_mut() {
            last.bracket_after = true;
        }
        // Measured: `[` and `^[` re-enable older link openers, `![` does not.
        if kind != BracketKind::Image {
            self.no_link_openers = false;
        }
        self.brackets.push(Bracket {
            kind,
            node,
            position: self.pos,
            bracket_after: false,
        });
    }

    /// cmark's `handle_close_bracket`: a link, an image, an attribute span,
    /// or a literal `]`.
    fn handle_close_bracket(&mut self) -> Option<usize> {
        self.pos += 1;
        let initial_pos = self.pos;
        let Some(opener) = self.brackets.len().checked_sub(1) else {
            return Some(self.text_node("]"));
        };
        let kind = self.brackets[opener].kind;
        // A link cannot contain a link (see `no_link_openers`).
        if kind == BracketKind::Link && self.no_link_openers {
            self.brackets.pop();
            return Some(self.text_node("]"));
        }

        let matched = if kind == BracketKind::Attributes {
            // `^[text](…)`: the parentheses must follow at once, and what is
            // inside them is Foundation's business, not the text's.
            let mut matched = None;
            if self.peek() == Some(b'(') {
                if let Some(length) = scan_attributes(self.bytes, self.pos + 1) {
                    self.pos += 1 + length + 1;
                    matched = Some((String::new(), String::new()));
                }
            }
            // Then Apple's fork reads a `[label]` straight after and throws
            // it away, span or no span: `^[a](b)[c]` draws "a", and
            // `^[a-z][0-9]` draws "^[a-z]". Measured; it loses text.
            if let Some((_, _, after)) = link_label(self.bytes, self.pos) {
                self.pos = after;
            }
            if matched.is_none() {
                // Unlike a link, the position is NOT rewound.
                self.brackets.pop();
                return Some(self.text_node("]"));
            }
            matched
        } else {
            self.inline_destination().or_else(|| {
                self.pos = initial_pos;
                self.reference_destination(opener, initial_pos)
            })
        };

        let Some((url, title)) = matched else {
            self.brackets.pop();
            self.pos = initial_pos;
            return Some(self.text_node("]"));
        };

        let node = self.add(match kind {
            BracketKind::Link => NodeKind::Link { url, title },
            BracketKind::Image => NodeKind::Image { url, title },
            BracketKind::Attributes => NodeKind::Attributes,
        });
        let opener_node = self.brackets[opener].node;
        self.insert_before(opener_node, node);
        self.adopt_siblings(opener_node, None, node);
        self.unlink(opener_node);
        let position = self.brackets[opener].position;
        self.process_emphasis(position);
        self.brackets.pop();
        // Image and attribute spans leave older openers alone — measured:
        // `^[[a](b) c](d)` still closes its attribute span.
        if kind == BracketKind::Link {
            self.no_link_openers = true;
        }
        None
    }

    /// `(destination "title")` right after the `]`, advancing past it.
    fn inline_destination(&mut self) -> Option<(String, String)> {
        if self.peek() != Some(b'(') {
            return None;
        }
        let bytes = self.bytes;
        let url_start = self.pos + 1 + scan_spacechars(bytes, self.pos + 1);
        let (length, url_range) = scan_link_url(bytes, url_start)?;
        let end_url = url_start + length;
        let start_title = end_url + scan_spacechars(bytes, end_url);
        let end_title = if start_title == end_url {
            start_title
        } else {
            start_title + scan_link_title(bytes, start_title)
        };
        let end_all = end_title + scan_spacechars(bytes, end_title);
        if bytes.get(end_all) != Some(&b')') {
            return None;
        }
        self.pos = end_all + 1;
        let url = clean_url(&self.text[url_range.0..url_range.1]);
        let title = clean_title(&self.text[start_title..end_title]);
        Some((url, title))
    }

    /// `[label]`, `[]` or nothing after the `]`, looked up in the
    /// definitions.
    fn reference_destination(
        &mut self,
        opener: usize,
        initial_pos: usize,
    ) -> Option<(String, String)> {
        let found = link_label(self.bytes, self.pos);
        let mut label: Option<&str> = None;
        if let Some((start, end, after)) = found {
            self.pos = after;
            let raw = trim_cmark_space(&self.text[start..end]);
            if !raw.is_empty() {
                label = Some(raw);
            }
        } else {
            self.pos = initial_pos;
        }
        if label.is_none() && !self.brackets[opener].bracket_after {
            // A shortcut `[a]` or a collapsed `[a][]`: the text is the label.
            let start = self.brackets[opener].position;
            label = Some(&self.text[start..initial_pos - 1]);
        }
        let (url, title) = self.references.get(label?)?;
        Some((url.clone(), title.clone()))
    }

    // MARK: The GFM autolink extension

    /// cmark-gfm's `in_bracket`: a `[` or `![` still open. Apple's `^[` does
    /// not count — measured: `^[a www.x.com` links the host, and in
    /// `^[a www.x.com](b)` the autolink runs on through `](b)`, so the span
    /// never closes and the whole line stays as typed.
    fn in_bracket(&self) -> bool {
        self.brackets
            .iter()
            .any(|bracket| bracket.kind != BracketKind::Attributes)
    }

    /// `http://`, `https://` and `ftp://` URLs, found at their `:`.
    fn url_match(&mut self) -> Option<usize> {
        if self.in_bracket() {
            return None;
        }
        let max_rewind = self.pos;
        let data = &self.bytes[self.pos..];
        let size = data.len();
        if size < 4 || data[1] != b'/' || data[2] != b'/' {
            return None;
        }
        let mut rewind = 0;
        while rewind < max_rewind && self.bytes[max_rewind - rewind - 1].is_ascii_alphabetic() {
            rewind += 1;
        }
        if !autolink_is_safe(&self.bytes[max_rewind - rewind..]) {
            return None;
        }
        let domain = check_domain(&data[3..], true);
        if domain == 0 {
            return None;
        }
        let mut link_end = 3 + domain;
        while link_end < size && !is_cmark_space_byte(data[link_end]) && data[link_end] != b'<' {
            link_end += 1;
        }
        let link_end = autolink_delim(data, link_end);
        if link_end == 0 {
            return None;
        }
        self.pos = max_rewind + link_end;
        self.unput(rewind);
        let url = self.text[max_rewind - rewind..max_rewind + link_end].to_string();
        let link = self.add(NodeKind::Link {
            url: url.clone(),
            title: String::new(),
        });
        let text = self.text_node(&url);
        self.append_child(link, text);
        Some(link)
    }

    /// `www.` hosts, found at their `w`.
    fn www_match(&mut self) -> Option<usize> {
        if self.in_bracket() {
            return None;
        }
        let max_rewind = self.pos;
        if max_rewind > 0 {
            let before = self.bytes[max_rewind - 1];
            if !b"*_~(".contains(&before) && !is_cmark_space_byte(before) {
                return None;
            }
        }
        let data = &self.bytes[self.pos..];
        let size = data.len();
        if size < 4 || &data[..4] != b"www." {
            return None;
        }
        let mut link_end = check_domain(data, false);
        if link_end == 0 {
            return None;
        }
        while link_end < size && !is_cmark_space_byte(data[link_end]) && data[link_end] != b'<' {
            link_end += 1;
        }
        let link_end = autolink_delim(data, link_end);
        if link_end == 0 {
            return None;
        }
        self.pos = max_rewind + link_end;
        let text = self.text[max_rewind..max_rewind + link_end].to_string();
        let link = self.add(NodeKind::Link {
            url: format!("http://{text}"),
            title: String::new(),
        });
        let child = self.text_node(&text);
        self.append_child(link, child);
        Some(link)
    }

    /// cmark's `cmark_node_unput`: take the scheme letters a URL match
    /// rewound over back off the text before it. They are ASCII letters at
    /// the very end of that text, so every cut is on a character boundary.
    fn unput(&mut self, mut count: usize) {
        let mut node = self.nodes[ROOT].last_child;
        while count > 0 {
            let Some(index) = node else { break };
            let NodeKind::Text(text) = &mut self.nodes[index].kind else {
                break;
            };
            if text.len() < count {
                count -= text.len();
                text.clear();
            } else {
                let keep = text.len() - count;
                if text.is_char_boundary(keep) {
                    text.truncate(keep);
                }
                count = 0;
            }
            node = self.nodes[index].prev;
        }
    }

    /// The extension's postprocess: email addresses in text that is not
    /// already inside a link. cmark merges neighbouring text nodes first, so
    /// an address split by an escape or an entity is still found.
    fn autolink_emails(&mut self) {
        self.consolidate_text(ROOT);
        let mut texts = Vec::new();
        self.collect_texts_outside_links(ROOT, &mut texts);
        for node in texts {
            self.autolink_emails_in(node);
        }
    }

    fn consolidate_text(&mut self, parent: usize) {
        let mut current = self.nodes[parent].first_child;
        while let Some(node) = current {
            if let NodeKind::Text(_) = self.nodes[node].kind {
                while let Some(next) = self.nodes[node].next {
                    let NodeKind::Text(more) = &self.nodes[next].kind else {
                        break;
                    };
                    let more = more.clone();
                    if let NodeKind::Text(text) = &mut self.nodes[node].kind {
                        text.push_str(&more);
                    }
                    self.unlink(next);
                }
            } else {
                self.consolidate_text(node);
            }
            current = self.nodes[node].next;
        }
    }

    fn collect_texts_outside_links(&self, parent: usize, out: &mut Vec<usize>) {
        let mut current = self.nodes[parent].first_child;
        while let Some(node) = current {
            match self.nodes[node].kind {
                NodeKind::Link { .. } => {}
                NodeKind::Text(_) => out.push(node),
                _ => self.collect_texts_outside_links(node, out),
            }
            current = self.nodes[node].next;
        }
    }

    /// cmark-gfm's `postprocess_text`, index for index.
    fn autolink_emails_in(&mut self, node: usize) {
        let literal = match &self.nodes[node].kind {
            NodeKind::Text(text) if text.contains('@') => text.clone(),
            _ => return,
        };
        let data = literal.as_bytes();
        let mut text_node = node;
        let mut start = 0usize;
        let mut offset = 0usize;
        let mut remaining = data.len();

        'scan: loop {
            if offset >= remaining {
                break;
            }
            let window = &data[start + offset..start + remaining];
            let Some(found) = window.iter().position(|&c| c == b'@') else {
                break;
            };
            let mut max_rewind = found;
            // Set once per `@` found by the search, NOT again when the scan
            // restarts from a later `@` — cmark's `goto found_at` jumps past
            // these, so dots counted before the second `@` still count.
            let mut auto_mailto = true;
            let mut is_xmpp = false;
            let mut np = 0;
            let mut rewind;
            let mut link_end;

            'found_at: loop {
                let at = start + offset + max_rewind;
                rewind = 0;
                while rewind < max_rewind {
                    let c = data[at - rewind - 1];
                    if c.is_ascii_alphanumeric() || b".+-_".contains(&c) {
                        rewind += 1;
                        continue;
                    }
                    if c == b':' {
                        if validate_protocol(b"mailto:", data, at, rewind, max_rewind) {
                            auto_mailto = false;
                            rewind += 1;
                            continue;
                        }
                        if validate_protocol(b"xmpp:", data, at, rewind, max_rewind) {
                            auto_mailto = false;
                            is_xmpp = true;
                            rewind += 1;
                            continue;
                        }
                    }
                    break;
                }
                if rewind == 0 {
                    offset += max_rewind + 1;
                    continue 'scan;
                }
                link_end = 1;
                while link_end < remaining - offset - max_rewind {
                    let c = data[at + link_end];
                    if c.is_ascii_alphanumeric() {
                        link_end += 1;
                        continue;
                    }
                    if c == b'@' {
                        // Another `@`: start again from it.
                        offset += max_rewind + 1;
                        max_rewind = link_end - 1;
                        continue 'found_at;
                    } else if c == b'.'
                        && link_end < remaining - offset - max_rewind - 1
                        && data[at + link_end + 1].is_ascii_alphanumeric()
                    {
                        np += 1;
                    } else if c == b'/' && is_xmpp {
                        // xmpp resources ride along.
                    } else if c != b'-' && c != b'_' {
                        break;
                    }
                    link_end += 1;
                }
                break;
            }

            let at = start + offset + max_rewind;
            let last = data[at + link_end - 1];
            if link_end < 2 || np == 0 || (!last.is_ascii_alphabetic() && last != b'.') {
                offset += max_rewind + link_end;
                continue;
            }
            let link_end = autolink_delim(&data[at..], link_end);
            if link_end == 0 {
                offset += max_rewind + 1;
                continue;
            }

            let email = &literal[at - rewind..at + link_end];
            let url = if auto_mailto {
                format!("mailto:{email}")
            } else {
                email.to_string()
            };
            let link = self.add(NodeKind::Link {
                url,
                title: String::new(),
            });
            let link_text = self.text_node(email);
            self.append_child(link, link_text);
            self.insert_after(text_node, link);
            let post = self.text_node(&literal[at + link_end..start + remaining]);
            self.insert_after(link, post);
            let before = literal[start..at - rewind].to_string();
            if let NodeKind::Text(text) = &mut self.nodes[text_node].kind {
                *text = before;
            }
            text_node = post;
            let consumed = offset + max_rewind + link_end;
            start += consumed;
            remaining -= consumed;
            offset = 0;
        }
    }

    // MARK: Foundation's conversion into runs

    /// Walk the tree into styled runs the way Foundation builds its
    /// `AttributedString`.
    ///
    /// Foundation SETS an inline style on entering emphasis, strong or
    /// strikethrough and CLEARS it on leaving — it does not restore what was
    /// there. So a span nested in one of its own kind switches the style off
    /// for the rest of the outer one: `*a *b* c*` draws " c" upright.
    /// Measured; it is Apple's behaviour, so it is this one's.
    fn render(&self, node: usize, style: &mut Style, out: &mut Text) {
        match &self.nodes[node].kind {
            NodeKind::Root | NodeKind::Attributes => self.render_children(node, style, out),
            NodeKind::Text(text) => out.push(text, style.clone()),
            NodeKind::SoftBreak => out.push("\n", style.clone()),
            NodeKind::LineBreak => out.push(
                "\n",
                Style {
                    line_break: true,
                    ..style.clone()
                },
            ),
            NodeKind::Code(code) => out.push(
                code,
                Style {
                    code: true,
                    ..style.clone()
                },
            ),
            NodeKind::Html(html) => out.push(
                html,
                Style {
                    html: true,
                    ..style.clone()
                },
            ),
            NodeKind::Emphasis => {
                style.emphasis = true;
                self.render_children(node, style, out);
                style.emphasis = false;
            }
            NodeKind::Strong => {
                style.strong = true;
                self.render_children(node, style, out);
                style.strong = false;
            }
            NodeKind::Strikethrough => {
                style.strikethrough = true;
                self.render_children(node, style, out);
                style.strikethrough = false;
            }
            NodeKind::Link { url, title } => {
                // The label is FLATTENED: `[*a*](b)` is an unstyled "a".
                let mut run = style.clone();
                if foundation_accepts_url(url) {
                    run.link = Some(make_link(url, title));
                }
                out.push(&self.flatten(node), run);
            }
            NodeKind::Image { url, title } => {
                let mut alt = self.flatten(node);
                if alt.is_empty() {
                    alt.push('\u{FFFC}');
                }
                let mut run = style.clone();
                if foundation_accepts_url(url) {
                    run.image = Some(make_link(url, title));
                }
                out.push(&alt, run);
            }
        }
    }

    fn render_children(&self, node: usize, style: &mut Style, out: &mut Text) {
        let mut current = self.nodes[node].first_child;
        while let Some(child) = current {
            self.render(child, style, out);
            current = self.nodes[child].next;
        }
    }

    /// The plain text of a link label or an image's alt text, as Foundation
    /// takes it: soft breaks stay `\n`, a hard break vanishes, and an image
    /// inside contributes only its own text.
    fn flatten(&self, node: usize) -> String {
        let mut out = String::new();
        let mut current = self.nodes[node].first_child;
        while let Some(child) = current {
            match &self.nodes[child].kind {
                NodeKind::Text(text) | NodeKind::Code(text) | NodeKind::Html(text) => {
                    out.push_str(text)
                }
                NodeKind::SoftBreak => out.push('\n'),
                NodeKind::LineBreak => {}
                _ => out.push_str(&self.flatten(child)),
            }
            current = self.nodes[child].next;
        }
        out
    }
}

fn make_link(url: &str, title: &str) -> Link {
    Link {
        destination: url.to_string(),
        title: if title.is_empty() {
            None
        } else {
            Some(title.to_string())
        },
    }
}

/// The bytes that end a run of plain text: cmark's `SPECIAL_CHARS`, the
/// extensions' `~` `:` `w`, and Apple's `^`. `\r` never reaches the parser.
fn is_special(byte: u8) -> bool {
    matches!(
        byte,
        b'\n'
            | b'\\'
            | b'`'
            | b'&'
            | b'_'
            | b'*'
            | b'['
            | b']'
            | b'<'
            | b'!'
            | b'~'
            | b':'
            | b'w'
            | b'^'
    )
}

/// CommonMark's left- and right-flanking tests over cmark's own predicates.
fn flanking(before: char, after: char) -> (bool, bool) {
    let left = !is_cmark_space(after)
        && (!is_cmark_punctuation(after) || is_cmark_space(before) || is_cmark_punctuation(before));
    let right = !is_cmark_space(before)
        && (!is_cmark_punctuation(before) || is_cmark_space(after) || is_cmark_punctuation(after));
    (left, right)
}

/// cmark's `S_normalize_code`: newlines become spaces, then ONE space comes
/// off each end when both ends have one and the span is not all spaces.
fn normalize_code(raw: &str) -> String {
    let text = raw.replace('\n', " ");
    let bytes = text.as_bytes();
    let all_spaces = bytes.iter().all(|&b| b == b' ');
    if !all_spaces && bytes.first() == Some(&b' ') && bytes.last() == Some(&b' ') {
        text[1..text.len() - 1].to_string()
    } else {
        text
    }
}

// MARK: - cmark's Unicode predicates

/// cmark's `cmark_utf8proc_is_space`: TAB, LF, FF, CR, SPACE and Unicode
/// `Zs`. Not `char::is_whitespace` (which adds VT, U+0085 and the line
/// separators) and not Foundation's `.whitespaces` (which adds U+200B).
/// Measured over every scalar through Foundation's emphasis rules.
fn is_cmark_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}'
    ) || matches!(c, '\u{202F}' | '\u{205F}' | '\u{3000}')
        || ('\u{2000}'..='\u{200A}').contains(&c)
}

/// cmark's `cmark_utf8proc_is_punctuation`: ASCII punctuation (symbols such
/// as `$`, `+`, `<` included) and, above ASCII, the frozen `P*` table.
/// Unlike Unicode's `S*` symbols, `€` and `©` are NOT punctuation here.
fn is_cmark_punctuation(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_punctuation();
    }
    in_ranges(CMARK_PUNCTUATION, c)
}

/// cmark's ASCII `cmark_isspace`: space, TAB, LF, VT, FF, CR.
fn is_cmark_space_byte(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

fn trim_cmark_space(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_ascii() && is_cmark_space_byte(c as u8))
}

// MARK: - cmark's scanners

/// `[ \t\v\f\r\n]*`
fn scan_spacechars(bytes: &[u8], pos: usize) -> usize {
    bytes.get(pos..).map_or(0, |rest| {
        rest.iter().take_while(|&&b| is_cmark_space_byte(b)).count()
    })
}

/// cmark's `manual_scan_link_url`: `<…>`, or a run that stops at a space or
/// at a `)` it did not open. Returns the length consumed and the
/// destination's range.
///
/// This is cmark 0.29's version: the parentheses need not balance by the
/// end (`[a](b(c )` links to `b(c`) — only more than 32 open is refused.
/// Both forms need a character AFTER them (cmark's paragraphs always end in
/// a newline; Foundation's text does not), which is what keeps a reference
/// definition at the very end of a body from being one.
fn scan_link_url(bytes: &[u8], offset: usize) -> Option<(usize, (usize, usize))> {
    let len = bytes.len();
    let mut i = offset;
    if i < len && bytes[i] == b'<' {
        i += 1;
        loop {
            if i >= len {
                return None;
            }
            match bytes[i] {
                b'>' => {
                    i += 1;
                    break;
                }
                b'\\' => i += 2,
                b'\n' | b'<' => return None,
                _ => i += 1,
            }
        }
        if i >= len {
            return None;
        }
        return Some((i - offset, (offset + 1, i - 1)));
    }
    let mut depth = 0usize;
    while i < len {
        let c = bytes[i];
        if c == b'\\' && i + 1 < len && bytes[i + 1].is_ascii_punctuation() {
            i += 2;
        } else if c == b'(' {
            depth += 1;
            i += 1;
            if depth > 32 {
                return None;
            }
        } else if c == b')' {
            if depth == 0 {
                break;
            }
            depth -= 1;
            i += 1;
        } else if is_cmark_space_byte(c) {
            if i == offset {
                return None;
            }
            break;
        } else {
            i += 1;
        }
    }
    if i >= len {
        return None;
    }
    Some((i - offset, (offset, i)))
}

/// Apple's `^[text](…)` contents: balanced parentheses (at most 32 deep),
/// backslash escapes, anything else — spaces and newlines included. Returns
/// the length up to the closing `)`. Measured, not documented.
fn scan_attributes(bytes: &[u8], offset: usize) -> Option<usize> {
    let len = bytes.len();
    let mut i = offset;
    let mut depth = 0usize;
    while i < len {
        let c = bytes[i];
        if c == b'\\' && i + 1 < len && bytes[i + 1].is_ascii_punctuation() {
            i += 2;
        } else if c == b'(' {
            depth += 1;
            i += 1;
            if depth > 32 {
                return None;
            }
        } else if c == b')' {
            if depth == 0 {
                break;
            }
            depth -= 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    if i >= len || depth != 0 {
        return None;
    }
    Some(i - offset)
}

/// cmark's `scan_link_title`, a re2c longest match of `"…"`, `'…'` or
/// `(…)`: inside, the closing character (and for parentheses the opening
/// one too) may appear only straight after a backslash. Returns the length,
/// or 0.
fn scan_link_title(bytes: &[u8], pos: usize) -> usize {
    let Some(&open) = bytes.get(pos) else {
        return 0;
    };
    let close = match open {
        b'"' => b'"',
        b'\'' => b'\'',
        b'(' => b')',
        _ => return 0,
    };
    let mut candidate = 0;
    let mut i = pos + 1;
    while i < bytes.len() {
        let c = bytes[i];
        let escaped = bytes[i - 1] == b'\\' && i - 1 > pos;
        if c == close {
            if !escaped {
                return i + 1 - pos;
            }
            candidate = i + 1 - pos;
        } else if open == b'(' && c == b'(' && !escaped {
            return candidate;
        }
        i += 1;
    }
    candidate
}

/// cmark's `link_label`: `[`, up to 1000 bytes without an unescaped bracket,
/// `]`. Returns the label's range and the position after the `]`.
fn link_label(bytes: &[u8], pos: usize) -> Option<(usize, usize, usize)> {
    if bytes.get(pos) != Some(&b'[') {
        return None;
    }
    let mut i = pos + 1;
    let mut length = 0;
    while i < bytes.len() && bytes[i] != b'[' && bytes[i] != b']' {
        if bytes[i] == b'\\' {
            i += 1;
            length += 1;
            if i < bytes.len() && bytes[i].is_ascii_punctuation() {
                i += 1;
                length += 1;
            }
        } else {
            i += 1;
            length += 1;
        }
        if length > 1000 {
            return None;
        }
    }
    if bytes.get(i) == Some(&b']') {
        Some((pos + 1, i, i + 1))
    } else {
        None
    }
}

/// cmark's `normalize_reference`: full Unicode case folding, trimmed, runs
/// of whitespace collapsed to one space. `None` when nothing is left.
fn normalize_label(label: &str) -> Option<String> {
    let mut folded = String::new();
    for c in label.chars() {
        match CASE_FOLD_EXTRA.binary_search_by(|&(key, _)| key.cmp(&c)) {
            Ok(index) => folded.push_str(CASE_FOLD_EXTRA[index].1),
            Err(_) => folded.extend(c.to_lowercase()),
        }
    }
    let mut out = String::new();
    let mut last_was_space = false;
    for c in trim_cmark_space(&folded).chars() {
        if c.is_ascii() && is_cmark_space_byte(c as u8) {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(c);
            last_was_space = false;
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// cmark's `cmark_parse_reference_inline`: one `[label]: destination
/// "title"` definition at the start of `input`, recorded in `references`.
/// Returns how much of `input` it used.
fn parse_reference_definition(input: &str, references: &mut References) -> Option<usize> {
    let bytes = input.as_bytes();
    let (label_start, label_end, mut pos) = link_label(bytes, 0)?;
    let label = trim_cmark_space(&input[label_start..label_end]);
    if label.is_empty() || bytes.get(pos) != Some(&b':') {
        return None;
    }
    pos += 1;
    pos = skip_spaces_and_newline(bytes, pos);
    let (length, url_range) = scan_link_url(bytes, pos)?;
    pos += length;
    let url = clean_url(&input[url_range.0..url_range.1]);

    let before_title = pos;
    pos = skip_spaces_and_newline(bytes, pos);
    let title_length = if pos == before_title {
        0
    } else {
        scan_link_title(bytes, pos)
    };
    let title = if title_length > 0 {
        let title = clean_title(&input[pos..pos + title_length]);
        pos += title_length;
        title
    } else {
        pos = before_title;
        String::new()
    };

    // The rest of the line must be blank — or, failing that with a title,
    // the line must end right after the destination. (cmark keeps the title
    // it matched even then; it is never drawn, so that costs nothing.)
    pos = skip_spaces(bytes, pos);
    pos = match skip_line_end(bytes, pos) {
        Some(end) => end,
        None if title_length > 0 => skip_line_end(bytes, skip_spaces(bytes, before_title))?,
        None => return None,
    };
    references.insert(label, url, title);
    Some(pos)
}

fn skip_spaces(bytes: &[u8], mut pos: usize) -> usize {
    while matches!(bytes.get(pos), Some(b' ') | Some(b'\t')) {
        pos += 1;
    }
    pos
}

/// cmark's `skip_line_end`: past one line ending, or at the end of input.
fn skip_line_end(bytes: &[u8], pos: usize) -> Option<usize> {
    match bytes.get(pos) {
        Some(b'\n') => Some(pos + 1),
        None => Some(pos),
        _ => None,
    }
}

/// cmark's `spnl`: spaces, at most one newline, spaces.
fn skip_spaces_and_newline(bytes: &[u8], pos: usize) -> usize {
    let pos = skip_spaces(bytes, pos);
    match bytes.get(pos) {
        Some(b'\n') => skip_spaces(bytes, pos + 1),
        _ => pos,
    }
}

/// cmark's `cmark_clean_url`: trimmed, entities decoded, then backslash
/// escapes removed — in that order, as cmark does it.
fn clean_url(raw: &str) -> String {
    let trimmed = trim_cmark_space(raw);
    if trimmed.is_empty() {
        return String::new();
    }
    unescape_backslashes(&unescape_html(trimmed))
}

/// cmark's `cmark_clean_title`: the quotes or parentheses come off, then
/// entities, then escapes.
fn clean_title(raw: &str) -> String {
    let bytes = raw.as_bytes();
    if bytes.is_empty() {
        return String::new();
    }
    let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
    let inner = if bytes.len() >= 2
        && ((first == b'\'' && last == b'\'')
            || (first == b'(' && last == b')')
            || (first == b'"' && last == b'"'))
    {
        &raw[1..raw.len() - 1]
    } else {
        raw
    };
    unescape_backslashes(&unescape_html(inner))
}

/// cmark's `cmark_strbuf_unescape`: a backslash before ASCII punctuation
/// goes.
fn unescape_backslashes(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut copied = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && bytes.get(i + 1).is_some_and(|b| b.is_ascii_punctuation()) {
            out.push_str(&text[copied..i]);
            copied = i + 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// cmark's `houdini_unescape_html`: every `&entity;` decoded, any other `&`
/// kept.
fn unescape_html(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut copied = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some((decoded, length)) = unescape_entity(&bytes[i + 1..]) {
                out.push_str(&text[copied..i]);
                out.push_str(&decoded);
                i += 1 + length;
                copied = i;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&text[copied..]);
    out
}

/// cmark's `houdini_unescape_ent`, given the bytes after an `&`: a numeric
/// reference of one to EIGHT digits (Apple's fork keeps the old limit), or
/// a named one from the HTML5 table. Returns the text and the bytes used.
fn unescape_entity(src: &[u8]) -> Option<(String, usize)> {
    if src.len() >= 3 && src[0] == b'#' {
        let mut codepoint: u32 = 0;
        let (mut i, digits_start) = if src[1].is_ascii_digit() {
            (1, 1)
        } else if src[1] == b'x' || src[1] == b'X' {
            (2, 2)
        } else {
            return None;
        };
        let hex = digits_start == 2;
        while i < src.len()
            && (if hex {
                src[i].is_ascii_hexdigit()
            } else {
                src[i].is_ascii_digit()
            })
        {
            let digit = (src[i] as char).to_digit(16).unwrap_or(0);
            codepoint = codepoint * if hex { 16 } else { 10 } + digit;
            if codepoint >= 0x110000 {
                codepoint = 0x110000;
            }
            i += 1;
        }
        let digits = i - digits_start;
        if (1..=8).contains(&digits) && src.get(i) == Some(&b';') {
            let c = if codepoint == 0 || (0xD800..0xE000).contains(&codepoint) {
                '\u{FFFD}'
            } else {
                char::from_u32(codepoint).unwrap_or('\u{FFFD}')
            };
            return Some((c.to_string(), i + 1));
        }
        return None;
    }
    let size = src.len().min(32);
    for i in 2..size {
        if src[i] == b' ' {
            break;
        }
        if src[i] == b';' {
            let name = &src[..i];
            let index = HTML_ENTITIES
                .binary_search_by(|&(key, _)| key.as_bytes().cmp(name))
                .ok()?;
            return Some((HTML_ENTITIES[index].1.to_string(), i + 1));
        }
    }
    None
}

/// `<scheme:…>`: a 2–32 character scheme, `:`, anything but controls,
/// spaces and angle brackets, `>`. Returns the length after the `<`.
fn scan_autolink_uri(bytes: &[u8], pos: usize) -> Option<usize> {
    let rest = bytes.get(pos..)?;
    if !rest.first()?.is_ascii_alphabetic() {
        return None;
    }
    let scheme = 1 + rest[1..]
        .iter()
        .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'.' || b == b'+' || b == b'-')
        .count();
    if !(2..=32).contains(&scheme) || rest.get(scheme) != Some(&b':') {
        return None;
    }
    let mut i = scheme + 1;
    while i < rest.len() && rest[i] > 0x20 && rest[i] != b'<' && rest[i] != b'>' {
        i += 1;
    }
    (rest.get(i) == Some(&b'>')).then_some(i + 1)
}

/// `<user@host>` in the HTML5 email grammar.
fn scan_autolink_email(bytes: &[u8], pos: usize) -> Option<usize> {
    let rest = bytes.get(pos..)?;
    let local = rest
        .iter()
        .take_while(|&&b| b.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&b))
        .count();
    if local == 0 || rest.get(local) != Some(&b'@') {
        return None;
    }
    let mut i = local + 1;
    loop {
        let label = rest[i..]
            .iter()
            .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'-')
            .count();
        if label == 0 || label > 63 || rest[i] == b'-' || rest[i + label - 1] == b'-' {
            return None;
        }
        i += label;
        match rest.get(i) {
            Some(b'.') => i += 1,
            Some(b'>') => return Some(i + 1),
            _ => return None,
        }
    }
}

/// Raw HTML after a `<`: an open or close tag, a comment, a processing
/// instruction, a declaration or CDATA. Returns the length after the `<`.
///
/// Apple's fork mixes cmark generations here: comments follow the newer
/// grammar (`<!-->` and `<!--->` are comments, `--` inside is fine) while
/// declarations still need an upper-case name AND a space (`<!DOCTYPE>` is
/// text). Both measured.
fn scan_html_tag(bytes: &[u8], pos: usize) -> Option<usize> {
    let rest = bytes.get(pos..)?;
    let first = *rest.first()?;
    let length = match first {
        b'!' => {
            if rest.starts_with(b"!--") {
                scan_html_comment(rest)?
            } else if rest.starts_with(b"![CDATA[") {
                scan_html_cdata(rest)?
            } else {
                scan_html_declaration(rest)?
            }
        }
        b'?' => scan_html_processing(rest)?,
        b'/' => scan_close_tag(rest)?,
        _ if first.is_ascii_alphabetic() => scan_open_tag(rest)?,
        _ => return None,
    };
    Some(length)
}

fn scan_html_comment(rest: &[u8]) -> Option<usize> {
    if rest.starts_with(b"!-->") {
        return Some(4);
    }
    if rest.starts_with(b"!--->") {
        return Some(5);
    }
    let mut i = 3;
    while i < rest.len() {
        if rest[i..].starts_with(b"-->") {
            return Some(i + 3);
        }
        if rest[i] == b'-' {
            if rest.get(i + 1) == Some(&b'-') {
                // `--` then anything but `>`.
                match rest.get(i + 2) {
                    Some(&c) if c != b'>' => i += 3,
                    _ => return None,
                }
            } else {
                match rest.get(i + 1) {
                    Some(_) => i += 2,
                    None => return None,
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

fn scan_html_cdata(rest: &[u8]) -> Option<usize> {
    let mut i = 8;
    while i < rest.len() {
        if rest[i..].starts_with(b"]]>") {
            return Some(i + 3);
        }
        if rest[i] == b']' {
            if rest.get(i + 1) == Some(&b']') {
                match rest.get(i + 2) {
                    Some(&c) if c != b'>' => i += 3,
                    _ => return None,
                }
            } else {
                match rest.get(i + 1) {
                    Some(_) => i += 2,
                    None => return None,
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

fn scan_html_declaration(rest: &[u8]) -> Option<usize> {
    let name = rest[1..]
        .iter()
        .take_while(|b| b.is_ascii_uppercase())
        .count();
    if name == 0 {
        return None;
    }
    let mut i = 1 + name;
    let spaces = scan_spacechars(rest, i);
    if spaces == 0 {
        return None;
    }
    i += spaces;
    while i < rest.len() && rest[i] != b'>' {
        i += 1;
    }
    (i < rest.len()).then_some(i + 1)
}

fn scan_html_processing(rest: &[u8]) -> Option<usize> {
    let mut i = 1;
    while i < rest.len() {
        if rest[i..].starts_with(b"?>") {
            return Some(i + 2);
        }
        if rest[i] == b'?' {
            match rest.get(i + 1) {
                Some(&c) if c != b'>' => i += 2,
                _ => return None,
            }
        } else {
            i += 1;
        }
    }
    None
}

fn scan_close_tag(rest: &[u8]) -> Option<usize> {
    let mut i = 1;
    i += scan_tag_name(rest, i)?;
    i += scan_spacechars(rest, i);
    (rest.get(i) == Some(&b'>')).then_some(i + 1)
}

/// `tagname attribute* spacechar* /? >`
fn scan_open_tag(rest: &[u8]) -> Option<usize> {
    let mut i = scan_tag_name(rest, 0)?;
    loop {
        let spaces = scan_spacechars(rest, i);
        if spaces == 0 {
            break;
        }
        let name_start = i + spaces;
        let Some(name) = scan_attribute_name(rest, name_start) else {
            break;
        };
        let mut end = name_start + name;
        let before_equals = end + scan_spacechars(rest, end);
        if rest.get(before_equals) == Some(&b'=') {
            let value_start = before_equals + 1 + scan_spacechars(rest, before_equals + 1);
            if let Some(value) = scan_attribute_value(rest, value_start) {
                end = value_start + value;
            }
        }
        i = end;
    }
    i += scan_spacechars(rest, i);
    if rest.get(i) == Some(&b'/') {
        i += 1;
    }
    (rest.get(i) == Some(&b'>')).then_some(i + 1)
}

/// `[A-Za-z][A-Za-z0-9-]*`
fn scan_tag_name(rest: &[u8], pos: usize) -> Option<usize> {
    if !rest.get(pos)?.is_ascii_alphabetic() {
        return None;
    }
    Some(
        1 + rest[pos + 1..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || **b == b'-')
            .count(),
    )
}

/// `[a-zA-Z_:][a-zA-Z0-9:._-]*`
fn scan_attribute_name(rest: &[u8], pos: usize) -> Option<usize> {
    let first = *rest.get(pos)?;
    if !(first.is_ascii_alphabetic() || first == b'_' || first == b':') {
        return None;
    }
    Some(
        1 + rest[pos + 1..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || b":._-".contains(b))
            .count(),
    )
}

/// An unquoted, single-quoted or double-quoted attribute value.
fn scan_attribute_value(rest: &[u8], pos: usize) -> Option<usize> {
    let first = *rest.get(pos)?;
    if first == b'"' || first == b'\'' {
        let close = rest[pos + 1..].iter().position(|&b| b == first)?;
        return Some(close + 2);
    }
    let length = rest[pos..]
        .iter()
        .take_while(|&&b| !is_cmark_space_byte(b) && !b"\"'=<>`".contains(&b))
        .count();
    (length > 0).then_some(length)
}

// MARK: - The GFM autolink extension's helpers (autolink.c)

/// A character a host may start with: not cmark whitespace, not cmark
/// punctuation. A position inside a multi-byte character is not one — cmark
/// walks bytes and a continuation byte does not decode.
fn is_valid_hostchar(data: &[u8], pos: usize) -> bool {
    decode_at(data, pos).is_some_and(|c| !is_cmark_space(c) && !is_cmark_punctuation(c))
}

fn is_utf8_continuation(byte: u8) -> bool {
    byte >> 6 == 2
}

/// The one UTF-8 sequence starting at `pos`, or `None` when `pos` is not
/// the start of one.
fn decode_at(data: &[u8], pos: usize) -> Option<char> {
    let first = *data.get(pos)?;
    let width = match first {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return None,
    };
    let sequence = data.get(pos..pos + width)?;
    std::str::from_utf8(sequence).ok()?.chars().next()
}

/// `sd_autolink_issafe`: the text starts with `http://`, `https://` or
/// `ftp://` (any case) and a host character.
fn autolink_is_safe(link: &[u8]) -> bool {
    [&b"http://"[..], b"https://", b"ftp://"]
        .iter()
        .any(|scheme| {
            link.len() > scheme.len()
                && link[..scheme.len()].eq_ignore_ascii_case(scheme)
                && is_valid_hostchar(link, scheme.len())
        })
}

/// `check_domain`: how much of `data` is a domain. An underscore in either
/// of the last two labels means it is not one; without `allow_short` it also
/// needs a dot. The first and the last byte are never examined — cmark's
/// loop runs from 1 to `size - 2`.
fn check_domain(data: &[u8], allow_short: bool) -> usize {
    let size = data.len();
    let (mut dots, mut underscores_before, mut underscores) = (0, 0, 0);
    let mut i = 1;
    while i + 1 < size {
        if data[i] == b'\\' && i + 2 < size {
            i += 1;
        }
        if data[i] == b'_' {
            underscores += 1;
        } else if data[i] == b'.' {
            underscores_before = underscores;
            underscores = 0;
            dots += 1;
        } else if !is_valid_hostchar(data, i) && data[i] != b'-' {
            break;
        }
        i += 1;
    }
    if underscores_before > 0 || underscores > 0 {
        return 0;
    }
    if allow_short || dots > 0 {
        i
    } else {
        0
    }
}

/// `autolink_delim`: trailing punctuation is not part of a URL, a `)` is
/// only when it closes a `(` inside, and `&name;` at the end is an entity.
fn autolink_delim(data: &[u8], mut link_end: usize) -> usize {
    if let Some(angle) = data[..link_end].iter().position(|&b| b == b'<') {
        link_end = angle;
    }
    while link_end > 0 {
        let last = data[link_end - 1];
        if b"?!.,:*_~'\"".contains(&last) {
            link_end -= 1;
        } else if last == b';' {
            // `&amp;` at the end: cut before the `&`.
            let mut new_end = link_end.saturating_sub(2);
            while new_end > 0 && data[new_end].is_ascii_alphabetic() {
                new_end -= 1;
            }
            if link_end >= 2 && new_end < link_end - 2 && data[new_end] == b'&' {
                link_end = new_end;
            } else {
                link_end -= 1;
            }
        } else if last == b')' {
            let opening = data[..link_end].iter().filter(|&&b| b == b'(').count();
            let closing = data[..link_end].iter().filter(|&&b| b == b')').count();
            if closing <= opening {
                break;
            }
            link_end -= 1;
        } else {
            break;
        }
    }
    link_end
}

/// `validate_protocol`: `mailto:` or `xmpp:` right before the address,
/// itself at the start or after something that is not a letter or digit.
fn validate_protocol(
    protocol: &[u8],
    data: &[u8],
    at: usize,
    rewind: usize,
    max_rewind: usize,
) -> bool {
    let len = protocol.len();
    if len > max_rewind - rewind {
        return false;
    }
    let start = at - rewind - len;
    if &data[start..start + len] != protocol {
        return false;
    }
    if len == max_rewind - rewind {
        return true;
    }
    !data[start - 1].is_ascii_alphanumeric()
}

// MARK: - Foundation's `URL(string:)`

/// Whether Foundation's `URL(string:)` would take this destination — a
/// destination it refuses is drawn as plain text, never as a link.
///
/// A best-effort mirror, measured rather than specified: empty is refused;
/// when the first of `: / ? # [ ] @` is a `:`, what comes before it must be
/// a valid scheme or nothing (`1a:b` is refused, `x@y:z` and `]a:b` are
/// not); after `//` the host must be a bracketed literal or characters RFC
/// 3986 allows, and the port digits. Paths, queries and fragments are
/// percent-encoded by Foundation, never refused. A host with anything
/// non-ASCII in it goes through IDNA — see `idna_allows` and
/// `idna_labels_valid`.
fn foundation_accepts_url(url: &str) -> bool {
    if url.is_empty() {
        return false;
    }
    let rest = match url.find([':', '/', '?', '#', '[', ']', '@']) {
        Some(index) if url.as_bytes()[index] == b':' => {
            let scheme = &url[..index];
            let valid_scheme = scheme
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic())
                && scheme
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b));
            if !scheme.is_empty() && !valid_scheme {
                return false;
            }
            &url[index + 1..]
        }
        _ => url,
    };
    let Some(after) = rest.strip_prefix("//") else {
        return true;
    };
    let authority = &after[..after.find(['/', '?', '#']).unwrap_or(after.len())];
    let host_and_port = match authority.rfind('@') {
        Some(index) => &authority[index + 1..],
        None => authority,
    };
    // A host typed as `[…]` is a literal as it stands; one that only becomes
    // `[…]` once IDNA drops what it ignores (`https://\u{200B}[b]`) is one
    // too. Inside, only the RFC 3986 literal characters.
    let literal_source = if host_and_port.starts_with('[') {
        host_and_port.to_string()
    } else {
        host_and_port
            .chars()
            .filter(|&c| !in_ranges(IDNA_IGNORED, c))
            .collect()
    };
    if let Some(literal) = literal_source.strip_prefix('[') {
        let Some(close) = literal.find(']') else {
            return false;
        };
        let contents_valid = literal[..close]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-._~!$&'()*+,;=:%".contains(c));
        if !contents_valid {
            return false;
        }
        let after_literal = &literal[close + 1..];
        return after_literal.is_empty()
            || after_literal
                .strip_prefix(':')
                .is_some_and(|port| port.bytes().all(|b| b.is_ascii_digit()));
    }
    let (host, port) = match host_and_port.find(':') {
        Some(index) => (&host_and_port[..index], &host_and_port[index + 1..]),
        None => (host_and_port, ""),
    };
    if !port.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let bytes = host.as_bytes();
    let hex_at = |index: usize| bytes.get(index).is_some_and(|b| b.is_ascii_hexdigit());
    let mut non_ascii = false;
    let mut chars = host.char_indices();
    while let Some((offset, c)) = chars.next() {
        if c.is_ascii_alphanumeric() || "-._~!$&'()*+,;=".contains(c) {
            continue;
        }
        if c == '%' {
            if !(hex_at(offset + 1) && hex_at(offset + 2)) {
                return false;
            }
            chars.next();
            chars.next();
            continue;
        }
        if c.is_ascii() || !idna_allows(c) {
            return false;
        }
        non_ascii = true;
    }
    if non_ascii && !idna_labels_valid(host) {
        return false;
    }
    true
}

/// UTS #46 over a host with anything non-ASCII in it, label by label (IDNA
/// knows three more dots): only the last label may be empty (`é.` is fine,
/// `é..com` is not), and a last label that IDNA empties (`é.\u{200B}`) only
/// while something non-ASCII is left elsewhere (`www.\u{200B}` is refused —
/// measured, not explained); a label may not start or end with `-` nor hold
/// `--` in its third and fourth places (CheckHyphens — ASCII-only hosts skip
/// IDNA and keep `-a.com`); and it may not start with a combining mark.
fn idna_labels_valid(host: &str) -> bool {
    let labels: Vec<&str> = host
        .split(['.', '\u{3002}', '\u{FF0E}', '\u{FF61}'])
        .collect();
    let last = labels.len() - 1;
    let keeps_non_ascii = host
        .chars()
        .any(|c| !c.is_ascii() && !in_ranges(IDNA_IGNORED, c));
    for (index, label) in labels.iter().enumerate() {
        let mapped: Vec<char> = label
            .chars()
            .filter(|&c| !in_ranges(IDNA_IGNORED, c))
            .collect();
        if mapped.is_empty() {
            if index != last || last == 0 || (!label.is_empty() && !keeps_non_ascii) {
                return false;
            }
            continue;
        }
        if mapped[0] == '-' || mapped[mapped.len() - 1] == '-' {
            return false;
        }
        if mapped.len() >= 4 && mapped[2] == '-' && mapped[3] == '-' {
            return false;
        }
        // A combining mark is exactly a character that would join an `a`
        // in front of it into one cluster.
        let joined: String = std::iter::once('a').chain(mapped.iter().copied()).collect();
        if joined
            .graphemes(true)
            .next()
            .is_some_and(|first| first.len() > 1)
        {
            return false;
        }
    }
    true
}

/// A non-ASCII character Foundation's IDNA lets into a host, as far as one
/// character decides it — so `https://example.com👍` is a link on Apple but
/// `https://example.com👨‍👩‍👧` is not (the family emoji holds U+200D). What
/// one character cannot decide — the bidi rule (a Hebrew letter in a Latin
/// label), the contextual rules — is not modelled.
fn idna_allows(c: char) -> bool {
    !(c.is_whitespace() || c.is_control() || in_ranges(IDNA_REFUSED, c))
}

fn in_ranges(ranges: &[(u32, u32)], c: char) -> bool {
    let value = c as u32;
    ranges
        .binary_search_by(|&(low, high)| {
            if high < value {
                std::cmp::Ordering::Less
            } else if low > value {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

// MARK: - Tables

/// cmark's `cmark_utf8proc_is_punctuation` above ASCII: the Unicode `P*`
/// categories as that (pre-0.31) table froze them. Extracted from Foundation by
/// probing every scalar, not copied from a spec, because a later Unicode or a
/// later cmark (0.31 adds the `S*` symbols) would answer differently.
#[rustfmt::skip]
const CMARK_PUNCTUATION: &[(u32, u32)] = &[
    (0xA1, 0xA1), (0xA7, 0xA7), (0xAB, 0xAB), (0xB6, 0xB7), (0xBB, 0xBB), (0xBF, 0xBF),
    (0x37E, 0x37E), (0x387, 0x387), (0x55A, 0x55F), (0x589, 0x58A), (0x5BE, 0x5BE), (0x5C0, 0x5C0),
    (0x5C3, 0x5C3), (0x5C6, 0x5C6), (0x5F3, 0x5F4), (0x609, 0x60A), (0x60C, 0x60D), (0x61B, 0x61B),
    (0x61E, 0x61F), (0x66A, 0x66D), (0x6D4, 0x6D4), (0x700, 0x70D), (0x7F7, 0x7F9), (0x830, 0x83E),
    (0x85E, 0x85E), (0x964, 0x965), (0x970, 0x970), (0xAF0, 0xAF0), (0xDF4, 0xDF4), (0xE4F, 0xE4F),
    (0xE5A, 0xE5B), (0xF04, 0xF12), (0xF14, 0xF14), (0xF3A, 0xF3D), (0xF85, 0xF85), (0xFD0, 0xFD4),
    (0xFD9, 0xFDA), (0x104A, 0x104F), (0x10FB, 0x10FB), (0x1360, 0x1368), (0x1400, 0x1400), (0x166D, 0x166E),
    (0x169B, 0x169C), (0x16EB, 0x16ED), (0x1735, 0x1736), (0x17D4, 0x17D6), (0x17D8, 0x17DA), (0x1800, 0x180A),
    (0x1944, 0x1945), (0x1A1E, 0x1A1F), (0x1AA0, 0x1AA6), (0x1AA8, 0x1AAD), (0x1B5A, 0x1B60), (0x1BFC, 0x1BFF),
    (0x1C3B, 0x1C3F), (0x1C7E, 0x1C7F), (0x1CC0, 0x1CC7), (0x1CD3, 0x1CD3), (0x2010, 0x2027), (0x2030, 0x2043),
    (0x2045, 0x2051), (0x2053, 0x205E), (0x207D, 0x207E), (0x208D, 0x208E), (0x2308, 0x230B), (0x2329, 0x232A),
    (0x2768, 0x2775), (0x27C5, 0x27C6), (0x27E6, 0x27EF), (0x2983, 0x2998), (0x29D8, 0x29DB), (0x29FC, 0x29FD),
    (0x2CF9, 0x2CFC), (0x2CFE, 0x2CFF), (0x2D70, 0x2D70), (0x2E00, 0x2E2E), (0x2E30, 0x2E42), (0x3001, 0x3003),
    (0x3008, 0x3011), (0x3014, 0x301F), (0x3030, 0x3030), (0x303D, 0x303D), (0x30A0, 0x30A0), (0x30FB, 0x30FB),
    (0xA4FE, 0xA4FF), (0xA60D, 0xA60F), (0xA673, 0xA673), (0xA67E, 0xA67E), (0xA6F2, 0xA6F7), (0xA874, 0xA877),
    (0xA8CE, 0xA8CF), (0xA8F8, 0xA8FA), (0xA92E, 0xA92F), (0xA95F, 0xA95F), (0xA9C1, 0xA9CD), (0xA9DE, 0xA9DF),
    (0xAA5C, 0xAA5F), (0xAADE, 0xAADF), (0xAAF0, 0xAAF1), (0xABEB, 0xABEB), (0xFD3E, 0xFD3F), (0xFE10, 0xFE19),
    (0xFE30, 0xFE52), (0xFE54, 0xFE61), (0xFE63, 0xFE63), (0xFE68, 0xFE68), (0xFE6A, 0xFE6B), (0xFF01, 0xFF03),
    (0xFF05, 0xFF0A), (0xFF0C, 0xFF0F), (0xFF1A, 0xFF1B), (0xFF1F, 0xFF20), (0xFF3B, 0xFF3D), (0xFF3F, 0xFF3F),
    (0xFF5B, 0xFF5B), (0xFF5D, 0xFF5D), (0xFF5F, 0xFF65), (0x10100, 0x10102), (0x1039F, 0x1039F), (0x103D0, 0x103D0),
    (0x1056F, 0x1056F), (0x10857, 0x10857), (0x1091F, 0x1091F), (0x1093F, 0x1093F), (0x10A50, 0x10A58), (0x10A7F, 0x10A7F),
    (0x10AF0, 0x10AF6), (0x10B39, 0x10B3F), (0x10B99, 0x10B9C), (0x11047, 0x1104D), (0x110BB, 0x110BC), (0x110BE, 0x110C1),
    (0x11140, 0x11143), (0x11174, 0x11175), (0x111C5, 0x111C8), (0x111CD, 0x111CD), (0x11238, 0x1123D), (0x114C6, 0x114C6),
    (0x115C1, 0x115C9), (0x11641, 0x11643), (0x12470, 0x12474), (0x16A6E, 0x16A6F), (0x16AF5, 0x16AF5), (0x16B37, 0x16B3B),
    (0x16B44, 0x16B44), (0x1BC9F, 0x1BC9F),
];

/// The HTML5 named character references cmark decodes (`&amp;`, `&ngE;`, ...),
/// name without the `;`, sorted for a binary search. All 2125 were checked
/// against Foundation's own output.
#[rustfmt::skip]
const HTML_ENTITIES: &[(&str, &str)] = &[
    ("AElig", "\u{C6}"), ("AMP", "&"), ("Aacute", "\u{C1}"), ("Abreve", "\u{102}"),
    ("Acirc", "\u{C2}"), ("Acy", "\u{410}"), ("Afr", "\u{1D504}"), ("Agrave", "\u{C0}"),
    ("Alpha", "\u{391}"), ("Amacr", "\u{100}"), ("And", "\u{2A53}"), ("Aogon", "\u{104}"),
    ("Aopf", "\u{1D538}"), ("ApplyFunction", "\u{2061}"), ("Aring", "\u{C5}"),
    ("Ascr", "\u{1D49C}"), ("Assign", "\u{2254}"), ("Atilde", "\u{C3}"), ("Auml", "\u{C4}"),
    ("Backslash", "\u{2216}"), ("Barv", "\u{2AE7}"), ("Barwed", "\u{2306}"),
    ("Bcy", "\u{411}"), ("Because", "\u{2235}"), ("Bernoullis", "\u{212C}"),
    ("Beta", "\u{392}"), ("Bfr", "\u{1D505}"), ("Bopf", "\u{1D539}"), ("Breve", "\u{2D8}"),
    ("Bscr", "\u{212C}"), ("Bumpeq", "\u{224E}"), ("CHcy", "\u{427}"), ("COPY", "\u{A9}"),
    ("Cacute", "\u{106}"), ("Cap", "\u{22D2}"), ("CapitalDifferentialD", "\u{2145}"),
    ("Cayleys", "\u{212D}"), ("Ccaron", "\u{10C}"), ("Ccedil", "\u{C7}"), ("Ccirc", "\u{108}"),
    ("Cconint", "\u{2230}"), ("Cdot", "\u{10A}"), ("Cedilla", "\u{B8}"),
    ("CenterDot", "\u{B7}"), ("Cfr", "\u{212D}"), ("Chi", "\u{3A7}"),
    ("CircleDot", "\u{2299}"), ("CircleMinus", "\u{2296}"), ("CirclePlus", "\u{2295}"),
    ("CircleTimes", "\u{2297}"), ("ClockwiseContourIntegral", "\u{2232}"),
    ("CloseCurlyDoubleQuote", "\u{201D}"), ("CloseCurlyQuote", "\u{2019}"),
    ("Colon", "\u{2237}"), ("Colone", "\u{2A74}"), ("Congruent", "\u{2261}"),
    ("Conint", "\u{222F}"), ("ContourIntegral", "\u{222E}"), ("Copf", "\u{2102}"),
    ("Coproduct", "\u{2210}"), ("CounterClockwiseContourIntegral", "\u{2233}"),
    ("Cross", "\u{2A2F}"), ("Cscr", "\u{1D49E}"), ("Cup", "\u{22D3}"), ("CupCap", "\u{224D}"),
    ("DD", "\u{2145}"), ("DDotrahd", "\u{2911}"), ("DJcy", "\u{402}"), ("DScy", "\u{405}"),
    ("DZcy", "\u{40F}"), ("Dagger", "\u{2021}"), ("Darr", "\u{21A1}"), ("Dashv", "\u{2AE4}"),
    ("Dcaron", "\u{10E}"), ("Dcy", "\u{414}"), ("Del", "\u{2207}"), ("Delta", "\u{394}"),
    ("Dfr", "\u{1D507}"), ("DiacriticalAcute", "\u{B4}"), ("DiacriticalDot", "\u{2D9}"),
    ("DiacriticalDoubleAcute", "\u{2DD}"), ("DiacriticalGrave", "`"),
    ("DiacriticalTilde", "\u{2DC}"), ("Diamond", "\u{22C4}"), ("DifferentialD", "\u{2146}"),
    ("Dopf", "\u{1D53B}"), ("Dot", "\u{A8}"), ("DotDot", "\u{20DC}"), ("DotEqual", "\u{2250}"),
    ("DoubleContourIntegral", "\u{222F}"), ("DoubleDot", "\u{A8}"),
    ("DoubleDownArrow", "\u{21D3}"), ("DoubleLeftArrow", "\u{21D0}"),
    ("DoubleLeftRightArrow", "\u{21D4}"), ("DoubleLeftTee", "\u{2AE4}"),
    ("DoubleLongLeftArrow", "\u{27F8}"), ("DoubleLongLeftRightArrow", "\u{27FA}"),
    ("DoubleLongRightArrow", "\u{27F9}"), ("DoubleRightArrow", "\u{21D2}"),
    ("DoubleRightTee", "\u{22A8}"), ("DoubleUpArrow", "\u{21D1}"),
    ("DoubleUpDownArrow", "\u{21D5}"), ("DoubleVerticalBar", "\u{2225}"),
    ("DownArrow", "\u{2193}"), ("DownArrowBar", "\u{2913}"), ("DownArrowUpArrow", "\u{21F5}"),
    ("DownBreve", "\u{311}"), ("DownLeftRightVector", "\u{2950}"),
    ("DownLeftTeeVector", "\u{295E}"), ("DownLeftVector", "\u{21BD}"),
    ("DownLeftVectorBar", "\u{2956}"), ("DownRightTeeVector", "\u{295F}"),
    ("DownRightVector", "\u{21C1}"), ("DownRightVectorBar", "\u{2957}"),
    ("DownTee", "\u{22A4}"), ("DownTeeArrow", "\u{21A7}"), ("Downarrow", "\u{21D3}"),
    ("Dscr", "\u{1D49F}"), ("Dstrok", "\u{110}"), ("ENG", "\u{14A}"), ("ETH", "\u{D0}"),
    ("Eacute", "\u{C9}"), ("Ecaron", "\u{11A}"), ("Ecirc", "\u{CA}"), ("Ecy", "\u{42D}"),
    ("Edot", "\u{116}"), ("Efr", "\u{1D508}"), ("Egrave", "\u{C8}"), ("Element", "\u{2208}"),
    ("Emacr", "\u{112}"), ("EmptySmallSquare", "\u{25FB}"),
    ("EmptyVerySmallSquare", "\u{25AB}"), ("Eogon", "\u{118}"), ("Eopf", "\u{1D53C}"),
    ("Epsilon", "\u{395}"), ("Equal", "\u{2A75}"), ("EqualTilde", "\u{2242}"),
    ("Equilibrium", "\u{21CC}"), ("Escr", "\u{2130}"), ("Esim", "\u{2A73}"),
    ("Eta", "\u{397}"), ("Euml", "\u{CB}"), ("Exists", "\u{2203}"),
    ("ExponentialE", "\u{2147}"), ("Fcy", "\u{424}"), ("Ffr", "\u{1D509}"),
    ("FilledSmallSquare", "\u{25FC}"), ("FilledVerySmallSquare", "\u{25AA}"),
    ("Fopf", "\u{1D53D}"), ("ForAll", "\u{2200}"), ("Fouriertrf", "\u{2131}"),
    ("Fscr", "\u{2131}"), ("GJcy", "\u{403}"), ("GT", ">"), ("Gamma", "\u{393}"),
    ("Gammad", "\u{3DC}"), ("Gbreve", "\u{11E}"), ("Gcedil", "\u{122}"), ("Gcirc", "\u{11C}"),
    ("Gcy", "\u{413}"), ("Gdot", "\u{120}"), ("Gfr", "\u{1D50A}"), ("Gg", "\u{22D9}"),
    ("Gopf", "\u{1D53E}"), ("GreaterEqual", "\u{2265}"), ("GreaterEqualLess", "\u{22DB}"),
    ("GreaterFullEqual", "\u{2267}"), ("GreaterGreater", "\u{2AA2}"),
    ("GreaterLess", "\u{2277}"), ("GreaterSlantEqual", "\u{2A7E}"),
    ("GreaterTilde", "\u{2273}"), ("Gscr", "\u{1D4A2}"), ("Gt", "\u{226B}"),
    ("HARDcy", "\u{42A}"), ("Hacek", "\u{2C7}"), ("Hat", "^"), ("Hcirc", "\u{124}"),
    ("Hfr", "\u{210C}"), ("HilbertSpace", "\u{210B}"), ("Hopf", "\u{210D}"),
    ("HorizontalLine", "\u{2500}"), ("Hscr", "\u{210B}"), ("Hstrok", "\u{126}"),
    ("HumpDownHump", "\u{224E}"), ("HumpEqual", "\u{224F}"), ("IEcy", "\u{415}"),
    ("IJlig", "\u{132}"), ("IOcy", "\u{401}"), ("Iacute", "\u{CD}"), ("Icirc", "\u{CE}"),
    ("Icy", "\u{418}"), ("Idot", "\u{130}"), ("Ifr", "\u{2111}"), ("Igrave", "\u{CC}"),
    ("Im", "\u{2111}"), ("Imacr", "\u{12A}"), ("ImaginaryI", "\u{2148}"),
    ("Implies", "\u{21D2}"), ("Int", "\u{222C}"), ("Integral", "\u{222B}"),
    ("Intersection", "\u{22C2}"), ("InvisibleComma", "\u{2063}"),
    ("InvisibleTimes", "\u{2062}"), ("Iogon", "\u{12E}"), ("Iopf", "\u{1D540}"),
    ("Iota", "\u{399}"), ("Iscr", "\u{2110}"), ("Itilde", "\u{128}"), ("Iukcy", "\u{406}"),
    ("Iuml", "\u{CF}"), ("Jcirc", "\u{134}"), ("Jcy", "\u{419}"), ("Jfr", "\u{1D50D}"),
    ("Jopf", "\u{1D541}"), ("Jscr", "\u{1D4A5}"), ("Jsercy", "\u{408}"), ("Jukcy", "\u{404}"),
    ("KHcy", "\u{425}"), ("KJcy", "\u{40C}"), ("Kappa", "\u{39A}"), ("Kcedil", "\u{136}"),
    ("Kcy", "\u{41A}"), ("Kfr", "\u{1D50E}"), ("Kopf", "\u{1D542}"), ("Kscr", "\u{1D4A6}"),
    ("LJcy", "\u{409}"), ("LT", "<"), ("Lacute", "\u{139}"), ("Lambda", "\u{39B}"),
    ("Lang", "\u{27EA}"), ("Laplacetrf", "\u{2112}"), ("Larr", "\u{219E}"),
    ("Lcaron", "\u{13D}"), ("Lcedil", "\u{13B}"), ("Lcy", "\u{41B}"),
    ("LeftAngleBracket", "\u{27E8}"), ("LeftArrow", "\u{2190}"), ("LeftArrowBar", "\u{21E4}"),
    ("LeftArrowRightArrow", "\u{21C6}"), ("LeftCeiling", "\u{2308}"),
    ("LeftDoubleBracket", "\u{27E6}"), ("LeftDownTeeVector", "\u{2961}"),
    ("LeftDownVector", "\u{21C3}"), ("LeftDownVectorBar", "\u{2959}"),
    ("LeftFloor", "\u{230A}"), ("LeftRightArrow", "\u{2194}"), ("LeftRightVector", "\u{294E}"),
    ("LeftTee", "\u{22A3}"), ("LeftTeeArrow", "\u{21A4}"), ("LeftTeeVector", "\u{295A}"),
    ("LeftTriangle", "\u{22B2}"), ("LeftTriangleBar", "\u{29CF}"),
    ("LeftTriangleEqual", "\u{22B4}"), ("LeftUpDownVector", "\u{2951}"),
    ("LeftUpTeeVector", "\u{2960}"), ("LeftUpVector", "\u{21BF}"),
    ("LeftUpVectorBar", "\u{2958}"), ("LeftVector", "\u{21BC}"), ("LeftVectorBar", "\u{2952}"),
    ("Leftarrow", "\u{21D0}"), ("Leftrightarrow", "\u{21D4}"),
    ("LessEqualGreater", "\u{22DA}"), ("LessFullEqual", "\u{2266}"),
    ("LessGreater", "\u{2276}"), ("LessLess", "\u{2AA1}"), ("LessSlantEqual", "\u{2A7D}"),
    ("LessTilde", "\u{2272}"), ("Lfr", "\u{1D50F}"), ("Ll", "\u{22D8}"),
    ("Lleftarrow", "\u{21DA}"), ("Lmidot", "\u{13F}"), ("LongLeftArrow", "\u{27F5}"),
    ("LongLeftRightArrow", "\u{27F7}"), ("LongRightArrow", "\u{27F6}"),
    ("Longleftarrow", "\u{27F8}"), ("Longleftrightarrow", "\u{27FA}"),
    ("Longrightarrow", "\u{27F9}"), ("Lopf", "\u{1D543}"), ("LowerLeftArrow", "\u{2199}"),
    ("LowerRightArrow", "\u{2198}"), ("Lscr", "\u{2112}"), ("Lsh", "\u{21B0}"),
    ("Lstrok", "\u{141}"), ("Lt", "\u{226A}"), ("Map", "\u{2905}"), ("Mcy", "\u{41C}"),
    ("MediumSpace", "\u{205F}"), ("Mellintrf", "\u{2133}"), ("Mfr", "\u{1D510}"),
    ("MinusPlus", "\u{2213}"), ("Mopf", "\u{1D544}"), ("Mscr", "\u{2133}"), ("Mu", "\u{39C}"),
    ("NJcy", "\u{40A}"), ("Nacute", "\u{143}"), ("Ncaron", "\u{147}"), ("Ncedil", "\u{145}"),
    ("Ncy", "\u{41D}"), ("NegativeMediumSpace", "\u{200B}"),
    ("NegativeThickSpace", "\u{200B}"), ("NegativeThinSpace", "\u{200B}"),
    ("NegativeVeryThinSpace", "\u{200B}"), ("NestedGreaterGreater", "\u{226B}"),
    ("NestedLessLess", "\u{226A}"), ("NewLine", "\u{A}"), ("Nfr", "\u{1D511}"),
    ("NoBreak", "\u{2060}"), ("NonBreakingSpace", "\u{A0}"), ("Nopf", "\u{2115}"),
    ("Not", "\u{2AEC}"), ("NotCongruent", "\u{2262}"), ("NotCupCap", "\u{226D}"),
    ("NotDoubleVerticalBar", "\u{2226}"), ("NotElement", "\u{2209}"), ("NotEqual", "\u{2260}"),
    ("NotEqualTilde", "\u{2242}\u{338}"), ("NotExists", "\u{2204}"),
    ("NotGreater", "\u{226F}"), ("NotGreaterEqual", "\u{2271}"),
    ("NotGreaterFullEqual", "\u{2267}\u{338}"), ("NotGreaterGreater", "\u{226B}\u{338}"),
    ("NotGreaterLess", "\u{2279}"), ("NotGreaterSlantEqual", "\u{2A7E}\u{338}"),
    ("NotGreaterTilde", "\u{2275}"), ("NotHumpDownHump", "\u{224E}\u{338}"),
    ("NotHumpEqual", "\u{224F}\u{338}"), ("NotLeftTriangle", "\u{22EA}"),
    ("NotLeftTriangleBar", "\u{29CF}\u{338}"), ("NotLeftTriangleEqual", "\u{22EC}"),
    ("NotLess", "\u{226E}"), ("NotLessEqual", "\u{2270}"), ("NotLessGreater", "\u{2278}"),
    ("NotLessLess", "\u{226A}\u{338}"), ("NotLessSlantEqual", "\u{2A7D}\u{338}"),
    ("NotLessTilde", "\u{2274}"), ("NotNestedGreaterGreater", "\u{2AA2}\u{338}"),
    ("NotNestedLessLess", "\u{2AA1}\u{338}"), ("NotPrecedes", "\u{2280}"),
    ("NotPrecedesEqual", "\u{2AAF}\u{338}"), ("NotPrecedesSlantEqual", "\u{22E0}"),
    ("NotReverseElement", "\u{220C}"), ("NotRightTriangle", "\u{22EB}"),
    ("NotRightTriangleBar", "\u{29D0}\u{338}"), ("NotRightTriangleEqual", "\u{22ED}"),
    ("NotSquareSubset", "\u{228F}\u{338}"), ("NotSquareSubsetEqual", "\u{22E2}"),
    ("NotSquareSuperset", "\u{2290}\u{338}"), ("NotSquareSupersetEqual", "\u{22E3}"),
    ("NotSubset", "\u{2282}\u{20D2}"), ("NotSubsetEqual", "\u{2288}"),
    ("NotSucceeds", "\u{2281}"), ("NotSucceedsEqual", "\u{2AB0}\u{338}"),
    ("NotSucceedsSlantEqual", "\u{22E1}"), ("NotSucceedsTilde", "\u{227F}\u{338}"),
    ("NotSuperset", "\u{2283}\u{20D2}"), ("NotSupersetEqual", "\u{2289}"),
    ("NotTilde", "\u{2241}"), ("NotTildeEqual", "\u{2244}"), ("NotTildeFullEqual", "\u{2247}"),
    ("NotTildeTilde", "\u{2249}"), ("NotVerticalBar", "\u{2224}"), ("Nscr", "\u{1D4A9}"),
    ("Ntilde", "\u{D1}"), ("Nu", "\u{39D}"), ("OElig", "\u{152}"), ("Oacute", "\u{D3}"),
    ("Ocirc", "\u{D4}"), ("Ocy", "\u{41E}"), ("Odblac", "\u{150}"), ("Ofr", "\u{1D512}"),
    ("Ograve", "\u{D2}"), ("Omacr", "\u{14C}"), ("Omega", "\u{3A9}"), ("Omicron", "\u{39F}"),
    ("Oopf", "\u{1D546}"), ("OpenCurlyDoubleQuote", "\u{201C}"),
    ("OpenCurlyQuote", "\u{2018}"), ("Or", "\u{2A54}"), ("Oscr", "\u{1D4AA}"),
    ("Oslash", "\u{D8}"), ("Otilde", "\u{D5}"), ("Otimes", "\u{2A37}"), ("Ouml", "\u{D6}"),
    ("OverBar", "\u{203E}"), ("OverBrace", "\u{23DE}"), ("OverBracket", "\u{23B4}"),
    ("OverParenthesis", "\u{23DC}"), ("PartialD", "\u{2202}"), ("Pcy", "\u{41F}"),
    ("Pfr", "\u{1D513}"), ("Phi", "\u{3A6}"), ("Pi", "\u{3A0}"), ("PlusMinus", "\u{B1}"),
    ("Poincareplane", "\u{210C}"), ("Popf", "\u{2119}"), ("Pr", "\u{2ABB}"),
    ("Precedes", "\u{227A}"), ("PrecedesEqual", "\u{2AAF}"),
    ("PrecedesSlantEqual", "\u{227C}"), ("PrecedesTilde", "\u{227E}"), ("Prime", "\u{2033}"),
    ("Product", "\u{220F}"), ("Proportion", "\u{2237}"), ("Proportional", "\u{221D}"),
    ("Pscr", "\u{1D4AB}"), ("Psi", "\u{3A8}"), ("QUOT", "\""), ("Qfr", "\u{1D514}"),
    ("Qopf", "\u{211A}"), ("Qscr", "\u{1D4AC}"), ("RBarr", "\u{2910}"), ("REG", "\u{AE}"),
    ("Racute", "\u{154}"), ("Rang", "\u{27EB}"), ("Rarr", "\u{21A0}"), ("Rarrtl", "\u{2916}"),
    ("Rcaron", "\u{158}"), ("Rcedil", "\u{156}"), ("Rcy", "\u{420}"), ("Re", "\u{211C}"),
    ("ReverseElement", "\u{220B}"), ("ReverseEquilibrium", "\u{21CB}"),
    ("ReverseUpEquilibrium", "\u{296F}"), ("Rfr", "\u{211C}"), ("Rho", "\u{3A1}"),
    ("RightAngleBracket", "\u{27E9}"), ("RightArrow", "\u{2192}"),
    ("RightArrowBar", "\u{21E5}"), ("RightArrowLeftArrow", "\u{21C4}"),
    ("RightCeiling", "\u{2309}"), ("RightDoubleBracket", "\u{27E7}"),
    ("RightDownTeeVector", "\u{295D}"), ("RightDownVector", "\u{21C2}"),
    ("RightDownVectorBar", "\u{2955}"), ("RightFloor", "\u{230B}"), ("RightTee", "\u{22A2}"),
    ("RightTeeArrow", "\u{21A6}"), ("RightTeeVector", "\u{295B}"),
    ("RightTriangle", "\u{22B3}"), ("RightTriangleBar", "\u{29D0}"),
    ("RightTriangleEqual", "\u{22B5}"), ("RightUpDownVector", "\u{294F}"),
    ("RightUpTeeVector", "\u{295C}"), ("RightUpVector", "\u{21BE}"),
    ("RightUpVectorBar", "\u{2954}"), ("RightVector", "\u{21C0}"),
    ("RightVectorBar", "\u{2953}"), ("Rightarrow", "\u{21D2}"), ("Ropf", "\u{211D}"),
    ("RoundImplies", "\u{2970}"), ("Rrightarrow", "\u{21DB}"), ("Rscr", "\u{211B}"),
    ("Rsh", "\u{21B1}"), ("RuleDelayed", "\u{29F4}"), ("SHCHcy", "\u{429}"),
    ("SHcy", "\u{428}"), ("SOFTcy", "\u{42C}"), ("Sacute", "\u{15A}"), ("Sc", "\u{2ABC}"),
    ("Scaron", "\u{160}"), ("Scedil", "\u{15E}"), ("Scirc", "\u{15C}"), ("Scy", "\u{421}"),
    ("Sfr", "\u{1D516}"), ("ShortDownArrow", "\u{2193}"), ("ShortLeftArrow", "\u{2190}"),
    ("ShortRightArrow", "\u{2192}"), ("ShortUpArrow", "\u{2191}"), ("Sigma", "\u{3A3}"),
    ("SmallCircle", "\u{2218}"), ("Sopf", "\u{1D54A}"), ("Sqrt", "\u{221A}"),
    ("Square", "\u{25A1}"), ("SquareIntersection", "\u{2293}"), ("SquareSubset", "\u{228F}"),
    ("SquareSubsetEqual", "\u{2291}"), ("SquareSuperset", "\u{2290}"),
    ("SquareSupersetEqual", "\u{2292}"), ("SquareUnion", "\u{2294}"), ("Sscr", "\u{1D4AE}"),
    ("Star", "\u{22C6}"), ("Sub", "\u{22D0}"), ("Subset", "\u{22D0}"),
    ("SubsetEqual", "\u{2286}"), ("Succeeds", "\u{227B}"), ("SucceedsEqual", "\u{2AB0}"),
    ("SucceedsSlantEqual", "\u{227D}"), ("SucceedsTilde", "\u{227F}"),
    ("SuchThat", "\u{220B}"), ("Sum", "\u{2211}"), ("Sup", "\u{22D1}"),
    ("Superset", "\u{2283}"), ("SupersetEqual", "\u{2287}"), ("Supset", "\u{22D1}"),
    ("THORN", "\u{DE}"), ("TRADE", "\u{2122}"), ("TSHcy", "\u{40B}"), ("TScy", "\u{426}"),
    ("Tab", "\u{9}"), ("Tau", "\u{3A4}"), ("Tcaron", "\u{164}"), ("Tcedil", "\u{162}"),
    ("Tcy", "\u{422}"), ("Tfr", "\u{1D517}"), ("Therefore", "\u{2234}"), ("Theta", "\u{398}"),
    ("ThickSpace", "\u{205F}\u{200A}"), ("ThinSpace", "\u{2009}"), ("Tilde", "\u{223C}"),
    ("TildeEqual", "\u{2243}"), ("TildeFullEqual", "\u{2245}"), ("TildeTilde", "\u{2248}"),
    ("Topf", "\u{1D54B}"), ("TripleDot", "\u{20DB}"), ("Tscr", "\u{1D4AF}"),
    ("Tstrok", "\u{166}"), ("Uacute", "\u{DA}"), ("Uarr", "\u{219F}"),
    ("Uarrocir", "\u{2949}"), ("Ubrcy", "\u{40E}"), ("Ubreve", "\u{16C}"), ("Ucirc", "\u{DB}"),
    ("Ucy", "\u{423}"), ("Udblac", "\u{170}"), ("Ufr", "\u{1D518}"), ("Ugrave", "\u{D9}"),
    ("Umacr", "\u{16A}"), ("UnderBar", "_"), ("UnderBrace", "\u{23DF}"),
    ("UnderBracket", "\u{23B5}"), ("UnderParenthesis", "\u{23DD}"), ("Union", "\u{22C3}"),
    ("UnionPlus", "\u{228E}"), ("Uogon", "\u{172}"), ("Uopf", "\u{1D54C}"),
    ("UpArrow", "\u{2191}"), ("UpArrowBar", "\u{2912}"), ("UpArrowDownArrow", "\u{21C5}"),
    ("UpDownArrow", "\u{2195}"), ("UpEquilibrium", "\u{296E}"), ("UpTee", "\u{22A5}"),
    ("UpTeeArrow", "\u{21A5}"), ("Uparrow", "\u{21D1}"), ("Updownarrow", "\u{21D5}"),
    ("UpperLeftArrow", "\u{2196}"), ("UpperRightArrow", "\u{2197}"), ("Upsi", "\u{3D2}"),
    ("Upsilon", "\u{3A5}"), ("Uring", "\u{16E}"), ("Uscr", "\u{1D4B0}"), ("Utilde", "\u{168}"),
    ("Uuml", "\u{DC}"), ("VDash", "\u{22AB}"), ("Vbar", "\u{2AEB}"), ("Vcy", "\u{412}"),
    ("Vdash", "\u{22A9}"), ("Vdashl", "\u{2AE6}"), ("Vee", "\u{22C1}"), ("Verbar", "\u{2016}"),
    ("Vert", "\u{2016}"), ("VerticalBar", "\u{2223}"), ("VerticalLine", "|"),
    ("VerticalSeparator", "\u{2758}"), ("VerticalTilde", "\u{2240}"),
    ("VeryThinSpace", "\u{200A}"), ("Vfr", "\u{1D519}"), ("Vopf", "\u{1D54D}"),
    ("Vscr", "\u{1D4B1}"), ("Vvdash", "\u{22AA}"), ("Wcirc", "\u{174}"), ("Wedge", "\u{22C0}"),
    ("Wfr", "\u{1D51A}"), ("Wopf", "\u{1D54E}"), ("Wscr", "\u{1D4B2}"), ("Xfr", "\u{1D51B}"),
    ("Xi", "\u{39E}"), ("Xopf", "\u{1D54F}"), ("Xscr", "\u{1D4B3}"), ("YAcy", "\u{42F}"),
    ("YIcy", "\u{407}"), ("YUcy", "\u{42E}"), ("Yacute", "\u{DD}"), ("Ycirc", "\u{176}"),
    ("Ycy", "\u{42B}"), ("Yfr", "\u{1D51C}"), ("Yopf", "\u{1D550}"), ("Yscr", "\u{1D4B4}"),
    ("Yuml", "\u{178}"), ("ZHcy", "\u{416}"), ("Zacute", "\u{179}"), ("Zcaron", "\u{17D}"),
    ("Zcy", "\u{417}"), ("Zdot", "\u{17B}"), ("ZeroWidthSpace", "\u{200B}"),
    ("Zeta", "\u{396}"), ("Zfr", "\u{2128}"), ("Zopf", "\u{2124}"), ("Zscr", "\u{1D4B5}"),
    ("aacute", "\u{E1}"), ("abreve", "\u{103}"), ("ac", "\u{223E}"),
    ("acE", "\u{223E}\u{333}"), ("acd", "\u{223F}"), ("acirc", "\u{E2}"), ("acute", "\u{B4}"),
    ("acy", "\u{430}"), ("aelig", "\u{E6}"), ("af", "\u{2061}"), ("afr", "\u{1D51E}"),
    ("agrave", "\u{E0}"), ("alefsym", "\u{2135}"), ("aleph", "\u{2135}"), ("alpha", "\u{3B1}"),
    ("amacr", "\u{101}"), ("amalg", "\u{2A3F}"), ("amp", "&"), ("and", "\u{2227}"),
    ("andand", "\u{2A55}"), ("andd", "\u{2A5C}"), ("andslope", "\u{2A58}"),
    ("andv", "\u{2A5A}"), ("ang", "\u{2220}"), ("ange", "\u{29A4}"), ("angle", "\u{2220}"),
    ("angmsd", "\u{2221}"), ("angmsdaa", "\u{29A8}"), ("angmsdab", "\u{29A9}"),
    ("angmsdac", "\u{29AA}"), ("angmsdad", "\u{29AB}"), ("angmsdae", "\u{29AC}"),
    ("angmsdaf", "\u{29AD}"), ("angmsdag", "\u{29AE}"), ("angmsdah", "\u{29AF}"),
    ("angrt", "\u{221F}"), ("angrtvb", "\u{22BE}"), ("angrtvbd", "\u{299D}"),
    ("angsph", "\u{2222}"), ("angst", "\u{C5}"), ("angzarr", "\u{237C}"), ("aogon", "\u{105}"),
    ("aopf", "\u{1D552}"), ("ap", "\u{2248}"), ("apE", "\u{2A70}"), ("apacir", "\u{2A6F}"),
    ("ape", "\u{224A}"), ("apid", "\u{224B}"), ("apos", "'"), ("approx", "\u{2248}"),
    ("approxeq", "\u{224A}"), ("aring", "\u{E5}"), ("ascr", "\u{1D4B6}"), ("ast", "*"),
    ("asymp", "\u{2248}"), ("asympeq", "\u{224D}"), ("atilde", "\u{E3}"), ("auml", "\u{E4}"),
    ("awconint", "\u{2233}"), ("awint", "\u{2A11}"), ("bNot", "\u{2AED}"),
    ("backcong", "\u{224C}"), ("backepsilon", "\u{3F6}"), ("backprime", "\u{2035}"),
    ("backsim", "\u{223D}"), ("backsimeq", "\u{22CD}"), ("barvee", "\u{22BD}"),
    ("barwed", "\u{2305}"), ("barwedge", "\u{2305}"), ("bbrk", "\u{23B5}"),
    ("bbrktbrk", "\u{23B6}"), ("bcong", "\u{224C}"), ("bcy", "\u{431}"), ("bdquo", "\u{201E}"),
    ("becaus", "\u{2235}"), ("because", "\u{2235}"), ("bemptyv", "\u{29B0}"),
    ("bepsi", "\u{3F6}"), ("bernou", "\u{212C}"), ("beta", "\u{3B2}"), ("beth", "\u{2136}"),
    ("between", "\u{226C}"), ("bfr", "\u{1D51F}"), ("bigcap", "\u{22C2}"),
    ("bigcirc", "\u{25EF}"), ("bigcup", "\u{22C3}"), ("bigodot", "\u{2A00}"),
    ("bigoplus", "\u{2A01}"), ("bigotimes", "\u{2A02}"), ("bigsqcup", "\u{2A06}"),
    ("bigstar", "\u{2605}"), ("bigtriangledown", "\u{25BD}"), ("bigtriangleup", "\u{25B3}"),
    ("biguplus", "\u{2A04}"), ("bigvee", "\u{22C1}"), ("bigwedge", "\u{22C0}"),
    ("bkarow", "\u{290D}"), ("blacklozenge", "\u{29EB}"), ("blacksquare", "\u{25AA}"),
    ("blacktriangle", "\u{25B4}"), ("blacktriangledown", "\u{25BE}"),
    ("blacktriangleleft", "\u{25C2}"), ("blacktriangleright", "\u{25B8}"),
    ("blank", "\u{2423}"), ("blk12", "\u{2592}"), ("blk14", "\u{2591}"), ("blk34", "\u{2593}"),
    ("block", "\u{2588}"), ("bne", "=\u{20E5}"), ("bnequiv", "\u{2261}\u{20E5}"),
    ("bnot", "\u{2310}"), ("bopf", "\u{1D553}"), ("bot", "\u{22A5}"), ("bottom", "\u{22A5}"),
    ("bowtie", "\u{22C8}"), ("boxDL", "\u{2557}"), ("boxDR", "\u{2554}"),
    ("boxDl", "\u{2556}"), ("boxDr", "\u{2553}"), ("boxH", "\u{2550}"), ("boxHD", "\u{2566}"),
    ("boxHU", "\u{2569}"), ("boxHd", "\u{2564}"), ("boxHu", "\u{2567}"), ("boxUL", "\u{255D}"),
    ("boxUR", "\u{255A}"), ("boxUl", "\u{255C}"), ("boxUr", "\u{2559}"), ("boxV", "\u{2551}"),
    ("boxVH", "\u{256C}"), ("boxVL", "\u{2563}"), ("boxVR", "\u{2560}"), ("boxVh", "\u{256B}"),
    ("boxVl", "\u{2562}"), ("boxVr", "\u{255F}"), ("boxbox", "\u{29C9}"),
    ("boxdL", "\u{2555}"), ("boxdR", "\u{2552}"), ("boxdl", "\u{2510}"), ("boxdr", "\u{250C}"),
    ("boxh", "\u{2500}"), ("boxhD", "\u{2565}"), ("boxhU", "\u{2568}"), ("boxhd", "\u{252C}"),
    ("boxhu", "\u{2534}"), ("boxminus", "\u{229F}"), ("boxplus", "\u{229E}"),
    ("boxtimes", "\u{22A0}"), ("boxuL", "\u{255B}"), ("boxuR", "\u{2558}"),
    ("boxul", "\u{2518}"), ("boxur", "\u{2514}"), ("boxv", "\u{2502}"), ("boxvH", "\u{256A}"),
    ("boxvL", "\u{2561}"), ("boxvR", "\u{255E}"), ("boxvh", "\u{253C}"), ("boxvl", "\u{2524}"),
    ("boxvr", "\u{251C}"), ("bprime", "\u{2035}"), ("breve", "\u{2D8}"), ("brvbar", "\u{A6}"),
    ("bscr", "\u{1D4B7}"), ("bsemi", "\u{204F}"), ("bsim", "\u{223D}"), ("bsime", "\u{22CD}"),
    ("bsol", "\\"), ("bsolb", "\u{29C5}"), ("bsolhsub", "\u{27C8}"), ("bull", "\u{2022}"),
    ("bullet", "\u{2022}"), ("bump", "\u{224E}"), ("bumpE", "\u{2AAE}"), ("bumpe", "\u{224F}"),
    ("bumpeq", "\u{224F}"), ("cacute", "\u{107}"), ("cap", "\u{2229}"), ("capand", "\u{2A44}"),
    ("capbrcup", "\u{2A49}"), ("capcap", "\u{2A4B}"), ("capcup", "\u{2A47}"),
    ("capdot", "\u{2A40}"), ("caps", "\u{2229}\u{FE00}"), ("caret", "\u{2041}"),
    ("caron", "\u{2C7}"), ("ccaps", "\u{2A4D}"), ("ccaron", "\u{10D}"), ("ccedil", "\u{E7}"),
    ("ccirc", "\u{109}"), ("ccups", "\u{2A4C}"), ("ccupssm", "\u{2A50}"), ("cdot", "\u{10B}"),
    ("cedil", "\u{B8}"), ("cemptyv", "\u{29B2}"), ("cent", "\u{A2}"), ("centerdot", "\u{B7}"),
    ("cfr", "\u{1D520}"), ("chcy", "\u{447}"), ("check", "\u{2713}"),
    ("checkmark", "\u{2713}"), ("chi", "\u{3C7}"), ("cir", "\u{25CB}"), ("cirE", "\u{29C3}"),
    ("circ", "\u{2C6}"), ("circeq", "\u{2257}"), ("circlearrowleft", "\u{21BA}"),
    ("circlearrowright", "\u{21BB}"), ("circledR", "\u{AE}"), ("circledS", "\u{24C8}"),
    ("circledast", "\u{229B}"), ("circledcirc", "\u{229A}"), ("circleddash", "\u{229D}"),
    ("cire", "\u{2257}"), ("cirfnint", "\u{2A10}"), ("cirmid", "\u{2AEF}"),
    ("cirscir", "\u{29C2}"), ("clubs", "\u{2663}"), ("clubsuit", "\u{2663}"), ("colon", ":"),
    ("colone", "\u{2254}"), ("coloneq", "\u{2254}"), ("comma", ","), ("commat", "@"),
    ("comp", "\u{2201}"), ("compfn", "\u{2218}"), ("complement", "\u{2201}"),
    ("complexes", "\u{2102}"), ("cong", "\u{2245}"), ("congdot", "\u{2A6D}"),
    ("conint", "\u{222E}"), ("copf", "\u{1D554}"), ("coprod", "\u{2210}"), ("copy", "\u{A9}"),
    ("copysr", "\u{2117}"), ("crarr", "\u{21B5}"), ("cross", "\u{2717}"),
    ("cscr", "\u{1D4B8}"), ("csub", "\u{2ACF}"), ("csube", "\u{2AD1}"), ("csup", "\u{2AD0}"),
    ("csupe", "\u{2AD2}"), ("ctdot", "\u{22EF}"), ("cudarrl", "\u{2938}"),
    ("cudarrr", "\u{2935}"), ("cuepr", "\u{22DE}"), ("cuesc", "\u{22DF}"),
    ("cularr", "\u{21B6}"), ("cularrp", "\u{293D}"), ("cup", "\u{222A}"),
    ("cupbrcap", "\u{2A48}"), ("cupcap", "\u{2A46}"), ("cupcup", "\u{2A4A}"),
    ("cupdot", "\u{228D}"), ("cupor", "\u{2A45}"), ("cups", "\u{222A}\u{FE00}"),
    ("curarr", "\u{21B7}"), ("curarrm", "\u{293C}"), ("curlyeqprec", "\u{22DE}"),
    ("curlyeqsucc", "\u{22DF}"), ("curlyvee", "\u{22CE}"), ("curlywedge", "\u{22CF}"),
    ("curren", "\u{A4}"), ("curvearrowleft", "\u{21B6}"), ("curvearrowright", "\u{21B7}"),
    ("cuvee", "\u{22CE}"), ("cuwed", "\u{22CF}"), ("cwconint", "\u{2232}"),
    ("cwint", "\u{2231}"), ("cylcty", "\u{232D}"), ("dArr", "\u{21D3}"), ("dHar", "\u{2965}"),
    ("dagger", "\u{2020}"), ("daleth", "\u{2138}"), ("darr", "\u{2193}"), ("dash", "\u{2010}"),
    ("dashv", "\u{22A3}"), ("dbkarow", "\u{290F}"), ("dblac", "\u{2DD}"),
    ("dcaron", "\u{10F}"), ("dcy", "\u{434}"), ("dd", "\u{2146}"), ("ddagger", "\u{2021}"),
    ("ddarr", "\u{21CA}"), ("ddotseq", "\u{2A77}"), ("deg", "\u{B0}"), ("delta", "\u{3B4}"),
    ("demptyv", "\u{29B1}"), ("dfisht", "\u{297F}"), ("dfr", "\u{1D521}"),
    ("dharl", "\u{21C3}"), ("dharr", "\u{21C2}"), ("diam", "\u{22C4}"),
    ("diamond", "\u{22C4}"), ("diamondsuit", "\u{2666}"), ("diams", "\u{2666}"),
    ("die", "\u{A8}"), ("digamma", "\u{3DD}"), ("disin", "\u{22F2}"), ("div", "\u{F7}"),
    ("divide", "\u{F7}"), ("divideontimes", "\u{22C7}"), ("divonx", "\u{22C7}"),
    ("djcy", "\u{452}"), ("dlcorn", "\u{231E}"), ("dlcrop", "\u{230D}"), ("dollar", "$"),
    ("dopf", "\u{1D555}"), ("dot", "\u{2D9}"), ("doteq", "\u{2250}"), ("doteqdot", "\u{2251}"),
    ("dotminus", "\u{2238}"), ("dotplus", "\u{2214}"), ("dotsquare", "\u{22A1}"),
    ("doublebarwedge", "\u{2306}"), ("downarrow", "\u{2193}"), ("downdownarrows", "\u{21CA}"),
    ("downharpoonleft", "\u{21C3}"), ("downharpoonright", "\u{21C2}"),
    ("drbkarow", "\u{2910}"), ("drcorn", "\u{231F}"), ("drcrop", "\u{230C}"),
    ("dscr", "\u{1D4B9}"), ("dscy", "\u{455}"), ("dsol", "\u{29F6}"), ("dstrok", "\u{111}"),
    ("dtdot", "\u{22F1}"), ("dtri", "\u{25BF}"), ("dtrif", "\u{25BE}"), ("duarr", "\u{21F5}"),
    ("duhar", "\u{296F}"), ("dwangle", "\u{29A6}"), ("dzcy", "\u{45F}"),
    ("dzigrarr", "\u{27FF}"), ("eDDot", "\u{2A77}"), ("eDot", "\u{2251}"),
    ("eacute", "\u{E9}"), ("easter", "\u{2A6E}"), ("ecaron", "\u{11B}"), ("ecir", "\u{2256}"),
    ("ecirc", "\u{EA}"), ("ecolon", "\u{2255}"), ("ecy", "\u{44D}"), ("edot", "\u{117}"),
    ("ee", "\u{2147}"), ("efDot", "\u{2252}"), ("efr", "\u{1D522}"), ("eg", "\u{2A9A}"),
    ("egrave", "\u{E8}"), ("egs", "\u{2A96}"), ("egsdot", "\u{2A98}"), ("el", "\u{2A99}"),
    ("elinters", "\u{23E7}"), ("ell", "\u{2113}"), ("els", "\u{2A95}"), ("elsdot", "\u{2A97}"),
    ("emacr", "\u{113}"), ("empty", "\u{2205}"), ("emptyset", "\u{2205}"),
    ("emptyv", "\u{2205}"), ("emsp", "\u{2003}"), ("emsp13", "\u{2004}"),
    ("emsp14", "\u{2005}"), ("eng", "\u{14B}"), ("ensp", "\u{2002}"), ("eogon", "\u{119}"),
    ("eopf", "\u{1D556}"), ("epar", "\u{22D5}"), ("eparsl", "\u{29E3}"), ("eplus", "\u{2A71}"),
    ("epsi", "\u{3B5}"), ("epsilon", "\u{3B5}"), ("epsiv", "\u{3F5}"), ("eqcirc", "\u{2256}"),
    ("eqcolon", "\u{2255}"), ("eqsim", "\u{2242}"), ("eqslantgtr", "\u{2A96}"),
    ("eqslantless", "\u{2A95}"), ("equals", "="), ("equest", "\u{225F}"),
    ("equiv", "\u{2261}"), ("equivDD", "\u{2A78}"), ("eqvparsl", "\u{29E5}"),
    ("erDot", "\u{2253}"), ("erarr", "\u{2971}"), ("escr", "\u{212F}"), ("esdot", "\u{2250}"),
    ("esim", "\u{2242}"), ("eta", "\u{3B7}"), ("eth", "\u{F0}"), ("euml", "\u{EB}"),
    ("euro", "\u{20AC}"), ("excl", "!"), ("exist", "\u{2203}"), ("expectation", "\u{2130}"),
    ("exponentiale", "\u{2147}"), ("fallingdotseq", "\u{2252}"), ("fcy", "\u{444}"),
    ("female", "\u{2640}"), ("ffilig", "\u{FB03}"), ("fflig", "\u{FB00}"),
    ("ffllig", "\u{FB04}"), ("ffr", "\u{1D523}"), ("filig", "\u{FB01}"), ("fjlig", "fj"),
    ("flat", "\u{266D}"), ("fllig", "\u{FB02}"), ("fltns", "\u{25B1}"), ("fnof", "\u{192}"),
    ("fopf", "\u{1D557}"), ("forall", "\u{2200}"), ("fork", "\u{22D4}"), ("forkv", "\u{2AD9}"),
    ("fpartint", "\u{2A0D}"), ("frac12", "\u{BD}"), ("frac13", "\u{2153}"),
    ("frac14", "\u{BC}"), ("frac15", "\u{2155}"), ("frac16", "\u{2159}"),
    ("frac18", "\u{215B}"), ("frac23", "\u{2154}"), ("frac25", "\u{2156}"),
    ("frac34", "\u{BE}"), ("frac35", "\u{2157}"), ("frac38", "\u{215C}"),
    ("frac45", "\u{2158}"), ("frac56", "\u{215A}"), ("frac58", "\u{215D}"),
    ("frac78", "\u{215E}"), ("frasl", "\u{2044}"), ("frown", "\u{2322}"),
    ("fscr", "\u{1D4BB}"), ("gE", "\u{2267}"), ("gEl", "\u{2A8C}"), ("gacute", "\u{1F5}"),
    ("gamma", "\u{3B3}"), ("gammad", "\u{3DD}"), ("gap", "\u{2A86}"), ("gbreve", "\u{11F}"),
    ("gcirc", "\u{11D}"), ("gcy", "\u{433}"), ("gdot", "\u{121}"), ("ge", "\u{2265}"),
    ("gel", "\u{22DB}"), ("geq", "\u{2265}"), ("geqq", "\u{2267}"), ("geqslant", "\u{2A7E}"),
    ("ges", "\u{2A7E}"), ("gescc", "\u{2AA9}"), ("gesdot", "\u{2A80}"),
    ("gesdoto", "\u{2A82}"), ("gesdotol", "\u{2A84}"), ("gesl", "\u{22DB}\u{FE00}"),
    ("gesles", "\u{2A94}"), ("gfr", "\u{1D524}"), ("gg", "\u{226B}"), ("ggg", "\u{22D9}"),
    ("gimel", "\u{2137}"), ("gjcy", "\u{453}"), ("gl", "\u{2277}"), ("glE", "\u{2A92}"),
    ("gla", "\u{2AA5}"), ("glj", "\u{2AA4}"), ("gnE", "\u{2269}"), ("gnap", "\u{2A8A}"),
    ("gnapprox", "\u{2A8A}"), ("gne", "\u{2A88}"), ("gneq", "\u{2A88}"), ("gneqq", "\u{2269}"),
    ("gnsim", "\u{22E7}"), ("gopf", "\u{1D558}"), ("grave", "`"), ("gscr", "\u{210A}"),
    ("gsim", "\u{2273}"), ("gsime", "\u{2A8E}"), ("gsiml", "\u{2A90}"), ("gt", ">"),
    ("gtcc", "\u{2AA7}"), ("gtcir", "\u{2A7A}"), ("gtdot", "\u{22D7}"), ("gtlPar", "\u{2995}"),
    ("gtquest", "\u{2A7C}"), ("gtrapprox", "\u{2A86}"), ("gtrarr", "\u{2978}"),
    ("gtrdot", "\u{22D7}"), ("gtreqless", "\u{22DB}"), ("gtreqqless", "\u{2A8C}"),
    ("gtrless", "\u{2277}"), ("gtrsim", "\u{2273}"), ("gvertneqq", "\u{2269}\u{FE00}"),
    ("gvnE", "\u{2269}\u{FE00}"), ("hArr", "\u{21D4}"), ("hairsp", "\u{200A}"),
    ("half", "\u{BD}"), ("hamilt", "\u{210B}"), ("hardcy", "\u{44A}"), ("harr", "\u{2194}"),
    ("harrcir", "\u{2948}"), ("harrw", "\u{21AD}"), ("hbar", "\u{210F}"), ("hcirc", "\u{125}"),
    ("hearts", "\u{2665}"), ("heartsuit", "\u{2665}"), ("hellip", "\u{2026}"),
    ("hercon", "\u{22B9}"), ("hfr", "\u{1D525}"), ("hksearow", "\u{2925}"),
    ("hkswarow", "\u{2926}"), ("hoarr", "\u{21FF}"), ("homtht", "\u{223B}"),
    ("hookleftarrow", "\u{21A9}"), ("hookrightarrow", "\u{21AA}"), ("hopf", "\u{1D559}"),
    ("horbar", "\u{2015}"), ("hscr", "\u{1D4BD}"), ("hslash", "\u{210F}"),
    ("hstrok", "\u{127}"), ("hybull", "\u{2043}"), ("hyphen", "\u{2010}"),
    ("iacute", "\u{ED}"), ("ic", "\u{2063}"), ("icirc", "\u{EE}"), ("icy", "\u{438}"),
    ("iecy", "\u{435}"), ("iexcl", "\u{A1}"), ("iff", "\u{21D4}"), ("ifr", "\u{1D526}"),
    ("igrave", "\u{EC}"), ("ii", "\u{2148}"), ("iiiint", "\u{2A0C}"), ("iiint", "\u{222D}"),
    ("iinfin", "\u{29DC}"), ("iiota", "\u{2129}"), ("ijlig", "\u{133}"), ("imacr", "\u{12B}"),
    ("image", "\u{2111}"), ("imagline", "\u{2110}"), ("imagpart", "\u{2111}"),
    ("imath", "\u{131}"), ("imof", "\u{22B7}"), ("imped", "\u{1B5}"), ("in", "\u{2208}"),
    ("incare", "\u{2105}"), ("infin", "\u{221E}"), ("infintie", "\u{29DD}"),
    ("inodot", "\u{131}"), ("int", "\u{222B}"), ("intcal", "\u{22BA}"),
    ("integers", "\u{2124}"), ("intercal", "\u{22BA}"), ("intlarhk", "\u{2A17}"),
    ("intprod", "\u{2A3C}"), ("iocy", "\u{451}"), ("iogon", "\u{12F}"), ("iopf", "\u{1D55A}"),
    ("iota", "\u{3B9}"), ("iprod", "\u{2A3C}"), ("iquest", "\u{BF}"), ("iscr", "\u{1D4BE}"),
    ("isin", "\u{2208}"), ("isinE", "\u{22F9}"), ("isindot", "\u{22F5}"),
    ("isins", "\u{22F4}"), ("isinsv", "\u{22F3}"), ("isinv", "\u{2208}"), ("it", "\u{2062}"),
    ("itilde", "\u{129}"), ("iukcy", "\u{456}"), ("iuml", "\u{EF}"), ("jcirc", "\u{135}"),
    ("jcy", "\u{439}"), ("jfr", "\u{1D527}"), ("jmath", "\u{237}"), ("jopf", "\u{1D55B}"),
    ("jscr", "\u{1D4BF}"), ("jsercy", "\u{458}"), ("jukcy", "\u{454}"), ("kappa", "\u{3BA}"),
    ("kappav", "\u{3F0}"), ("kcedil", "\u{137}"), ("kcy", "\u{43A}"), ("kfr", "\u{1D528}"),
    ("kgreen", "\u{138}"), ("khcy", "\u{445}"), ("kjcy", "\u{45C}"), ("kopf", "\u{1D55C}"),
    ("kscr", "\u{1D4C0}"), ("lAarr", "\u{21DA}"), ("lArr", "\u{21D0}"), ("lAtail", "\u{291B}"),
    ("lBarr", "\u{290E}"), ("lE", "\u{2266}"), ("lEg", "\u{2A8B}"), ("lHar", "\u{2962}"),
    ("lacute", "\u{13A}"), ("laemptyv", "\u{29B4}"), ("lagran", "\u{2112}"),
    ("lambda", "\u{3BB}"), ("lang", "\u{27E8}"), ("langd", "\u{2991}"), ("langle", "\u{27E8}"),
    ("lap", "\u{2A85}"), ("laquo", "\u{AB}"), ("larr", "\u{2190}"), ("larrb", "\u{21E4}"),
    ("larrbfs", "\u{291F}"), ("larrfs", "\u{291D}"), ("larrhk", "\u{21A9}"),
    ("larrlp", "\u{21AB}"), ("larrpl", "\u{2939}"), ("larrsim", "\u{2973}"),
    ("larrtl", "\u{21A2}"), ("lat", "\u{2AAB}"), ("latail", "\u{2919}"), ("late", "\u{2AAD}"),
    ("lates", "\u{2AAD}\u{FE00}"), ("lbarr", "\u{290C}"), ("lbbrk", "\u{2772}"),
    ("lbrace", "{"), ("lbrack", "["), ("lbrke", "\u{298B}"), ("lbrksld", "\u{298F}"),
    ("lbrkslu", "\u{298D}"), ("lcaron", "\u{13E}"), ("lcedil", "\u{13C}"),
    ("lceil", "\u{2308}"), ("lcub", "{"), ("lcy", "\u{43B}"), ("ldca", "\u{2936}"),
    ("ldquo", "\u{201C}"), ("ldquor", "\u{201E}"), ("ldrdhar", "\u{2967}"),
    ("ldrushar", "\u{294B}"), ("ldsh", "\u{21B2}"), ("le", "\u{2264}"),
    ("leftarrow", "\u{2190}"), ("leftarrowtail", "\u{21A2}"), ("leftharpoondown", "\u{21BD}"),
    ("leftharpoonup", "\u{21BC}"), ("leftleftarrows", "\u{21C7}"),
    ("leftrightarrow", "\u{2194}"), ("leftrightarrows", "\u{21C6}"),
    ("leftrightharpoons", "\u{21CB}"), ("leftrightsquigarrow", "\u{21AD}"),
    ("leftthreetimes", "\u{22CB}"), ("leg", "\u{22DA}"), ("leq", "\u{2264}"),
    ("leqq", "\u{2266}"), ("leqslant", "\u{2A7D}"), ("les", "\u{2A7D}"), ("lescc", "\u{2AA8}"),
    ("lesdot", "\u{2A7F}"), ("lesdoto", "\u{2A81}"), ("lesdotor", "\u{2A83}"),
    ("lesg", "\u{22DA}\u{FE00}"), ("lesges", "\u{2A93}"), ("lessapprox", "\u{2A85}"),
    ("lessdot", "\u{22D6}"), ("lesseqgtr", "\u{22DA}"), ("lesseqqgtr", "\u{2A8B}"),
    ("lessgtr", "\u{2276}"), ("lesssim", "\u{2272}"), ("lfisht", "\u{297C}"),
    ("lfloor", "\u{230A}"), ("lfr", "\u{1D529}"), ("lg", "\u{2276}"), ("lgE", "\u{2A91}"),
    ("lhard", "\u{21BD}"), ("lharu", "\u{21BC}"), ("lharul", "\u{296A}"),
    ("lhblk", "\u{2584}"), ("ljcy", "\u{459}"), ("ll", "\u{226A}"), ("llarr", "\u{21C7}"),
    ("llcorner", "\u{231E}"), ("llhard", "\u{296B}"), ("lltri", "\u{25FA}"),
    ("lmidot", "\u{140}"), ("lmoust", "\u{23B0}"), ("lmoustache", "\u{23B0}"),
    ("lnE", "\u{2268}"), ("lnap", "\u{2A89}"), ("lnapprox", "\u{2A89}"), ("lne", "\u{2A87}"),
    ("lneq", "\u{2A87}"), ("lneqq", "\u{2268}"), ("lnsim", "\u{22E6}"), ("loang", "\u{27EC}"),
    ("loarr", "\u{21FD}"), ("lobrk", "\u{27E6}"), ("longleftarrow", "\u{27F5}"),
    ("longleftrightarrow", "\u{27F7}"), ("longmapsto", "\u{27FC}"),
    ("longrightarrow", "\u{27F6}"), ("looparrowleft", "\u{21AB}"),
    ("looparrowright", "\u{21AC}"), ("lopar", "\u{2985}"), ("lopf", "\u{1D55D}"),
    ("loplus", "\u{2A2D}"), ("lotimes", "\u{2A34}"), ("lowast", "\u{2217}"), ("lowbar", "_"),
    ("loz", "\u{25CA}"), ("lozenge", "\u{25CA}"), ("lozf", "\u{29EB}"), ("lpar", "("),
    ("lparlt", "\u{2993}"), ("lrarr", "\u{21C6}"), ("lrcorner", "\u{231F}"),
    ("lrhar", "\u{21CB}"), ("lrhard", "\u{296D}"), ("lrm", "\u{200E}"), ("lrtri", "\u{22BF}"),
    ("lsaquo", "\u{2039}"), ("lscr", "\u{1D4C1}"), ("lsh", "\u{21B0}"), ("lsim", "\u{2272}"),
    ("lsime", "\u{2A8D}"), ("lsimg", "\u{2A8F}"), ("lsqb", "["), ("lsquo", "\u{2018}"),
    ("lsquor", "\u{201A}"), ("lstrok", "\u{142}"), ("lt", "<"), ("ltcc", "\u{2AA6}"),
    ("ltcir", "\u{2A79}"), ("ltdot", "\u{22D6}"), ("lthree", "\u{22CB}"),
    ("ltimes", "\u{22C9}"), ("ltlarr", "\u{2976}"), ("ltquest", "\u{2A7B}"),
    ("ltrPar", "\u{2996}"), ("ltri", "\u{25C3}"), ("ltrie", "\u{22B4}"), ("ltrif", "\u{25C2}"),
    ("lurdshar", "\u{294A}"), ("luruhar", "\u{2966}"), ("lvertneqq", "\u{2268}\u{FE00}"),
    ("lvnE", "\u{2268}\u{FE00}"), ("mDDot", "\u{223A}"), ("macr", "\u{AF}"),
    ("male", "\u{2642}"), ("malt", "\u{2720}"), ("maltese", "\u{2720}"), ("map", "\u{21A6}"),
    ("mapsto", "\u{21A6}"), ("mapstodown", "\u{21A7}"), ("mapstoleft", "\u{21A4}"),
    ("mapstoup", "\u{21A5}"), ("marker", "\u{25AE}"), ("mcomma", "\u{2A29}"),
    ("mcy", "\u{43C}"), ("mdash", "\u{2014}"), ("measuredangle", "\u{2221}"),
    ("mfr", "\u{1D52A}"), ("mho", "\u{2127}"), ("micro", "\u{B5}"), ("mid", "\u{2223}"),
    ("midast", "*"), ("midcir", "\u{2AF0}"), ("middot", "\u{B7}"), ("minus", "\u{2212}"),
    ("minusb", "\u{229F}"), ("minusd", "\u{2238}"), ("minusdu", "\u{2A2A}"),
    ("mlcp", "\u{2ADB}"), ("mldr", "\u{2026}"), ("mnplus", "\u{2213}"), ("models", "\u{22A7}"),
    ("mopf", "\u{1D55E}"), ("mp", "\u{2213}"), ("mscr", "\u{1D4C2}"), ("mstpos", "\u{223E}"),
    ("mu", "\u{3BC}"), ("multimap", "\u{22B8}"), ("mumap", "\u{22B8}"),
    ("nGg", "\u{22D9}\u{338}"), ("nGt", "\u{226B}\u{20D2}"), ("nGtv", "\u{226B}\u{338}"),
    ("nLeftarrow", "\u{21CD}"), ("nLeftrightarrow", "\u{21CE}"), ("nLl", "\u{22D8}\u{338}"),
    ("nLt", "\u{226A}\u{20D2}"), ("nLtv", "\u{226A}\u{338}"), ("nRightarrow", "\u{21CF}"),
    ("nVDash", "\u{22AF}"), ("nVdash", "\u{22AE}"), ("nabla", "\u{2207}"),
    ("nacute", "\u{144}"), ("nang", "\u{2220}\u{20D2}"), ("nap", "\u{2249}"),
    ("napE", "\u{2A70}\u{338}"), ("napid", "\u{224B}\u{338}"), ("napos", "\u{149}"),
    ("napprox", "\u{2249}"), ("natur", "\u{266E}"), ("natural", "\u{266E}"),
    ("naturals", "\u{2115}"), ("nbsp", "\u{A0}"), ("nbump", "\u{224E}\u{338}"),
    ("nbumpe", "\u{224F}\u{338}"), ("ncap", "\u{2A43}"), ("ncaron", "\u{148}"),
    ("ncedil", "\u{146}"), ("ncong", "\u{2247}"), ("ncongdot", "\u{2A6D}\u{338}"),
    ("ncup", "\u{2A42}"), ("ncy", "\u{43D}"), ("ndash", "\u{2013}"), ("ne", "\u{2260}"),
    ("neArr", "\u{21D7}"), ("nearhk", "\u{2924}"), ("nearr", "\u{2197}"),
    ("nearrow", "\u{2197}"), ("nedot", "\u{2250}\u{338}"), ("nequiv", "\u{2262}"),
    ("nesear", "\u{2928}"), ("nesim", "\u{2242}\u{338}"), ("nexist", "\u{2204}"),
    ("nexists", "\u{2204}"), ("nfr", "\u{1D52B}"), ("ngE", "\u{2267}\u{338}"),
    ("nge", "\u{2271}"), ("ngeq", "\u{2271}"), ("ngeqq", "\u{2267}\u{338}"),
    ("ngeqslant", "\u{2A7E}\u{338}"), ("nges", "\u{2A7E}\u{338}"), ("ngsim", "\u{2275}"),
    ("ngt", "\u{226F}"), ("ngtr", "\u{226F}"), ("nhArr", "\u{21CE}"), ("nharr", "\u{21AE}"),
    ("nhpar", "\u{2AF2}"), ("ni", "\u{220B}"), ("nis", "\u{22FC}"), ("nisd", "\u{22FA}"),
    ("niv", "\u{220B}"), ("njcy", "\u{45A}"), ("nlArr", "\u{21CD}"),
    ("nlE", "\u{2266}\u{338}"), ("nlarr", "\u{219A}"), ("nldr", "\u{2025}"),
    ("nle", "\u{2270}"), ("nleftarrow", "\u{219A}"), ("nleftrightarrow", "\u{21AE}"),
    ("nleq", "\u{2270}"), ("nleqq", "\u{2266}\u{338}"), ("nleqslant", "\u{2A7D}\u{338}"),
    ("nles", "\u{2A7D}\u{338}"), ("nless", "\u{226E}"), ("nlsim", "\u{2274}"),
    ("nlt", "\u{226E}"), ("nltri", "\u{22EA}"), ("nltrie", "\u{22EC}"), ("nmid", "\u{2224}"),
    ("nopf", "\u{1D55F}"), ("not", "\u{AC}"), ("notin", "\u{2209}"),
    ("notinE", "\u{22F9}\u{338}"), ("notindot", "\u{22F5}\u{338}"), ("notinva", "\u{2209}"),
    ("notinvb", "\u{22F7}"), ("notinvc", "\u{22F6}"), ("notni", "\u{220C}"),
    ("notniva", "\u{220C}"), ("notnivb", "\u{22FE}"), ("notnivc", "\u{22FD}"),
    ("npar", "\u{2226}"), ("nparallel", "\u{2226}"), ("nparsl", "\u{2AFD}\u{20E5}"),
    ("npart", "\u{2202}\u{338}"), ("npolint", "\u{2A14}"), ("npr", "\u{2280}"),
    ("nprcue", "\u{22E0}"), ("npre", "\u{2AAF}\u{338}"), ("nprec", "\u{2280}"),
    ("npreceq", "\u{2AAF}\u{338}"), ("nrArr", "\u{21CF}"), ("nrarr", "\u{219B}"),
    ("nrarrc", "\u{2933}\u{338}"), ("nrarrw", "\u{219D}\u{338}"), ("nrightarrow", "\u{219B}"),
    ("nrtri", "\u{22EB}"), ("nrtrie", "\u{22ED}"), ("nsc", "\u{2281}"), ("nsccue", "\u{22E1}"),
    ("nsce", "\u{2AB0}\u{338}"), ("nscr", "\u{1D4C3}"), ("nshortmid", "\u{2224}"),
    ("nshortparallel", "\u{2226}"), ("nsim", "\u{2241}"), ("nsime", "\u{2244}"),
    ("nsimeq", "\u{2244}"), ("nsmid", "\u{2224}"), ("nspar", "\u{2226}"),
    ("nsqsube", "\u{22E2}"), ("nsqsupe", "\u{22E3}"), ("nsub", "\u{2284}"),
    ("nsubE", "\u{2AC5}\u{338}"), ("nsube", "\u{2288}"), ("nsubset", "\u{2282}\u{20D2}"),
    ("nsubseteq", "\u{2288}"), ("nsubseteqq", "\u{2AC5}\u{338}"), ("nsucc", "\u{2281}"),
    ("nsucceq", "\u{2AB0}\u{338}"), ("nsup", "\u{2285}"), ("nsupE", "\u{2AC6}\u{338}"),
    ("nsupe", "\u{2289}"), ("nsupset", "\u{2283}\u{20D2}"), ("nsupseteq", "\u{2289}"),
    ("nsupseteqq", "\u{2AC6}\u{338}"), ("ntgl", "\u{2279}"), ("ntilde", "\u{F1}"),
    ("ntlg", "\u{2278}"), ("ntriangleleft", "\u{22EA}"), ("ntrianglelefteq", "\u{22EC}"),
    ("ntriangleright", "\u{22EB}"), ("ntrianglerighteq", "\u{22ED}"), ("nu", "\u{3BD}"),
    ("num", "#"), ("numero", "\u{2116}"), ("numsp", "\u{2007}"), ("nvDash", "\u{22AD}"),
    ("nvHarr", "\u{2904}"), ("nvap", "\u{224D}\u{20D2}"), ("nvdash", "\u{22AC}"),
    ("nvge", "\u{2265}\u{20D2}"), ("nvgt", ">\u{20D2}"), ("nvinfin", "\u{29DE}"),
    ("nvlArr", "\u{2902}"), ("nvle", "\u{2264}\u{20D2}"), ("nvlt", "<\u{20D2}"),
    ("nvltrie", "\u{22B4}\u{20D2}"), ("nvrArr", "\u{2903}"), ("nvrtrie", "\u{22B5}\u{20D2}"),
    ("nvsim", "\u{223C}\u{20D2}"), ("nwArr", "\u{21D6}"), ("nwarhk", "\u{2923}"),
    ("nwarr", "\u{2196}"), ("nwarrow", "\u{2196}"), ("nwnear", "\u{2927}"), ("oS", "\u{24C8}"),
    ("oacute", "\u{F3}"), ("oast", "\u{229B}"), ("ocir", "\u{229A}"), ("ocirc", "\u{F4}"),
    ("ocy", "\u{43E}"), ("odash", "\u{229D}"), ("odblac", "\u{151}"), ("odiv", "\u{2A38}"),
    ("odot", "\u{2299}"), ("odsold", "\u{29BC}"), ("oelig", "\u{153}"), ("ofcir", "\u{29BF}"),
    ("ofr", "\u{1D52C}"), ("ogon", "\u{2DB}"), ("ograve", "\u{F2}"), ("ogt", "\u{29C1}"),
    ("ohbar", "\u{29B5}"), ("ohm", "\u{3A9}"), ("oint", "\u{222E}"), ("olarr", "\u{21BA}"),
    ("olcir", "\u{29BE}"), ("olcross", "\u{29BB}"), ("oline", "\u{203E}"), ("olt", "\u{29C0}"),
    ("omacr", "\u{14D}"), ("omega", "\u{3C9}"), ("omicron", "\u{3BF}"), ("omid", "\u{29B6}"),
    ("ominus", "\u{2296}"), ("oopf", "\u{1D560}"), ("opar", "\u{29B7}"), ("operp", "\u{29B9}"),
    ("oplus", "\u{2295}"), ("or", "\u{2228}"), ("orarr", "\u{21BB}"), ("ord", "\u{2A5D}"),
    ("order", "\u{2134}"), ("orderof", "\u{2134}"), ("ordf", "\u{AA}"), ("ordm", "\u{BA}"),
    ("origof", "\u{22B6}"), ("oror", "\u{2A56}"), ("orslope", "\u{2A57}"), ("orv", "\u{2A5B}"),
    ("oscr", "\u{2134}"), ("oslash", "\u{F8}"), ("osol", "\u{2298}"), ("otilde", "\u{F5}"),
    ("otimes", "\u{2297}"), ("otimesas", "\u{2A36}"), ("ouml", "\u{F6}"),
    ("ovbar", "\u{233D}"), ("par", "\u{2225}"), ("para", "\u{B6}"), ("parallel", "\u{2225}"),
    ("parsim", "\u{2AF3}"), ("parsl", "\u{2AFD}"), ("part", "\u{2202}"), ("pcy", "\u{43F}"),
    ("percnt", "%"), ("period", "."), ("permil", "\u{2030}"), ("perp", "\u{22A5}"),
    ("pertenk", "\u{2031}"), ("pfr", "\u{1D52D}"), ("phi", "\u{3C6}"), ("phiv", "\u{3D5}"),
    ("phmmat", "\u{2133}"), ("phone", "\u{260E}"), ("pi", "\u{3C0}"),
    ("pitchfork", "\u{22D4}"), ("piv", "\u{3D6}"), ("planck", "\u{210F}"),
    ("planckh", "\u{210E}"), ("plankv", "\u{210F}"), ("plus", "+"), ("plusacir", "\u{2A23}"),
    ("plusb", "\u{229E}"), ("pluscir", "\u{2A22}"), ("plusdo", "\u{2214}"),
    ("plusdu", "\u{2A25}"), ("pluse", "\u{2A72}"), ("plusmn", "\u{B1}"),
    ("plussim", "\u{2A26}"), ("plustwo", "\u{2A27}"), ("pm", "\u{B1}"),
    ("pointint", "\u{2A15}"), ("popf", "\u{1D561}"), ("pound", "\u{A3}"), ("pr", "\u{227A}"),
    ("prE", "\u{2AB3}"), ("prap", "\u{2AB7}"), ("prcue", "\u{227C}"), ("pre", "\u{2AAF}"),
    ("prec", "\u{227A}"), ("precapprox", "\u{2AB7}"), ("preccurlyeq", "\u{227C}"),
    ("preceq", "\u{2AAF}"), ("precnapprox", "\u{2AB9}"), ("precneqq", "\u{2AB5}"),
    ("precnsim", "\u{22E8}"), ("precsim", "\u{227E}"), ("prime", "\u{2032}"),
    ("primes", "\u{2119}"), ("prnE", "\u{2AB5}"), ("prnap", "\u{2AB9}"),
    ("prnsim", "\u{22E8}"), ("prod", "\u{220F}"), ("profalar", "\u{232E}"),
    ("profline", "\u{2312}"), ("profsurf", "\u{2313}"), ("prop", "\u{221D}"),
    ("propto", "\u{221D}"), ("prsim", "\u{227E}"), ("prurel", "\u{22B0}"),
    ("pscr", "\u{1D4C5}"), ("psi", "\u{3C8}"), ("puncsp", "\u{2008}"), ("qfr", "\u{1D52E}"),
    ("qint", "\u{2A0C}"), ("qopf", "\u{1D562}"), ("qprime", "\u{2057}"), ("qscr", "\u{1D4C6}"),
    ("quaternions", "\u{210D}"), ("quatint", "\u{2A16}"), ("quest", "?"),
    ("questeq", "\u{225F}"), ("quot", "\""), ("rAarr", "\u{21DB}"), ("rArr", "\u{21D2}"),
    ("rAtail", "\u{291C}"), ("rBarr", "\u{290F}"), ("rHar", "\u{2964}"),
    ("race", "\u{223D}\u{331}"), ("racute", "\u{155}"), ("radic", "\u{221A}"),
    ("raemptyv", "\u{29B3}"), ("rang", "\u{27E9}"), ("rangd", "\u{2992}"),
    ("range", "\u{29A5}"), ("rangle", "\u{27E9}"), ("raquo", "\u{BB}"), ("rarr", "\u{2192}"),
    ("rarrap", "\u{2975}"), ("rarrb", "\u{21E5}"), ("rarrbfs", "\u{2920}"),
    ("rarrc", "\u{2933}"), ("rarrfs", "\u{291E}"), ("rarrhk", "\u{21AA}"),
    ("rarrlp", "\u{21AC}"), ("rarrpl", "\u{2945}"), ("rarrsim", "\u{2974}"),
    ("rarrtl", "\u{21A3}"), ("rarrw", "\u{219D}"), ("ratail", "\u{291A}"),
    ("ratio", "\u{2236}"), ("rationals", "\u{211A}"), ("rbarr", "\u{290D}"),
    ("rbbrk", "\u{2773}"), ("rbrace", "}"), ("rbrack", "]"), ("rbrke", "\u{298C}"),
    ("rbrksld", "\u{298E}"), ("rbrkslu", "\u{2990}"), ("rcaron", "\u{159}"),
    ("rcedil", "\u{157}"), ("rceil", "\u{2309}"), ("rcub", "}"), ("rcy", "\u{440}"),
    ("rdca", "\u{2937}"), ("rdldhar", "\u{2969}"), ("rdquo", "\u{201D}"),
    ("rdquor", "\u{201D}"), ("rdsh", "\u{21B3}"), ("real", "\u{211C}"),
    ("realine", "\u{211B}"), ("realpart", "\u{211C}"), ("reals", "\u{211D}"),
    ("rect", "\u{25AD}"), ("reg", "\u{AE}"), ("rfisht", "\u{297D}"), ("rfloor", "\u{230B}"),
    ("rfr", "\u{1D52F}"), ("rhard", "\u{21C1}"), ("rharu", "\u{21C0}"), ("rharul", "\u{296C}"),
    ("rho", "\u{3C1}"), ("rhov", "\u{3F1}"), ("rightarrow", "\u{2192}"),
    ("rightarrowtail", "\u{21A3}"), ("rightharpoondown", "\u{21C1}"),
    ("rightharpoonup", "\u{21C0}"), ("rightleftarrows", "\u{21C4}"),
    ("rightleftharpoons", "\u{21CC}"), ("rightrightarrows", "\u{21C9}"),
    ("rightsquigarrow", "\u{219D}"), ("rightthreetimes", "\u{22CC}"), ("ring", "\u{2DA}"),
    ("risingdotseq", "\u{2253}"), ("rlarr", "\u{21C4}"), ("rlhar", "\u{21CC}"),
    ("rlm", "\u{200F}"), ("rmoust", "\u{23B1}"), ("rmoustache", "\u{23B1}"),
    ("rnmid", "\u{2AEE}"), ("roang", "\u{27ED}"), ("roarr", "\u{21FE}"), ("robrk", "\u{27E7}"),
    ("ropar", "\u{2986}"), ("ropf", "\u{1D563}"), ("roplus", "\u{2A2E}"),
    ("rotimes", "\u{2A35}"), ("rpar", ")"), ("rpargt", "\u{2994}"), ("rppolint", "\u{2A12}"),
    ("rrarr", "\u{21C9}"), ("rsaquo", "\u{203A}"), ("rscr", "\u{1D4C7}"), ("rsh", "\u{21B1}"),
    ("rsqb", "]"), ("rsquo", "\u{2019}"), ("rsquor", "\u{2019}"), ("rthree", "\u{22CC}"),
    ("rtimes", "\u{22CA}"), ("rtri", "\u{25B9}"), ("rtrie", "\u{22B5}"), ("rtrif", "\u{25B8}"),
    ("rtriltri", "\u{29CE}"), ("ruluhar", "\u{2968}"), ("rx", "\u{211E}"),
    ("sacute", "\u{15B}"), ("sbquo", "\u{201A}"), ("sc", "\u{227B}"), ("scE", "\u{2AB4}"),
    ("scap", "\u{2AB8}"), ("scaron", "\u{161}"), ("sccue", "\u{227D}"), ("sce", "\u{2AB0}"),
    ("scedil", "\u{15F}"), ("scirc", "\u{15D}"), ("scnE", "\u{2AB6}"), ("scnap", "\u{2ABA}"),
    ("scnsim", "\u{22E9}"), ("scpolint", "\u{2A13}"), ("scsim", "\u{227F}"),
    ("scy", "\u{441}"), ("sdot", "\u{22C5}"), ("sdotb", "\u{22A1}"), ("sdote", "\u{2A66}"),
    ("seArr", "\u{21D8}"), ("searhk", "\u{2925}"), ("searr", "\u{2198}"),
    ("searrow", "\u{2198}"), ("sect", "\u{A7}"), ("semi", ";"), ("seswar", "\u{2929}"),
    ("setminus", "\u{2216}"), ("setmn", "\u{2216}"), ("sext", "\u{2736}"),
    ("sfr", "\u{1D530}"), ("sfrown", "\u{2322}"), ("sharp", "\u{266F}"), ("shchcy", "\u{449}"),
    ("shcy", "\u{448}"), ("shortmid", "\u{2223}"), ("shortparallel", "\u{2225}"),
    ("shy", "\u{AD}"), ("sigma", "\u{3C3}"), ("sigmaf", "\u{3C2}"), ("sigmav", "\u{3C2}"),
    ("sim", "\u{223C}"), ("simdot", "\u{2A6A}"), ("sime", "\u{2243}"), ("simeq", "\u{2243}"),
    ("simg", "\u{2A9E}"), ("simgE", "\u{2AA0}"), ("siml", "\u{2A9D}"), ("simlE", "\u{2A9F}"),
    ("simne", "\u{2246}"), ("simplus", "\u{2A24}"), ("simrarr", "\u{2972}"),
    ("slarr", "\u{2190}"), ("smallsetminus", "\u{2216}"), ("smashp", "\u{2A33}"),
    ("smeparsl", "\u{29E4}"), ("smid", "\u{2223}"), ("smile", "\u{2323}"), ("smt", "\u{2AAA}"),
    ("smte", "\u{2AAC}"), ("smtes", "\u{2AAC}\u{FE00}"), ("softcy", "\u{44C}"), ("sol", "/"),
    ("solb", "\u{29C4}"), ("solbar", "\u{233F}"), ("sopf", "\u{1D564}"),
    ("spades", "\u{2660}"), ("spadesuit", "\u{2660}"), ("spar", "\u{2225}"),
    ("sqcap", "\u{2293}"), ("sqcaps", "\u{2293}\u{FE00}"), ("sqcup", "\u{2294}"),
    ("sqcups", "\u{2294}\u{FE00}"), ("sqsub", "\u{228F}"), ("sqsube", "\u{2291}"),
    ("sqsubset", "\u{228F}"), ("sqsubseteq", "\u{2291}"), ("sqsup", "\u{2290}"),
    ("sqsupe", "\u{2292}"), ("sqsupset", "\u{2290}"), ("sqsupseteq", "\u{2292}"),
    ("squ", "\u{25A1}"), ("square", "\u{25A1}"), ("squarf", "\u{25AA}"), ("squf", "\u{25AA}"),
    ("srarr", "\u{2192}"), ("sscr", "\u{1D4C8}"), ("ssetmn", "\u{2216}"),
    ("ssmile", "\u{2323}"), ("sstarf", "\u{22C6}"), ("star", "\u{2606}"),
    ("starf", "\u{2605}"), ("straightepsilon", "\u{3F5}"), ("straightphi", "\u{3D5}"),
    ("strns", "\u{AF}"), ("sub", "\u{2282}"), ("subE", "\u{2AC5}"), ("subdot", "\u{2ABD}"),
    ("sube", "\u{2286}"), ("subedot", "\u{2AC3}"), ("submult", "\u{2AC1}"),
    ("subnE", "\u{2ACB}"), ("subne", "\u{228A}"), ("subplus", "\u{2ABF}"),
    ("subrarr", "\u{2979}"), ("subset", "\u{2282}"), ("subseteq", "\u{2286}"),
    ("subseteqq", "\u{2AC5}"), ("subsetneq", "\u{228A}"), ("subsetneqq", "\u{2ACB}"),
    ("subsim", "\u{2AC7}"), ("subsub", "\u{2AD5}"), ("subsup", "\u{2AD3}"),
    ("succ", "\u{227B}"), ("succapprox", "\u{2AB8}"), ("succcurlyeq", "\u{227D}"),
    ("succeq", "\u{2AB0}"), ("succnapprox", "\u{2ABA}"), ("succneqq", "\u{2AB6}"),
    ("succnsim", "\u{22E9}"), ("succsim", "\u{227F}"), ("sum", "\u{2211}"),
    ("sung", "\u{266A}"), ("sup", "\u{2283}"), ("sup1", "\u{B9}"), ("sup2", "\u{B2}"),
    ("sup3", "\u{B3}"), ("supE", "\u{2AC6}"), ("supdot", "\u{2ABE}"), ("supdsub", "\u{2AD8}"),
    ("supe", "\u{2287}"), ("supedot", "\u{2AC4}"), ("suphsol", "\u{27C9}"),
    ("suphsub", "\u{2AD7}"), ("suplarr", "\u{297B}"), ("supmult", "\u{2AC2}"),
    ("supnE", "\u{2ACC}"), ("supne", "\u{228B}"), ("supplus", "\u{2AC0}"),
    ("supset", "\u{2283}"), ("supseteq", "\u{2287}"), ("supseteqq", "\u{2AC6}"),
    ("supsetneq", "\u{228B}"), ("supsetneqq", "\u{2ACC}"), ("supsim", "\u{2AC8}"),
    ("supsub", "\u{2AD4}"), ("supsup", "\u{2AD6}"), ("swArr", "\u{21D9}"),
    ("swarhk", "\u{2926}"), ("swarr", "\u{2199}"), ("swarrow", "\u{2199}"),
    ("swnwar", "\u{292A}"), ("szlig", "\u{DF}"), ("target", "\u{2316}"), ("tau", "\u{3C4}"),
    ("tbrk", "\u{23B4}"), ("tcaron", "\u{165}"), ("tcedil", "\u{163}"), ("tcy", "\u{442}"),
    ("tdot", "\u{20DB}"), ("telrec", "\u{2315}"), ("tfr", "\u{1D531}"), ("there4", "\u{2234}"),
    ("therefore", "\u{2234}"), ("theta", "\u{3B8}"), ("thetasym", "\u{3D1}"),
    ("thetav", "\u{3D1}"), ("thickapprox", "\u{2248}"), ("thicksim", "\u{223C}"),
    ("thinsp", "\u{2009}"), ("thkap", "\u{2248}"), ("thksim", "\u{223C}"), ("thorn", "\u{FE}"),
    ("tilde", "\u{2DC}"), ("times", "\u{D7}"), ("timesb", "\u{22A0}"),
    ("timesbar", "\u{2A31}"), ("timesd", "\u{2A30}"), ("tint", "\u{222D}"),
    ("toea", "\u{2928}"), ("top", "\u{22A4}"), ("topbot", "\u{2336}"), ("topcir", "\u{2AF1}"),
    ("topf", "\u{1D565}"), ("topfork", "\u{2ADA}"), ("tosa", "\u{2929}"),
    ("tprime", "\u{2034}"), ("trade", "\u{2122}"), ("triangle", "\u{25B5}"),
    ("triangledown", "\u{25BF}"), ("triangleleft", "\u{25C3}"), ("trianglelefteq", "\u{22B4}"),
    ("triangleq", "\u{225C}"), ("triangleright", "\u{25B9}"), ("trianglerighteq", "\u{22B5}"),
    ("tridot", "\u{25EC}"), ("trie", "\u{225C}"), ("triminus", "\u{2A3A}"),
    ("triplus", "\u{2A39}"), ("trisb", "\u{29CD}"), ("tritime", "\u{2A3B}"),
    ("trpezium", "\u{23E2}"), ("tscr", "\u{1D4C9}"), ("tscy", "\u{446}"), ("tshcy", "\u{45B}"),
    ("tstrok", "\u{167}"), ("twixt", "\u{226C}"), ("twoheadleftarrow", "\u{219E}"),
    ("twoheadrightarrow", "\u{21A0}"), ("uArr", "\u{21D1}"), ("uHar", "\u{2963}"),
    ("uacute", "\u{FA}"), ("uarr", "\u{2191}"), ("ubrcy", "\u{45E}"), ("ubreve", "\u{16D}"),
    ("ucirc", "\u{FB}"), ("ucy", "\u{443}"), ("udarr", "\u{21C5}"), ("udblac", "\u{171}"),
    ("udhar", "\u{296E}"), ("ufisht", "\u{297E}"), ("ufr", "\u{1D532}"), ("ugrave", "\u{F9}"),
    ("uharl", "\u{21BF}"), ("uharr", "\u{21BE}"), ("uhblk", "\u{2580}"),
    ("ulcorn", "\u{231C}"), ("ulcorner", "\u{231C}"), ("ulcrop", "\u{230F}"),
    ("ultri", "\u{25F8}"), ("umacr", "\u{16B}"), ("uml", "\u{A8}"), ("uogon", "\u{173}"),
    ("uopf", "\u{1D566}"), ("uparrow", "\u{2191}"), ("updownarrow", "\u{2195}"),
    ("upharpoonleft", "\u{21BF}"), ("upharpoonright", "\u{21BE}"), ("uplus", "\u{228E}"),
    ("upsi", "\u{3C5}"), ("upsih", "\u{3D2}"), ("upsilon", "\u{3C5}"),
    ("upuparrows", "\u{21C8}"), ("urcorn", "\u{231D}"), ("urcorner", "\u{231D}"),
    ("urcrop", "\u{230E}"), ("uring", "\u{16F}"), ("urtri", "\u{25F9}"), ("uscr", "\u{1D4CA}"),
    ("utdot", "\u{22F0}"), ("utilde", "\u{169}"), ("utri", "\u{25B5}"), ("utrif", "\u{25B4}"),
    ("uuarr", "\u{21C8}"), ("uuml", "\u{FC}"), ("uwangle", "\u{29A7}"), ("vArr", "\u{21D5}"),
    ("vBar", "\u{2AE8}"), ("vBarv", "\u{2AE9}"), ("vDash", "\u{22A8}"), ("vangrt", "\u{299C}"),
    ("varepsilon", "\u{3F5}"), ("varkappa", "\u{3F0}"), ("varnothing", "\u{2205}"),
    ("varphi", "\u{3D5}"), ("varpi", "\u{3D6}"), ("varpropto", "\u{221D}"),
    ("varr", "\u{2195}"), ("varrho", "\u{3F1}"), ("varsigma", "\u{3C2}"),
    ("varsubsetneq", "\u{228A}\u{FE00}"), ("varsubsetneqq", "\u{2ACB}\u{FE00}"),
    ("varsupsetneq", "\u{228B}\u{FE00}"), ("varsupsetneqq", "\u{2ACC}\u{FE00}"),
    ("vartheta", "\u{3D1}"), ("vartriangleleft", "\u{22B2}"), ("vartriangleright", "\u{22B3}"),
    ("vcy", "\u{432}"), ("vdash", "\u{22A2}"), ("vee", "\u{2228}"), ("veebar", "\u{22BB}"),
    ("veeeq", "\u{225A}"), ("vellip", "\u{22EE}"), ("verbar", "|"), ("vert", "|"),
    ("vfr", "\u{1D533}"), ("vltri", "\u{22B2}"), ("vnsub", "\u{2282}\u{20D2}"),
    ("vnsup", "\u{2283}\u{20D2}"), ("vopf", "\u{1D567}"), ("vprop", "\u{221D}"),
    ("vrtri", "\u{22B3}"), ("vscr", "\u{1D4CB}"), ("vsubnE", "\u{2ACB}\u{FE00}"),
    ("vsubne", "\u{228A}\u{FE00}"), ("vsupnE", "\u{2ACC}\u{FE00}"),
    ("vsupne", "\u{228B}\u{FE00}"), ("vzigzag", "\u{299A}"), ("wcirc", "\u{175}"),
    ("wedbar", "\u{2A5F}"), ("wedge", "\u{2227}"), ("wedgeq", "\u{2259}"),
    ("weierp", "\u{2118}"), ("wfr", "\u{1D534}"), ("wopf", "\u{1D568}"), ("wp", "\u{2118}"),
    ("wr", "\u{2240}"), ("wreath", "\u{2240}"), ("wscr", "\u{1D4CC}"), ("xcap", "\u{22C2}"),
    ("xcirc", "\u{25EF}"), ("xcup", "\u{22C3}"), ("xdtri", "\u{25BD}"), ("xfr", "\u{1D535}"),
    ("xhArr", "\u{27FA}"), ("xharr", "\u{27F7}"), ("xi", "\u{3BE}"), ("xlArr", "\u{27F8}"),
    ("xlarr", "\u{27F5}"), ("xmap", "\u{27FC}"), ("xnis", "\u{22FB}"), ("xodot", "\u{2A00}"),
    ("xopf", "\u{1D569}"), ("xoplus", "\u{2A01}"), ("xotime", "\u{2A02}"),
    ("xrArr", "\u{27F9}"), ("xrarr", "\u{27F6}"), ("xscr", "\u{1D4CD}"),
    ("xsqcup", "\u{2A06}"), ("xuplus", "\u{2A04}"), ("xutri", "\u{25B3}"),
    ("xvee", "\u{22C1}"), ("xwedge", "\u{22C0}"), ("yacute", "\u{FD}"), ("yacy", "\u{44F}"),
    ("ycirc", "\u{177}"), ("ycy", "\u{44B}"), ("yen", "\u{A5}"), ("yfr", "\u{1D536}"),
    ("yicy", "\u{457}"), ("yopf", "\u{1D56A}"), ("yscr", "\u{1D4CE}"), ("yucy", "\u{44E}"),
    ("yuml", "\u{FF}"), ("zacute", "\u{17A}"), ("zcaron", "\u{17E}"), ("zcy", "\u{437}"),
    ("zdot", "\u{17C}"), ("zeetrf", "\u{2128}"), ("zeta", "\u{3B6}"), ("zfr", "\u{1D537}"),
    ("zhcy", "\u{436}"), ("zigrarr", "\u{21DD}"), ("zopf", "\u{1D56B}"), ("zscr", "\u{1D4CF}"),
    ("zwj", "\u{200D}"), ("zwnj", "\u{200C}"),
];

/// Where Unicode full case folding (what cmark's `cmark_utf8proc_case_fold`
/// applies to a reference label) is not `char::to_lowercase`: `ß` folds to
/// "ss", final `ς` to `σ`, Cherokee to its capitals. Generated from Python's
/// `str.casefold` (Unicode 15.1); sorted for a binary search.
#[rustfmt::skip]
const CASE_FOLD_EXTRA: &[(char, &str)] = &[
    ('\u{B5}', "\u{3BC}"), ('\u{DF}', "ss"), ('\u{149}', "\u{2BC}n"), ('\u{17F}', "s"),
    ('\u{1F0}', "j\u{30C}"), ('\u{345}', "\u{3B9}"), ('\u{390}', "\u{3B9}\u{308}\u{301}"),
    ('\u{3B0}', "\u{3C5}\u{308}\u{301}"), ('\u{3C2}', "\u{3C3}"), ('\u{3D0}', "\u{3B2}"),
    ('\u{3D1}', "\u{3B8}"), ('\u{3D5}', "\u{3C6}"), ('\u{3D6}', "\u{3C0}"),
    ('\u{3F0}', "\u{3BA}"), ('\u{3F1}', "\u{3C1}"), ('\u{3F5}', "\u{3B5}"),
    ('\u{587}', "\u{565}\u{582}"), ('\u{13A0}', "\u{13A0}"), ('\u{13A1}', "\u{13A1}"),
    ('\u{13A2}', "\u{13A2}"), ('\u{13A3}', "\u{13A3}"), ('\u{13A4}', "\u{13A4}"),
    ('\u{13A5}', "\u{13A5}"), ('\u{13A6}', "\u{13A6}"), ('\u{13A7}', "\u{13A7}"),
    ('\u{13A8}', "\u{13A8}"), ('\u{13A9}', "\u{13A9}"), ('\u{13AA}', "\u{13AA}"),
    ('\u{13AB}', "\u{13AB}"), ('\u{13AC}', "\u{13AC}"), ('\u{13AD}', "\u{13AD}"),
    ('\u{13AE}', "\u{13AE}"), ('\u{13AF}', "\u{13AF}"), ('\u{13B0}', "\u{13B0}"),
    ('\u{13B1}', "\u{13B1}"), ('\u{13B2}', "\u{13B2}"), ('\u{13B3}', "\u{13B3}"),
    ('\u{13B4}', "\u{13B4}"), ('\u{13B5}', "\u{13B5}"), ('\u{13B6}', "\u{13B6}"),
    ('\u{13B7}', "\u{13B7}"), ('\u{13B8}', "\u{13B8}"), ('\u{13B9}', "\u{13B9}"),
    ('\u{13BA}', "\u{13BA}"), ('\u{13BB}', "\u{13BB}"), ('\u{13BC}', "\u{13BC}"),
    ('\u{13BD}', "\u{13BD}"), ('\u{13BE}', "\u{13BE}"), ('\u{13BF}', "\u{13BF}"),
    ('\u{13C0}', "\u{13C0}"), ('\u{13C1}', "\u{13C1}"), ('\u{13C2}', "\u{13C2}"),
    ('\u{13C3}', "\u{13C3}"), ('\u{13C4}', "\u{13C4}"), ('\u{13C5}', "\u{13C5}"),
    ('\u{13C6}', "\u{13C6}"), ('\u{13C7}', "\u{13C7}"), ('\u{13C8}', "\u{13C8}"),
    ('\u{13C9}', "\u{13C9}"), ('\u{13CA}', "\u{13CA}"), ('\u{13CB}', "\u{13CB}"),
    ('\u{13CC}', "\u{13CC}"), ('\u{13CD}', "\u{13CD}"), ('\u{13CE}', "\u{13CE}"),
    ('\u{13CF}', "\u{13CF}"), ('\u{13D0}', "\u{13D0}"), ('\u{13D1}', "\u{13D1}"),
    ('\u{13D2}', "\u{13D2}"), ('\u{13D3}', "\u{13D3}"), ('\u{13D4}', "\u{13D4}"),
    ('\u{13D5}', "\u{13D5}"), ('\u{13D6}', "\u{13D6}"), ('\u{13D7}', "\u{13D7}"),
    ('\u{13D8}', "\u{13D8}"), ('\u{13D9}', "\u{13D9}"), ('\u{13DA}', "\u{13DA}"),
    ('\u{13DB}', "\u{13DB}"), ('\u{13DC}', "\u{13DC}"), ('\u{13DD}', "\u{13DD}"),
    ('\u{13DE}', "\u{13DE}"), ('\u{13DF}', "\u{13DF}"), ('\u{13E0}', "\u{13E0}"),
    ('\u{13E1}', "\u{13E1}"), ('\u{13E2}', "\u{13E2}"), ('\u{13E3}', "\u{13E3}"),
    ('\u{13E4}', "\u{13E4}"), ('\u{13E5}', "\u{13E5}"), ('\u{13E6}', "\u{13E6}"),
    ('\u{13E7}', "\u{13E7}"), ('\u{13E8}', "\u{13E8}"), ('\u{13E9}', "\u{13E9}"),
    ('\u{13EA}', "\u{13EA}"), ('\u{13EB}', "\u{13EB}"), ('\u{13EC}', "\u{13EC}"),
    ('\u{13ED}', "\u{13ED}"), ('\u{13EE}', "\u{13EE}"), ('\u{13EF}', "\u{13EF}"),
    ('\u{13F0}', "\u{13F0}"), ('\u{13F1}', "\u{13F1}"), ('\u{13F2}', "\u{13F2}"),
    ('\u{13F3}', "\u{13F3}"), ('\u{13F4}', "\u{13F4}"), ('\u{13F5}', "\u{13F5}"),
    ('\u{13F8}', "\u{13F0}"), ('\u{13F9}', "\u{13F1}"), ('\u{13FA}', "\u{13F2}"),
    ('\u{13FB}', "\u{13F3}"), ('\u{13FC}', "\u{13F4}"), ('\u{13FD}', "\u{13F5}"),
    ('\u{1C80}', "\u{432}"), ('\u{1C81}', "\u{434}"), ('\u{1C82}', "\u{43E}"),
    ('\u{1C83}', "\u{441}"), ('\u{1C84}', "\u{442}"), ('\u{1C85}', "\u{442}"),
    ('\u{1C86}', "\u{44A}"), ('\u{1C87}', "\u{463}"), ('\u{1C88}', "\u{A64B}"),
    ('\u{1E96}', "h\u{331}"), ('\u{1E97}', "t\u{308}"), ('\u{1E98}', "w\u{30A}"),
    ('\u{1E99}', "y\u{30A}"), ('\u{1E9A}', "a\u{2BE}"), ('\u{1E9B}', "\u{1E61}"),
    ('\u{1E9E}', "ss"), ('\u{1F50}', "\u{3C5}\u{313}"), ('\u{1F52}', "\u{3C5}\u{313}\u{300}"),
    ('\u{1F54}', "\u{3C5}\u{313}\u{301}"), ('\u{1F56}', "\u{3C5}\u{313}\u{342}"),
    ('\u{1F80}', "\u{1F00}\u{3B9}"), ('\u{1F81}', "\u{1F01}\u{3B9}"),
    ('\u{1F82}', "\u{1F02}\u{3B9}"), ('\u{1F83}', "\u{1F03}\u{3B9}"),
    ('\u{1F84}', "\u{1F04}\u{3B9}"), ('\u{1F85}', "\u{1F05}\u{3B9}"),
    ('\u{1F86}', "\u{1F06}\u{3B9}"), ('\u{1F87}', "\u{1F07}\u{3B9}"),
    ('\u{1F88}', "\u{1F00}\u{3B9}"), ('\u{1F89}', "\u{1F01}\u{3B9}"),
    ('\u{1F8A}', "\u{1F02}\u{3B9}"), ('\u{1F8B}', "\u{1F03}\u{3B9}"),
    ('\u{1F8C}', "\u{1F04}\u{3B9}"), ('\u{1F8D}', "\u{1F05}\u{3B9}"),
    ('\u{1F8E}', "\u{1F06}\u{3B9}"), ('\u{1F8F}', "\u{1F07}\u{3B9}"),
    ('\u{1F90}', "\u{1F20}\u{3B9}"), ('\u{1F91}', "\u{1F21}\u{3B9}"),
    ('\u{1F92}', "\u{1F22}\u{3B9}"), ('\u{1F93}', "\u{1F23}\u{3B9}"),
    ('\u{1F94}', "\u{1F24}\u{3B9}"), ('\u{1F95}', "\u{1F25}\u{3B9}"),
    ('\u{1F96}', "\u{1F26}\u{3B9}"), ('\u{1F97}', "\u{1F27}\u{3B9}"),
    ('\u{1F98}', "\u{1F20}\u{3B9}"), ('\u{1F99}', "\u{1F21}\u{3B9}"),
    ('\u{1F9A}', "\u{1F22}\u{3B9}"), ('\u{1F9B}', "\u{1F23}\u{3B9}"),
    ('\u{1F9C}', "\u{1F24}\u{3B9}"), ('\u{1F9D}', "\u{1F25}\u{3B9}"),
    ('\u{1F9E}', "\u{1F26}\u{3B9}"), ('\u{1F9F}', "\u{1F27}\u{3B9}"),
    ('\u{1FA0}', "\u{1F60}\u{3B9}"), ('\u{1FA1}', "\u{1F61}\u{3B9}"),
    ('\u{1FA2}', "\u{1F62}\u{3B9}"), ('\u{1FA3}', "\u{1F63}\u{3B9}"),
    ('\u{1FA4}', "\u{1F64}\u{3B9}"), ('\u{1FA5}', "\u{1F65}\u{3B9}"),
    ('\u{1FA6}', "\u{1F66}\u{3B9}"), ('\u{1FA7}', "\u{1F67}\u{3B9}"),
    ('\u{1FA8}', "\u{1F60}\u{3B9}"), ('\u{1FA9}', "\u{1F61}\u{3B9}"),
    ('\u{1FAA}', "\u{1F62}\u{3B9}"), ('\u{1FAB}', "\u{1F63}\u{3B9}"),
    ('\u{1FAC}', "\u{1F64}\u{3B9}"), ('\u{1FAD}', "\u{1F65}\u{3B9}"),
    ('\u{1FAE}', "\u{1F66}\u{3B9}"), ('\u{1FAF}', "\u{1F67}\u{3B9}"),
    ('\u{1FB2}', "\u{1F70}\u{3B9}"), ('\u{1FB3}', "\u{3B1}\u{3B9}"),
    ('\u{1FB4}', "\u{3AC}\u{3B9}"), ('\u{1FB6}', "\u{3B1}\u{342}"),
    ('\u{1FB7}', "\u{3B1}\u{342}\u{3B9}"), ('\u{1FBC}', "\u{3B1}\u{3B9}"),
    ('\u{1FBE}', "\u{3B9}"), ('\u{1FC2}', "\u{1F74}\u{3B9}"), ('\u{1FC3}', "\u{3B7}\u{3B9}"),
    ('\u{1FC4}', "\u{3AE}\u{3B9}"), ('\u{1FC6}', "\u{3B7}\u{342}"),
    ('\u{1FC7}', "\u{3B7}\u{342}\u{3B9}"), ('\u{1FCC}', "\u{3B7}\u{3B9}"),
    ('\u{1FD2}', "\u{3B9}\u{308}\u{300}"), ('\u{1FD3}', "\u{3B9}\u{308}\u{301}"),
    ('\u{1FD6}', "\u{3B9}\u{342}"), ('\u{1FD7}', "\u{3B9}\u{308}\u{342}"),
    ('\u{1FE2}', "\u{3C5}\u{308}\u{300}"), ('\u{1FE3}', "\u{3C5}\u{308}\u{301}"),
    ('\u{1FE4}', "\u{3C1}\u{313}"), ('\u{1FE6}', "\u{3C5}\u{342}"),
    ('\u{1FE7}', "\u{3C5}\u{308}\u{342}"), ('\u{1FF2}', "\u{1F7C}\u{3B9}"),
    ('\u{1FF3}', "\u{3C9}\u{3B9}"), ('\u{1FF4}', "\u{3CE}\u{3B9}"),
    ('\u{1FF6}', "\u{3C9}\u{342}"), ('\u{1FF7}', "\u{3C9}\u{342}\u{3B9}"),
    ('\u{1FFC}', "\u{3C9}\u{3B9}"), ('\u{AB70}', "\u{13A0}"), ('\u{AB71}', "\u{13A1}"),
    ('\u{AB72}', "\u{13A2}"), ('\u{AB73}', "\u{13A3}"), ('\u{AB74}', "\u{13A4}"),
    ('\u{AB75}', "\u{13A5}"), ('\u{AB76}', "\u{13A6}"), ('\u{AB77}', "\u{13A7}"),
    ('\u{AB78}', "\u{13A8}"), ('\u{AB79}', "\u{13A9}"), ('\u{AB7A}', "\u{13AA}"),
    ('\u{AB7B}', "\u{13AB}"), ('\u{AB7C}', "\u{13AC}"), ('\u{AB7D}', "\u{13AD}"),
    ('\u{AB7E}', "\u{13AE}"), ('\u{AB7F}', "\u{13AF}"), ('\u{AB80}', "\u{13B0}"),
    ('\u{AB81}', "\u{13B1}"), ('\u{AB82}', "\u{13B2}"), ('\u{AB83}', "\u{13B3}"),
    ('\u{AB84}', "\u{13B4}"), ('\u{AB85}', "\u{13B5}"), ('\u{AB86}', "\u{13B6}"),
    ('\u{AB87}', "\u{13B7}"), ('\u{AB88}', "\u{13B8}"), ('\u{AB89}', "\u{13B9}"),
    ('\u{AB8A}', "\u{13BA}"), ('\u{AB8B}', "\u{13BB}"), ('\u{AB8C}', "\u{13BC}"),
    ('\u{AB8D}', "\u{13BD}"), ('\u{AB8E}', "\u{13BE}"), ('\u{AB8F}', "\u{13BF}"),
    ('\u{AB90}', "\u{13C0}"), ('\u{AB91}', "\u{13C1}"), ('\u{AB92}', "\u{13C2}"),
    ('\u{AB93}', "\u{13C3}"), ('\u{AB94}', "\u{13C4}"), ('\u{AB95}', "\u{13C5}"),
    ('\u{AB96}', "\u{13C6}"), ('\u{AB97}', "\u{13C7}"), ('\u{AB98}', "\u{13C8}"),
    ('\u{AB99}', "\u{13C9}"), ('\u{AB9A}', "\u{13CA}"), ('\u{AB9B}', "\u{13CB}"),
    ('\u{AB9C}', "\u{13CC}"), ('\u{AB9D}', "\u{13CD}"), ('\u{AB9E}', "\u{13CE}"),
    ('\u{AB9F}', "\u{13CF}"), ('\u{ABA0}', "\u{13D0}"), ('\u{ABA1}', "\u{13D1}"),
    ('\u{ABA2}', "\u{13D2}"), ('\u{ABA3}', "\u{13D3}"), ('\u{ABA4}', "\u{13D4}"),
    ('\u{ABA5}', "\u{13D5}"), ('\u{ABA6}', "\u{13D6}"), ('\u{ABA7}', "\u{13D7}"),
    ('\u{ABA8}', "\u{13D8}"), ('\u{ABA9}', "\u{13D9}"), ('\u{ABAA}', "\u{13DA}"),
    ('\u{ABAB}', "\u{13DB}"), ('\u{ABAC}', "\u{13DC}"), ('\u{ABAD}', "\u{13DD}"),
    ('\u{ABAE}', "\u{13DE}"), ('\u{ABAF}', "\u{13DF}"), ('\u{ABB0}', "\u{13E0}"),
    ('\u{ABB1}', "\u{13E1}"), ('\u{ABB2}', "\u{13E2}"), ('\u{ABB3}', "\u{13E3}"),
    ('\u{ABB4}', "\u{13E4}"), ('\u{ABB5}', "\u{13E5}"), ('\u{ABB6}', "\u{13E6}"),
    ('\u{ABB7}', "\u{13E7}"), ('\u{ABB8}', "\u{13E8}"), ('\u{ABB9}', "\u{13E9}"),
    ('\u{ABBA}', "\u{13EA}"), ('\u{ABBB}', "\u{13EB}"), ('\u{ABBC}', "\u{13EC}"),
    ('\u{ABBD}', "\u{13ED}"), ('\u{ABBE}', "\u{13EE}"), ('\u{ABBF}', "\u{13EF}"),
    ('\u{FB00}', "ff"), ('\u{FB01}', "fi"), ('\u{FB02}', "fl"), ('\u{FB03}', "ffi"),
    ('\u{FB04}', "ffl"), ('\u{FB05}', "st"), ('\u{FB06}', "st"),
    ('\u{FB13}', "\u{574}\u{576}"), ('\u{FB14}', "\u{574}\u{565}"),
    ('\u{FB15}', "\u{574}\u{56B}"), ('\u{FB16}', "\u{57E}\u{576}"),
    ('\u{FB17}', "\u{574}\u{56D}"),
];

/// Scalars Foundation's IDNA refuses in a host whatever surrounds them:
/// private use, the format characters UTS #46 disallows (joiners, bidi
/// controls, Arabic number signs, tags), and the compatibility characters
/// that map onto ASCII a host may not hold (`´` to a space, U+1FEF to a
/// backtick, fullwidth brackets, U+FFFD). Measured over every scalar;
/// letters refused only for the bidi rule are left out on purpose.
#[rustfmt::skip]
const IDNA_REFUSED: &[(u32, u32)] = &[
    (0xA8, 0xA8), (0xAF, 0xAF), (0xB4, 0xB4), (0xB8, 0xB8), (0x2D8, 0x2DD), (0x37A, 0x37A),
    (0x384, 0x385), (0x600, 0x605), (0x61C, 0x61C), (0x6DD, 0x6DD), (0x70F, 0x70F), (0x890, 0x891),
    (0x8E2, 0x8E2), (0x1FBD, 0x1FBD), (0x1FBF, 0x1FC1), (0x1FCD, 0x1FCF), (0x1FDD, 0x1FDF), (0x1FED, 0x1FEF),
    (0x1FFD, 0x1FFE), (0x200C, 0x200F), (0x2017, 0x2017), (0x2024, 0x2026), (0x202A, 0x202E), (0x203E, 0x203E),
    (0x2047, 0x2049), (0x2066, 0x2069), (0x2100, 0x2101), (0x2105, 0x2106), (0x2135, 0x2138), (0x2488, 0x249B),
    (0x2A74, 0x2A74), (0x2FF0, 0x2FFF), (0x309B, 0x309C), (0x31EF, 0x31EF), (0x33C2, 0x33C2), (0x33C7, 0x33C7),
    (0x33D8, 0x33D8), (0xE000, 0xF8FF), (0xFE12, 0xFE13), (0xFE16, 0xFE16), (0xFE19, 0xFE19), (0xFE30, 0xFE30),
    (0xFE37, 0xFE38), (0xFE47, 0xFE4C), (0xFE52, 0xFE52), (0xFE55, 0xFE56), (0xFE5B, 0xFE5C), (0xFE5F, 0xFE5F),
    (0xFE64, 0xFE65), (0xFE68, 0xFE68), (0xFE6A, 0xFE6B), (0xFF02, 0xFF03), (0xFF05, 0xFF05), (0xFF0F, 0xFF0F),
    (0xFF1A, 0xFF1A), (0xFF1C, 0xFF1C), (0xFF1E, 0xFF20), (0xFF3B, 0xFF3E), (0xFF40, 0xFF40), (0xFF5B, 0xFF5D),
    (0xFFE3, 0xFFE3), (0xFFF9, 0xFFFD), (0x110BD, 0x110BD), (0x110CD, 0x110CD), (0x13430, 0x1343F), (0x1F100, 0x1F100),
    (0xE0001, 0xE0001), (0xE0020, 0xE007F), (0xF0000, 0x10FFFF),
];

/// Scalars IDNA removes from a host before anything else: soft hyphen,
/// the zero-width space and joiners' quieter cousins, variation selectors,
/// the byte-order mark. A host of nothing but these is no host at all.
#[rustfmt::skip]
const IDNA_IGNORED: &[(u32, u32)] = &[
    (0xAD, 0xAD), (0x34F, 0x34F), (0x180B, 0x180F), (0x200B, 0x200B), (0x2060, 0x2064), (0x206A, 0x206F),
    (0xFE00, 0xFE0F), (0xFEFF, 0xFEFF), (0x1BCA0, 0x1BCA3), (0x1D173, 0x1D17A), (0xE0100, 0xE01EF),
];

// MARK: - Tests

#[cfg(test)]
mod tests {
    //! What a chat bubble renders, and — more importantly — what it leaves
    //! completely alone. Every Swift test in MessageMarkdownTests.swift is
    //! here under its own name with the same inputs and expectations; the
    //! rest pin what the port had to learn from Foundation to get there.
    //!
    //! A handful of the Swift tests reach past the markdown into
    //! MessageLinks (scheme normalisation, the preview card, the `@ai` mark).
    //! The `composition` module below does exactly those steps, in Apple's
    //! order, over this module's output — which is also the clearest
    //! statement of how the web has to compose the pieces.

    use super::*;

    /// The rendered characters, which is what everything downstream indexes.
    fn plain(body: &str) -> String {
        render(body).plain()
    }

    /// The text of every text block, in order.
    fn block_texts(body: &str) -> Vec<String> {
        blocks(body)
            .iter()
            .filter_map(|block| match block {
                Block::Text(text) => Some(text.plain()),
                Block::Table(_) => None,
            })
            .collect()
    }

    /// The first table in a body, or `None` when it has none.
    fn table_in(body: &str) -> Option<Table> {
        blocks(body).into_iter().find_map(|block| match block {
            Block::Table(table) => Some(table),
            Block::Text(_) => None,
        })
    }

    fn cells(row: &[Text]) -> Vec<String> {
        row.iter().map(Text::plain).collect()
    }

    fn rows(table: &Table) -> Vec<Vec<String>> {
        table.rows.iter().map(|row| cells(row)).collect()
    }

    /// `(text, destination)` for every linked span.
    fn links(text: &Text) -> Vec<(String, String)> {
        text.spans
            .iter()
            .filter_map(|span| {
                span.style
                    .link
                    .as_ref()
                    .map(|link| (span.text.clone(), link.destination.clone()))
            })
            .collect()
    }

    fn only_text(body: &str) -> Text {
        match blocks(body).as_slice() {
            [Block::Text(text)] => text.clone(),
            other => panic!("{body:?} is not one text block: {other:?}"),
        }
    }

    /// The MessageLinks steps the Swift tests reach, over this module's
    /// output and in Apple's order. NOT a port of MessageLinks: the platform
    /// detector is not here, and every vector below is one where the
    /// markdown pass has already produced every link there is (the GFM
    /// autolink finds bare URLs first, and the detector never overwrites a
    /// link), so it would add nothing.
    mod composition {
        use super::super::*;

        /// `MessageLinks.normalized` (MessageLinks.swift:402-406): a
        /// destination with no scheme gains `https://`; one with a scheme
        /// is left exactly as written.
        pub fn normalized(destination: &str) -> String {
            if scheme(destination).is_some() {
                destination.to_string()
            } else {
                format!("https://{destination}")
            }
        }

        /// Foundation's `URL.scheme`, as far as these vectors need it.
        pub fn scheme(destination: &str) -> Option<&str> {
            let index = destination.find([':', '/', '?', '#', '[', ']', '@'])?;
            let candidate = &destination[..index];
            let valid = destination.as_bytes()[index] == b':'
                && candidate
                    .bytes()
                    .next()
                    .is_some_and(|b| b.is_ascii_alphabetic())
                && candidate
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b));
            valid.then_some(candidate)
        }

        /// `MessageLinks.firstWebLinkAsDrawn` (MessageLinks.swift:148-161):
        /// over the FLAT render, the first link whose normalised scheme is
        /// https.
        pub fn first_web_link_as_drawn(body: &str) -> Option<String> {
            render(body).spans.iter().find_map(|span| {
                let url = normalized(&span.style.link.as_ref()?.destination);
                (scheme(&url)?.eq_ignore_ascii_case("https")).then_some(url)
            })
        }

        /// `AssistantMention.ranges` (AssistantMention.swift:60-93): every
        /// `@ai`, ASCII case-insensitive, with an ASCII letter, digit or `_`
        /// on neither side. Byte ranges into `rendered`.
        pub fn ai_ranges(rendered: &str) -> Vec<(usize, usize)> {
            let bytes = rendered.as_bytes();
            let boundary = |b: u8| !(b.is_ascii_alphanumeric() || b == b'_');
            (0..bytes.len().saturating_sub(2))
                .filter(|&i| {
                    bytes[i] == b'@'
                        && bytes[i + 1] | 0x20 == b'a'
                        && bytes[i + 2] | 0x20 == b'i'
                        && (i == 0 || boundary(bytes[i - 1]))
                        && (i + 3 == bytes.len() || boundary(bytes[i + 3]))
                })
                .map(|i| (i, i + 3))
                .collect()
        }

        /// The runs drawn bold once `@ai` is marked the way
        /// `highlightMentions` marks it (MessageLinks.swift:384-393): the
        /// mark REPLACES the run's inline style with bold and, on another
        /// person's bubble, adds the accent colour — so a marked `@ai` is
        /// always a run of its own.
        pub fn bold_runs_after_mention_mark(text: &Text) -> Vec<String> {
            let marks = ai_ranges(&text.plain());
            let is_marked = |position: usize| {
                marks
                    .iter()
                    .any(|&(start, end)| position >= start && position < end)
            };
            // (text, bold, marked) pieces: each span cut where a mark starts
            // or ends. `@ai` is ASCII, so every cut is a char boundary.
            let mut pieces: Vec<(String, bool, bool)> = Vec::new();
            let mut offset = 0;
            for span in &text.spans {
                let mut cuts = vec![0, span.text.len()];
                for &(start, end) in &marks {
                    for cut in [start, end] {
                        if cut > offset && cut < offset + span.text.len() {
                            cuts.push(cut - offset);
                        }
                    }
                }
                cuts.sort_unstable();
                cuts.dedup();
                for pair in cuts.windows(2) {
                    let marked = is_marked(offset + pair[0]);
                    pieces.push((
                        span.text[pair[0]..pair[1]].to_string(),
                        span.style.strong || marked,
                        marked,
                    ));
                }
                offset += span.text.len();
            }
            let mut runs: Vec<(String, bool, bool)> = Vec::new();
            for (piece, bold, marked) in pieces {
                match runs.last_mut() {
                    // Marked pieces carry identical attributes, so they merge.
                    Some((run, true, true)) if marked => run.push_str(&piece),
                    _ => runs.push((piece, bold, marked)),
                }
            }
            runs.into_iter()
                .filter(|(_, bold, _)| *bold)
                .map(|(run, _, _)| run)
                .collect()
        }
    }

    // MARK: The Swift suite, test for test

    #[test]
    fn ordinary_text_is_untouched() {
        let untouched = [
            "Dinner at 7?",
            "see you at the shop",
            "2 * 3 * 4 = 24",
            "call me on 555-1234",
            "https://example.com/a_b_c",
            "he said \"what?\" and left",
            "snake_case_name stays whole",
            "a * b",
            "5 < 6 > 4",
            "cost: $5 (a bargain)",
            "",
            "🎉🎉🎉",
        ];
        for body in untouched {
            assert_eq!(plain(body), body, "rewritten: {body}");
        }
    }

    #[test]
    fn flanking_rules() {
        assert_eq!(
            plain("call user_name_field now"),
            "call user_name_field now"
        );
        assert_eq!(plain("a_b_c"), "a_b_c");
        assert_eq!(plain("2 * 3 * 4 = 24"), "2 * 3 * 4 = 24");
        assert_eq!(plain("a * b"), "a * b");
        assert_eq!(plain("*real italic*"), "real italic");
        assert_eq!(plain("5*6*7"), "567");
        assert_eq!(plain("_x_"), "x");
        assert_eq!(plain("__init__ is special"), "init is special");
    }

    #[test]
    fn the_subset_renders() {
        assert_eq!(plain("**bold**"), "bold");
        assert_eq!(plain("*italic*"), "italic");
        assert_eq!(plain("~~gone~~"), "gone");
        assert_eq!(plain("`code`"), "code");
        assert_eq!(plain("say **hello** there"), "say hello there");
    }

    #[test]
    fn nested_emphasis() {
        assert_eq!(
            plain("*this is **very** important*"),
            "this is very important"
        );
        assert_eq!(plain("*a **b** c*"), "a b c");
        assert_eq!(plain("_a __b__ c_"), "a b c");
    }

    #[test]
    fn markdown_link() {
        let rendered = render("see [the menu](https://example.com/menu)");
        assert_eq!(rendered.plain(), "see the menu");
        assert_eq!(
            links(&rendered),
            vec![(
                "the menu".to_string(),
                "https://example.com/menu".to_string()
            )]
        );
    }

    #[test]
    fn fenced_block() {
        let rendered = plain("try this:\n```\nlet x = **not bold**\n```\ndone");
        assert!(
            rendered.contains("let x = **not bold**"),
            "fence contents are literal: {rendered}"
        );
        assert!(rendered.contains("try this:"));
        assert!(rendered.contains("done"));
        assert!(
            !rendered.contains("```"),
            "the fence markers themselves are consumed"
        );
    }

    #[test]
    fn unclosed_fence() {
        let body = "```\nhalf written";
        assert_eq!(plain(body), body);
    }

    #[test]
    fn fence_language_tag() {
        let rendered = plain("```swift\nlet x = 1\n```");
        assert!(rendered.contains("let x = 1"));
        assert!(
            !rendered.contains("swift"),
            "the tag is metadata, not text: {rendered}"
        );
    }

    /// THE property: markdown removes characters, so a link found over the
    /// raw body would land on the wrong glyphs. Here the markdown pass
    /// itself produces the link (GFM autolink) over the RENDERED text.
    #[test]
    fn detected_link_offsets_survive_markdown() {
        let rendered = render("**important** go to https://example.com now");
        assert_eq!(rendered.plain(), "important go to https://example.com now");
        let linked: Vec<String> = links(&rendered).into_iter().map(|(text, _)| text).collect();
        assert_eq!(linked, vec!["https://example.com"]);
    }

    #[test]
    fn label_that_looks_like_a_url() {
        let rendered = render("[https://www.paypal.com](https://evil.example)");
        let destinations: Vec<String> = links(&rendered)
            .into_iter()
            .map(|(_, destination)| composition::normalized(&destination))
            .collect();
        assert_eq!(
            destinations,
            vec!["https://evil.example"],
            "exactly one destination, the one written"
        );
    }

    #[test]
    fn schemeless_destination() {
        let rendered = render("[here](example.com/x)");
        // The markdown hands on the destination as typed…
        assert_eq!(
            links(&rendered),
            vec![("here".to_string(), "example.com/x".to_string())]
        );
        // …and MessageLinks' normalisation is what makes it openable.
        let destinations: Vec<String> = links(&rendered)
            .into_iter()
            .map(|(_, destination)| composition::normalized(&destination))
            .collect();
        assert_eq!(destinations, vec!["https://example.com/x"]);
    }

    #[test]
    fn preview_uses_rendered_text() {
        use composition::first_web_link_as_drawn as card;
        assert_eq!(
            card("**https://example.com**").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            card("see [the menu](https://example.com/menu)").as_deref(),
            Some("https://example.com/menu")
        );
        assert_eq!(
            card("[here](example.com/x)").as_deref(),
            Some("https://example.com/x")
        );
        assert_eq!(
            card("[https://www.paypal.com](https://evil.example)").as_deref(),
            Some("https://evil.example")
        );
        assert_eq!(card("see http://example.com"), None);
        assert_eq!(card("no links here at all"), None);
    }

    #[test]
    fn mention_highlight() {
        let text = only_text("hey @ai and @aiden");
        assert_eq!(
            composition::bold_runs_after_mention_mark(&text),
            vec!["@ai"]
        );
    }

    #[test]
    fn emoji_survive() {
        for body in ["😀", "👨‍👩‍👧‍👦", "🇷🇸", "❤️"] {
            assert_eq!(plain(body), body);
        }
    }

    #[test]
    fn line_breaks() {
        let body = "first line\nsecond line\n\nafter a gap";
        assert_eq!(plain(body), body);
    }

    #[test]
    fn fence_keeps_the_preceding_newline() {
        assert_eq!(
            plain("try this:\n```\nlet x = 1\n```\ndone"),
            "try this:\nlet x = 1\ndone"
        );
        assert_eq!(plain("```\nlet x = 1\n```"), "let x = 1");
    }

    #[test]
    fn headings() {
        assert_eq!(plain("# Big"), "Big");
        assert_eq!(plain("## Medium"), "Medium");
        assert_eq!(plain("### Small"), "Small");
        assert_eq!(plain("# Done #"), "Done #");
        assert_eq!(plain("## say **hello**"), "say hello");
        // A heading is a font over the whole line, and one run of it.
        let heading = render("# Big");
        assert_eq!(heading.spans.len(), 1);
        assert_ne!(heading.spans[0].style.font, Font::Body);
        // Three DISTINCT steps.
        let steps: std::collections::HashSet<Font> = ["# x", "## x", "### x"]
            .iter()
            .map(|body| render(body).spans[0].style.font)
            .collect();
        assert_eq!(steps.len(), 3, "got {steps:?}");
    }

    #[test]
    fn heading_near_misses() {
        let untouched = [
            "#Heading",
            "#### X",
            "##### X",
            "# ",
            "#",
            "###",
            "a # b",
            "  # indented",
            "#1 fan",
        ];
        for body in untouched {
            assert_eq!(plain(body), body, "rewritten: {body}");
        }
        assert_eq!(plain("look:\n## Menu\nfries"), "look:\nMenu\nfries");
    }

    #[test]
    fn bullets() {
        assert_eq!(plain("- milk"), "• milk");
        assert_eq!(plain("* milk"), "• milk");
        assert_eq!(plain("+ milk"), "• milk");
        assert_eq!(plain("- a\n* b\n+ c"), "• a\n• b\n• c");
        assert_eq!(plain("- a\n  - b\n    - c"), "• a\n  • b\n    • c");
        assert_eq!(plain("\t- tabbed"), "\t• tabbed");
        assert_eq!(plain("- **milk** and eggs"), "• milk and eggs");
        assert_eq!(plain("* italic *not* here"), "• italic not here");
    }

    #[test]
    fn bullet_near_misses() {
        let untouched = [
            "- ",
            "-",
            "---",
            "***",
            "-no space",
            "2 * 3 * 4 = 24",
            "a - b",
            "5-6",
        ];
        for body in untouched {
            assert_eq!(plain(body), body, "rewritten: {body}");
        }
    }

    #[test]
    fn ordered_lists_are_untouched() {
        for body in [
            "1. milk",
            "1) milk",
            "1. a\n2. b\n3. c",
            "10. ten",
            "1.no space",
        ] {
            assert_eq!(plain(body), body, "rewritten: {body}");
        }
    }

    #[test]
    fn table_parsing() {
        let body = "| day | who  | cost |\n| :-- | :--: | ---: |\n| Mon | Ann  | 5    |\n| Tue | Bob  |\n| Wed | Cat  | 7 | extra |";
        let table = table_in(body).expect("a table");
        assert_eq!(cells(&table.header), vec!["day", "who", "cost"]);
        assert_eq!(
            table.alignments,
            vec![
                ColumnAlignment::Leading,
                ColumnAlignment::Center,
                ColumnAlignment::Trailing
            ]
        );
        assert_eq!(table.column_count(), 3);
        assert_eq!(
            rows(&table),
            vec![
                vec!["Mon", "Ann", "5"],
                vec!["Tue", "Bob", ""],
                vec!["Wed", "Cat", "7"]
            ]
        );
    }

    #[test]
    fn table_without_edge_pipes() {
        let table = table_in("day | who\n--- | ---\nMon | Ann").expect("a table");
        assert_eq!(cells(&table.header), vec!["day", "who"]);
        assert_eq!(
            table.alignments,
            vec![ColumnAlignment::Leading, ColumnAlignment::Leading]
        );
        assert_eq!(rows(&table), vec![vec!["Mon", "Ann"]]);
    }

    #[test]
    fn table_escaped_pipe() {
        let table = table_in("| a | b |\n| - | - |\n| x \\| y | z |").expect("a table");
        assert_eq!(rows(&table), vec![vec!["x | y", "z"]]);
    }

    #[test]
    fn table_cells_have_no_links() {
        let body = "| what | where |\n| --- | --- |\n| menu | [the menu](https://example.com/menu) |\n| site | https://example.com |";
        let table = table_in(body).expect("a table");
        assert_eq!(
            rows(&table),
            vec![
                vec!["menu", "[the menu](https://example.com/menu)"],
                vec!["site", "https://example.com"]
            ]
        );
        for row in &table.rows {
            for cell in row {
                assert!(
                    cell.spans.iter().all(|span| span.style.link.is_none()),
                    "a cell carried a link"
                );
            }
        }
        let emphasised = table_in("| a |\n| - |\n| **b** ~~c~~ `d` |").expect("a table");
        assert_eq!(rows(&emphasised), vec![vec!["b c d"]]);
    }

    #[test]
    fn table_splits_the_body() {
        let body = "before\n| a | b |\n| - | - |\n| 1 | 2 |\nafter";
        let blocks = blocks(body);
        assert_eq!(blocks.len(), 3);
        assert!(blocks[1].is_table());
        assert_eq!(block_texts(body), vec!["before", "after"]);
    }

    #[test]
    fn table_near_misses() {
        let untouched = [
            "some | thing\n---",
            "| a | b |\n| --- |",
            "| --- | --- |\n| a | b |",
            "| a | b |\n| x | y |",
            "| a | b |",
            "cost: $5 (a bargain)",
        ];
        for body in untouched {
            assert_eq!(plain(body), body, "rewritten: {body}");
            assert!(table_in(body).is_none(), "found a table in: {body}");
        }
    }

    #[test]
    fn flat_render_keeps_table_rows() {
        let body = "| day | who |\n| --- | --- |\n| Mon | Ann |";
        assert_eq!(plain(body), body);
    }

    #[test]
    fn contract_heading_and_bullet_beat_table() {
        let heading = "# Q | A\n--- | ---\n1 | 2";
        assert!(
            table_in(heading).is_none(),
            "the heading was eaten by a table"
        );
        assert_eq!(plain(heading), "Q | A\n--- | ---\n1 | 2");
        let bullet = "- a | b\n--- | ---\n1 | 2";
        assert!(
            table_in(bullet).is_none(),
            "the bullet was eaten by a table"
        );
        assert_eq!(plain(bullet), "• a | b\n--- | ---\n1 | 2");
    }

    #[test]
    fn contract_table_ends_at_heading_or_bullet() {
        let body = "| a | b |\n| --- | --- |\n| 1 | 2 |\n# Heading | x\n- item | y";
        let table = table_in(body).expect("a table");
        assert_eq!(
            rows(&table),
            vec![vec!["1", "2"]],
            "the heading was swallowed as a row"
        );
        assert_eq!(block_texts(body), vec!["Heading | x\n• item | y"]);
    }

    #[test]
    fn contract_lone_pipe_ends_the_table() {
        let body = "| a | b |\n| --- | --- |\n| 1 | 2 |\n|\n| 3 | 4 |";
        let table = table_in(body).expect("a table");
        assert_eq!(cells(&table.header), vec!["a", "b"]);
        assert_eq!(
            rows(&table),
            vec![vec!["1", "2"]],
            "a phantom empty row was padded in"
        );
        assert_eq!(block_texts(body), vec!["|\n| 3 | 4 |"]);
    }

    #[test]
    fn contract_delimiter_needs_a_pipe() {
        let rule = "| Total |\n---\n| 12 |";
        assert!(
            table_in(rule).is_none(),
            "a signature rule became a one-column table"
        );
        assert_eq!(plain(rule), rule);
        let real = table_in("| Total |\n| --- |\n| 12 |").expect("a table");
        assert_eq!(cells(&real.header), vec!["Total"]);
        assert_eq!(rows(&real), vec![vec!["12"]]);
    }

    #[test]
    fn contract_whitespace_is_not_content() {
        for body in ["#  ", "##   ", "###  ", "-  ", "*  ", "+  ", "- \t"] {
            assert_eq!(plain(body), body, "rewritten: {body:?}");
        }
        assert!(plain("#  Big").ends_with("Big"));
        assert!(plain("-  milk").ends_with("milk"));
    }

    #[test]
    fn contract_table_cell_url_still_previews() {
        let body = "| what | where |\n| --- | --- |\n| site | https://example.com/a |";
        let table = table_in(body).expect("a table");
        assert_eq!(rows(&table), vec![vec!["site", "https://example.com/a"]]);
        for row in &table.rows {
            assert!(row
                .iter()
                .all(|cell| cell.spans.iter().all(|span| span.style.link.is_none())));
        }
        assert_eq!(
            composition::first_web_link_as_drawn(body).as_deref(),
            Some("https://example.com/a")
        );
    }

    #[test]
    fn one_text_block() {
        let bodies = [
            "",
            "Dinner at 7?",
            "first line\nsecond line\n\nafter a gap",
            "# Heading\n- one\n- two\n1. three",
            "**bold** and [a link](https://example.com) and @ai",
            "try this:\n```\nlet x = 1\n```\ndone",
            "🎉🎉🎉",
            "| not | a table",
        ];
        for body in bodies {
            let blocks = blocks(body);
            assert_eq!(blocks.len(), 1, "{body:?} → {} blocks", blocks.len());
            assert!(!blocks[0].is_table());
        }
    }

    #[test]
    fn blocks_keep_their_own_offsets() {
        let body =
            "**ask** @ai\n| a | b |\n| - | - |\n| 1 | 2 |\n**then** see https://example.com now";
        let blocks = blocks(body);
        assert_eq!(blocks.len(), 3);
        assert!(blocks[1].is_table());
        let (Block::Text(first), Block::Text(last)) = (&blocks[0], &blocks[2]) else {
            panic!("expected text, table, text — got {blocks:?}");
        };
        assert_eq!(first.plain(), "ask @ai");
        assert_eq!(
            composition::bold_runs_after_mention_mark(first),
            vec!["ask", "@ai"]
        );
        assert_eq!(last.plain(), "then see https://example.com now");
        let linked: Vec<String> = links(last).into_iter().map(|(text, _)| text).collect();
        assert_eq!(linked, vec!["https://example.com"]);
    }

    // MARK: Unicode: never a byte offset off a character boundary

    /// The shape this project has shipped a panic in ("Привет" crashed a
    /// server request path): walk EVERY character-boundary prefix and suffix
    /// of every vector through both entry points. Only "no panic" is
    /// asserted — the oracle differential is what checks the answers.
    #[test]
    fn no_character_boundary_panics_anywhere() {
        let markers = [
            "*", "**", "_", "__", "~", "~~", "`", "```", "# ", "## ", "#", "- ", "* ", "+ ", "|",
            "\\", "[", "](", ")", "![", "^[", "<", ">", "&", "&amp;", ":", "://", "https://",
            "www.", "@", "\n", "\r\n", "\r", ":-:",
        ];
        let neighbours = [
            "Привет",
            "日本語",
            "👨‍👩‍👧‍👦",
            "👍🏽",
            "🇷🇸",
            "e\u{301}",
            "\u{301}",
            "\u{200D}",
            "\u{600}",
            "\u{1FEF}",
            "\u{A0}",
            "\u{200B}",
            "\u{3000}",
            "\u{2028}",
            "#\u{FE0F}\u{20E3}",
            "\u{0}",
        ];
        let mut vectors = Vec::new();
        for marker in markers {
            for neighbour in neighbours {
                vectors.push(format!("{neighbour}{marker}{neighbour}{marker}{neighbour}"));
                vectors.push(format!("{marker}{neighbour}{marker}"));
                vectors.push(format!(
                    "| {neighbour}{marker} | b |\n| - | - |\n| {marker}{neighbour} |"
                ));
                vectors.push(format!(
                    "{marker}{neighbour}\n```\n{neighbour}{marker}\n```\n{marker} {neighbour}"
                ));
            }
        }
        for vector in &vectors {
            let boundaries: Vec<usize> = vector
                .char_indices()
                .map(|(i, _)| i)
                .chain([vector.len()])
                .collect();
            for &i in &boundaries {
                for piece in [&vector[..i], &vector[i..]] {
                    let _ = blocks(piece);
                    let _ = render(piece);
                }
            }
        }
    }

    #[test]
    fn markup_around_cyrillic_cjk_and_emoji() {
        let bold = render("**Привет**");
        assert_eq!(bold.plain(), "Привет");
        assert!(bold.spans[0].style.strong);
        // An underscore between letters is not emphasis — in any script.
        assert_eq!(plain("при_вет_мир"), "при_вет_мир");
        // An asterisk between letters is, as in ASCII.
        assert_eq!(plain("при*вет*мир"), "приветмир");
        assert_eq!(plain("日本*語*です"), "日本語です");
        let family = render("~~👨‍👩‍👧‍👦~~ and *👍🏽*");
        assert_eq!(family.plain(), "👨‍👩‍👧‍👦 and 👍🏽");
        assert!(family.spans[0].style.strikethrough);
        assert!(family.spans[2].style.emphasis);
        assert_eq!(plain("# 日本語"), "日本語");
        assert_eq!(render("# 日本語").spans[0].style.font, Font::Heading(1));
        assert_eq!(plain("- 🇷🇸 flag"), "• 🇷🇸 flag");
        assert_eq!(plain("`код`"), "код");
        let link = render("[ссылка 🔗](https://пример.рф/путь)");
        assert_eq!(
            links(&link),
            vec![(
                "ссылка 🔗".to_string(),
                "https://пример.рф/путь".to_string()
            )]
        );
        let table = table_in("| имя | 名前 |\n| :-: | --: |\n| Аня 👩 | 花子 |").expect("a table");
        assert_eq!(cells(&table.header), vec!["имя", "名前"]);
        assert_eq!(rows(&table), vec![vec!["Аня 👩", "花子"]]);
    }

    /// A combining mark or a joiner straight after a marker makes ONE Swift
    /// `Character` with it, and that character is not the marker.
    #[test]
    fn a_mark_riding_a_marker_defeats_it() {
        for body in [
            "#\u{301} Big",        // the hash carries the accent
            "# \u{301}Big",        // the SPACE carries it
            "#\u{200D} Big",       // a joiner attaches too
            "#\u{FE0F}\u{20E3} x", // the keycap emoji #️⃣ is not a hash
            "\u{600}# Big",        // a prepended mark joins the hash from in front
            "#\u{600} Big",        // …and the space from in front
            "-\u{301} milk",
            "- \u{301}milk",
        ] {
            assert_eq!(plain(body), body, "rewritten: {body:?}");
        }
        // The third backtick carries a mark: no fence, and the inline parser
        // — which works in bytes — reads the backticks as a code span.
        let code = render("```\u{301}\ncode\n```");
        assert_eq!(code.plain(), "\u{301} code ");
        assert!(code.spans[0].style.code);
        // A pipe with a mark is not a cell boundary; a dash with one is not
        // a delimiter.
        let table = table_in("|\u{301} a | b |\n| - | - |").expect("a table");
        assert_eq!(cells(&table.header), vec!["|\u{301} a", "b"]);
        assert!(table_in("| a | b |\n| -\u{301} | - |").is_none());
        assert!(table_in("| a | b |\n| :\u{301}- | - |").is_none());
    }

    /// U+1FEF GREEK VARIA decomposes to a backtick, so Swift's `hasPrefix`
    /// counts it as one; cmark, which reads bytes, never does.
    #[test]
    fn greek_varia_is_a_fence_backtick_to_swift() {
        let fence = render("\u{1FEF}\u{1FEF}\u{1FEF}\ncode\n\u{1FEF}\u{1FEF}\u{1FEF}");
        assert_eq!(fence.plain(), "code");
        assert_eq!(fence.spans[0].style.font, Font::Monospaced);
        assert_eq!(plain("\u{1FEF}code\u{1FEF}"), "\u{1FEF}code\u{1FEF}");
    }

    // MARK: The Unicode predicates, each against the Swift one it mirrors

    /// Foundation's `CharacterSet.whitespaces` — what the heading and bullet
    /// rules trim with — is neither Unicode `White_Space` (Rust's
    /// `char::is_whitespace`) nor cmark's space.
    #[test]
    fn foundation_whitespace_is_its_own_set() {
        assert!(is_foundation_whitespace('\u{200B}') && !'\u{200B}'.is_whitespace());
        for c in [
            '\n', '\r', '\u{B}', '\u{C}', '\u{85}', '\u{2028}', '\u{2029}',
        ] {
            assert!(!is_foundation_whitespace(c) && c.is_whitespace(), "{c:?}");
        }
        for c in [
            '\t', ' ', '\u{A0}', '\u{1680}', '\u{2000}', '\u{200A}', '\u{202F}', '\u{205F}',
            '\u{3000}',
        ] {
            assert!(is_foundation_whitespace(c) && c.is_whitespace(), "{c:?}");
        }
        assert!(!is_foundation_whitespace('\u{180E}') && !'\u{180E}'.is_whitespace());
        // What that does to the grammar:
        assert_eq!(plain("# \u{200B}"), "# \u{200B}"); // a heading of nothing
        assert_eq!(plain("- \u{2028}"), "• \u{2028}"); // a bullet whose text is a separator
        assert_eq!(plain("# \u{3000}"), "# \u{3000}");
        assert_eq!(super::cells("| \u{200B} |"), vec![""]);
    }

    /// cmark's space and punctuation — what emphasis flanking reads — are
    /// cmark's own tables, measured off Foundation.
    #[test]
    fn cmark_space_and_punctuation_are_cmarks_own() {
        assert!(!is_cmark_space('\u{B}')); // VT is not, though White_Space says so
        assert!(is_cmark_space('\u{C}'));
        assert!(!is_cmark_space('\u{200B}'));
        assert!(!is_cmark_space('\u{85}'));
        assert!(is_cmark_space('\u{3000}'));
        assert!(
            is_cmark_punctuation('$') && is_cmark_punctuation('~') && is_cmark_punctuation('«')
        );
        assert!(
            !is_cmark_punctuation('€')
                && !is_cmark_punctuation('©')
                && !is_cmark_punctuation('\u{2028}')
        );
        assert_eq!(plain("a*€b*"), "a€b"); // `€` is not punctuation, so the run can open
        assert_eq!(plain("a*«b*"), "a*«b*"); // `«` is, so a letter before it cannot
        assert_eq!(plain("*\u{B}b*"), "\u{B}b");
    }

    #[test]
    fn a_swift_character_is_a_whole_cluster() {
        assert!(cluster_is("`", '`') && cluster_is("\u{1FEF}", '`'));
        assert!(!cluster_is("#\u{301}", '#') && !cluster_is("\r\n", '\n'));
        assert!(has_fence_prefix("``\u{1FEF}swift") && !has_fence_prefix("``\u{60}\u{301}"));
        assert!(starts_with_cluster("|a", '|') && !starts_with_cluster("|\u{301}a", '|'));
    }

    // MARK: What Foundation does on top of CommonMark

    #[test]
    fn bare_urls_hosts_and_addresses_are_links_in_the_parser() {
        let found = |body: &str| links(&render(body));
        let pair =
            |text: &str, destination: &str| vec![(text.to_string(), destination.to_string())];
        assert_eq!(
            found("see www.example.com"),
            pair("www.example.com", "http://www.example.com")
        );
        assert_eq!(
            found("write to nettrash@nettrash.me please"),
            pair("nettrash@nettrash.me", "mailto:nettrash@nettrash.me")
        );
        assert_eq!(
            found("go to https://example.com/a_b_c."),
            pair("https://example.com/a_b_c", "https://example.com/a_b_c")
        );
        assert_eq!(
            found("(https://en.wikipedia.org/wiki/Foo_(bar))"),
            pair(
                "https://en.wikipedia.org/wiki/Foo_(bar)",
                "https://en.wikipedia.org/wiki/Foo_(bar)"
            )
        );
        assert_eq!(
            found("mailto:x@y.com"),
            pair("mailto:x@y.com", "mailto:x@y.com")
        );
        assert_eq!(found("<https://a.b>"), pair("https://a.b", "https://a.b"));
        // Inside an open `[` there are no autolinks.
        assert!(found("[see www.example.com").is_empty());
        // A URL run straight into an emoji keeps it — and is still a link…
        assert_eq!(
            found("https://example.com👍"),
            pair("https://example.com👍", "https://example.com👍")
        );
        // …unless IDNA refuses the host: the family emoji holds U+200D.
        assert!(found("https://example.com👨‍👩‍👧").is_empty());
        // An underscore in the last two labels is not a host.
        assert!(found("http://a_b.c").is_empty());
    }

    #[test]
    fn a_link_label_is_flattened() {
        let link = render("[*a* `b` ~~c~~](https://x.com)");
        assert_eq!(link.spans.len(), 1);
        let span = &link.spans[0];
        assert_eq!(span.text, "a b c");
        assert!(!span.style.emphasis && !span.style.code && !span.style.strikethrough);
        assert!(span.style.link.is_some());
        // A hard break inside a label vanishes; a soft one stays.
        assert_eq!(plain("[a\\\nb](https://c.d)"), "ab");
        assert_eq!(plain("[a\nb](https://c.d)"), "a\nb");
        // An empty label draws nothing; an image with no alt draws U+FFFC.
        assert_eq!(plain("x[](https://a.b)y"), "xy");
        assert_eq!(plain("![](https://a.b)"), "\u{FFFC}");
        // An image draws its alt text, flattened, and carries its source.
        let image = render("![a **b**](https://i.png \"t\")");
        assert_eq!(image.plain(), "a b");
        assert_eq!(
            image.spans[0].style.image,
            Some(Link {
                destination: "https://i.png".to_string(),
                title: Some("t".to_string())
            })
        );
    }

    #[test]
    fn a_leading_reference_definition_vanishes() {
        let used = render("[1]: https://example.com\nsee [1]");
        assert_eq!(used.plain(), "see 1");
        assert_eq!(
            links(&used),
            vec![("1".to_string(), "https://example.com".to_string())]
        );
        // The shape a family member types, and loses a line to on Apple.
        assert_eq!(plain("[todo]: milk\nbuy eggs"), "buy eggs");
        // At the very end of the text it is not one (no newline after it).
        assert_eq!(plain("[todo]: milk"), "[todo]: milk");
        // Only at the very start.
        assert_eq!(plain("hi\n[a]: /u\n[a]"), "hi\n[a]: /u\n[a]");
        // Labels match by full Unicode case folding: ß is SS.
        assert_eq!(
            links(&render("[ß]: /u\n[SS]")),
            vec![("SS".to_string(), "/u".to_string())]
        );
    }

    #[test]
    fn apple_attribute_spans_lose_their_markers() {
        let span = render("^[hi](inflect: true)");
        assert_eq!(span.plain(), "hi");
        assert_eq!(span.spans[0].style, Style::default());
        assert_eq!(plain("x^[a](b)y"), "xay");
        assert_eq!(plain("^[a **b**](c)"), "a b");
        assert_eq!(plain("^[a]("), "^[a](");
        // A `[label]` straight after is swallowed, span or no span.
        assert_eq!(plain("^[a](b)[c]d"), "ad");
        assert_eq!(plain("^[a-z][0-9]"), "^[a-z]");
        // `^[` does not stop autolinks; an unclosed one stays text.
        assert_eq!(
            links(&render("^[a www.x.com")),
            vec![("www.x.com".to_string(), "http://www.x.com".to_string())]
        );
    }

    #[test]
    fn carriage_returns_and_nul() {
        assert_eq!(plain("a\r\nb"), "a\nb");
        assert_eq!(plain("a\rb"), "a\nb");
        // The line split leaves the `\r` on a heading line, where the inline
        // parser makes it a second `\n`: Apple draws a blank line here.
        assert_eq!(plain("# Big\r\nx"), "Big\n\nx");
        assert_eq!(plain("\0x"), "\u{FFFD}x");
        // A fence is never inline-parsed, so it keeps its bytes.
        assert_eq!(plain("```\na\0b\n```"), "a\0b");
    }

    #[test]
    fn entities_and_escapes() {
        assert_eq!(
            plain("AT&T &amp; &copy; &#65; &#x1F600; &bogus; &AMP;"),
            "AT&T & © A 😀 &bogus; &"
        );
        assert_eq!(plain("&ngE;"), "\u{2267}\u{338}");
        assert_eq!(plain("&#12345678;"), "\u{FFFD}"); // eight digits still decode
        assert_eq!(plain("&#123456789;"), "&#123456789;");
        assert_eq!(plain("\\*not\\* \\a \\"), "*not* \\a \\");
        // A backslash before a newline is a hard break, and goes.
        let hard = render("a\\\nb");
        assert_eq!(hard.plain(), "a\nb");
        assert!(hard.spans[1].style.line_break);
        // Two trailing spaces are just spaces here.
        assert_eq!(plain("a  \nb"), "a  \nb");
    }

    /// Raw HTML is kept as the characters it is — drawn, never obeyed.
    #[test]
    fn raw_html_is_text() {
        let text = render("a <b>bold?</b> c");
        assert_eq!(text.plain(), "a <b>bold?</b> c");
        let html: Vec<&str> = text
            .spans
            .iter()
            .filter(|span| span.style.html)
            .map(|span| span.text.as_str())
            .collect();
        assert_eq!(html, vec!["<b>", "</b>"]);
        assert!(text.spans.iter().all(|span| !span.style.strong));
        assert_eq!(
            plain("<script>alert(1)</script>"),
            "<script>alert(1)</script>"
        );
        assert_eq!(plain("<!-- note -->"), "<!-- note -->");
        assert_eq!(plain("5 < 6 > 4"), "5 < 6 > 4");
    }

    /// Foundation sets an inline style on entering a span and CLEARS it on
    /// leaving, so a span nested in one of its own kind ends the outer one's
    /// style early.
    #[test]
    fn a_nested_span_of_the_same_kind_switches_the_style_off() {
        let italic = render("*a *b* c*");
        assert_eq!(italic.spans.len(), 2);
        assert_eq!(
            (
                italic.spans[0].text.as_str(),
                italic.spans[0].style.emphasis
            ),
            ("a b", true)
        );
        assert_eq!(
            (
                italic.spans[1].text.as_str(),
                italic.spans[1].style.emphasis
            ),
            (" c", false)
        );
        let bold = render("**a **b** c**");
        assert_eq!(
            (bold.spans[1].text.as_str(), bold.spans[1].style.strong),
            (" c", false)
        );
        // Different kinds are unaffected.
        let mixed = render("*a **b** c*");
        assert!(mixed.spans.iter().all(|span| span.style.emphasis));
    }

    /// cmark-gfm steps over `~` when it asks what flanks a `*` or `_` run.
    #[test]
    fn tildes_are_transparent_to_emphasis() {
        let both = render("**~~a~~**");
        assert!(both.spans[0].style.strong && both.spans[0].style.strikethrough);
        assert_eq!(plain("*(x)*~a"), "*(x)*~a"); // the closer sees `a`, not `~`
        assert_eq!(plain("*(x)*~ a"), "(x)~ a");
        assert_eq!(plain("a~*(b)*"), "a~*(b)*");
        assert_eq!(plain("~*(b)*"), "~(b)");
        assert_eq!(plain("~s~ and ~~s~~ but ~~~s~~~"), "s and s but ~~~s~~~");
    }

    #[test]
    fn link_openers_come_back_after_a_new_bracket() {
        let reopened = render("[a [b](c) [x] d](e)");
        assert_eq!(
            links(&reopened),
            vec![("a b [x] d".to_string(), "e".to_string())]
        );
        // A `^[` re-enables them too; an `![` does not.
        let by_attribute = render("[a [b](c) ^[x] d](e)");
        assert_eq!(
            links(&by_attribute),
            vec![("a b ^[x] d".to_string(), "e".to_string())]
        );
        let by_image = render("[a [b](c) ![x] d](e)");
        assert_eq!(by_image.plain(), "[a b ![x] d](e)");
        let spent = render("[a [b](c) d](e)");
        assert_eq!(spent.plain(), "[a b d](e)");
        assert_eq!(links(&spent), vec![("b".to_string(), "c".to_string())]);
        // `![^` is not an image: cmark-gfm keeps it for footnotes.
        let not_image = render("![^a](x)");
        assert_eq!(not_image.plain(), "!^a");
        assert!(not_image
            .spans
            .iter()
            .all(|span| span.style.image.is_none()));
        // The destination's parentheses need not balance (cmark 0.29).
        assert_eq!(
            links(&render("[a](b(c )")),
            vec![("a".to_string(), "b(c".to_string())]
        );
    }

    /// A destination Foundation's `URL(string:)` refuses is drawn as text.
    #[test]
    fn foundation_refuses_some_destinations() {
        for body in [
            "[a]()",
            "[a](<>)",
            "[a](<http://ex ample.com>)",
            "[a](http://a:b)",
            "[a](1a:b)",
            "[a](http://a|b)",
            "[a](http://é-)",
        ] {
            let text = render(body);
            assert_eq!(text.plain(), "a", "{body}");
            assert!(links(&text).is_empty(), "{body} became a link");
        }
        for body in [
            "[a](x@y:z)",
            "[a](<b c>)",
            "[a](http://a_b.com)",
            "[a](http://[::1]:80)",
            "[a](https://пример.рф)",
            "[a](#top)",
        ] {
            assert_eq!(links(&render(body)).len(), 1, "{body} is a link on Apple");
        }
        assert!(render("![a](<>)").spans[0].style.image.is_none());
    }

    // MARK: The shape the web draws

    #[test]
    fn a_body_lays_out_as_runs_with_fonts() {
        let text = only_text("# Title\n- **milk**\n```\nlet x = 1\n```\nbye");
        let shape: Vec<(&str, Font, bool)> = text
            .spans
            .iter()
            .map(|span| (span.text.as_str(), span.style.font, span.style.strong))
            .collect();
        assert_eq!(
            shape,
            vec![
                ("Title", Font::Heading(1), false),
                ("\n• ", Font::Body, false),
                ("milk", Font::Body, true),
                ("\n", Font::Body, false),
                ("let x = 1", Font::Monospaced, false),
                ("\nbye", Font::Body, false),
            ]
        );
    }

    #[test]
    fn the_table_api() {
        let table = table_in("| a | b |\n| :-: | --: |").expect("a table");
        assert_eq!(table.column_count(), 2);
        assert_eq!(table.alignment(0), ColumnAlignment::Center);
        assert_eq!(table.alignment(1), ColumnAlignment::Trailing);
        assert_eq!(table.alignment(7), ColumnAlignment::Leading);
        assert!(table.rows.is_empty());
        // A body that is only a table is one table block, no text around it.
        assert_eq!(blocks("| a |\n| - |").len(), 1);
        assert!(!blocks("").is_empty());
    }

    // MARK: Apple's behaviour where it is a bug (pinned, not fixed)

    /// MessageMarkdown.swift:425-428 re-parses a cell with EVERY bracket
    /// escaped once a link formed — code spans and URLs included, where a
    /// backslash is not an escape. The backslashes are then drawn.
    #[test]
    fn cell_escaping_shows_backslashes_in_code() {
        let table = table_in("| a |\n| - |\n| [d](https://x) `arr[0]` |").expect("a table");
        assert_eq!(rows(&table), vec![vec!["[d](https://x) arr\\[0\\]"]]);
        let url = table_in("| a |\n| - |\n| [d](https://x) https://y.com/[z] |").expect("a table");
        assert_eq!(
            rows(&url),
            vec![vec!["[d](https://x) https://y.com/\\[z\\]"]]
        );
    }
}
