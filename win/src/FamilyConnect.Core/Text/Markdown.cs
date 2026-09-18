using System.Text;

namespace FamilyConnect.Core;

/// <summary>
/// Markdown in a chat bubble — <c>fc_text::markdown</c>, ported line for line and held to it by
/// <c>MarkdownOracleTests</c>. The Rust is itself a port of the Apple client's <c>MessageMarkdown.swift</c> and of
/// Foundation's cmark-gfm fork, measured rather than specified, so every odd rule here is Apple's.
/// </summary>
/// <remarks>
/// <para>
/// <b>HEADINGS AND BULLETS ARE RUNS, NOT BLOCKS:</b> only a table splits a body, so link detection, the <c>@ai</c>
/// highlight and member mentions each run over ONE string per text block — the order the Apple client runs them in.
/// </para>
/// <para>
/// <b>BYTES, AS THE ORIGINAL:</b> the parser walks UTF-8 bytes and every offset is a byte offset, so a rule written
/// against a byte in Rust is the same rule here. Unicode questions go to <see cref="RustChar"/>, never to .NET.
/// </para>
/// <para>A RENDERING convention, not a wire format (docs/protocol.md, "A body is plain text on the wire").</para>
/// </remarks>
public static partial class Markdown
{
    private static readonly MarkdownStyle Monospaced = new(Face: MarkdownFace.Monospaced);

    /// <summary>A body as the blocks it lays out in — what a bubble draws. Never empty.</summary>
    public static IReadOnlyList<MarkdownBlock> Blocks(string body) =>
        [.. Parse(Encoding.UTF8.GetBytes(body), tables: true).Select(piece => piece.Table is { } table
            ? (MarkdownBlock)new MarkdownTableBlock(table)
            : new MarkdownTextBlock(piece.Text!.ToText()))];

    /// <summary>A body as ONE string, tables NOT recognised — what the link preview card indexes.</summary>
    public static MarkdownText Render(string body)
    {
        var output = new MarkdownRuns();
        foreach (var piece in Parse(Encoding.UTF8.GetBytes(body), tables: false))
        {
            if (piece.Text is { } text)
            {
                output.Append(text);
            }
        }
        return output.ToText();
    }

    /// <summary>Fences first, then line structure inside each plain segment.</summary>
    private static List<(MarkdownRuns? Text, MarkdownTable? Table)> Parse(byte[] body, bool tables)
    {
        var output = new List<(MarkdownRuns? Text, MarkdownTable? Table)>();
        var current = new MarkdownRuns();
        foreach (var (code, segment) in Segments(body))
        {
            if (code)
            {
                current.Push(segment, Monospaced);
                continue;
            }
            // Newlines BETWEEN pieces, never around a table.
            var needsSeparator = false;
            foreach (var (lines, table) in Pieces(segment, tables))
            {
                if (lines is not null)
                {
                    if (needsSeparator)
                    {
                        current.Push("\n"u8, MarkdownStyle.Plain);
                    }
                    current.Append(lines);
                    needsSeparator = true;
                }
                else
                {
                    if (!current.IsEmpty)
                    {
                        output.Add((current, null));
                        current = new MarkdownRuns();
                    }
                    output.Add((null, table));
                    needsSeparator = false;
                }
            }
        }
        if (!current.IsEmpty)
        {
            output.Add((current, null));
        }
        if (output.Count == 0)
        {
            output.Add((new MarkdownRuns(), null));
        }
        return output;
    }

    /// <summary>One fence-free segment's pieces: ordinary lines inline-parsed together, a heading or bullet alone. Split on U+000A only.</summary>
    private static List<(MarkdownRuns? Lines, MarkdownTable? Table)> Pieces(byte[] segment, bool tables)
    {
        var lines = Split(segment);
        var pieces = new List<(MarkdownRuns? Lines, MarkdownTable? Table)>();
        var plain = new List<byte[]>();

        void FlushPlain()
        {
            if (plain.Count == 0)
            {
                return;
            }
            pieces.Add((Inline(Join(plain)), null));
            plain.Clear();
        }

        var index = 0;
        while (index < lines.Count)
        {
            var line = lines[index];
            if (Heading(line) is { } heading)
            {
                FlushPlain();
                var run = Inline(line[heading.Content..]);
                run.SetFace(heading.Level switch { 1 => MarkdownFace.Heading1, 2 => MarkdownFace.Heading2, _ => MarkdownFace.Heading3 });
                pieces.Add((run, null));
                index++;
                continue;
            }
            if (Bullet(line) is { } bullet)
            {
                FlushPlain();
                // The indent is copied verbatim: nested lists without a parser that can mis-nest one.
                var run = new MarkdownRuns();
                run.Push([.. line.AsSpan(0, bullet.Marker), .. "• "u8], MarkdownStyle.Plain);
                run.Append(Inline(line[bullet.Content..]));
                pieces.Add((run, null));
                index++;
                continue;
            }
            if (tables && Table(index, lines) is { } found)
            {
                FlushPlain();
                pieces.Add((null, found.Table));
                index = found.End;
                continue;
            }
            plain.Add(line);
            index++;
        }
        FlushPlain();
        return pieces;
    }

    private static List<byte[]> Split(byte[] text)
    {
        var lines = new List<byte[]>();
        var start = 0;
        for (var at = 0; at < text.Length; at++)
        {
            if (text[at] == (byte)'\n')
            {
                lines.Add(text[start..at]);
                start = at + 1;
            }
        }
        lines.Add(text[start..]);
        return lines;
    }

    private static byte[] Join(List<byte[]> lines)
    {
        var joined = new List<byte>();
        for (var index = 0; index < lines.Count; index++)
        {
            if (index > 0)
            {
                joined.Add((byte)'\n');
            }
            joined.AddRange(lines[index]);
        }
        return [.. joined];
    }

    // ---- Swift's string semantics ------------------------------------------------------------------------------

    /// <summary>
    /// Swift's <c>Character == "x"</c> at <paramref name="at"/>: the width of the cluster when it is exactly that one
    /// markup character, 0 otherwise. U+1FEF GREEK VARIA is canonically a backtick, the only twin among these.
    /// </summary>
    private static int ClusterIs(ReadOnlySpan<byte> text, int at, char ascii)
    {
        if (at >= text.Length)
        {
            return 0;
        }
        int width;
        if (text[at] == ascii)
        {
            width = 1;
        }
        else if (ascii == '`' && text[at..].StartsWith("`"u8))
        {
            width = 3;
        }
        else
        {
            return 0;
        }
        return RustChar.StandsAlone(text, at, width) ? width : 0;
    }

    /// <summary>
    /// The cluster walks below step scalar by scalar: a scalar that is not a lone markup character never matters, and the
    /// rest of a multi-scalar cluster is never a lone markup character, so the walk makes the same decisions.
    /// </summary>
    private static int Step(ReadOnlySpan<byte> text, int at) => RustChar.Width(text[at]);

    /// <summary>Foundation's <c>CharacterSet.whitespaces</c>: Zs, TAB and U+200B.</summary>
    private static bool IsFoundationWhitespace(int c) =>
        c is '\t' or ' ' or 0xA0 or 0x1680 or 0x202F or 0x205F or 0x3000 or >= 0x2000 and <= 0x200B;

    /// <summary><c>trimmingCharacters(in: .whitespaces)</c>, scalar by scalar: the range left.</summary>
    private static (int Start, int End) TrimFoundationWhitespace(ReadOnlySpan<byte> text)
    {
        var start = 0;
        while (start < text.Length && IsFoundationWhitespace(RustChar.At(text, start)))
        {
            start += Step(text, start);
        }
        var end = text.Length;
        while (end > start)
        {
            var (scalar, at) = RustChar.Before(text, end);
            if (!IsFoundationWhitespace(scalar))
            {
                break;
            }
            end = at;
        }
        return (start, end);
    }

    private static bool IsBlank(ReadOnlySpan<byte> text)
    {
        var (start, end) = TrimFoundationWhitespace(text);
        return start == end;
    }

    // ---- headings, bullets -------------------------------------------------------------------------------------

    /// <summary>One to three <c>#</c>, then a space, then something that is not whitespace. No closing-sequence stripping.</summary>
    private static (int Level, int Content)? Heading(ReadOnlySpan<byte> line)
    {
        var level = 0;
        var at = 0;
        while (at < line.Length)
        {
            if (level < 3 && ClusterIs(line, at, '#') > 0)
            {
                level++;
                at++;
                continue;
            }
            if (level == 0 || ClusterIs(line, at, ' ') == 0)
            {
                return null;
            }
            return IsBlank(line[(at + 1)..]) ? null : (level, at + 1);
        }
        return null;
    }

    /// <summary><c>- </c>, <c>* </c> or <c>+ </c> after any leading spaces or tabs, then content.</summary>
    private static (int Marker, int Content)? Bullet(ReadOnlySpan<byte> line)
    {
        var at = 0;
        int? marker = null;
        while (at < line.Length)
        {
            if (ClusterIs(line, at, ' ') > 0 || ClusterIs(line, at, '\t') > 0)
            {
                at++;
                continue;
            }
            if (ClusterIs(line, at, '-') > 0 || ClusterIs(line, at, '*') > 0 || ClusterIs(line, at, '+') > 0)
            {
                marker = at;
            }
            break;
        }
        if (marker is not { } found || found + 1 >= line.Length || ClusterIs(line, found + 1, ' ') == 0)
        {
            return null;
        }
        return IsBlank(line[(found + 2)..]) ? null : (found, found + 2);
    }

    // ---- tables ------------------------------------------------------------------------------------------------

    /// <summary>A header row, a delimiter row with the SAME number of cells, then rows until the first line that is not one.</summary>
    private static (MarkdownTable Table, int End)? Table(int start, List<byte[]> lines)
    {
        if (start + 1 >= lines.Count || !ContainsUnescapedPipe(lines[start]))
        {
            return null;
        }
        var header = Cells(lines[start]);
        if (header.Count == 0)
        {
            return null;
        }
        if (DelimiterRow(lines[start + 1]) is not { } alignments || alignments.Count != header.Count)
        {
            return null;
        }
        var rows = new List<IReadOnlyList<MarkdownText>>();
        var index = start + 2;
        while (index < lines.Count)
        {
            var line = lines[index];
            if (!ContainsUnescapedPipe(line) || IsBlank(line) || Heading(line) is not null || Bullet(line) is not null)
            {
                break;
            }
            var parsed = Cells(line);
            if (parsed.Count == 0)
            {
                break;
            }
            var row = parsed.Select(Cell).ToList();
            // Ragged rows are padded or cut rather than dropping the table.
            if (row.Count > header.Count)
            {
                row.RemoveRange(header.Count, row.Count - header.Count);
            }
            while (row.Count < header.Count)
            {
                row.Add(new MarkdownRuns().ToText());
            }
            rows.Add(row);
            index++;
        }
        return (new MarkdownTable(alignments, [.. header.Select(Cell)], rows), index);
    }

    /// <summary>The delimiter row's alignments, or null. A PIPE IS REQUIRED: a bare <c>---</c> is a rule.</summary>
    private static List<MarkdownAlignment>? DelimiterRow(byte[] line)
    {
        if (!ContainsUnescapedPipe(line))
        {
            return null;
        }
        var parts = Cells(line);
        if (parts.Count == 0)
        {
            return null;
        }
        var alignments = new List<MarkdownAlignment>();
        foreach (var part in parts)
        {
            ReadOnlySpan<byte> text = part;
            var left = ClusterIs(text, 0, ':') > 0;
            var from = left ? 1 : 0;
            var right = text.Length - 1 >= from && ClusterIs(text, text.Length - 1, ':') > 0;
            var to = right ? text.Length - 1 : text.Length;
            if (to <= from)
            {
                return null;
            }
            for (var at = from; at < to; at++)
            {
                if (ClusterIs(text, at, '-') == 0)
                {
                    return null;
                }
            }
            alignments.Add((left, right) switch
            {
                (true, true) => MarkdownAlignment.Center,
                (_, true) => MarkdownAlignment.Trailing,
                _ => MarkdownAlignment.Leading,
            });
        }
        return alignments;
    }

    /// <summary>A row split on unescaped <c>|</c>, the optional edge pipes dropped, each cell trimmed. <c>\|</c> stays as written.</summary>
    private static List<byte[]> Cells(byte[] line)
    {
        var (start, end) = TrimFoundationWhitespace(line);
        ReadOnlySpan<byte> text = line.AsSpan(start, end - start);
        var parts = new List<List<byte>>();
        var current = new List<byte>();
        var escaped = false;
        var at = 0;
        while (at < text.Length)
        {
            var width = Step(text, at);
            if (escaped)
            {
                current.AddRange(text.Slice(at, width));
                escaped = false;
            }
            else if (ClusterIs(text, at, '\\') > 0)
            {
                current.Add((byte)'\\');
                escaped = true;
            }
            else if (ClusterIs(text, at, '|') > 0)
            {
                parts.Add(current);
                current = [];
            }
            else
            {
                current.AddRange(text.Slice(at, width));
            }
            at += width;
        }
        parts.Add(current);
        if (ClusterIs(text, 0, '|') > 0 && parts.Count > 0)
        {
            parts.RemoveAt(0);
        }
        if (EndsWithUnescapedPipe(text) && parts.Count > 0)
        {
            parts.RemoveAt(parts.Count - 1);
        }
        return [.. parts.Select(part =>
        {
            var bytes = part.ToArray();
            var (from, to) = TrimFoundationWhitespace(bytes);
            return bytes[from..to];
        })];
    }

    /// <summary>One cell: emphasis, code, strikethrough and escapes — but NEVER a link.</summary>
    private static MarkdownText Cell(byte[] source)
    {
        var rendered = Inline(source);
        if (!rendered.HasLink)
        {
            return rendered.ToText();
        }
        var escaped = new List<byte>(source.Length + 8);
        foreach (var value in source)
        {
            if (value is (byte)'[' or (byte)']')
            {
                escaped.Add((byte)'\\');
            }
            escaped.Add(value);
        }
        var literal = Inline([.. escaped]);
        literal.StripLinks();
        return literal.ToText();
    }

    private static bool ContainsUnescapedPipe(ReadOnlySpan<byte> line)
    {
        var escaped = false;
        var at = 0;
        while (at < line.Length)
        {
            var width = Step(line, at);
            if (escaped)
            {
                escaped = false;
            }
            else if (ClusterIs(line, at, '\\') > 0)
            {
                escaped = true;
            }
            else if (ClusterIs(line, at, '|') > 0)
            {
                return true;
            }
            at += width;
        }
        return false;
    }

    /// <summary>A trailing <c>|</c> that is a cell boundary: an even number of backslashes before it.</summary>
    private static bool EndsWithUnescapedPipe(ReadOnlySpan<byte> text)
    {
        if (text.IsEmpty || ClusterIs(text, text.Length - 1, '|') == 0)
        {
            return false;
        }
        var backslashes = 0;
        for (var at = text.Length - 2; at >= 0 && ClusterIs(text, at, '\\') > 0; at--)
        {
            backslashes++;
        }
        return backslashes % 2 == 0;
    }

    // ---- fences ------------------------------------------------------------------------------------------------

    /// <summary>A line whose first three clusters are backticks (U+1FEF counts as one).</summary>
    private static bool HasFencePrefix(ReadOnlySpan<byte> line)
    {
        var at = 0;
        for (var tick = 0; tick < 3; tick++)
        {
            var width = ClusterIs(line, at, '`');
            if (width == 0)
            {
                return false;
            }
            at += width;
        }
        return true;
    }

    /// <summary>Alternating plain and fenced-code segments. An unclosed fence is not a fence; the language tag is dropped.</summary>
    private static List<(bool Code, byte[] Text)> Segments(byte[] body)
    {
        if (Array.IndexOf(body, (byte)'`') < 0 && body.AsSpan().IndexOf("`"u8) < 0)
        {
            return [(false, body)];
        }
        var result = new List<(bool Code, byte[] Text)>();
        var plain = new List<byte[]>();
        var code = new List<byte[]>();
        var inFence = false;
        foreach (var line in Split(body))
        {
            var opensOrCloses = HasFencePrefix(line);
            if (!inFence && opensOrCloses)
            {
                inFence = true;
                // Keep the newline that ENDED the line before the fence.
                plain.Add([]);
                continue;
            }
            if (inFence && opensOrCloses)
            {
                inFence = false;
                result.Add((false, Join(plain)));
                plain.Clear();
                result.Add((true, Join(code)));
                code.Clear();
                plain.Add([]);
                continue;
            }
            (inFence ? code : plain).Add(line);
        }
        if (inFence)
        {
            return [(false, body)];
        }
        result.Add((false, Join(plain)));
        result.RemoveAll(segment => !segment.Code && segment.Text.Length == 0);
        return result;
    }

    // ---- inline: Foundation's AttributedString(markdown:) --------------------------------------------------------

    /// <summary>
    /// Foundation's parser held to inline syntax, preserving whitespace: CR LF and CR become LF and NUL becomes U+FFFD;
    /// reference definitions at the very start come out; cmark-gfm's inline parser and autolink extension; flattened.
    /// </summary>
    private static MarkdownRuns Inline(byte[] text)
    {
        var source = new List<byte>(text.Length);
        for (var at = 0; at < text.Length; at++)
        {
            switch (text[at])
            {
                case (byte)'\r':
                    source.Add((byte)'\n');
                    if (at + 1 < text.Length && text[at + 1] == (byte)'\n')
                    {
                        at++;
                    }
                    break;
                case 0:
                    source.AddRange("�"u8);
                    break;
                default:
                    source.Add(text[at]);
                    break;
            }
        }
        var bytes = source.ToArray();
        var references = new Dictionary<string, (byte[] Url, byte[] Title)>();
        var start = 0;
        while (start < bytes.Length && bytes[start] == (byte)'[')
        {
            if (ParseReferenceDefinition(bytes.AsSpan(start), references) is not { } length)
            {
                break;
            }
            start += length;
        }
        var subject = new Subject(bytes[start..], references);
        subject.Parse();
        subject.AutolinkEmails();
        var output = new MarkdownRuns();
        var style = MarkdownStyle.Plain;
        subject.Render(Subject.Root, ref style, output);
        return output;
    }
}
