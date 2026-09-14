using System.Text;

namespace FamilyConnect.Core;

public static partial class Links
{
    // ---- email addresses -------------------------------------------------------------------------------------------

    private static void EmailLinks(byte[] text, List<Found> found)
    {
        for (var at = Array.IndexOf(text, (byte)'@'); at >= 0; at = Array.IndexOf(text, (byte)'@', at + 1))
        {
            if (LocalPart(text, at) is not { Valid: true } local || EmailDomain(text, at + 1) is not { } domainEnd)
            {
                continue;
            }
            var target = new List<byte>();
            var start = local.Start;
            if (MailtoPrefix(text, local.Start) is { } prefix)
            {
                start = prefix;
                target.AddRange(text.AsSpan(prefix, local.Start - prefix));
            }
            else
            {
                target.AddRange("mailto:"u8);
            }
            var end = domainEnd;
            // `mailto:` brings its query with it.
            if (start < local.Start && CharAt(text, domainEnd) == '?')
            {
                var raw = ScanBody(text, domainEnd, stop: false);
                end = TrimTail(text, start, domainEnd, CutUnclosed(text, domainEnd, raw), stop: false);
            }
            EncodeLocal(text.AsSpan(local.Start, at - local.Start), target);
            target.Add((byte)'@');
            target.AddRange(text.AsSpan(at + 1, domainEnd - at - 1));
            EncodeRest(text.AsSpan(domainEnd, end - domainEnd), target);
            found.Add(new Found(start, end, Encoding.UTF8.GetString([.. target])));
        }
    }

    /// <summary>An address's local part ending at an <c>@</c>, and whether Apple takes it.</summary>
    private readonly record struct Local(int Start, bool Valid);

    private static Local? LocalPart(ReadOnlySpan<byte> text, int at)
    {
        var start = at;
        var spoiled = false;
        while (CharBefore(text, start) is { } c)
        {
            if (SpoilsLocal(c))
            {
                spoiled = true;
            }
            else if (!IsLocalChar(c))
            {
                break;
            }
            start -= Utf8Length(c);
        }
        // What a local part may not start with is dropped rather than refused.
        while (CharAt(text, start) is { } c && start < at && !StartsLocal(c))
        {
            start += Utf8Length(c);
        }
        return start < at ? new Local(start, !spoiled && text[at - 1] != (byte)'.') : null;
    }

    /// <summary>RFC 5322's atext, and beyond ASCII what Apple takes.</summary>
    private static bool IsLocalChar(int c) =>
        c < 0x80
            ? RustChar.IsAsciiAlphanumeric(c) || "!#$%&'*+-./=?^_`{|}~".Contains((char)c)
            : RustChar.IsLowercase(c) || RustChar.IsUppercase(c) || RustChar.IsNumeric(c) || (InEmailScript(c) && !IsUrlStop(c));

    /// <summary>A letter or mark Apple reads as PART of a local part and then refuses the whole address over.</summary>
    private static bool SpoilsLocal(int c) =>
        c >= 0x80 && !IsLocalChar(c) && !IsHanOrKana(c) && (RustChar.IsAlphabetic(c) || IsCombining(c) || c == 0xAD);

    /// <summary>Hebrew, Arabic, Devanagari, Gurmukhi, Gujarati, Thai and Hangul.</summary>
    private static bool InEmailScript(int c) =>
        c is >= 0x0591 and <= 0x05F4 or >= 0x0600 and <= 0x06FF or >= 0x0750 and <= 0x077F or >= 0x08A0 and <= 0x08FF
            or >= 0x0900 and <= 0x097F or >= 0x0A00 and <= 0x0A7F or >= 0x0A80 and <= 0x0AFF or >= 0x0E01 and <= 0x0E5B
            or >= 0x1100 and <= 0x11FF or >= 0x3130 and <= 0x318F or >= 0xA960 and <= 0xA97F or >= 0xAC00 and <= 0xD7FF
            or >= 0xFB1D and <= 0xFB4F or >= 0xFB50 and <= 0xFDFF or >= 0xFE70 and <= 0xFEFF or >= 0xFFA0 and <= 0xFFDC;

    /// <summary>What Apple lets a local part start with: <c>_</c>, <c>#</c> and <c>?</c> stay, every other symbol is dropped.</summary>
    private static bool StartsLocal(int c) => c >= 0x80 || RustChar.IsAsciiAlphanumeric(c) || c is '_' or '#' or '?';

    /// <summary>The start of a <c>mailto:</c> (any case) right before the local part, when not glued to a word.</summary>
    private static int? MailtoPrefix(ReadOnlySpan<byte> text, int local)
    {
        var prefix = local - 7;
        if (prefix < 0 || !Ascii.EqualsIgnoreCase(text[prefix..local], "mailto:"u8))
        {
            return null;
        }
        return CharBefore(text, prefix) is { } before && IsWord(before) ? null : prefix;
    }

    /// <summary>Where the domain starting at <paramref name="from"/> ends, or null when it is not one.</summary>
    private static int? EmailDomain(byte[] text, int from)
    {
        var labels = HostLabels(text, from, LabelKind.Email, email: true);
        if (labels.Count < 2)
        {
            return null;
        }
        var last = labels[^1];
        // A domain may be CJK, but CJK straight after a Latin TLD is the sentence resuming.
        if (FirstCjk(text, last.Start, last.End) is { } cjk)
        {
            var cut = text.AsSpan(last.Start, cjk - last.Start);
            if (cjk > last.Start && (AllAsciiLetters(cut) || EmailUnicodeTld(Utf8(cut))))
            {
                last = (last.Start, cjk);
            }
        }
        var tld = text.AsSpan(last.Start, last.End - last.Start);
        bool Octet(int start, int end)
        {
            var digits = text.AsSpan(start, end - start);
            if (digits.IsEmpty)
            {
                return false;
            }
            var value = 0;
            foreach (var b in digits)
            {
                if (!RustChar.IsAsciiDigit(b))
                {
                    return false;
                }
                value = value * 10 + (b - '0');
                if (value > ushort.MaxValue)
                {
                    return false;
                }
            }
            return value <= 255;
        }
        var dottedQuad = labels.Count == 4 && Octet(labels[0].Start, labels[0].End) && Octet(labels[1].Start, labels[1].End)
            && Octet(labels[2].Start, labels[2].End) && Octet(last.Start, last.End);
        int end;
        if (dottedQuad || AllAsciiLetters(tld) || EmailUnicodeTld(Utf8(tld)))
        {
            end = last.End;
        }
        else
        {
            // `user@example.com-x` is `user@example.com`.
            var letters = tld.IndexOfAnyExcept(AsciiLetters);
            if (letters > 0 && tld[letters] == (byte)'-')
            {
                end = last.Start + letters;
            }
            else
            {
                return null;
            }
        }
        var glued = CharAt(text, end) is { } c && (RustChar.IsAsciiDigit(c) || c == '_' || ContinuesName(c));
        return glued ? null : end;
    }

    /// <summary>A local part as Apple's <c>mailto:</c> URL spells it.</summary>
    private static void EncodeLocal(ReadOnlySpan<byte> local, List<byte> output)
    {
        var at = 0;
        while (at < local.Length)
        {
            var width = RustChar.Width(local[at]);
            if (local[at] is (byte)'%' or (byte)'#' or (byte)'^' or (byte)'`' or (byte)'{' or (byte)'}' or (byte)'|' or >= 0x80)
            {
                PushEscaped(local.Slice(at, width), output);
            }
            else
            {
                output.Add(local[at]);
            }
            at += width;
        }
    }

    // ---- phone numbers ---------------------------------------------------------------------------------------------

    private enum Start
    {
        Here,
        /// <summary>Not here, and not anywhere in the run of digits that follows either.</summary>
        NotThisRun,
        /// <summary>Not here, but maybe at the next group.</summary>
        NotHere,
    }

    private enum Refused
    {
        None,
        /// <summary>Not at this start; a later group may still begin one.</summary>
        Here,
        /// <summary>More digits than any number has: none anywhere in this run.</summary>
        Run,
    }

    private static void PhoneLinks(byte[] text, List<Found> found)
    {
        var at = 0;
        while (CharAt(text, at) is { } c)
        {
            if (c == '+' || c == '(' || IsDigit(c))
            {
                switch (PhoneStart(text, at))
                {
                    case Start.Here:
                        if (PhoneAt(text, at, out var refused) is { } f)
                        {
                            at = f.End;
                            found.Add(f);
                            continue;
                        }
                        if (refused == Refused.Run)
                        {
                            at = RunEnd(text, at);
                            continue;
                        }
                        break;
                    case Start.NotThisRun:
                        at = RunEnd(text, at);
                        continue;
                }
            }
            at += Utf8Length(c);
        }
    }

    /// <summary>Whether a phone number may start at <paramref name="at"/>, judging by what is before it.</summary>
    private static Start PhoneStart(ReadOnlySpan<byte> text, int at)
    {
        if (CharBefore(text, at) is not { } before)
        {
            return Start.Here;
        }
        var beforeThat = CharBefore(text, at - Utf8Length(before));
        if (IsDigit(before))
        {
            return Start.NotHere;
        }
        if (RustChar.IsAsciiLetter(before) || before is '/' or '*' or '_' or '+')
        {
            return Start.NotThisRun;
        }
        if (before is '.' or ',' or ':' && beforeThat is { } digit && RustChar.IsAsciiDigit(digit))
        {
            return Start.NotThisRun;
        }
        return before is '$' or 0x20AC or 0xA3 or 0xA5 or 0x20BD or 0x20B4 or 0x20B9 or 0x20A9 or 0xA2 ? Start.NotHere : Start.Here;
    }

    /// <summary>The end of the run of digits and separators at <paramref name="at"/> — always past the character asked about.</summary>
    private static int RunEnd(ReadOnlySpan<byte> text, int at)
    {
        var end = at;
        while (CharAt(text, end) is { } c)
        {
            var spacedDigit = c is ' ' or 0xA0 && CharAt(text, end + Utf8Length(c)) is { } next && IsDigit(next);
            if (!(IsDigit(c) || c is '-' or '.' or '/' or '(' or ')' or '+' || spacedDigit))
            {
                break;
            }
            end += Utf8Length(c);
        }
        return end > at ? end : at + (CharAt(text, at) is { } here ? Utf8Length(here) : 1);
    }

    /// <summary>The phone number starting at <paramref name="at"/>, if one does.</summary>
    private static Found? PhoneAt(byte[] text, int at, out Refused refused)
    {
        int end;
        string phone;
        if (VanityAt(text, at) is { } vanity)
        {
            (end, phone) = (vanity, Utf8(text.AsSpan(at, vanity - at)));
        }
        else if (CalledAt(text, at) is { } called)
        {
            (end, phone) = (called, Utf8(text.AsSpan(at, called - at)));
        }
        else
        {
            if (NumberAt(text, at, out refused) is not { } number)
            {
                return null;
            }
            if (ExtensionAt(text, number.End) is { } extension)
            {
                (end, phone) = (extension.End, $"{number.Dialled};{extension.Digits}");
            }
            else
            {
                // What follows must not carry on the number.
                if (CharAt(text, number.End) is { } c && (IsWord(c) || IsCombining(c)
                    || c is '-' or '%' or 0xB0 or '_' or '#' or '+' or '=' or '&' or '@' or '$' or '~' or '^' or '`' or '\\'))
                {
                    refused = Refused.Here;
                    return null;
                }
                (end, phone) = (number.End, number.Dialled);
            }
        }
        if (TelUrl(phone) is not { } target)
        {
            refused = Refused.Here;
            return null;
        }
        refused = Refused.None;
        return new Found(at, end, target);
    }

    /// <summary><c>tel:+15551234567</c> written out: linked whole, to exactly what was written. Lowercase <c>tel:</c> only.</summary>
    private static void TelLinks(byte[] text, List<Found> found)
    {
        var search = 0;
        while (text.AsSpan(search).IndexOf("tel:"u8) is var hit and >= 0)
        {
            var at = search + hit;
            search = at + 4;
            if (CharBefore(text, at) is { } before && IsWord(before))
            {
                continue;
            }
            var body = at + 4;
            var raw = ScanBody(text, body, stop: false);
            var end = TrimTail(text, at, body, CutUnclosed(text, body, raw), stop: false);
            var written = text.AsSpan(body, end - body);
            // A literal with no digit dials nothing.
            if (written.IndexOfAnyInRange((byte)'0', (byte)'9') < 0)
            {
                continue;
            }
            if (PhoneAt(text, body, out _) is { } phone && phone.End > end)
            {
                continue;
            }
            var target = new List<byte>();
            target.AddRange("tel:"u8);
            EncodeRest(written, target);
            found.Add(new Found(at, end, Encoding.UTF8.GetString([.. target])));
        }
    }

    private readonly record struct Number(int End, string Dialled);

    /// <summary>One group of digits: <c>555</c>, or <c>(555)</c>.</summary>
    private readonly record struct Group(int Start, int End, int Digits, bool Paren)
    {
        /// <summary>Its first digit, in ASCII — past the bracket of a <c>(555)</c>.</summary>
        public int? Lead(ReadOnlySpan<byte> text) => CharAt(text, Start + (Paren ? 1 : 0)) is { } c ? AsciiDigit(c) : null;
    }

    private static Group? GroupAt(ReadOnlySpan<byte> text, int at)
    {
        var paren = CharAt(text, at) == '(';
        var pos = paren ? at + 1 : at;
        var digits = 0;
        while (CharAt(text, pos) is { } c && IsDigit(c))
        {
            digits++;
            pos += Utf8Length(c);
        }
        if (digits == 0)
        {
            return null;
        }
        if (paren)
        {
            if (digits > 5 || CharAt(text, pos) != ')')
            {
                return null;
            }
            pos++;
        }
        return new Group(at, pos, digits, paren);
    }

    /// <summary>The separators Apple reads between the groups of a number.</summary>
    private static bool IsSeparator(int c) => c is ' ' or '-' or '.' or '/' or 0xA0 or 0x2013 or 0x2014;

    /// <summary>The number starting at <paramref name="at"/>: <c>+CC …</c> international, or a national one in groups.</summary>
    private static Number? NumberAt(byte[] text, int at, out Refused refused)
    {
        refused = Refused.Here;
        var plus = CharAt(text, at) == '+';
        var pos = plus ? at + 1 : at;
        var groups = new List<Group>();
        var separators = new List<int>();
        while (GroupAt(text, pos) is { } group)
        {
            pos = group.End;
            var leadingZero = groups.Count == 0 && !group.Paren && group.Lead(text) == '0';
            groups.Add(group);
            // Reading stops long after any number would have.
            if (groups.Count > 12 || groups.Sum(g => g.Digits) > 30)
            {
                break;
            }
            var next = CharAt(text, pos);
            if (next == '/' && !(groups.Count == 1 && leadingZero))
            {
                break;
            }
            if (next is ' ' or 0xA0 && separators.Any(s => s is '-' or '.' or 0x2013 or 0x2014))
            {
                break;
            }
            if (next is { } separator && IsSeparator(separator) && GroupAt(text, pos + Utf8Length(separator)) is not null)
            {
                separators.Add(separator);
                pos += Utf8Length(separator);
            }
            else if (next is { } c && ((group.Paren && IsDigit(c)) || (c == '(' && GroupAt(text, pos) is { Paren: true })))
            {
                // `(555)1234567`, `8(800)5553535`: brackets need no separator.
                separators.Add(0);
            }
            else
            {
                break;
            }
        }
        if (groups.Count == 0)
        {
            return null;
        }
        if (groups.Sum(g => g.Digits) > 15)
        {
            // A whole unseparated number, then more; anything else this long is no number at all.
            var whole = groups.GetRange(0, 1);
            if (plus || whole[0].Digits < 9 || !NationalShape(text, whole, [], whole[0].Digits))
            {
                refused = Refused.Run;
                return null;
            }
            pos = groups[0].End;
            groups.RemoveRange(1, groups.Count - 1);
            separators.Clear();
        }
        var first = groups[0];
        // `+44 (0)20 …`: the trunk zero is not dialled from abroad.
        var trunk = plus && groups.Count > 1 && groups[1].Paren && text.AsSpan(groups[1].Start, groups[1].End - groups[1].Start).SequenceEqual("(0)"u8);
        var total = groups.Sum(g => g.Digits) - (trunk ? 1 : 0);
        var valid = plus
            ? !first.Paren && (groups.Count == 1 ? total is >= 8 and <= 13 : first.Digits is >= 1 and <= 3 && total is >= 8 and <= 15)
            : NationalShape(text, groups, separators, total);
        if (!valid)
        {
            return null;
        }
        var dialled = new StringBuilder();
        var walk = at;
        while (walk < pos)
        {
            var c = RustChar.At(text, walk);
            var width = RustChar.Width(text[walk]);
            if (!(trunk && walk >= groups[1].Start && walk < groups[1].End))
            {
                dialled.Append(new Rune(IsDigit(c) ? AsciiDigit(c) : c).ToString());
            }
            walk += width;
        }
        refused = Refused.None;
        return new Number(pos, dialled.ToString());
    }

    /// <summary>Whether groups with no country code have a phone number's shape.</summary>
    private static bool NationalShape(ReadOnlySpan<byte> text, List<Group> groups, List<int> separators, int total)
    {
        if (groups.Count == 1)
        {
            // Unseparated, Apple takes 9, 10, 11, 13 and 14 digits — not 12.
            return !groups[0].Paren && total is 9 or 10 or 11 or 13 or 14;
        }
        if (total is < 7 or > 15)
        {
            return false;
        }
        for (var i = 0; i < groups.Count; i++)
        {
            var g = groups[i];
            if (g.Digits == 1 && !(i == 0 && !g.Paren && g.Lead(text) is '1' or '8' && total == 11))
            {
                return false;
            }
        }
        var lengths = groups.Select(g => g.Digits).ToArray();
        bool Years(ReadOnlySpan<byte> within, Group g) =>
            g.Digits == 4 && g.Start + 2 <= within.Length && (within.Slice(g.Start, 2).SequenceEqual("19"u8) || within.Slice(g.Start, 2).SequenceEqual("20"u8));
        if (lengths is [4, 2, 2] or [2, 2, 4] or [5, 4])
        {
            return false;
        }
        if (lengths is [4, 4] && Years(text, groups[0]) && Years(text, groups[1]))
        {
            return false;
        }
        if (total == 7)
        {
            return lengths switch
            {
                [3, 4] => true,
                [3, 2, 2] or [2, 2, 3] => separators.All(s => s == '-'),
                _ => false,
            };
        }
        return true;
    }

    /// <summary><c> x89</c>, <c> ext. 89</c>, <c>, ext 89</c>, <c> (ext 89)</c>, <c>;89</c>, <c>;ext=89</c> after a number.</summary>
    private static (int End, string Digits)? ExtensionAt(ReadOnlySpan<byte> text, int at)
    {
        if (ExtensionKey(text[at..]) is not { } key)
        {
            return null;
        }
        var pos = at + key.Skip;
        var digits = new StringBuilder();
        while (CharAt(text, pos) is { } c && IsDigit(c) && digits.Length != 7)
        {
            digits.Append((char)AsciiDigit(c));
            pos += Utf8Length(c);
        }
        if (digits.Length < key.MinDigits)
        {
            return null;
        }
        if (key.Closing)
        {
            if (CharAt(text, pos) != ')')
            {
                return null;
            }
            pos++;
        }
        if (CharAt(text, pos) is { } after && (IsWord(after) || RustChar.IsAsciiDigit(after)))
        {
            return null;
        }
        return (pos, digits.ToString());
    }

    /// <summary>The words that introduce an extension: how many bytes to skip, the fewest digits, and whether a <c>)</c> closes it.</summary>
    private static (int Skip, int MinDigits, bool Closing)? ExtensionKey(ReadOnlySpan<byte> rest)
    {
        if (rest.StartsWith(" ("u8))
        {
            var after = rest[2..];
            if (KeyLength(after) is not { } length)
            {
                return null;
            }
            var space = length < after.Length && after[length] == (byte)' ' ? 1 : 0;
            return (2 + length + space, 1, true);
        }
        var i = 0;
        if (rest.StartsWith(", "u8))
        {
            i = 2;
        }
        else if (rest.StartsWith(" "u8))
        {
            i = 1;
        }
        if (i < rest.Length && rest[i] == (byte)';')
        {
            return (i + 1 + (rest[(i + 1)..].StartsWith("ext="u8) ? 4 : 0), 1, false);
        }
        if (KeyLength(rest[i..]) is not { } key)
        {
            return null;
        }
        var x = key == 1;
        // A bare `x` needs two digits, and a comma only ever comes before a spelled-out one.
        if (x && i == 2)
        {
            return null;
        }
        i += key;
        if (i < rest.Length && rest[i] == (byte)'.')
        {
            i++;
        }
        if (i < rest.Length && rest[i] == (byte)':')
        {
            i++;
        }
        if (i < rest.Length && rest[i] == (byte)' ')
        {
            i++;
        }
        return (i, x ? 2 : 1, false);
    }

    private static int? KeyLength(ReadOnlySpan<byte> rest)
    {
        foreach (var key in new[] { "extension"u8.ToArray(), "ext"u8.ToArray(), "x"u8.ToArray() })
        {
            if (rest.Length >= key.Length && Ascii.EqualsIgnoreCase(rest[..key.Length], key))
            {
                return key.Length;
            }
        }
        return null;
    }

    private static bool IsUpperOrDigit(int value) => value is >= 'A' and <= 'Z' or >= '0' and <= '9';

    private static int ByteAt(ReadOnlySpan<byte> text, int at) => at < text.Length ? text[at] : -1;

    /// <summary><c>1-800-GOT-JUNK</c>: a North American number with its last seven to nine characters spelled in capitals.</summary>
    private static int? VanityAt(ReadOnlySpan<byte> text, int at)
    {
        var pos = at;
        var country = false;
        if (text[pos..].StartsWith("+1"u8) || text[pos..].StartsWith("1"u8))
        {
            country = true;
            pos += text[pos] == (byte)'+' ? 2 : 1;
            if (CharAt(text, pos) is ' ' or '-' or '.')
            {
                pos++;
            }
        }
        var paren = CharAt(text, pos) == '(';
        var areaStart = pos + (paren ? 1 : 0);
        if (areaStart + 3 > text.Length)
        {
            return null;
        }
        var area = text.Slice(areaStart, 3);
        if (area.IndexOfAnyExceptInRange((byte)'0', (byte)'9') >= 0)
        {
            return null;
        }
        pos = areaStart + 3;
        if (paren)
        {
            if (CharAt(text, pos) != ')')
            {
                return null;
            }
            pos++;
        }
        if (!country && Encoding.ASCII.GetString(area) is not ("800" or "888" or "877" or "866" or "855" or "844" or "833"))
        {
            return null;
        }
        if (CharAt(text, pos) is ' ' or '-' or '.')
        {
            pos++;
        }
        if (CharAt(text, pos) is not (>= 'A' and <= 'Z'))
        {
            return null;
        }
        var count = 0;
        var end = pos;
        while (true)
        {
            var chunk = 0;
            while (pos + chunk < text.Length && IsUpperOrDigit(text[pos + chunk]))
            {
                chunk++;
            }
            if (chunk == 0 || count + chunk > 9)
            {
                break;
            }
            count += chunk;
            pos += chunk;
            end = pos;
            if (ByteAt(text, pos) is '-' or ' ' && ByteAt(text, pos + 1) is var following && IsUpperOrDigit(following))
            {
                pos++;
            }
            else
            {
                break;
            }
        }
        // Carrying on past nine is not a number.
        var glued = (CharAt(text, end) is { } c && RustChar.IsAlphanumeric(c))
            || (ByteAt(text, end) == '-' && IsUpperOrDigit(ByteAt(text, end + 1)));
        return count is >= 7 and <= 9 && !glued ? end : null;
    }

    /// <summary><c>call 911</c>: three to six digits right after "call" or "dial" (any case, then spaces or a colon).</summary>
    private static int? CalledAt(ReadOnlySpan<byte> text, int at)
    {
        var beforeLength = at;
        while (beforeLength > 0 && text[beforeLength - 1] is (byte)' ' or (byte)':')
        {
            beforeLength--;
        }
        var word = beforeLength - 4;
        if (word < 0)
        {
            return null;
        }
        var spoken = text[word..beforeLength];
        if (beforeLength == at
            || !(Ascii.EqualsIgnoreCase(spoken, "call"u8) || Ascii.EqualsIgnoreCase(spoken, "dial"u8))
            || (CharBefore(text, word) is { } before && IsWord(before)))
        {
            return null;
        }
        var digits = 0;
        while (at + digits < text.Length && RustChar.IsAsciiDigit(text[at + digits]))
        {
            digits++;
        }
        var end = at + digits;
        var glued = CharAt(text, end) is { } c
            && (IsWord(c) || c == '-' || (IsSeparator(c) && CharAt(text, end + Utf8Length(c)) is { } next && IsDigit(next)));
        return digits is >= 3 and <= 6 && !glued ? end : null;
    }

    /// <summary>Swift's <c>telURL(for:)</c>: the extension rides in RFC 3966 <c>;ext=</c> form, vanity letters become keypad digits.</summary>
    private static string? TelUrl(string phone)
    {
        var (main, extension) = SplitExtension(phone);
        if (main is null)
        {
            return null;
        }
        var number = DialableDigits(main);
        if (number.Length == 0)
        {
            return null;
        }
        var tel = $"tel:{number}";
        if (extension is not null)
        {
            var digits = string.Concat(extension.Where(c => c is >= '0' and <= '9'));
            if (digits.Length > 0)
            {
                tel += $";ext={digits}";
            }
        }
        return tel;
    }

    /// <summary>Swift's <c>split(separator: ";", maxSplits: 1, omittingEmptySubsequences: true)</c>.</summary>
    private static (string? Main, string? Extension) SplitExtension(string phone)
    {
        phone = phone.TrimStart(';');
        if (phone.Length == 0)
        {
            return (null, null);
        }
        var split = phone.IndexOf(';');
        if (split < 0)
        {
            return (phone, null);
        }
        var rest = phone[(split + 1)..];
        return (phone[..split], rest.Length > 0 ? rest : null);
    }

    /// <summary>Swift's <c>dialableDigits</c>: digits, <c>+</c> and <c>*</c> pass, vanity letters become keypad digits, <c>#</c> is encoded.</summary>
    private static string DialableDigits(string text)
    {
        var output = new StringBuilder();
        foreach (var rune in text.EnumerateRunes())
        {
            foreach (var c in RustChar.Uppercase(rune.Value))
            {
                switch (c)
                {
                    case >= '0' and <= '9' or '+' or '*':
                        output.Append((char)c);
                        break;
                    case '#':
                        output.Append("%23");
                        break;
                    case 'A' or 'B' or 'C':
                        output.Append('2');
                        break;
                    case 'D' or 'E' or 'F':
                        output.Append('3');
                        break;
                    case 'G' or 'H' or 'I':
                        output.Append('4');
                        break;
                    case 'J' or 'K' or 'L':
                        output.Append('5');
                        break;
                    case 'M' or 'N' or 'O':
                        output.Append('6');
                        break;
                    case 'P' or 'Q' or 'R' or 'S':
                        output.Append('7');
                        break;
                    case 'T' or 'U' or 'V':
                        output.Append('8');
                        break;
                    case 'W' or 'X' or 'Y' or 'Z':
                        output.Append('9');
                        break;
                }
            }
        }
        return output.ToString();
    }
}
