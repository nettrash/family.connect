using System.Text;

namespace FamilyConnect.Core;

public static partial class Markdown
{
    private static MarkdownLink MakeLink(byte[] url, byte[] title) =>
        new(Encoding.UTF8.GetString(url), title.Length == 0 ? null : Encoding.UTF8.GetString(title));

    /// <summary>The bytes that end a run of plain text: cmark's <c>SPECIAL_CHARS</c>, the extensions' <c>~ : w</c>, and Apple's <c>^</c>.</summary>
    private static bool IsSpecial(byte value) =>
        value is (byte)'\n' or (byte)'\\' or (byte)'`' or (byte)'&' or (byte)'_' or (byte)'*' or (byte)'[' or (byte)']'
            or (byte)'<' or (byte)'!' or (byte)'~' or (byte)':' or (byte)'w' or (byte)'^';

    /// <summary>CommonMark's left- and right-flanking tests over cmark's own predicates.</summary>
    private static (bool Left, bool Right) Flanking(int before, int after) =>
        (!IsCmarkSpace(after) && (!IsCmarkPunctuation(after) || IsCmarkSpace(before) || IsCmarkPunctuation(before)),
         !IsCmarkSpace(before) && (!IsCmarkPunctuation(before) || IsCmarkSpace(after) || IsCmarkPunctuation(after)));

    /// <summary>cmark's <c>S_normalize_code</c>: newlines become spaces, then ONE space off each end when both have one and not all are spaces.</summary>
    private static byte[] NormalizeCode(ReadOnlySpan<byte> raw)
    {
        var text = raw.ToArray();
        for (var at = 0; at < text.Length; at++)
        {
            if (text[at] == (byte)'\n')
            {
                text[at] = (byte)' ';
            }
        }
        var allSpaces = Array.TrueForAll(text, value => value == (byte)' ');
        return !allSpaces && text[0] == (byte)' ' && text[^1] == (byte)' ' ? text[1..^1] : text;
    }

    // ---- cmark's Unicode predicates --------------------------------------------------------------------------------

    /// <summary>cmark's <c>cmark_utf8proc_is_space</c>: TAB, LF, FF, CR, SPACE and Zs.</summary>
    private static bool IsCmarkSpace(int c) =>
        c is '\t' or '\n' or 0x0C or '\r' or ' ' or 0xA0 or 0x1680 or 0x202F or 0x205F or 0x3000 or >= 0x2000 and <= 0x200A;

    /// <summary>cmark's <c>cmark_utf8proc_is_punctuation</c>: ASCII punctuation, and the frozen P* table above it.</summary>
    private static bool IsCmarkPunctuation(int c) =>
        c < 0x80 ? RustChar.IsAsciiPunctuation(c) : RustChar.InRanges(MarkdownTables.CmarkPunctuation, c);

    /// <summary>cmark's ASCII <c>cmark_isspace</c>: space, TAB, LF, VT, FF, CR.</summary>
    private static bool IsCmarkSpaceByte(byte value) => value is (byte)' ' or (byte)'\t' or (byte)'\n' or 0x0B or 0x0C or (byte)'\r';

    private static ReadOnlySpan<byte> TrimCmarkSpace(ReadOnlySpan<byte> text)
    {
        var start = 0;
        while (start < text.Length && IsCmarkSpaceByte(text[start]))
        {
            start++;
        }
        var end = text.Length;
        while (end > start && IsCmarkSpaceByte(text[end - 1]))
        {
            end--;
        }
        return text[start..end];
    }

    private static int At(ReadOnlySpan<byte> bytes, int at) => at >= 0 && at < bytes.Length ? bytes[at] : -1;

    // ---- cmark's scanners --------------------------------------------------------------------------------------

    /// <summary><c>[ \t\v\f\r\n]*</c></summary>
    private static int ScanSpacechars(ReadOnlySpan<byte> bytes, int pos)
    {
        var count = 0;
        while (pos + count < bytes.Length && IsCmarkSpaceByte(bytes[pos + count]))
        {
            count++;
        }
        return count;
    }

    /// <summary>
    /// cmark 0.29's <c>manual_scan_link_url</c>: <c>&lt;…&gt;</c>, or a run to a space or an unopened <c>)</c> (more than 32
    /// open refused). Both forms need a character AFTER them. The length consumed and the destination's range.
    /// </summary>
    private static (int Length, int Start, int End)? ScanLinkUrl(ReadOnlySpan<byte> bytes, int offset)
    {
        var length = bytes.Length;
        var i = offset;
        if (i < length && bytes[i] == (byte)'<')
        {
            i++;
            while (true)
            {
                if (i >= length)
                {
                    return null;
                }
                var c = bytes[i];
                if (c == (byte)'>')
                {
                    i++;
                    break;
                }
                if (c == (byte)'\\')
                {
                    i += 2;
                }
                else if (c is (byte)'\n' or (byte)'<')
                {
                    return null;
                }
                else
                {
                    i++;
                }
            }
            if (i >= length)
            {
                return null;
            }
            return (i - offset, offset + 1, i - 1);
        }
        var depth = 0;
        while (i < length)
        {
            var c = bytes[i];
            if (c == (byte)'\\' && i + 1 < length && RustChar.IsAsciiPunctuation(bytes[i + 1]))
            {
                i += 2;
            }
            else if (c == (byte)'(')
            {
                depth++;
                i++;
                if (depth > 32)
                {
                    return null;
                }
            }
            else if (c == (byte)')')
            {
                if (depth == 0)
                {
                    break;
                }
                depth--;
                i++;
            }
            else if (IsCmarkSpaceByte(c))
            {
                if (i == offset)
                {
                    return null;
                }
                break;
            }
            else
            {
                i++;
            }
        }
        if (i >= length)
        {
            return null;
        }
        return (i - offset, offset, i);
    }

    /// <summary>Apple's <c>^[text](…)</c> contents: balanced parentheses (≤ 32 deep), escapes, anything else. The length to the closing <c>)</c>.</summary>
    private static int? ScanAttributes(ReadOnlySpan<byte> bytes, int offset)
    {
        var i = offset;
        var depth = 0;
        while (i < bytes.Length)
        {
            var c = bytes[i];
            if (c == (byte)'\\' && i + 1 < bytes.Length && RustChar.IsAsciiPunctuation(bytes[i + 1]))
            {
                i += 2;
            }
            else if (c == (byte)'(')
            {
                depth++;
                i++;
                if (depth > 32)
                {
                    return null;
                }
            }
            else if (c == (byte)')')
            {
                if (depth == 0)
                {
                    break;
                }
                depth--;
                i++;
            }
            else
            {
                i++;
            }
        }
        return i >= bytes.Length || depth != 0 ? null : i - offset;
    }

    /// <summary>cmark's <c>scan_link_title</c>: <c>"…"</c>, <c>'…'</c> or <c>(…)</c>, the longest match. The length, or 0.</summary>
    private static int ScanLinkTitle(ReadOnlySpan<byte> bytes, int pos)
    {
        if (pos >= bytes.Length)
        {
            return 0;
        }
        var open = bytes[pos];
        byte close;
        switch (open)
        {
            case (byte)'"':
                close = (byte)'"';
                break;
            case (byte)'\'':
                close = (byte)'\'';
                break;
            case (byte)'(':
                close = (byte)')';
                break;
            default:
                return 0;
        }
        var candidate = 0;
        for (var i = pos + 1; i < bytes.Length; i++)
        {
            var c = bytes[i];
            var escaped = bytes[i - 1] == (byte)'\\' && i - 1 > pos;
            if (c == close)
            {
                if (!escaped)
                {
                    return i + 1 - pos;
                }
                candidate = i + 1 - pos;
            }
            else if (open == (byte)'(' && c == (byte)'(' && !escaped)
            {
                return candidate;
            }
        }
        return candidate;
    }

    /// <summary>cmark's <c>link_label</c>: <c>[</c>, up to 1000 bytes without an unescaped bracket, <c>]</c>.</summary>
    private static (int Start, int End, int After)? LinkLabel(ReadOnlySpan<byte> bytes, int pos)
    {
        if (At(bytes, pos) != '[')
        {
            return null;
        }
        var i = pos + 1;
        var length = 0;
        while (i < bytes.Length && bytes[i] != (byte)'[' && bytes[i] != (byte)']')
        {
            if (bytes[i] == (byte)'\\')
            {
                i++;
                length++;
                if (i < bytes.Length && RustChar.IsAsciiPunctuation(bytes[i]))
                {
                    i++;
                    length++;
                }
            }
            else
            {
                i++;
                length++;
            }
            if (length > 1000)
            {
                return null;
            }
        }
        return At(bytes, i) == ']' ? (pos + 1, i, i + 1) : null;
    }

    /// <summary>cmark's <c>normalize_reference</c>: full case folding, trimmed, whitespace runs collapsed. Null when nothing is left.</summary>
    private static string? NormalizeLabel(ReadOnlySpan<byte> label)
    {
        var folded = new List<byte>(label.Length);
        var at = 0;
        while (at < label.Length)
        {
            var c = RustChar.At(label, at);
            at += RustChar.Width(label[at]);
            if (CaseFoldExtra(c) is { } extra)
            {
                folded.AddRange(Encoding.UTF8.GetBytes(extra));
            }
            else
            {
                RustChar.AppendLowercase(folded, c);
            }
        }
        var trimmed = TrimCmarkSpace([.. folded]);
        var output = new List<byte>(trimmed.Length);
        var lastWasSpace = false;
        foreach (var value in trimmed)
        {
            if (IsCmarkSpaceByte(value))
            {
                if (!lastWasSpace)
                {
                    output.Add((byte)' ');
                    lastWasSpace = true;
                }
            }
            else
            {
                output.Add(value);
                lastWasSpace = false;
            }
        }
        return output.Count == 0 ? null : Encoding.UTF8.GetString([.. output]);
    }

    private static string? CaseFoldExtra(int c)
    {
        var table = MarkdownTables.CaseFoldExtra;
        var (low, high) = (0, table.Length - 1);
        while (low <= high)
        {
            var middle = (low + high) >>> 1;
            if (table[middle].Scalar < c)
            {
                low = middle + 1;
            }
            else if (table[middle].Scalar > c)
            {
                high = middle - 1;
            }
            else
            {
                return table[middle].Folded;
            }
        }
        return null;
    }

    /// <summary>cmark's <c>cmark_parse_reference_inline</c>: one definition at the start of <paramref name="input"/>, recorded. How much it used.</summary>
    private static int? ParseReferenceDefinition(ReadOnlySpan<byte> input, Dictionary<string, (byte[] Url, byte[] Title)> references)
    {
        if (LinkLabel(input, 0) is not { } found)
        {
            return null;
        }
        var label = TrimCmarkSpace(input[found.Start..found.End]);
        var pos = found.After;
        if (label.IsEmpty || At(input, pos) != ':')
        {
            return null;
        }
        pos = SkipSpacesAndNewline(input, pos + 1);
        if (ScanLinkUrl(input, pos) is not { } url)
        {
            return null;
        }
        pos += url.Length;
        var destination = CleanUrl(input[url.Start..url.End]);

        var beforeTitle = pos;
        pos = SkipSpacesAndNewline(input, pos);
        var titleLength = pos == beforeTitle ? 0 : ScanLinkTitle(input, pos);
        byte[] title;
        if (titleLength > 0)
        {
            title = CleanTitle(input.Slice(pos, titleLength));
            pos += titleLength;
        }
        else
        {
            pos = beforeTitle;
            title = [];
        }

        // The rest of the line must be blank — or, failing that with a title, the line must end right after the destination.
        pos = SkipSpaces(input, pos);
        if (SkipLineEnd(input, pos) is { } end)
        {
            pos = end;
        }
        else if (titleLength > 0 && SkipLineEnd(input, SkipSpaces(input, beforeTitle)) is { } early)
        {
            pos = early;
        }
        else
        {
            return null;
        }
        if (NormalizeLabel(label) is { } key)
        {
            references.TryAdd(key, (destination, title));
        }
        return pos;
    }

    private static int SkipSpaces(ReadOnlySpan<byte> bytes, int pos)
    {
        while (At(bytes, pos) is ' ' or '\t')
        {
            pos++;
        }
        return pos;
    }

    /// <summary>cmark's <c>skip_line_end</c>: past one line ending, or at the end of input.</summary>
    private static int? SkipLineEnd(ReadOnlySpan<byte> bytes, int pos) => At(bytes, pos) switch
    {
        '\n' => pos + 1,
        -1 => pos,
        _ => null,
    };

    /// <summary>cmark's <c>spnl</c>: spaces, at most one newline, spaces.</summary>
    private static int SkipSpacesAndNewline(ReadOnlySpan<byte> bytes, int pos)
    {
        pos = SkipSpaces(bytes, pos);
        return At(bytes, pos) == '\n' ? SkipSpaces(bytes, pos + 1) : pos;
    }

    /// <summary>cmark's <c>cmark_clean_url</c>: trimmed, entities decoded, then escapes removed.</summary>
    private static byte[] CleanUrl(ReadOnlySpan<byte> raw)
    {
        var trimmed = TrimCmarkSpace(raw);
        return trimmed.IsEmpty ? [] : UnescapeBackslashes(UnescapeHtml(trimmed));
    }

    /// <summary>cmark's <c>cmark_clean_title</c>: the quotes or parentheses off, then entities, then escapes.</summary>
    private static byte[] CleanTitle(ReadOnlySpan<byte> raw)
    {
        if (raw.IsEmpty)
        {
            return [];
        }
        var (first, last) = (raw[0], raw[^1]);
        var inner = raw.Length >= 2 && ((first == (byte)'\'' && last == (byte)'\'') || (first == (byte)'(' && last == (byte)')') || (first == (byte)'"' && last == (byte)'"'))
            ? raw[1..^1]
            : raw;
        return UnescapeBackslashes(UnescapeHtml(inner));
    }

    /// <summary>cmark's <c>cmark_strbuf_unescape</c>: a backslash before ASCII punctuation goes.</summary>
    private static byte[] UnescapeBackslashes(ReadOnlySpan<byte> text)
    {
        var output = new List<byte>(text.Length);
        var i = 0;
        var copied = 0;
        while (i < text.Length)
        {
            if (text[i] == (byte)'\\' && i + 1 < text.Length && RustChar.IsAsciiPunctuation(text[i + 1]))
            {
                output.AddRange(text[copied..i]);
                copied = i + 1;
                i += 2;
            }
            else
            {
                i++;
            }
        }
        output.AddRange(text[copied..]);
        return [.. output];
    }

    /// <summary>cmark's <c>houdini_unescape_html</c>: every <c>&amp;entity;</c> decoded, any other <c>&amp;</c> kept.</summary>
    private static byte[] UnescapeHtml(ReadOnlySpan<byte> text)
    {
        var output = new List<byte>(text.Length);
        var i = 0;
        var copied = 0;
        while (i < text.Length)
        {
            if (text[i] == (byte)'&' && UnescapeEntity(text[(i + 1)..]) is { } entity)
            {
                output.AddRange(text[copied..i]);
                output.AddRange(entity.Decoded);
                i += 1 + entity.Length;
                copied = i;
                continue;
            }
            i++;
        }
        output.AddRange(text[copied..]);
        return [.. output];
    }

    /// <summary>
    /// cmark's <c>houdini_unescape_ent</c>, given the bytes after an <c>&amp;</c>: a numeric reference of one to EIGHT digits,
    /// or a named one from the HTML5 table. The text and the bytes used.
    /// </summary>
    private static (byte[] Decoded, int Length)? UnescapeEntity(ReadOnlySpan<byte> src)
    {
        if (src.Length >= 3 && src[0] == (byte)'#')
        {
            int i;
            if (RustChar.IsAsciiDigit(src[1]))
            {
                i = 1;
            }
            else if (src[1] is (byte)'x' or (byte)'X')
            {
                i = 2;
            }
            else
            {
                return null;
            }
            var digitsStart = i;
            var hex = digitsStart == 2;
            long codepoint = 0;
            while (i < src.Length && (hex ? RustChar.IsAsciiHexDigit(src[i]) : RustChar.IsAsciiDigit(src[i])))
            {
                var digit = src[i] switch
                {
                    >= (byte)'0' and <= (byte)'9' => src[i] - '0',
                    >= (byte)'a' and <= (byte)'f' => src[i] - 'a' + 10,
                    _ => src[i] - 'A' + 10,
                };
                codepoint = codepoint * (hex ? 16 : 10) + digit;
                if (codepoint >= 0x110000)
                {
                    codepoint = 0x110000;
                }
                i++;
            }
            var digits = i - digitsStart;
            if (digits is >= 1 and <= 8 && At(src, i) == ';')
            {
                var scalar = codepoint == 0 || codepoint is >= 0xD800 and < 0xE000 || codepoint >= 0x110000 ? 0xFFFD : (int)codepoint;
                var decoded = new List<byte>(4);
                RustChar.AppendScalar(decoded, scalar);
                return ([.. decoded], i + 1);
            }
            return null;
        }
        var size = Math.Min(src.Length, 32);
        for (var i = 2; i < size; i++)
        {
            if (src[i] == (byte)' ')
            {
                break;
            }
            if (src[i] == (byte)';')
            {
                return Entity(src[..i]) is { } value ? (Encoding.UTF8.GetBytes(value), i + 1) : null;
            }
        }
        return null;
    }

    /// <summary>The named entity, by binary search over the table's bytes.</summary>
    private static string? Entity(ReadOnlySpan<byte> name)
    {
        var table = MarkdownTables.HtmlEntities;
        var (low, high) = (0, table.Length - 1);
        while (low <= high)
        {
            var middle = (low + high) >>> 1;
            var order = CompareAscii(table[middle].Name, name);
            if (order < 0)
            {
                low = middle + 1;
            }
            else if (order > 0)
            {
                high = middle - 1;
            }
            else
            {
                return table[middle].Value;
            }
        }
        return null;
    }

    /// <summary>An ASCII key against arbitrary bytes, byte order.</summary>
    private static int CompareAscii(string key, ReadOnlySpan<byte> name)
    {
        var shared = Math.Min(key.Length, name.Length);
        for (var i = 0; i < shared; i++)
        {
            if (key[i] != name[i])
            {
                return key[i] < name[i] ? -1 : 1;
            }
        }
        return key.Length.CompareTo(name.Length);
    }

    /// <summary><c>&lt;scheme:…&gt;</c>: a 2–32 character scheme, <c>:</c>, no controls, spaces or angle brackets, <c>&gt;</c>. The length after the <c>&lt;</c>.</summary>
    private static int? ScanAutolinkUri(ReadOnlySpan<byte> bytes, int pos)
    {
        if (pos > bytes.Length)
        {
            return null;
        }
        var rest = bytes[pos..];
        if (rest.IsEmpty || !RustChar.IsAsciiLetter(rest[0]))
        {
            return null;
        }
        var scheme = 1;
        while (scheme < rest.Length && (RustChar.IsAsciiAlphanumeric(rest[scheme]) || rest[scheme] is (byte)'.' or (byte)'+' or (byte)'-'))
        {
            scheme++;
        }
        if (scheme is < 2 or > 32 || At(rest, scheme) != ':')
        {
            return null;
        }
        var i = scheme + 1;
        while (i < rest.Length && rest[i] > 0x20 && rest[i] != (byte)'<' && rest[i] != (byte)'>')
        {
            i++;
        }
        return At(rest, i) == '>' ? i + 1 : null;
    }

    /// <summary><c>&lt;user@host&gt;</c> in the HTML5 email grammar.</summary>
    private static int? ScanAutolinkEmail(ReadOnlySpan<byte> bytes, int pos)
    {
        if (pos > bytes.Length)
        {
            return null;
        }
        var rest = bytes[pos..];
        var local = 0;
        while (local < rest.Length && (RustChar.IsAsciiAlphanumeric(rest[local]) || ".!#$%&'*+/=?^_`{|}~-"u8.Contains(rest[local])))
        {
            local++;
        }
        if (local == 0 || At(rest, local) != '@')
        {
            return null;
        }
        var i = local + 1;
        while (true)
        {
            var label = 0;
            while (i + label < rest.Length && (RustChar.IsAsciiAlphanumeric(rest[i + label]) || rest[i + label] == (byte)'-'))
            {
                label++;
            }
            if (label == 0 || label > 63 || rest[i] == (byte)'-' || rest[i + label - 1] == (byte)'-')
            {
                return null;
            }
            i += label;
            switch (At(rest, i))
            {
                case '.':
                    i++;
                    break;
                case '>':
                    return i + 1;
                default:
                    return null;
            }
        }
    }

    /// <summary>Raw HTML after a <c>&lt;</c>: a tag, a comment, a processing instruction, a declaration or CDATA. The length after the <c>&lt;</c>.</summary>
    private static int? ScanHtmlTag(ReadOnlySpan<byte> bytes, int pos)
    {
        if (pos >= bytes.Length)
        {
            return null;
        }
        var rest = bytes[pos..];
        var first = rest[0];
        return first switch
        {
            (byte)'!' when rest.StartsWith("!--"u8) => ScanHtmlComment(rest),
            (byte)'!' when rest.StartsWith("![CDATA["u8) => ScanHtmlCdata(rest),
            (byte)'!' => ScanHtmlDeclaration(rest),
            (byte)'?' => ScanHtmlProcessing(rest),
            (byte)'/' => ScanCloseTag(rest),
            _ when RustChar.IsAsciiLetter(first) => ScanOpenTag(rest),
            _ => null,
        };
    }

    private static int? ScanHtmlComment(ReadOnlySpan<byte> rest)
    {
        if (rest.StartsWith("!-->"u8))
        {
            return 4;
        }
        if (rest.StartsWith("!--->"u8))
        {
            return 5;
        }
        var i = 3;
        while (i < rest.Length)
        {
            if (rest[i..].StartsWith("-->"u8))
            {
                return i + 3;
            }
            if (rest[i] == (byte)'-')
            {
                if (At(rest, i + 1) == '-')
                {
                    var next = At(rest, i + 2);
                    if (next < 0 || next == '>')
                    {
                        return null;
                    }
                    i += 3;
                }
                else
                {
                    if (i + 1 >= rest.Length)
                    {
                        return null;
                    }
                    i += 2;
                }
            }
            else
            {
                i++;
            }
        }
        return null;
    }

    private static int? ScanHtmlCdata(ReadOnlySpan<byte> rest)
    {
        var i = 8;
        while (i < rest.Length)
        {
            if (rest[i..].StartsWith("]]>"u8))
            {
                return i + 3;
            }
            if (rest[i] == (byte)']')
            {
                if (At(rest, i + 1) == ']')
                {
                    var next = At(rest, i + 2);
                    if (next < 0 || next == '>')
                    {
                        return null;
                    }
                    i += 3;
                }
                else
                {
                    if (i + 1 >= rest.Length)
                    {
                        return null;
                    }
                    i += 2;
                }
            }
            else
            {
                i++;
            }
        }
        return null;
    }

    private static int? ScanHtmlDeclaration(ReadOnlySpan<byte> rest)
    {
        var name = 0;
        while (1 + name < rest.Length && rest[1 + name] is >= (byte)'A' and <= (byte)'Z')
        {
            name++;
        }
        if (name == 0)
        {
            return null;
        }
        var i = 1 + name;
        var spaces = ScanSpacechars(rest, i);
        if (spaces == 0)
        {
            return null;
        }
        i += spaces;
        while (i < rest.Length && rest[i] != (byte)'>')
        {
            i++;
        }
        return i < rest.Length ? i + 1 : null;
    }

    private static int? ScanHtmlProcessing(ReadOnlySpan<byte> rest)
    {
        var i = 1;
        while (i < rest.Length)
        {
            if (rest[i..].StartsWith("?>"u8))
            {
                return i + 2;
            }
            if (rest[i] == (byte)'?')
            {
                var next = At(rest, i + 1);
                if (next < 0 || next == '>')
                {
                    return null;
                }
                i += 2;
            }
            else
            {
                i++;
            }
        }
        return null;
    }

    private static int? ScanCloseTag(ReadOnlySpan<byte> rest)
    {
        if (ScanTagName(rest, 1) is not { } name)
        {
            return null;
        }
        var i = 1 + name;
        i += ScanSpacechars(rest, i);
        return At(rest, i) == '>' ? i + 1 : null;
    }

    /// <summary><c>tagname attribute* spacechar* /? &gt;</c></summary>
    private static int? ScanOpenTag(ReadOnlySpan<byte> rest)
    {
        if (ScanTagName(rest, 0) is not { } i)
        {
            return null;
        }
        while (true)
        {
            var spaces = ScanSpacechars(rest, i);
            if (spaces == 0)
            {
                break;
            }
            var nameStart = i + spaces;
            if (ScanAttributeName(rest, nameStart) is not { } name)
            {
                break;
            }
            var end = nameStart + name;
            var beforeEquals = end + ScanSpacechars(rest, end);
            if (At(rest, beforeEquals) == '=')
            {
                var valueStart = beforeEquals + 1 + ScanSpacechars(rest, beforeEquals + 1);
                if (ScanAttributeValue(rest, valueStart) is { } value)
                {
                    end = valueStart + value;
                }
            }
            i = end;
        }
        i += ScanSpacechars(rest, i);
        if (At(rest, i) == '/')
        {
            i++;
        }
        return At(rest, i) == '>' ? i + 1 : null;
    }

    /// <summary><c>[A-Za-z][A-Za-z0-9-]*</c></summary>
    private static int? ScanTagName(ReadOnlySpan<byte> rest, int pos)
    {
        if (pos >= rest.Length || !RustChar.IsAsciiLetter(rest[pos]))
        {
            return null;
        }
        var length = 1;
        while (pos + length < rest.Length && (RustChar.IsAsciiAlphanumeric(rest[pos + length]) || rest[pos + length] == (byte)'-'))
        {
            length++;
        }
        return length;
    }

    /// <summary><c>[a-zA-Z_:][a-zA-Z0-9:._-]*</c></summary>
    private static int? ScanAttributeName(ReadOnlySpan<byte> rest, int pos)
    {
        if (pos >= rest.Length || !(RustChar.IsAsciiLetter(rest[pos]) || rest[pos] is (byte)'_' or (byte)':'))
        {
            return null;
        }
        var length = 1;
        while (pos + length < rest.Length && (RustChar.IsAsciiAlphanumeric(rest[pos + length]) || rest[pos + length] is (byte)':' or (byte)'.' or (byte)'_' or (byte)'-'))
        {
            length++;
        }
        return length;
    }

    /// <summary>An unquoted, single-quoted or double-quoted attribute value.</summary>
    private static int? ScanAttributeValue(ReadOnlySpan<byte> rest, int pos)
    {
        if (pos >= rest.Length)
        {
            return null;
        }
        var first = rest[pos];
        if (first is (byte)'"' or (byte)'\'')
        {
            var close = rest[(pos + 1)..].IndexOf(first);
            return close < 0 ? null : close + 2;
        }
        var length = 0;
        while (pos + length < rest.Length && !IsCmarkSpaceByte(rest[pos + length]) && !"\"'=<>`"u8.Contains(rest[pos + length]))
        {
            length++;
        }
        return length > 0 ? length : null;
    }

    // ---- the GFM autolink extension's helpers (autolink.c) -----------------------------------------------------------

    /// <summary>A character a host may start with: not cmark whitespace nor punctuation. Inside a multi-byte character is not one.</summary>
    private static bool IsValidHostchar(ReadOnlySpan<byte> data, int pos) =>
        RustChar.DecodeAt(data, pos) is { } c && !IsCmarkSpace(c) && !IsCmarkPunctuation(c);

    /// <summary><c>sd_autolink_issafe</c>: <c>http://</c>, <c>https://</c> or <c>ftp://</c> (any case), then a host character.</summary>
    private static bool AutolinkIsSafe(ReadOnlySpan<byte> link) =>
        SafeScheme(link, "http://"u8) || SafeScheme(link, "https://"u8) || SafeScheme(link, "ftp://"u8);

    private static bool SafeScheme(ReadOnlySpan<byte> link, ReadOnlySpan<byte> scheme) =>
        link.Length > scheme.Length && Ascii.EqualsIgnoreCase(link[..scheme.Length], scheme) && IsValidHostchar(link, scheme.Length);

    /// <summary><c>check_domain</c>: how much of <paramref name="data"/> is a domain. The first and last byte are never examined.</summary>
    private static int CheckDomain(ReadOnlySpan<byte> data, bool allowShort)
    {
        var size = data.Length;
        var (dots, underscoresBefore, underscores) = (0, 0, 0);
        var i = 1;
        while (i + 1 < size)
        {
            if (data[i] == (byte)'\\' && i + 2 < size)
            {
                i++;
            }
            if (data[i] == (byte)'_')
            {
                underscores++;
            }
            else if (data[i] == (byte)'.')
            {
                underscoresBefore = underscores;
                underscores = 0;
                dots++;
            }
            else if (!IsValidHostchar(data, i) && data[i] != (byte)'-')
            {
                break;
            }
            i++;
        }
        if (underscoresBefore > 0 || underscores > 0)
        {
            return 0;
        }
        return allowShort || dots > 0 ? i : 0;
    }

    /// <summary><c>autolink_delim</c>: trailing punctuation is not the URL's, a <c>)</c> only when it closes one inside, <c>&amp;name;</c> at the end an entity.</summary>
    private static int AutolinkDelim(ReadOnlySpan<byte> data, int linkEnd)
    {
        var angle = data[..linkEnd].IndexOf((byte)'<');
        if (angle >= 0)
        {
            linkEnd = angle;
        }
        while (linkEnd > 0)
        {
            var last = data[linkEnd - 1];
            if ("?!.,:*_~'\""u8.Contains(last))
            {
                linkEnd--;
            }
            else if (last == (byte)';')
            {
                var newEnd = Math.Max(linkEnd - 2, 0);
                while (newEnd > 0 && RustChar.IsAsciiLetter(data[newEnd]))
                {
                    newEnd--;
                }
                if (linkEnd >= 2 && newEnd < linkEnd - 2 && data[newEnd] == (byte)'&')
                {
                    linkEnd = newEnd;
                }
                else
                {
                    linkEnd--;
                }
            }
            else if (last == (byte)')')
            {
                var opening = System.MemoryExtensions.Count(data[..linkEnd], (byte)'(');
                var closing = System.MemoryExtensions.Count(data[..linkEnd], (byte)')');
                if (closing <= opening)
                {
                    break;
                }
                linkEnd--;
            }
            else
            {
                break;
            }
        }
        return linkEnd;
    }

    /// <summary><c>validate_protocol</c>: <c>mailto:</c> or <c>xmpp:</c> right before the address, at the start or after a non-alphanumeric.</summary>
    private static bool ValidateProtocol(ReadOnlySpan<byte> protocol, ReadOnlySpan<byte> data, int at, int rewind, int maxRewind)
    {
        var length = protocol.Length;
        if (length > maxRewind - rewind)
        {
            return false;
        }
        var start = at - rewind - length;
        if (!data.Slice(start, length).SequenceEqual(protocol))
        {
            return false;
        }
        return length == maxRewind - rewind || !RustChar.IsAsciiAlphanumeric(data[start - 1]);
    }

    // ---- Foundation's URL(string:) ---------------------------------------------------------------------------

    /// <summary>Whether Foundation's <c>URL(string:)</c> would take this destination — one it refuses is plain text, never a link.</summary>
    private static bool FoundationAcceptsUrl(byte[] url)
    {
        if (url.Length == 0)
        {
            return false;
        }
        ReadOnlySpan<byte> rest = url;
        var index = rest.IndexOfAny(":/?#[]@"u8);
        if (index >= 0 && url[index] == (byte)':')
        {
            var scheme = rest[..index];
            var validScheme = !scheme.IsEmpty && RustChar.IsAsciiLetter(scheme[0]);
            foreach (var value in scheme)
            {
                validScheme &= RustChar.IsAsciiAlphanumeric(value) || value is (byte)'+' or (byte)'-' or (byte)'.';
            }
            if (!scheme.IsEmpty && !validScheme)
            {
                return false;
            }
            rest = rest[(index + 1)..];
        }
        if (!rest.StartsWith("//"u8))
        {
            return true;
        }
        var after = rest[2..];
        var authorityEnd = after.IndexOfAny("/?#"u8);
        var authority = authorityEnd < 0 ? after : after[..authorityEnd];
        var at = authority.LastIndexOf((byte)'@');
        var hostAndPort = at < 0 ? authority : authority[(at + 1)..];

        // A host typed as `[…]` is a literal; one that only becomes `[…]` once IDNA drops what it ignores is one too.
        byte[] literalSource;
        if (hostAndPort.StartsWith("["u8))
        {
            literalSource = hostAndPort.ToArray();
        }
        else
        {
            var kept = new List<byte>(hostAndPort.Length);
            var walk = 0;
            while (walk < hostAndPort.Length)
            {
                var width = RustChar.Width(hostAndPort[walk]);
                if (!RustChar.InRanges(MarkdownTables.IdnaIgnored, RustChar.At(hostAndPort, walk)))
                {
                    kept.AddRange(hostAndPort.Slice(walk, width));
                }
                walk += width;
            }
            literalSource = [.. kept];
        }
        if (literalSource.Length > 0 && literalSource[0] == (byte)'[')
        {
            ReadOnlySpan<byte> literal = literalSource.AsSpan(1);
            var close = literal.IndexOf((byte)']');
            if (close < 0)
            {
                return false;
            }
            foreach (var value in literal[..close])
            {
                if (!(RustChar.IsAsciiAlphanumeric(value) || "-._~!$&'()*+,;=:%"u8.Contains(value)))
                {
                    return false;
                }
            }
            var afterLiteral = literal[(close + 1)..];
            return afterLiteral.IsEmpty || (afterLiteral[0] == (byte)':' && AllDigits(afterLiteral[1..]));
        }
        var colon = hostAndPort.IndexOf((byte)':');
        var host = colon < 0 ? hostAndPort : hostAndPort[..colon];
        var port = colon < 0 ? ReadOnlySpan<byte>.Empty : hostAndPort[(colon + 1)..];
        if (!AllDigits(port))
        {
            return false;
        }
        var nonAscii = false;
        var offset = 0;
        while (offset < host.Length)
        {
            var value = host[offset];
            if (RustChar.IsAsciiAlphanumeric(value) || "-._~!$&'()*+,;=".Contains((char)value) && value < 0x80)
            {
                offset++;
                continue;
            }
            if (value == (byte)'%')
            {
                if (!(RustChar.IsAsciiHexDigit(At(host, offset + 1)) && RustChar.IsAsciiHexDigit(At(host, offset + 2))))
                {
                    return false;
                }
                offset += 3;
                continue;
            }
            var c = RustChar.At(host, offset);
            if (c < 0x80 || !IdnaAllows(c))
            {
                return false;
            }
            nonAscii = true;
            offset += RustChar.Width(value);
        }
        return !nonAscii || IdnaLabelsValid(host);
    }

    private static bool AllDigits(ReadOnlySpan<byte> text)
    {
        foreach (var value in text)
        {
            if (!RustChar.IsAsciiDigit(value))
            {
                return false;
            }
        }
        return true;
    }

    /// <summary>
    /// UTS #46 over a host with anything non-ASCII in it, label by label: only the last label may be empty, one IDNA empties
    /// only while something non-ASCII is left elsewhere; CheckHyphens; no leading combining mark.
    /// </summary>
    private static bool IdnaLabelsValid(ReadOnlySpan<byte> host)
    {
        var labels = new List<List<int>>([[]]);
        var keepsNonAscii = false;
        var walk = 0;
        while (walk < host.Length)
        {
            var c = RustChar.At(host, walk);
            walk += RustChar.Width(host[walk]);
            if (c is '.' or 0x3002 or 0xFF0E or 0xFF61)
            {
                labels.Add([]);
                continue;
            }
            labels[^1].Add(c);
            keepsNonAscii |= c >= 0x80 && !RustChar.InRanges(MarkdownTables.IdnaIgnored, c);
        }
        var last = labels.Count - 1;
        for (var index = 0; index < labels.Count; index++)
        {
            var label = labels[index];
            var mapped = label.Where(c => !RustChar.InRanges(MarkdownTables.IdnaIgnored, c)).ToList();
            if (mapped.Count == 0)
            {
                if (index != last || last == 0 || (label.Count > 0 && !keepsNonAscii))
                {
                    return false;
                }
                continue;
            }
            if (mapped[0] == '-' || mapped[^1] == '-')
            {
                return false;
            }
            if (mapped.Count >= 4 && mapped[2] == '-' && mapped[3] == '-')
            {
                return false;
            }
            // A combining mark is exactly a character that would join an `a` in front of it into one cluster.
            if (RustChar.JoinsTheCharacterBefore(mapped[0]))
            {
                return false;
            }
        }
        return true;
    }

    /// <summary>A non-ASCII character Foundation's IDNA lets into a host, as far as one character decides it.</summary>
    private static bool IdnaAllows(int c) =>
        !(RustChar.IsWhitespace(c) || RustChar.IsControl(c) || RustChar.InRanges(MarkdownTables.IdnaRefused, c));
}
