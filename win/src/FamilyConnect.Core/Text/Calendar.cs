using System.Globalization;
using System.Text;

namespace FamilyConnect.Core;

/// <summary>
/// One event as an <c>.ics</c> file — ported from <c>fc_text::calendar</c> and pinned to it by the
/// oracle.
/// </summary>
/// <remarks>
/// <para>
/// <b>NOTHING HERE IS ON THE WIRE.</b> The protocol carries no calendar and no <c>.ics</c>: a
/// client that can put an event in the system calendar builds it locally out of the title, the
/// times and the place, and the server neither generates one nor knows whether anybody kept it.
/// What is copied is those three things — not who is coming (the family's business, not the
/// calendar's) and not the backdrop.
/// </para>
/// <para>
/// The three rules that are easy to get wrong, and why each is here rather than in a view:
/// <b>CRLF line endings</b>, because RFC 5545 §3.1 says so and a file with bare newlines is
/// refused outright by some calendars and silently truncated by others; <b>escaping</b>, because a
/// backslash, a semicolon and a comma are the separators of the format itself and a title with a
/// comma in it would otherwise become two properties; and <b>folding at 75 OCTETS</b>, counted in
/// bytes and never split inside a UTF-8 sequence — a family writing in Cyrillic hits the limit at
/// half the characters an English one does, and a fold inside a code point produces a file no
/// calendar can read.
/// </para>
/// </remarks>
public static class Calendar
{
    /// <summary>A content line's octet budget, from RFC 5545 §3.1.</summary>
    public const int FoldAt = 75;

    /// <summary>
    /// An instant in the shape iCalendar wants. The wire carries an instant and every client
    /// draws it in the reader's own zone, but a calendar file wants UTC and says so with the Z.
    /// </summary>
    /// <remarks>
    /// The pattern has no <c>:</c> and no <c>/</c> in it, so unlike the wire's own spelling
    /// (<see cref="Store.Times.Rfc3339"/>) it carries no culture-sensitive placeholder at all —
    /// the invariant culture here is belt beside braces, and a mutation run says so.
    /// </remarks>
    public static string Stamp(DateTimeOffset when) =>
        when.UtcDateTime.ToString("yyyyMMdd'T'HHmmss'Z'", CultureInfo.InvariantCulture);

    /// <summary>
    /// An <c>.ics</c> file holding one event. <paramref name="uid"/> must be STABLE for the event,
    /// so a calendar that already has it updates rather than duplicating.
    /// </summary>
    public static string OneEvent(
        string uid,
        string title,
        string startsAt,
        string? endsAt,
        string? place,
        string stampedAt)
    {
        string[] lines =
        [
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            // Who wrote the file, in the shape RFC 5545 asks for. No product registry, no
            // version: a calendar shows this to nobody.
            "PRODID:-//nettrash//Family Connect//EN",
            "CALSCALE:GREGORIAN",
            // PUBLISH, not REQUEST: this is a copy of something the family already agreed on,
            // not an invitation with attendees to answer — who is coming lives on the board.
            "METHOD:PUBLISH",
            "BEGIN:VEVENT",
            $"UID:{Escape(uid)}",
            $"DTSTAMP:{Escape(stampedAt)}",
            $"DTSTART:{Escape(startsAt)}",
            // An event with no end is an hour by convention, and the convention is the CLIENT's:
            // rather than invent one here, the file simply has no DTEND, which every calendar
            // reads as its own default duration.
            endsAt is null ? string.Empty : $"DTEND:{Escape(endsAt)}",
            $"SUMMARY:{Escape(title)}",
            place is null ? string.Empty : $"LOCATION:{Escape(place)}",
            "END:VEVENT",
            "END:VCALENDAR",
        ];
        var file = new StringBuilder();
        foreach (var line in lines)
        {
            if (line.Length == 0)
            {
                continue;
            }
            file.Append(Fold(line)).Append("\r\n");
        }
        return file.ToString();
    }

    /// <summary>A TEXT value's own characters, kept from being read as the format's.</summary>
    private static string Escape(string value)
    {
        var text = new StringBuilder(value.Length);
        foreach (var character in value)
        {
            switch (character)
            {
                case '\\':
                    text.Append("\\\\");
                    break;
                case ';':
                    text.Append("\\;");
                    break;
                case ',':
                    text.Append("\\,");
                    break;
                case '\n':
                    text.Append("\\n");
                    break;
                case '\r':
                    // Half a line ending: the `\n` beside it carries the meaning.
                    break;
                default:
                    text.Append(character);
                    break;
            }
        }
        return text.ToString();
    }

    /// <summary>
    /// A content line broken at 75 OCTETS, continued with a leading space, and never split inside
    /// a UTF-8 sequence.
    /// </summary>
    private static string Fold(string line)
    {
        if (Encoding.UTF8.GetByteCount(line) <= FoldAt)
        {
            return line;
        }
        var folded = new StringBuilder(line.Length + 8);
        var room = FoldAt;
        // BY CODE POINT — a rune — which is what "never inside a multi-octet UTF-8 sequence"
        // means and what every other port of this does (Rust iterates `chars()`). NOT by grapheme
        // cluster: that would keep a ZWJ emoji whole and fold a byte EARLIER than the others do,
        // and a file that differs from the other three clients' is the thing the oracle exists to
        // catch. It is also not by UTF-16 char, which would split an astral code point in half.
        foreach (var rune in line.EnumerateRunes())
        {
            var width = rune.Utf8SequenceLength;
            if (width > room)
            {
                folded.Append("\r\n ");
                // The continuation's leading space is itself an octet of the folded line.
                room = FoldAt - 1;
            }
            folded.Append(rune);
            room -= width;
        }
        return folded.ToString();
    }
}
