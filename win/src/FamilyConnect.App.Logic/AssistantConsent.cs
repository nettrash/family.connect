using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

/// <summary>
/// Nothing a member writes reaches the model before that member has said yes (docs/protocol.md,
/// "Consenting to the assistant").
/// </summary>
/// <remarks>
/// <para>
/// Ported by value from <c>ios/FamilyConnect/Models/AssistantConsent.swift</c>, and the same two
/// jobs: deciding which drafts have to be asked about, and saying what the person must be told
/// before they answer. The first MIRRORS <c>model_surface</c> in
/// <c>server/src/handlers_chat.cs</c> — a disagreement there is either a message refused after it
/// was typed, or one sent having asked nothing.
/// </para>
/// <para>
/// No XAML in here on purpose, like the rest of this assembly: the rule is pinned by an ordinary
/// unit test rather than inferred from a window.
/// </para>
/// </remarks>
public static class AssistantConsent
{
    /// <summary>
    /// Would a message with this body, in this chat, be sent to the model? The member's own
    /// <c>ai</c> chat, where everything goes, and the family chat, where only an <c>@ai</c> does.
    /// <c>/draw</c> needs no case of its own: in the family chat it is <c>@ai /draw</c>, a
    /// mention, and in the assistant's own chat it is that chat.
    /// </summary>
    public static bool ReachesTheModel(string? chatKind, string? body) => chatKind switch
    {
        "ai" => true,
        "family" => AssistantText.Mentions(body ?? string.Empty),
        _ => false,
    };

    /// <summary>
    /// Is there an assistant this client may offer at all? A server that names no processor has
    /// one it must not use: the disclosure would have a hole exactly where the person needs to
    /// read, and "some third party" is not something anybody can weigh.
    /// </summary>
    public static bool IsAvailable(string? processor) => !string.IsNullOrWhiteSpace(processor);

    /// <summary>Must this member be asked before this message is sent?</summary>
    public static bool IsRequired(string? chatKind, string? body, string? processor, string? agreedAt) =>
        IsAvailable(processor) && string.IsNullOrWhiteSpace(agreedAt) && ReachesTheModel(chatKind, body);

    /// <summary>
    /// Would this message reach a model whose owner the server will not name, so this client must
    /// hold it back entirely?
    /// </summary>
    /// <remarks>
    /// <paramref name="hasAssistant"/> is what keeps this from swallowing ordinary words: on a
    /// server with no assistant, <c>@ai</c> in the family chat is three characters that reach
    /// nobody, and refusing to send them would break a conversation to protect nothing.
    /// </remarks>
    public static bool IsWithheldFromAnUnnamedAssistant(
        string? chatKind, string? body, bool hasAssistant, string? processor) =>
        hasAssistant && !IsAvailable(processor) && ReachesTheModel(chatKind, body);

    /// <summary>
    /// What the person is told BEFORE they answer, in the order it is shown — all of it on the
    /// screen where they answer, not only in a policy behind a link (App Store Review Guideline
    /// 5.1.1(i), and protocol.md, "What a client must say before it asks").
    /// </summary>
    /// <remarks>
    /// The two family-chat lines depend on the owner's <c>ai_history</c>: with it on a mention
    /// takes the chat's recent history with it, and with it off it takes nothing but itself.
    /// Saying the wrong one of those would be worse than saying neither.
    /// </remarks>
    public static IReadOnlyList<string> Disclosure(
        string processor, bool familyHistory, bool familyVision, IStringCatalog say)
    {
        ArgumentNullException.ThrowIfNull(say);
        var lines = new List<string>
        {
            say.Format("What you write to the assistant leaves this family's server and is sent to %@.", processor),
            familyHistory
                ? say.Format(
                    "In the family chat only a message that says %@ is sent — and with it the last 30 days of that chat, up to 200 messages, including what other people wrote, their names and the times.",
                    AssistantText.Token)
                : say.Format(
                    "In the family chat only a message that says %@ is sent, and nothing else from that chat goes with it.",
                    AssistantText.Token),
        };
        if (familyVision)
        {
            lines.Add(say.Get("A photo is sent only when you attach one to a message for the assistant, and only while your family allows it."));
        }

        lines.Add(say.Get("The answer comes back as a message in that chat, where everyone in the chat can read it."));
        lines.Add(say.Get("You can stop this at any time in Settings. What has already been sent cannot be taken back."));
        return lines;
    }
}
