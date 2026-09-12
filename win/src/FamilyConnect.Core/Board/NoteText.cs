namespace FamilyConnect.Core.Board;

/// <summary>
/// What a note may SAY (docs/protocol.md, "Board"): the caps, counted the way the server counts
/// them, and the counter that only shows when it is worth showing.
/// </summary>
/// <remarks>
/// <para>
/// The server counts UNICODE SCALARS (Rust's <c>chars()</c>). .NET strings are UTF-16, so
/// <c>string.Length</c> would give an emoji note half its allowance and a grapheme count would be
/// smaller than the server's for a family emoji or a combining mark — a note that looked under the
/// cap would come back refused. Everything here counts scalars, via
/// <see cref="System.Globalization.StringInfo"/>'s rune enumeration.
/// </para>
/// <para>
/// Web counterpart: <c>fc_text::board</c>. Apple: <c>NoteText</c>. Android: <c>NoteTexts</c>.
/// </para>
/// </remarks>
public static class NoteText
{
    /// <summary>"text is trimmed, non-empty and at most 280 characters".</summary>
    public const int MaxTextChars = 280;

    /// <summary>The longest an event's place may be, counted the same way.</summary>
    public const int MaxPlaceChars = 200;

    /// <summary>One line of a task list: a thing to do, not a paragraph about it.</summary>
    public const int MaxTaskItemChars = 100;

    /// <summary>
    /// The most lines one list may hold — the server's own <c>max_task_items</c> default. A client
    /// that lets somebody type a twenty-first line is a client whose save fails for a reason
    /// nobody can see, so the add is disabled at the ceiling instead.
    /// </summary>
    public const int MaxTaskItems = 20;

    /// <summary>The counter shows only in the last 40, so an ordinary note is written in peace.</summary>
    public const int CounterFrom = 40;

    /// <summary>How many Unicode scalars this text is, as the server counts them.</summary>
    public static int Scalars(string text)
    {
        var count = 0;
        foreach (var _ in text.EnumerateRunes())
        {
            count++;
        }
        return count;
    }

    /// <summary>The first <paramref name="max"/> scalars, which is what may be kept of it.</summary>
    public static string Capped(string text, int max)
    {
        if (max <= 0)
        {
            return string.Empty;
        }
        var kept = 0;
        var units = 0;
        foreach (var rune in text.EnumerateRunes())
        {
            if (kept == max)
            {
                break;
            }
            kept++;
            units += rune.Utf16SequenceLength;
        }
        return units >= text.Length ? text : text[..units];
    }

    /// <summary>How many more characters the author may type. Never negative.</summary>
    public static int Remaining(string text) => Math.Max(0, MaxTextChars - Scalars(text));

    /// <summary>Whether the counter is worth showing yet.</summary>
    public static bool ShowsCounter(string text) => Remaining(text) <= CounterFrom;

    /// <summary>
    /// Cut what went past <paramref name="max"/> out of what was just typed or pasted — the
    /// scalars immediately before the caret — rather than off the END of the note, which ate the
    /// words after the place somebody was typing in and moved their caret there besides. Only
    /// when the caret is too near the start to take it all from there is the end cut.
    /// </summary>
    /// <remarks>
    /// The caret is in UTF-16 units, which is what a text box reports and what the web's
    /// <c>cap_at_caret</c> takes; the answer is the text kept and where the caret goes.
    /// </remarks>
    public static (string Kept, int Caret) CapAtCaret(string value, int caret, int max)
    {
        var count = Scalars(value);
        if (count <= max)
        {
            return (value, caret);
        }
        caret = Math.Clamp(caret, 0, value.Length);
        // The caret in units, moved back to a scalar boundary: a caret inside a surrogate pair
        // would cut a character in half.
        var caretUnit = 0;
        var starts = new List<int>();
        var units = 0;
        foreach (var rune in value.EnumerateRunes())
        {
            if (units >= caret)
            {
                break;
            }
            starts.Add(units);
            units += rune.Utf16SequenceLength;
            caretUnit = units;
        }
        var removable = Math.Min(count - max, starts.Count);
        var cutFrom = removable == 0 ? caretUnit : starts[starts.Count - removable];
        var kept = Capped(string.Concat(value.AsSpan(0, cutFrom), value.AsSpan(caretUnit)), max);
        return (kept, cutFrom);
    }

    /// <summary>
    /// How a list says what is done of it, under its title — the sentence every client shows.
    /// </summary>
    public static string DoneOf(int done, int total, IStringCatalog strings) =>
        strings.Format("%lld of %lld done", done, total);
}
