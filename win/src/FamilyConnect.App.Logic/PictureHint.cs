using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>
/// The line said while a member is asking for a picture: the images deployment's filter refuses most descriptions that
/// name a real person, a public figure or a brand, and nothing else would tell them which word was the problem
/// (docs/protocol.md, "A refused description is reworded once"). The server rewords a refused description once on its
/// own; this line is so that it seldom has to.
/// </summary>
/// <remarks>
/// Only where a picture is really being asked for, on a server that really draws — a hint about a refusal that cannot
/// happen is noise. No XAML in here, like the rest of this assembly: when it shows is pinned by an ordinary unit test.
/// </remarks>
public static class PictureHint
{
    /// <summary>What the line says.</summary>
    public static string Sentence(IStringCatalog say) =>
        say.Get("Describe people and things in general words — real names and brands are often refused.");

    /// <summary>
    /// Under the composer: a draft that has begun a <c>/draw</c> request — the token first, past leading white space, and
    /// followed by white space, before any description is typed — that the server will answer with a picture. In the
    /// assistant's own chat that is the draft; in the family chat only an <c>@ai</c> reaches the assistant at all
    /// (<see cref="AssistantConsent.ReachesTheModel"/>). Never while a message is being edited: an edit cannot become a
    /// request.
    /// </summary>
    /// <remarks>
    /// Where the server draws is where "Ask for a picture" is offered: an assistant with <c>images</c> and a NAMED
    /// processor (<see cref="AssistantButtons"/>). The family chat's <c>@ai /draw</c> is drawn by the same deployment, so
    /// it is refused by the same filter and gets the same line.
    /// </remarks>
    public static bool ForComposer(string? chatKind, string? draft, AssistantDto? assistant, bool editing) =>
        !editing
        && assistant is { Images: true }
        && AssistantConsent.IsAvailable(assistant.Processor)
        && AssistantConsent.ReachesTheModel(chatKind, draft)
        && AssistantText.BeginsPictureRequest(draft ?? string.Empty);

    /// <summary>
    /// "Draw a backdrop" on an event — and the line beside it, since the title IS the description: the author's own event
    /// on a server that can draw at all (docs/protocol.md, "Board"; <c>pictures_unavailable</c> otherwise).
    /// </summary>
    /// <remarks>
    /// And, like "Ask for a picture" (<see cref="AssistantButtons"/>), only where the server NAMES who draws: the title
    /// is the author's words going to the model, which needs their consent (<see cref="BackdropConsent"/>), and a
    /// consent question with a hole where the recipient goes cannot be asked.
    /// </remarks>
    public static bool OffersBackdrop(bool editable, AssistantDto? assistant) =>
        editable && assistant is { Images: true } && AssistantConsent.IsAvailable(assistant.Processor);
}
