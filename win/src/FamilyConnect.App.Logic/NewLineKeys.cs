namespace FamilyConnect.App.Logic;

/// <summary>
/// New lines in what people write (issue #95). Enter on its own sends; Enter held with ANY of the modifiers people reach
/// for — Shift, Ctrl or Alt — is a new line in the field. No setting: everybody's habit works, whichever app taught it.
/// The same answer on every desktop client: the Mac's <c>NewLineKeys</c> (⌥ ⌘ ⌃ ⇧) and the web's
/// <c>composer::return_makes_line</c>. The Windows key is not among them — Windows keeps its combinations for itself.
/// </summary>
public static class NewLineKeys
{
    /// <summary>Enter pressed with these held: a new line, or not (then it is the field's own Enter — Send in a composer).</summary>
    public static bool EnterMakesLine(bool shift, bool control, bool alt) => shift || control || alt;

    /// <summary>
    /// Lines as every other client writes them. A WinUI <c>TextBox</c> keeps a line break as a lone carriage return, and
    /// sent like that a two-line message reached the web as ONE line (CSS draws a CR as a space, and the web's markdown
    /// splits lines at line feeds only) — so a CR, alone or before a LF, goes out as a LF. Mentions carry names, not
    /// offsets, so nothing else moves.
    /// </summary>
    public static string ForWire(string text) =>
        text.Contains('\r', StringComparison.Ordinal)
            ? text.Replace("\r\n", "\n", StringComparison.Ordinal).Replace('\r', '\n')
            : text;
}
