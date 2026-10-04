using System.Text;

namespace FamilyConnect.Core;

/// <summary>
/// The owner's places for the daily greeting's weather, as this client lets them be typed and sends them
/// (docs/protocol.md, "Today's weather, for places the owner chose"; issue #72) — the server's own rules for a name
/// (<c>validate_greeting_places</c>), applied BEFORE the request so that nothing the owner can type is refused.
/// </summary>
/// <remarks>
/// <para>
/// <b>RUST'S CHARACTERS, NOT .NET'S.</b> "Whitespace", "control character" and "80 characters" are the server's words, so
/// they are asked of <see cref="RustChar"/> (Rust's own tables) and counted in Unicode scalars — a .NET <c>Length</c>
/// counts UTF-16 units, and a name of astral characters would be cut at half the length the server allows.
/// </para>
/// <para>
/// <b>REPEATS ARE THE SERVER'S TO DROP.</b> The server drops a name equal to an earlier one once both are lower-cased, and
/// its lower-casing is Rust's <c>str::to_lowercase</c>, final sigma and all. A client that dropped them too would be a
/// second answer to that question, which could disagree; this side sends what was typed and then shows the list the
/// server KEPT. With at most three fields, a repeat can never push the list over the limit.
/// </para>
/// </remarks>
public static class GreetingPlaces
{
    /// <summary>The most places a family's greeting mentions the weather for (docs/protocol.md, "Limits": 3, fixed).</summary>
    public const int MaxPlaces = 3;

    /// <summary>The longest name, in characters (Unicode scalars), once its whitespace is folded.</summary>
    public const int MaxChars = 80;

    /// <summary>Why a name would be refused; <see cref="None"/> when it would be kept.</summary>
    public enum Problem
    {
        None,
        Empty,
        Control,
        TooLong,

        /// <summary>A field past the <see cref="MaxPlaces"/>th that holds a name.</summary>
        TooMany,
    }

    /// <summary>
    /// What one field asks for, and the first field that would be refused (1-based, like the server's message), or none.
    /// </summary>
    /// <param name="Names">The names to send, folded, with the blank fields left out — a field nobody typed in is no place.</param>
    public sealed record Checked(IReadOnlyList<string> Names, int? BadField, Problem Why)
    {
        public bool Ok => BadField is null;
    }

    /// <summary>
    /// The name as the server keeps it: trimmed, with every run of whitespace inside folded to one space — Rust's
    /// <c>split_whitespace().join(" ")</c>.
    /// </summary>
    public static string Fold(string? raw)
    {
        if (string.IsNullOrEmpty(raw))
        {
            return string.Empty;
        }
        var folded = new StringBuilder(raw.Length);
        var gap = false;
        foreach (var rune in raw.EnumerateRunes())
        {
            if (RustChar.IsWhitespace(rune.Value))
            {
                gap = folded.Length > 0;
                continue;
            }
            if (gap)
            {
                folded.Append(' ');
                gap = false;
            }
            folded.Append(rune.ToString());
        }
        return folded.ToString();
    }

    /// <summary>How long a name is as the server counts it: Unicode scalars, not UTF-16 units.</summary>
    public static int Length(string? name) => string.IsNullOrEmpty(name) ? 0 : name.EnumerateRunes().Count();

    /// <summary>Why the server would refuse this name, as typed — it is folded first, as the server folds it.</summary>
    public static Problem Judge(string? raw)
    {
        var name = Fold(raw);
        if (name.Length == 0)
        {
            return Problem.Empty;
        }
        if (name.EnumerateRunes().Any(rune => RustChar.IsControl(rune.Value)))
        {
            return Problem.Control;
        }
        return Length(name) > MaxChars ? Problem.TooLong : Problem.None;
    }

    /// <summary>
    /// The fields as a request: each folded, the blank ones left out, and the first one the server would refuse named.
    /// More than <see cref="MaxPlaces"/> non-blank fields is a problem of the first one past the limit.
    /// </summary>
    public static Checked Check(IEnumerable<string?> fields)
    {
        ArgumentNullException.ThrowIfNull(fields);
        var names = new List<string>();
        var field = 0;
        foreach (var raw in fields)
        {
            field++;
            switch (Judge(raw))
            {
                case Problem.Empty:
                    continue;
                case Problem.None:
                    if (names.Count == MaxPlaces)
                    {
                        return new Checked(names, field, Problem.TooMany);
                    }
                    names.Add(Fold(raw));
                    break;
                case var why:
                    return new Checked(names, field, why);
            }
        }
        return new Checked(names, null, Problem.None);
    }

    /// <summary>
    /// What a field holds as the person types or pastes into it: the control characters that are not whitespace taken out
    /// (the server refuses them, and nobody means one in a place name; a tab or a line break is whitespace, which the
    /// server folds), and the rest cut at <see cref="MaxChars"/> characters, never inside a surrogate pair. Whitespace is
    /// NOT folded here — that would eat the space somebody has just typed before the next word.
    /// </summary>
    public static string Typed(string? text)
    {
        if (string.IsNullOrEmpty(text))
        {
            return string.Empty;
        }
        var kept = new StringBuilder(text.Length);
        var count = 0;
        foreach (var rune in text.EnumerateRunes())
        {
            if (RustChar.IsControl(rune.Value) && !RustChar.IsWhitespace(rune.Value))
            {
                continue;
            }
            if (count == MaxChars)
            {
                break;
            }
            kept.Append(rune.ToString());
            count++;
        }
        return kept.ToString();
    }
}
