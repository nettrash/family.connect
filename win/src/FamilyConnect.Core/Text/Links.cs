using System.Text;

namespace FamilyConnect.Core;

/// <summary>One tappable range in a laid-out text run: UTF-8 byte offsets, always on character boundaries.</summary>
/// <param name="Target">What a tap opens. Detected: always http(s), mailto or tel. Declared: gate it with <see cref="Links.IsOpenable"/>.</param>
public readonly record struct LinkSpan(int Start, int End, string Text, string Target);

/// <summary>
/// Tappable data in message bodies — web links, email addresses and phone numbers — found in the text AS DRAWN:
/// <c>fc_text::links</c>, ported line for line and held to it by <c>MarkdownOracleTests</c>.
/// </summary>
/// <remarks>
/// <para>
/// The Rust mirrors the rules Swift wrote itself exactly and approximates NSDataDetector from measurement; see its module
/// header for where it is not Apple. Only http, https, mailto and tel are ever links.
/// </para>
/// <para>Detection runs over ONE laid-out text block, never the raw body: markdown deletes characters.</para>
/// </remarks>
public static partial class Links
{
    /// <summary>Every web link, email address and phone number in <paramref name="text"/>, in order, never overlapping.</summary>
    public static IReadOnlyList<LinkSpan> Detect(string text) => Detect(Encoding.UTF8.GetBytes(text));

    internal static List<LinkSpan> Detect(byte[] text)
    {
        var found = new List<Found>();
        SchemeLinks(text, found);
        TelLinks(text, found);
        EmailLinks(text, found);
        BareLinks(text, found);
        PhoneLinks(text, found);
        // Leftmost first, and the longest of those: an address wins over the host inside it.
        var spans = new List<LinkSpan>();
        var taken = 0;
        foreach (var f in found.OrderBy(f => f.Start).ThenByDescending(f => f.End))
        {
            if (f.Start < taken)
            {
                continue;
            }
            taken = f.End;
            if (f.Target is { } target)
            {
                spans.Add(new LinkSpan(f.Start, f.End, Encoding.UTF8.GetString(text, f.Start, f.End - f.Start), target));
            }
        }
        return spans;
    }

    /// <summary>
    /// The markdown's own links, plus every detected link that overlaps none of them, in order. A SAFETY rule: in
    /// <c>[https://www.paypal.com](https://evil.example)</c> the author's destination stands and the detector's duplicate goes.
    /// </summary>
    public static IReadOnlyList<LinkSpan> Merge(IReadOnlyList<LinkSpan> declared, IReadOnlyList<LinkSpan> detected)
    {
        var output = new List<LinkSpan>(declared);
        foreach (var span in detected)
        {
            if (!output.Any(kept => Overlaps(kept, span)))
            {
                output.Add(span);
            }
        }
        return [.. output.OrderBy(span => span.Start)];
    }

    /// <summary>A markdown destination made openable: <c>https://</c> in front when Foundation sees no scheme, anything else as written.</summary>
    public static string NormalizeDestination(string destination) =>
        destination.StartsWith(':') || SchemeOf(destination) is not null ? destination : $"https://{destination}";

    /// <summary>The first https link — what the preview card describes. Only https.</summary>
    public static LinkSpan? FirstWebLink(IReadOnlyList<LinkSpan> spans)
    {
        foreach (var span in spans)
        {
            if (SchemeOf(span.Target) is { } scheme && scheme.Equals("https", StringComparison.OrdinalIgnoreCase))
            {
                return span;
            }
        }
        return null;
    }

    /// <summary>Whether a target may be opened: http, https, mailto or tel.</summary>
    public static bool IsOpenable(string target) =>
        SchemeOf(target) is { } scheme && (scheme.Equals("http", StringComparison.OrdinalIgnoreCase)
            || scheme.Equals("https", StringComparison.OrdinalIgnoreCase)
            || scheme.Equals("mailto", StringComparison.OrdinalIgnoreCase)
            || scheme.Equals("tel", StringComparison.OrdinalIgnoreCase));

    /// <summary>A match before the overlaps are settled. A null target is a region that is not a link and may not hold one.</summary>
    private readonly record struct Found(int Start, int End, string? Target);

    /// <summary>Whether two ranges share a character; an empty range shares none.</summary>
    private static bool Overlaps(LinkSpan a, LinkSpan b) =>
        a.Start < a.End && b.Start < b.End && a.Start < b.End && b.Start < a.End;

    /// <summary>RFC 3986's scheme before the first colon — Foundation's <c>URL.scheme</c>.</summary>
    private static string? SchemeOf(string target)
    {
        var colon = target.IndexOf(':');
        if (colon < 0)
        {
            return null;
        }
        var scheme = target[..colon];
        var valid = scheme.Length > 0 && RustChar.IsAsciiLetter(scheme[0]);
        foreach (var c in scheme.Skip(1))
        {
            valid &= RustChar.IsAsciiAlphanumeric(c) || c is '+' or '-' or '.';
        }
        return valid ? scheme : null;
    }

    // ---- characters ------------------------------------------------------------------------------------------------

    /// <summary>The char that starts at byte <paramref name="at"/>, or null past the end or off a boundary.</summary>
    private static int? CharAt(ReadOnlySpan<byte> text, int at) =>
        at >= 0 && at < text.Length && !RustChar.IsContinuation(text[at]) ? RustChar.At(text, at) : null;

    /// <summary>The char that ends at byte <paramref name="at"/>, or null at the start or off a boundary.</summary>
    private static int? CharBefore(ReadOnlySpan<byte> text, int at) =>
        at > 0 && at <= text.Length && (at == text.Length || !RustChar.IsContinuation(text[at])) ? RustChar.Before(text, at).Scalar : null;

    private static int Utf8Length(int c) => c < 0x80 ? 1 : c < 0x800 ? 2 : c < 0x10000 ? 3 : 4;

    /// <summary>A letter or decimal digit of a script written with spaces between its words — what glues.</summary>
    private static bool IsWord(int c) =>
        c < 0x80 ? RustChar.IsAsciiAlphanumeric(c) : (RustChar.IsAlphabetic(c) && !IsCjk(c)) || (RustChar.IsNumeric(c) && IsPoison(c));

    /// <summary>What may not come right after a host name or an email address.</summary>
    private static bool ContinuesName(int c) =>
        c >= 0x80 && (IsWord(c) || IsCombining(c) || (IsPoison(c) && !IsSoftPoison(c)) || RustChar.InRanges(LinkTables.HostBad, c));

    /// <summary>Han, kana, Hangul syllables and their iteration marks — the scripts written without spaces.</summary>
    private static bool IsCjk(int c) => c is >= 0x02B9 and <= 0x02BA or >= 0x02C6 and <= 0x02CF or 0x02EC or 0x0374 or 0x2E2F
        or >= 0x3005 and <= 0x3007 or >= 0x3021 and <= 0x3029 or >= 0x3031 and <= 0x3035 or >= 0x3038 and <= 0x303B
        or >= 0x3041 and <= 0x3096 or >= 0x309D and <= 0x309F or >= 0x30A1 and <= 0x30FA or >= 0x30FC and <= 0x30FF
        or >= 0x31F0 and <= 0x31FF or >= 0x3400 and <= 0x4DBF or >= 0x4E00 and <= 0x9FFF
        or 0xA67F or >= 0xA717 and <= 0xA71F or 0xA788 or >= 0xAC00 and <= 0xD7A3
        or >= 0xF900 and <= 0xFAD9 or >= 0xFF66 and <= 0xFF9F
        or 0x16FE3 or >= 0x17000 and <= 0x1B2FB or >= 0x20000 and <= 0x323AF;

    /// <summary><see cref="IsCjk"/> without the Hangul syllables.</summary>
    private static bool IsHanOrKana(int c) => IsCjk(c) && c is not (>= 0xAC00 and <= 0xD7A3);

    /// <summary>The combining-diacritic blocks.</summary>
    private static bool IsCombining(int c) =>
        c is >= 0x0300 and <= 0x036F or >= 0x1AB0 and <= 0x1AFF or >= 0x1DC0 and <= 0x1DFF or >= 0x20D0 and <= 0x20FF or >= 0xFE20 and <= 0xFE2F;

    /// <summary>An ASCII digit, or a fullwidth one.</summary>
    private static bool IsDigit(int c) => c is >= '0' and <= '9' or >= 0xFF10 and <= 0xFF19;

    private static int AsciiDigit(int c) => c is >= 0xFF10 and <= 0xFF19 ? '0' + (c - 0xFF10) : c;

    /// <summary>A character that ends a URL: whitespace, controls and <c>"</c> in ASCII, Apple's list beyond.</summary>
    private static bool IsUrlStop(int c) => c < 0x80 ? c <= ' ' || c == '"' || c == 0x7F : RustChar.InRanges(LinkTables.UrlStop, c);

    /// <summary>A character a URL holds but cannot survive.</summary>
    private static bool IsPoison(int c) => c >= 0x80 && RustChar.InRanges(LinkTables.UrlPoison, c);

    /// <summary>Poison that only ENDS a link when nothing but more of it follows.</summary>
    private static bool IsSoftPoison(int c) =>
        c is 0x05F4 or 0x061C or 0x2024 or 0x2027 or >= 0x202A and <= 0x202E or >= 0x2066 and <= 0x2069 or 0xFE13 or 0xFE52 or 0xFF07;

    /// <summary>Punctuation that ends a sentence rather than a URL.</summary>
    private static bool IsTrailing(int c) => c is '!' or ',' or '.' or ';' or 0x201C or 0x201D or 0x2026 or 0x30FB or 0x200C;

    /// <summary>A character a path may hold and a host may not.</summary>
    private static bool IsHostBad(int c) =>
        c < 0x80 ? c is '%' or '[' or '\\' or ']' or '^' or '`' or '|' : RustChar.InRanges(LinkTables.HostBad, c) || IsPoison(c);

    /// <summary>Whether every char of <c>text[from..to]</c> passes.</summary>
    private static bool AllChars(ReadOnlySpan<byte> text, int from, int to, Func<int, bool> test)
    {
        var at = from;
        while (at < to)
        {
            var c = RustChar.At(text, at);
            if (!test(c))
            {
                return false;
            }
            at += RustChar.Width(text[at]);
        }
        return true;
    }

    private static int IndexOfAny(ReadOnlySpan<byte> text, int from, int to, ReadOnlySpan<byte> values)
    {
        var found = text[from..to].IndexOfAny(values);
        return found < 0 ? -1 : from + found;
    }

    // ---- URLs with a scheme --------------------------------------------------------------------------------------

    private readonly record struct Url(int End, int AuthorityStart, int AuthorityEnd);

    /// <summary>Every <c>scheme://…</c>: a link for http or https, a region nothing else may claim otherwise.</summary>
    private static void SchemeLinks(byte[] text, List<Found> found)
    {
        var search = 0;
        while (text.AsSpan(search).IndexOf("://"u8) is var hit and >= 0)
        {
            var colon = search + hit;
            search = colon + 3;
            // The longest run before `://` that starts with a letter; `.` left out, because Apple reads `x.https://` as https.
            var run = colon;
            while (run > 0 && (RustChar.IsAsciiAlphanumeric(text[run - 1]) || text[run - 1] is (byte)'+' or (byte)'-'))
            {
                run--;
            }
            var letter = -1;
            for (var at = run; at < colon; at++)
            {
                if (RustChar.IsAsciiLetter(text[at]))
                {
                    letter = at;
                    break;
                }
            }
            if (letter < 0)
            {
                continue;
            }
            var start = letter;
            // Glued to a word, it is not a scheme.
            if (CharBefore(text, start) is { } before && IsWord(before))
            {
                continue;
            }
            var body = colon + 3;
            var raw = ScanBody(text, body, stop: false);
            var scheme = Encoding.ASCII.GetString(text, start, colon - start);
            var web = scheme.Equals("http", StringComparison.OrdinalIgnoreCase) || scheme.Equals("https", StringComparison.OrdinalIgnoreCase);
            if (ScanUrl(text, start, body, raw) is { } url)
            {
                found.Add(new Found(start, url.End, web ? UrlTarget(text, start, url) : null));
            }
            else if (raw > body)
            {
                // A URL Apple refuses is refused whole.
                found.Add(new Found(start, raw, null));
            }
        }
    }

    /// <summary>The URL whose body starts at <paramref name="body"/> and whose characters run to <paramref name="raw"/>, or null.</summary>
    private static Url? ScanUrl(byte[] text, int start, int body, int raw)
    {
        var end = CutUnclosed(text, body, raw);
        var authorityEnd = IndexOfAny(text, body, end, "/?#"u8) is var slash and >= 0 ? slash : end;
        if (FirstPoison(text, body, end) is { } at)
        {
            if (at < authorityEnd)
            {
                if (!AllChars(text, at, authorityEnd, IsSoftPoison))
                {
                    return null;
                }
                end = at;
            }
            else
            {
                end = AllChars(text, at, end, IsSoftPoison) ? at : authorityEnd;
            }
        }
        end = TrimTail(text, start, body, end, stop: false);
        var authority = (Start: body, End: Math.Min(authorityEnd, end));
        if (end == body || !ValidAuthority(text.AsSpan(authority.Start, Math.Max(0, authority.End - authority.Start))))
        {
            return null;
        }
        return new Url(end, authority.Start, authority.End);
    }

    /// <summary>Where the first poison in <c>text[from..end]</c> sits. A ZWJ is poison only as the very last character.</summary>
    private static int? FirstPoison(ReadOnlySpan<byte> text, int from, int end)
    {
        var at = from;
        while (at < end)
        {
            var c = RustChar.At(text, at);
            var width = RustChar.Width(text[at]);
            if (IsPoison(c) || (c == 0x200D && at + width == end))
            {
                return at;
            }
            at += width;
        }
        return null;
    }

    /// <summary>
    /// How far the characters a URL can hold go. After a scheme a closer ends it unless any opener has been seen; in the path
    /// of a host with no scheme (<paramref name="stop"/>) every bracket ends it.
    /// </summary>
    private static int ScanBody(ReadOnlySpan<byte> text, int from, bool stop)
    {
        var end = from;
        var opened = false;
        var at = from;
        while (at < text.Length)
        {
            var c = RustChar.At(text, at);
            if (IsUrlStop(c))
            {
                break;
            }
            if (c is '(' or '{' or '<' or ')' or '}' or '>' && stop)
            {
                break;
            }
            if (c is '(' or '{' or '<')
            {
                opened = true;
            }
            else if (c is ')' or '}' or '>' && !opened)
            {
                break;
            }
            at += RustChar.Width(text[at]);
            end = at;
        }
        return end;
    }

    /// <summary>Cut the URL back to an opener that nothing closes after it.</summary>
    private static int CutUnclosed(ReadOnlySpan<byte> text, int from, int end)
    {
        var body = text[from..end];
        var afterClose = body.LastIndexOfAny(")}>"u8) + 1;
        var opener = body[afterClose..].IndexOfAny("({<"u8);
        return opener < 0 ? end : from + afterClose + opener;
    }

    /// <summary>Drop sentence punctuation from the end, and ONE closing <c>'</c> or <c>]</c> when the URL starts right after its opener.</summary>
    private static int TrimTail(ReadOnlySpan<byte> text, int start, int floor, int end, bool stop)
    {
        var opener = CharBefore(text, start);
        var paired = false;
        while (end > floor)
        {
            if (CharBefore(text, end) is not { } last)
            {
                break;
            }
            if (IsTrailing(last) || (stop && last == ']'))
            {
                end -= Utf8Length(last);
            }
            else if (!paired && ((opener == '\'' && last == '\'') || (opener == '[' && last == ']')))
            {
                paired = true;
                end--;
            }
            else
            {
                break;
            }
        }
        return end;
    }

    /// <summary><c>userinfo@host:port</c>: the host holding nothing a host cannot, the port nothing but digits (possibly none).</summary>
    private static bool ValidAuthority(ReadOnlySpan<byte> authority)
    {
        var at = authority.LastIndexOf((byte)'@');
        if (at >= 0 && at + 1 == authority.Length)
        {
            return false;
        }
        var hostPort = at >= 0 ? authority[(at + 1)..] : authority;
        ReadOnlySpan<byte> host;
        ReadOnlySpan<byte> port;
        if (hostPort.StartsWith("["u8))
        {
            var close = hostPort[1..].IndexOf((byte)']');
            if (close < 0)
            {
                return false;
            }
            host = [];
            port = hostPort[(close + 2)..];
        }
        else
        {
            var colon = hostPort.IndexOf((byte)':');
            host = colon < 0 ? hostPort : hostPort[..colon];
            port = colon < 0 ? [] : hostPort[colon..];
        }
        var portOk = port.IsEmpty || (port[0] == (byte)':' && port[1..].IndexOfAnyExceptInRange((byte)'0', (byte)'9') < 0);
        return portOk && AllChars(host, 0, host.Length, c => !IsHostBad(c));
    }

    /// <summary>The URL as a tap opens it: scheme and authority as typed (an <c>@</c> inside the userinfo escaped), the rest encoded.</summary>
    private static string UrlTarget(byte[] text, int start, Url url)
    {
        var output = new List<byte>(url.End - start + 8);
        output.AddRange(text.AsSpan(start, url.AuthorityStart - start));
        var authority = text.AsSpan(url.AuthorityStart, url.AuthorityEnd - url.AuthorityStart);
        var at = authority.LastIndexOf((byte)'@');
        if (at >= 0)
        {
            foreach (var value in authority[..at])
            {
                if (value == (byte)'@')
                {
                    output.AddRange("%40"u8);
                }
                else
                {
                    output.Add(value);
                }
            }
            output.AddRange(authority[at..]);
        }
        else
        {
            output.AddRange(authority);
        }
        EncodeRest(text.AsSpan(url.AuthorityEnd, url.End - url.AuthorityEnd), output);
        return Encoding.UTF8.GetString([.. output]);
    }

    /// <summary>Percent-encode a path, query and fragment the way Apple's URL does.</summary>
    private static void EncodeRest(ReadOnlySpan<byte> rest, List<byte> output)
    {
        var fragment = false;
        var at = 0;
        while (at < rest.Length)
        {
            var c = RustChar.At(rest, at);
            var width = RustChar.Width(rest[at]);
            var escapeStart = at + 2 < rest.Length && RustChar.IsAsciiHexDigit(rest[at + 1]) && RustChar.IsAsciiHexDigit(rest[at + 2]);
            if (c == '%' && escapeStart)
            {
                output.Add((byte)'%');
            }
            else if (c == '#' && !fragment)
            {
                fragment = true;
                output.Add((byte)'#');
            }
            else if (c is '%' or '#' or '[' or '\\' or ']' or '^' or '`' or '{' or '|' or '}' or '<' or '>' || c >= 0x80)
            {
                PushEscaped(rest.Slice(at, width), output);
            }
            else
            {
                output.Add((byte)c);
            }
            at += width;
        }
    }

    private static void PushEscaped(ReadOnlySpan<byte> character, List<byte> output)
    {
        const string Hex = "0123456789ABCDEF";
        foreach (var value in character)
        {
            output.Add((byte)'%');
            output.Add((byte)Hex[value >> 4]);
            output.Add((byte)Hex[value & 0x0F]);
        }
    }

    // ---- host names without a scheme -------------------------------------------------------------------------------

    private static bool BareTld(string label) =>
        LinkTables.BareTlds.Contains(label)
        || LinkTables.UnicodeTlds.Contains(label)
        || label == "BO"
        || label.Equals("com", StringComparison.OrdinalIgnoreCase)
        || label.Equals("edu", StringComparison.OrdinalIgnoreCase)
        || label.Equals("gov", StringComparison.OrdinalIgnoreCase)
        || label.Equals("net", StringComparison.OrdinalIgnoreCase)
        || label.Equals("org", StringComparison.OrdinalIgnoreCase);

    private static bool EmailUnicodeTld(string label) => LinkTables.UnicodeTlds.Contains(label) || LinkTables.EmailUnicodeTlds.Contains(label);

    private static bool AllAsciiLetters(ReadOnlySpan<byte> text) => text.IndexOfAnyExcept(AsciiLetters) < 0;

    private static readonly System.Buffers.SearchValues<byte> AsciiLetters =
        System.Buffers.SearchValues.Create("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"u8);

    private static string Utf8(ReadOnlySpan<byte> text) => Encoding.UTF8.GetString(text);

    /// <summary>Every <c>example.com</c>, <c>www.example.zz/path</c> or <c>пример.рф</c> with no scheme.</summary>
    private static void BareLinks(byte[] text, List<Found> found)
    {
        var at = 0;
        while (CharAt(text, at) is { } c)
        {
            // Only at the start of a word; a mark on the letter before still belongs to that letter's word.
            bool startsWord;
            if (CharBefore(text, at) is not { } before)
            {
                startsWord = true;
            }
            else if (IsCombining(before))
            {
                startsWord = !(CharBefore(text, at - Utf8Length(before)) is { } baseChar && (IsWord(baseChar) || IsCombining(baseChar)));
            }
            else
            {
                startsWord = !IsWord(before);
            }
            if (IsWord(c) && startsWord && BareLinkAt(text, at) is { } f)
            {
                at = f.End;
                found.Add(f);
                continue;
            }
            at += Utf8Length(c);
        }
    }

    private static Found? BareLinkAt(byte[] text, int start)
    {
        // Right after `@` is an address's domain — unless Apple refused the local part.
        if (CharBefore(text, start) == '@' && (LocalPart(text, start - 1) is not { } local || local.Valid))
        {
            return null;
        }
        if (LabelEnd(text, start, LabelKind.Bare) is not { } first)
        {
            return null;
        }
        var www = Ascii.EqualsIgnoreCase(text.AsSpan(start, first - start), "www"u8);
        var labels = HostLabels(text, start, www ? LabelKind.Www : LabelKind.Bare, email: false);
        if (labels.Count < 2)
        {
            return null;
        }
        var last = labels[^1];
        // After `www.` a name may be CJK, but the TLD is not.
        if (www && FirstCjk(text, last.Start, last.End) is { } cjk)
        {
            var cutLength = cjk - last.Start;
            var cut = text.AsSpan(last.Start, cutLength);
            if (cutLength > 0 && (AllAsciiLetters(cut) || LinkTables.UnicodeTlds.Contains(Utf8(cut))))
            {
                last = (last.Start, cjk);
                labels[^1] = last;
            }
        }
        var tld = Utf8(text.AsSpan(last.Start, last.End - last.Start));
        var tldOk = BareTld(tld) || (www && labels.Count >= 3 && AllAsciiLetters(text.AsSpan(last.Start, last.End - last.Start)));
        if (!tldOk)
        {
            return null;
        }
        // A letter of a script Apple will not read in a bare host voids the whole name.
        var checkedCount = LinkTables.UnicodeTlds.Contains(tld) ? labels.Count - 1 : labels.Count;
        for (var index = 0; index < checkedCount; index++)
        {
            if (!AllChars(text, labels[index].Start, labels[index].End, c => !VoidsBareHost(c)))
            {
                return null;
            }
        }
        var hostEnd = last.End;
        if (CharAt(text, hostEnd) is { } next && (next is '@' or '_' || ContinuesName(next)))
        {
            return null;
        }
        var authorityEnd = BarePort(text, hostEnd) ?? hostEnd;
        var end = authorityEnd;
        // A query or fragment counts only after a path.
        if (CharAt(text, end) == '/')
        {
            var pathEnd = ScanBody(text, end, stop: true);
            if (FirstPoison(text, end, pathEnd) is { } poison)
            {
                pathEnd = AllChars(text, poison, pathEnd, IsSoftPoison) ? poison : authorityEnd;
            }
            end = TrimTail(text, start, authorityEnd, pathEnd, stop: true);
        }
        var target = new List<byte>();
        target.AddRange("http://"u8);
        target.AddRange(text.AsSpan(start, authorityEnd - start));
        EncodeRest(text.AsSpan(authorityEnd, end - authorityEnd), target);
        return new Found(start, end, Encoding.UTF8.GetString([.. target]));
    }

    private static int? FirstCjk(ReadOnlySpan<byte> text, int from, int to)
    {
        var at = from;
        while (at < to)
        {
            if (IsCjk(RustChar.At(text, at)))
            {
                return at;
            }
            at += RustChar.Width(text[at]);
        }
        return null;
    }

    /// <summary>A letter Apple refuses in a bare host name.</summary>
    private static bool VoidsBareHost(int c) =>
        c >= 0x80 && ((RustChar.IsAlphabetic(c) && !RustChar.IsLowercase(c) && !RustChar.IsUppercase(c) && !IsCjk(c)) || IsCombining(c));

    /// <summary><c>:8080</c> after a bare host — two to five digits, and then not a letter.</summary>
    private static int? BarePort(ReadOnlySpan<byte> text, int at)
    {
        if (at >= text.Length || text[at] != (byte)':')
        {
            return null;
        }
        var digits = 0;
        while (at + 1 + digits < text.Length && RustChar.IsAsciiDigit(text[at + 1 + digits]))
        {
            digits++;
        }
        var end = at + 1 + digits;
        var glued = CharAt(text, end) is { } c && IsWord(c);
        return digits is >= 2 and <= 5 && !glued ? end : null;
    }

    private enum LabelKind
    {
        /// <summary>A bare host: letters and digits of the spaced scripts.</summary>
        Bare,
        /// <summary>A bare host after <c>www.</c>, where CJK names count too.</summary>
        Www,
        /// <summary>An email domain: any letter or decimal digit, CJK included.</summary>
        Email,
    }

    private static bool IsLabelChar(int c, LabelKind kind) =>
        c < 0x80 ? RustChar.IsAsciiAlphanumeric(c) : kind == LabelKind.Bare ? IsWord(c) || IsCombining(c) : IsWord(c) || IsCjk(c) || IsCombining(c);

    /// <summary>Where the label at <paramref name="at"/> ends: letters and digits, single <c>-</c> or <c>_</c> between, <c>xn--</c> allowed to start.</summary>
    private static int? LabelEnd(ReadOnlySpan<byte> text, int at, LabelKind kind)
    {
        var pos = text[at..].StartsWith("xn--"u8) ? at + 4 : at;
        var end = at;
        while (true)
        {
            var run = pos;
            while (CharAt(text, pos) is { } c && IsLabelChar(c, kind))
            {
                pos += Utf8Length(c);
            }
            if (pos == run)
            {
                break;
            }
            end = pos;
            if (CharAt(text, pos) is '-' or '_' && CharAt(text, pos + 1) is { } after && IsLabelChar(after, kind))
            {
                pos++;
            }
            else
            {
                break;
            }
        }
        return end > at ? end : null;
    }

    /// <summary>The labels of the host name at <paramref name="start"/>. A known internationalised TLD counts as a label in any script.</summary>
    private static List<(int Start, int End)> HostLabels(byte[] text, int start, LabelKind kind, bool email)
    {
        var labels = new List<(int Start, int End)>();
        var at = start;
        while (true)
        {
            string? unicodeTld = null;
            if (at > start)
            {
                foreach (var tld in email ? LinkTables.UnicodeTlds.Concat(LinkTables.EmailUnicodeTlds) : LinkTables.UnicodeTlds)
                {
                    var bytes = Encoding.UTF8.GetBytes(tld);
                    if (text.AsSpan(at).StartsWith(bytes) && !(CharAt(text, at + bytes.Length) is { } c && IsLabelChar(c, kind)))
                    {
                        unicodeTld = tld;
                        break;
                    }
                }
            }
            int end;
            if (unicodeTld is not null)
            {
                end = at + Encoding.UTF8.GetByteCount(unicodeTld);
            }
            else if (LabelEnd(text, at, kind) is { } labelEnd)
            {
                end = labelEnd;
            }
            else
            {
                break;
            }
            labels.Add((at, end));
            if (unicodeTld is not null || CharAt(text, end) != '.')
            {
                break;
            }
            at = end + 1;
        }
        return labels;
    }
}
