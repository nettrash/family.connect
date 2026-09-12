namespace FamilyConnect.Core.Board;

/// <summary>
/// A note's SIZE: a step the author chooses, drawn at each client's own idiom — never a
/// measurement on the wire (docs/protocol.md, "Board").
/// </summary>
public enum NoteSize
{
    Small,
    Medium,
    Large,
}

/// <summary>What a note IS.</summary>
public enum NoteKind
{
    Text,
    Photo,
    Event,
    Tasks,
}

/// <summary>A note's hand: an INTENT, which each client resolves to a face of its own.</summary>
public enum NoteFont
{
    Plain,
    Serif,
    Mono,
    Casual,
}

/// <summary>Whether somebody is coming to an event.</summary>
public enum RsvpAnswer
{
    Going,
    Maybe,
    No,
}

/// <summary>
/// The names a note's look travels under, and the fallbacks that keep an unknown one readable.
/// </summary>
/// <remarks>
/// EVERY name that comes off the wire falls back rather than failing: a note from a newer server
/// with a fourth size or a fifth colour still has to be readable, and a hole in the family's shared
/// layout would be worse than a sticker drawn in the default. The fallback is for DRAWING only —
/// see <see cref="Notes.PatchSize"/> for why an edit must not write it back.
/// </remarks>
public static class Notes
{
    // ---- size ------------------------------------------------------------

    /// <summary>Small to large, the order a picker offers them in.</summary>
    public static readonly NoteSize[] Sizes = [NoteSize.Small, NoteSize.Medium, NoteSize.Large];

    /// <summary>An unknown or absent name reads as medium — what every note was before sizes.</summary>
    public static NoteSize SizeFrom(string? name) => name switch
    {
        "small" => NoteSize.Small,
        "large" => NoteSize.Large,
        _ => NoteSize.Medium,
    };

    public static string NameOf(NoteSize size) => size switch
    {
        NoteSize.Small => "small",
        NoteSize.Large => "large",
        _ => "medium",
    };

    /// <summary>
    /// What an edit PATCHes for size: the chosen name when the author changed it, null when they
    /// did not. The distinction is for a name this client does not know — a fourth size from a
    /// newer server DRAWS as medium and must not be WRITTEN BACK as medium because somebody fixed
    /// a typo in the text.
    /// </summary>
    public static string? PatchSize(NoteSize chosen, string? stored) =>
        chosen == SizeFrom(stored) ? null : NameOf(chosen);

    // ---- kind ------------------------------------------------------------

    /// <summary>An unknown kind DRAWS AS TEXT rather than being dropped: it still has a slot.</summary>
    public static NoteKind KindFrom(string? name) => name switch
    {
        "photo" => NoteKind.Photo,
        "event" => NoteKind.Event,
        "tasks" => NoteKind.Tasks,
        _ => NoteKind.Text,
    };

    public static string NameOf(NoteKind kind) => kind switch
    {
        NoteKind.Photo => "photo",
        NoteKind.Event => "event",
        NoteKind.Tasks => "tasks",
        _ => "text",
    };

    // ---- colour ----------------------------------------------------------

    /// <summary>The six colours the protocol allows, in the picker's order.</summary>
    public static readonly string[] Colors = ["yellow", "pink", "blue", "green", "orange", "purple"];

    /// <summary>
    /// A colour name's pastel — the apps' own values. An unknown name draws yellow, the first and
    /// the most note-like, rather than failing.
    /// </summary>
    /// <remarks>
    /// These are FIXED LIGHT colours in every appearance, which is why the ink on a note is forced
    /// dark rather than following the theme: a dark mode's white would be unreadable on yellow.
    /// </remarks>
    public static string ColorHex(string? name) => name switch
    {
        "pink" => "#fcc7d9",
        "blue" => "#c2e0fc",
        "green" => "#c9f0c9",
        "orange" => "#ffd9b3",
        "purple" => "#e0d1fa",
        _ => "#fff2b3",
    };

    // ---- font ------------------------------------------------------------

    /// <summary>Plainest first, as the picker shows them.</summary>
    public static readonly NoteFont[] Fonts =
        [NoteFont.Plain, NoteFont.Serif, NoteFont.Mono, NoteFont.Casual];

    public static NoteFont FontFrom(string? name) => name switch
    {
        "serif" => NoteFont.Serif,
        "mono" => NoteFont.Mono,
        "casual" => NoteFont.Casual,
        _ => NoteFont.Plain,
    };

    public static string NameOf(NoteFont font) => font switch
    {
        NoteFont.Serif => "serif",
        NoteFont.Mono => "mono",
        NoteFont.Casual => "casual",
        _ => "plain",
    };

    /// <summary>The same rule as <see cref="PatchSize"/>, one field over.</summary>
    public static string? PatchFont(NoteFont chosen, string? stored) =>
        chosen == FontFrom(stored) ? null : NameOf(chosen);

    /// <summary>
    /// The face each hand asks for on Windows, as a font family list.
    /// </summary>
    /// <remarks>
    /// The hand is an intent, so this is the Windows answer to it and nobody else's: Segoe UI for
    /// plain (the system face), Georgia for serif, Cascadia Mono for code, and Segoe Print for the
    /// casual one — all in-box on Windows 10 and 11, so nothing is bundled and nothing is
    /// synthesised. A face that is missing falls through the list the same way the web's does.
    /// </remarks>
    public static string FontFamily(NoteFont font) => font switch
    {
        NoteFont.Serif => "Georgia, 'Times New Roman', serif",
        NoteFont.Mono => "Cascadia Mono, Consolas, monospace",
        NoteFont.Casual => "Segoe Print, Ink Free, Segoe UI",
        _ => "Segoe UI Variable Text, Segoe UI",
    };

    // ---- rsvp ------------------------------------------------------------

    public static readonly RsvpAnswer[] Answers = [RsvpAnswer.Going, RsvpAnswer.Maybe, RsvpAnswer.No];

    /// <summary>
    /// An answer from a newer server is not drawn as one of these — better no button lit than a
    /// claim that somebody said something else.
    /// </summary>
    public static RsvpAnswer? AnswerFrom(string? name) => name switch
    {
        "going" => RsvpAnswer.Going,
        "maybe" => RsvpAnswer.Maybe,
        "no" => RsvpAnswer.No,
        _ => null,
    };

    public static string NameOf(RsvpAnswer answer) => answer switch
    {
        RsvpAnswer.Going => "going",
        RsvpAnswer.Maybe => "maybe",
        _ => "no",
    };

    /// <summary>The word on the button, through the catalogue.</summary>
    public static string Title(RsvpAnswer answer, IStringCatalog strings) => answer switch
    {
        RsvpAnswer.Going => strings.Get("Going"),
        RsvpAnswer.Maybe => strings.Get("Maybe"),
        _ => strings.Get("Can't"),
    };

    /// <summary>
    /// Who is coming, as the STICKER says it: the counts, not the names — a sticker has room for
    /// the news and the note that opens has room for the people. Nothing at all while nobody is
    /// going or thinking about it.
    /// </summary>
    public static string? GoingLine(int going, int maybe, IStringCatalog strings) => (going, maybe) switch
    {
        (0, 0) => null,
        (_, 0) => strings.Format("%lld going", going),
        (0, _) => strings.Format("%lld maybe", maybe),
        _ => strings.Format("%lld going, %lld maybe", going, maybe),
    };
}
