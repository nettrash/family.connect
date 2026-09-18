using FamilyConnect.Core.Protocol;

namespace FamilyConnect.Core;

/// <summary>Which of the member limit's three footers applies.</summary>
public enum CapKind
{
    /// <summary>No cap of the owner's own: the operator's ceiling binds.</summary>
    OpenToCeiling,

    /// <summary>
    /// The door is at or below the roster. Legal and deliberate — an owner who inherits a large family
    /// must still be able to shut the door — and NOBODY is removed.
    /// </summary>
    Frozen,

    /// <summary>Room to spare, against the LOWER of the cap and the ceiling.</summary>
    Room,
}

/// <summary>The member limit, as its footer says it.</summary>
public readonly record struct CapState(CapKind Kind, int Members, int Seats, int Ceiling);

/// <summary>
/// The owner's house rules: the join policy, the member limit and the assistant's switches — the
/// rules of <c>fc_text::account</c> and the web client's family pane, held to the Rust original by
/// <c>ChatOracleTests</c>.
/// </summary>
public static class HouseRules
{
    /// <summary>How many photos a question may carry to the model (<c>fc_text::assistant_pictures</c>).</summary>
    public const int MaxPicturesPerQuestion = 4;

    /// <summary>The three policies the protocol allows, with the key each is said by.</summary>
    public static IReadOnlyList<(string Code, string Key)> Policies { get; } =
        [("open", "Join immediately"), ("approval", "Need approval"), ("closed", "Nobody")];

    /// <summary>
    /// The nine languages a family may declare, each named IN ITSELF — a family picks the language
    /// they speak by the name they know it by.
    /// </summary>
    public static IReadOnlyList<(string Tag, string Name)> FamilyLanguages { get; } =
    [
        ("en", "English"), ("de", "Deutsch"), ("es", "Español"), ("fr", "Français"), ("ja", "日本語"),
        ("ru", "Русский"), ("sr", "Српски"), ("sr-Latn", "Srpski (latinica)"), ("zh-Hans", "简体中文"),
    ];

    /// <summary>
    /// The seats a family actually has: the door takes the LOWER of the owner's cap and the ceiling,
    /// and a stored cap is never re-validated when the ceiling moves.
    /// </summary>
    public static CapState Cap(int? cap, int members, int ceiling)
    {
        if (cap is not { } own)
        {
            return new CapState(CapKind.OpenToCeiling, members, 0, ceiling);
        }
        var seats = Math.Min(own, ceiling);
        return new CapState(seats <= members ? CapKind.Frozen : CapKind.Room, members, seats, ceiling);
    }

    public static string CapFooter(CapState state, IStringCatalog say) => state.Kind switch
    {
        CapKind.OpenToCeiling => say.Plural(
            "No limit of your own. This server allows up to %lld members in a family.", state.Ceiling, state.Ceiling),
        CapKind.Frozen => say.Plural(
            "%lld members now. Nobody new can join until somebody leaves; no one is removed.", state.Members, state.Members),
        _ => say.Plural("%lld of %lld seats used.", state.Members, state.Members, state.Seats),
    };

    /// <summary>A cap stepped or typed to, held inside 1…ceiling.</summary>
    public static int ClampCap(int value, int ceiling) => Math.Min(Math.Max(value, 1), Math.Max(ceiling, 1));

    /// <summary>
    /// The cap proposed when the limit is first turned on: the family frozen where it stands, which is
    /// what reaching for "limit members" almost always means — inside the same bounds.
    /// </summary>
    public static int SeedCap(int members, int ceiling) => ClampCap(members, ceiling);

    public static string PolicyCaption(string? policy, IStringCatalog say) => policy switch
    {
        "approval" => say.Get("With approval, join requests wait here until you approve them."),
        "closed" => say.Get("The invite code stops working — nobody new can join. Requests already waiting are unaffected, and you can still approve them."),
        _ => say.Get("Anyone with the invite code joins straight away."),
    };

    /// <summary>
    /// Why a switch that rides on vision does nothing, or null when it does: the server cannot look
    /// at pictures; vision is off, and the server refuses the switch while it is; or the history it
    /// draws from is not sent (<paramref name="whenHistoryOff"/>, which differs per switch).
    /// </summary>
    public static string? PicturesNote(bool serverVision, FamilyDto shown, string whenHistoryOff, IStringCatalog say) =>
        !serverVision ? say.Get("Not available here: the assistant on this server can't look at pictures.")
        : !shown.AiVision ? say.Get("Turn on Can be shown photos first — the server refuses this while that is off.")
        : !shown.AiHistory ? whenHistoryOff
        : null;

    public static string AssistantFailure(ApiError error, IStringCatalog say) =>
        error.Code == ErrorCodes.NotFamilyOwner
            ? say.Get("Only the family owner can change this.")
            : say.Get("Couldn't save that. Try again.");
}
