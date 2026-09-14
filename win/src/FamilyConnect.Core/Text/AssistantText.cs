using System.Globalization;

namespace FamilyConnect.Core;

/// <summary>
/// What a message body ASKS the assistant for: <c>@ai</c> and <c>/draw</c> (<c>fc_text::assistant</c>, ios
/// <c>AssistantMention</c>). A WIRE CONTRACT: the server decides from the same grammar whether a family-chat message
/// reaches the assistant at all and whether a request goes to an image model instead, so every client says what the
/// server will do.
/// </summary>
/// <remarks>
/// <para>
/// <b><c>@ai</c></b> is matched case-insensitively over ASCII only, and must sit between boundaries: the start or end of
/// the body, or anything that is not an ASCII letter, digit or <c>_</c> — which stops <c>anna@ai.example</c> and
/// <c>@aiden</c>. The server tests BYTES; a non-ASCII neighbour is a boundary either way, so testing its UTF-16 unit
/// here is the same statement.
/// </para>
/// <para>
/// <b><c>/draw</c></b> must be the first thing in the body, past leading white space and ONE leading <c>@ai</c>; it must
/// be followed by white space; and the prompt after it, trimmed, must not be empty. White space is Unicode White_Space —
/// the server's, which is .NET's <c>char.IsWhiteSpace</c> (the poll oracle pins that).
/// </para>
/// </remarks>
public static class AssistantText
{
    public const string Token = "@ai";
    public const string DrawToken = "/draw";

    /// <summary>Does this body address the assistant? The server's <c>mentions_assistant</c>, answer for answer.</summary>
    public static bool Mentions(string body)
    {
        for (var index = 0; index + Token.Length <= body.Length; index++)
        {
            if (MentionAt(body, index))
            {
                return true;
            }
        }
        return false;
    }

    /// <summary>
    /// Every <c>@ai</c> in <paramref name="body"/> — all of them, for a bubble to draw — as UTF-8 byte ranges grown to the grapheme
    /// clusters they touch, in order (<c>fc_text::assistant::ranges</c>).
    /// </summary>
    public static IReadOnlyList<(int Start, int End)> Ranges(string body)
    {
        var found = new List<(int Start, int End)>();
        int[]? clusters = null;
        var bytes = 0;
        for (var index = 0; index < body.Length; index++)
        {
            if (MentionAt(body, index))
            {
                clusters ??= global::FamilyConnect.Core.Mentions.ClusterBoundaries(body);
                found.Add(global::FamilyConnect.Core.Mentions.Widen(clusters, bytes, bytes + Token.Length));
            }
            bytes += char.IsHighSurrogate(body[index]) ? 0 : char.IsLowSurrogate(body[index]) ? 4 : body[index] < 0x80 ? 1 : body[index] < 0x800 ? 2 : 3;
        }
        return found;
    }

    /// <summary>The picture this body asks for, or null because it asks for none.</summary>
    public static string? DrawPrompt(string body) => Scan(body)?.Prompt;

    /// <summary>Does this body ask for a picture? Exactly when <see cref="DrawPrompt"/> answers one.</summary>
    public static bool AsksForPicture(string body) => Scan(body) is not null;

    /// <summary>The <c>/draw</c> token as a UTF-8 byte range, exactly when <see cref="DrawPrompt"/> answers — both off one scan.</summary>
    public static (int Start, int End)? DrawTokenRange(string body)
    {
        if (Scan(body) is not { } scan)
        {
            return null;
        }
        var start = System.Text.Encoding.UTF8.GetByteCount(body.AsSpan(0, scan.Index));
        return (start, start + DrawToken.Length);
    }

    /// <summary>
    /// The draft "ask the assistant" leaves behind: <c>@ai </c> appended — after a space when the last character is not one
    /// — or the draft untouched when it already mentions the assistant. Appended, never inserted at the caret.
    /// </summary>
    public static string WithAssistantMention(string draft)
    {
        if (Mentions(draft))
        {
            return draft;
        }
        if (draft.Length == 0)
        {
            return $"{Token} ";
        }
        return LastCharacter(draft) == " " ? $"{draft}{Token} " : $"{draft} {Token} ";
    }

    private static bool IsBoundary(char unit) => !(unit is (>= 'a' and <= 'z') or (>= 'A' and <= 'Z') or (>= '0' and <= '9') or '_');

    private static bool MentionAt(string body, int index)
    {
        var end = index + Token.Length;
        return end <= body.Length
            && body[index] == '@'
            && body[index + 1] is 'a' or 'A'
            && body[index + 2] is 'i' or 'I'
            && (index == 0 || IsBoundary(body[index - 1]))
            && (end == body.Length || IsBoundary(body[end]));
    }

    private static (int Index, string Prompt)? Scan(string body)
    {
        var index = SkippingWhiteSpace(body, 0);
        // ONE leading mention, and only a leading one: `look @ai /draw a cat` is an ordinary message.
        if (MentionAt(body, index))
        {
            index = SkippingWhiteSpace(body, index + Token.Length);
        }
        var end = index + DrawToken.Length;
        if (end > body.Length || !IsDrawToken(body, index))
        {
            return null;
        }
        // The end of the body is no request, and anything but white space makes a longer word. Every White_Space scalar is
        // a single UTF-16 unit.
        if (end == body.Length || !char.IsWhiteSpace(body[end]))
        {
            return null;
        }
        var prompt = body[end..].Trim();
        return prompt.Length > 0 ? (index, prompt) : null;
    }

    private static bool IsDrawToken(string body, int index)
    {
        for (var at = 0; at < DrawToken.Length; at++)
        {
            var unit = body[index + at];
            var lowered = unit is >= 'A' and <= 'Z' ? (char)(unit + 32) : unit;
            if (lowered != DrawToken[at])
            {
                return false;
            }
        }
        return true;
    }

    private static int SkippingWhiteSpace(string body, int from)
    {
        while (from < body.Length && char.IsWhiteSpace(body[from]))
        {
            from++;
        }
        return from;
    }

    /// <summary>The last grapheme cluster, as Swift's <c>hasSuffix</c> compares it.</summary>
    private static string LastCharacter(string text)
    {
        var last = string.Empty;
        var elements = StringInfo.GetTextElementEnumerator(text);
        while (elements.MoveNext())
        {
            last = elements.GetTextElement();
        }
        return last;
    }
}
