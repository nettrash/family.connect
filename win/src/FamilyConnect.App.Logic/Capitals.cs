using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

/// <summary>
/// Capital letters at the start of a sentence (issue #97; docs/capitals-and-rich-markdown-2026-10-17.md, "The rule"). No
/// Windows keyboard capitalises a sentence, so the app does it itself, always on, in the multi-line prose fields — the
/// composer, the thread reply, a note, a report's note. The same rule as the web's <c>fc_text::composer::capitalizes</c> and
/// the Mac's <c>Capitals.swift</c>, held to the same table.
/// </summary>
/// <remarks>
/// <para>
/// A field wires it in two places. When the text is about to change, <see cref="AfterTyping"/> says whether the change was
/// one typed letter that becomes a capital, and if so what to put in the field instead; the field keeps the
/// <see cref="CapitalTakeBack"/> it returns. When Backspace is pressed, <see cref="CapitalTakeBack.Backspace"/> says whether it
/// takes that capital back. Anything else — another key, a click, a paste, a selection — forgets the take-back.
/// </para>
/// <para>
/// Offsets are UTF-16 indices into the field's own text, as a <c>TextBox</c> counts them, and a line break may be the lone
/// carriage return a <c>TextBox</c> keeps.
/// </para>
/// </remarks>
public static class Capitals
{
    /// <summary>The marks that may open a word before its first letter: <c>"(Hi</c>.</summary>
    private static readonly char[] OpeningMarks = ['"', '\'', '(', '[', '«', '“', '‘'];

    /// <summary>The marks that may close a sentence after its stop: <c>(Done.)</c>.</summary>
    private static readonly char[] ClosingMarks = ['"', '\'', ')', ']', '»', '”', '’'];

    private static readonly char[] LineBreaks = ['\n', '\r'];

    private static readonly char[] Blanks = [' ', '\t', '\n', '\r'];

    /// <summary>
    /// The capital the rule would put in place of <paramref name="typed"/>, or null. <paramref name="typed"/> must be ONE
    /// character — one UTF-16 unit, or a surrogate pair — that is a lower-case letter whose upper case (Rust's full mapping, the
    /// invariant one, never a locale's) is exactly one character, and a different one. <c>ß</c> (upper case <c>SS</c>) has none.
    /// </summary>
    public static string? CapitalOf(string typed)
    {
        int scalar;
        if (typed.Length == 1 && !char.IsSurrogate(typed[0]))
        {
            scalar = typed[0];
        }
        else if (typed.Length == 2 && char.IsSurrogatePair(typed[0], typed[1]))
        {
            scalar = char.ConvertToUtf32(typed[0], typed[1]);
        }
        else
        {
            return null;
        }
        return RustCase.IsLowercase(scalar) && RustCase.SingleUppercase(scalar) is { } upper ? char.ConvertFromUtf32(upper) : null;
    }

    /// <summary>
    /// Whether <paramref name="typed"/>, the one character the keyboard just produced, goes in as its capital.
    /// <paramref name="before"/> is the field's text before the caret (the selection's start). YES only when all four hold:
    /// <list type="number">
    /// <item><paramref name="typed"/> has a capital (<see cref="CapitalOf"/>);</item>
    /// <item>a word is starting: with any opening marks typed just before it set aside, <paramref name="before"/> is empty or
    /// ends in a space, a tab or a line break;</item>
    /// <item>a sentence is starting: <paramref name="before"/>, without those marks and trailing spaces and tabs, is empty, ends
    /// in a line break, is nothing on its line but a list, heading or quote marker, or ends in <c>.</c>, <c>!</c> or <c>?</c>
    /// (closing marks after it allowed) — where a <c>.</c> after a <c>.</c> (an ellipsis) or ending an abbreviation
    /// (<c>e.g.</c>, <c>U.S.</c>) does not count;</item>
    /// <item>the caret is not in code: an even number of fence lines before it, and an even number of backticks on its own
    /// line.</item>
    /// </list>
    /// </summary>
    public static bool Capitalizes(string before, string typed)
    {
        if (CapitalOf(typed) is null)
        {
            return false;
        }
        var core = before.TrimEnd(OpeningMarks);
        // 2. The word is just starting.
        if (core.Length > 0 && Array.IndexOf(Blanks, core[^1]) < 0)
        {
            return false;
        }
        // 3. So is a sentence — 4. and the caret is not in code.
        return StartsSentence(core.TrimEnd(' ', '\t')) && !InCode(before);
    }

    /// <summary>
    /// The one character a change typed, or null when it was anything else. Call it BEFORE the change lands (a
    /// <c>TextBox</c>'s <c>BeforeTextChanging</c>, where <c>Text</c>, <c>SelectionStart</c> and <c>SelectionLength</c> are
    /// still the old ones): the change typed <c>Typed</c> at <c>Start</c> exactly when <paramref name="newText"/> is
    /// <paramref name="oldText"/> with its selection replaced by one character.
    /// </summary>
    /// <remarks>
    /// The old selection is what makes the answer sound. Judged from the two texts and the caret alone, a Backspace that
    /// takes one <c>a</c> out of <c>aab</c> looks exactly like typing <c>a</c> over a selected <c>aa</c> — and would be
    /// capitalised.
    /// </remarks>
    public static (int Start, string Typed)? TypedCharacter(string oldText, int selectionStart, int selectionLength, string newText)
    {
        if (selectionStart < 0 || selectionLength < 0 || selectionStart + selectionLength > oldText.Length)
        {
            return null;
        }
        var tail = oldText.Length - selectionStart - selectionLength;
        var typedLength = newText.Length - selectionStart - tail;
        if (typedLength is not (1 or 2)
            || string.CompareOrdinal(newText, 0, oldText, 0, selectionStart) != 0
            || string.CompareOrdinal(newText, newText.Length - tail, oldText, oldText.Length - tail, tail) != 0)
        {
            return null;
        }
        var typed = newText.Substring(selectionStart, typedLength);
        var oneCharacter = typedLength == 1 ? !char.IsSurrogate(typed[0]) : char.IsSurrogatePair(typed[0], typed[1]);
        return oneCharacter ? (selectionStart, typed) : null;
    }

    /// <summary>
    /// What a field holds instead of <paramref name="newText"/> when the change the rule watches for happened — one typed
    /// letter, at a sentence's start — or null when the change stands as it is. Arguments as <see cref="TypedCharacter"/>.
    /// The edit's <c>Caret</c> is right behind the capital; its <c>TakeBack</c> is what the field keeps for the next Backspace.
    /// </summary>
    public static CapitalEdit? AfterTyping(string oldText, int selectionStart, int selectionLength, string newText)
    {
        if (TypedCharacter(oldText, selectionStart, selectionLength, newText) is not { } typed
            || !Capitalizes(newText[..typed.Start], typed.Typed)
            || CapitalTakeBack.After(typed.Start, typed.Typed) is not { } takeBack)
        {
            return null;
        }
        var end = typed.Start + typed.Typed.Length;
        var text = string.Concat(newText.AsSpan(0, typed.Start), takeBack.Capital, newText.AsSpan(end));
        return new CapitalEdit(text, typed.Start + takeBack.Capital.Length, takeBack);
    }

    /// <summary>Rule 3 over the text before the caret with the opening marks and trailing blanks gone.</summary>
    private static bool StartsSentence(string text)
    {
        if (text.Length == 0 || Array.IndexOf(LineBreaks, text[^1]) >= 0)
        {
            return true;
        }
        var line = text[(text.LastIndexOfAny(LineBreaks) + 1)..];
        if (IsMarker(line.TrimStart(' ', '\t')))
        {
            return true;
        }
        var stopped = text.TrimEnd(ClosingMarks);
        if (stopped.Length == 0)
        {
            return false;
        }
        return stopped[^1] switch
        {
            '!' or '?' => true,
            // An ellipsis trails off rather than ending anything.
            '.' => !(stopped.Length >= 2 && stopped[^2] == '.') && !IsAbbreviation(stopped),
            _ => false,
        };
    }

    /// <summary>A list, heading or quote marker and nothing else: <c>-</c>, <c>*</c>, <c>+</c>, <c>&gt;</c>, one to six <c>#</c>, or digits then <c>.</c> or <c>)</c>.</summary>
    private static bool IsMarker(string line)
    {
        if (line is "-" or "*" or "+" or ">")
        {
            return true;
        }
        if (line.Length is >= 1 and <= 6 && line.All(c => c == '#'))
        {
            return true;
        }
        var digits = 0;
        while (digits < line.Length && char.IsAsciiDigit(line[digits]))
        {
            digits++;
        }
        return digits > 0 && line[digits..] is "." or ")";
    }

    /// <summary>
    /// Whether the <c>.</c> that ends <paramref name="text"/> ends an abbreviation: the word back to the previous space holds
    /// another <c>.</c>, and every piece between the dots is one or two ASCII letters (<c>e.g.</c>, <c>a.m.</c>, <c>U.S.</c>) —
    /// which <c>example.com.</c> is not.
    /// </summary>
    private static bool IsAbbreviation(string text)
    {
        var word = text[(text.LastIndexOfAny(Blanks) + 1)..];
        var pieces = word[..^1];
        return pieces.Contains('.')
            && pieces.Split('.').All(piece => piece.Length is 1 or 2 && piece.All(char.IsAsciiLetter));
    }

    /// <summary>
    /// Rule 4: inside a fence (an odd number of lines start with three backticks) or a code span (an odd number of backticks on
    /// the caret's line). Emoji as you type (#101, <see cref="EmojiTyping"/>) skips code by the same test, as the web's
    /// <c>fc_text::emoji_typing</c> reuses <c>composer::in_code</c>.
    /// </summary>
    internal static bool InCode(string before)
    {
        var fences = before.Split(LineBreaks).Count(line => line.StartsWith("```", StringComparison.Ordinal));
        if (fences % 2 == 1)
        {
            return true;
        }
        var line = before[(before.LastIndexOfAny(LineBreaks) + 1)..];
        return line.Count(c => c == '`') % 2 == 1;
    }
}

/// <summary>What a field holds after the rule capitalised a letter: its text, where the caret goes, and how to take it back.</summary>
public sealed record CapitalEdit(string Text, int Caret, CapitalTakeBack TakeBack);

/// <summary>
/// A capital the rule just put in, remembered so the Backspace pressed straight after it can put the lower-case letter back
/// (docs/capitals-and-rich-markdown-2026-10-17.md, "Taking it back"). The field keeps one from the moment it capitalises
/// until anything else happens — another key, a click, a paste, a selection — and then drops it.
/// </summary>
/// <param name="At">Where the capital starts, in UTF-16 units.</param>
/// <param name="Capital">The capital that was put in.</param>
/// <param name="Typed">What was typed: the lower-case letter a Backspace puts back.</param>
public sealed record CapitalTakeBack(int At, string Capital, string Typed)
{
    /// <summary>Remember that <paramref name="typed"/> went in at <paramref name="at"/> as its capital, or null when it has none.</summary>
    public static CapitalTakeBack? After(int at, string typed) =>
        Capitals.CapitalOf(typed) is { } capital ? new CapitalTakeBack(at, capital, typed) : null;

    /// <summary>
    /// A Backspace with the caret at <paramref name="caret"/>: the text with the lower-case letter back and the caret right
    /// behind it — or null when something is selected, the caret is not right behind the capital, or the capital is no longer
    /// there, and the Backspace is an ordinary one.
    /// </summary>
    public (string Text, int Caret)? Backspace(string text, int caret, int selectionLength)
    {
        var end = At + Capital.Length;
        if (selectionLength != 0 || caret != end || At < 0 || end > text.Length
            || string.CompareOrdinal(text, At, Capital, 0, Capital.Length) != 0)
        {
            return null;
        }
        return (string.Concat(text.AsSpan(0, At), Typed, text.AsSpan(end)), At + Typed.Length);
    }
}
