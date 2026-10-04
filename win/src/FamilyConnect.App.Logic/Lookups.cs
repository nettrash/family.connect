using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>What a member answered on the assistant question, where the server can look things up.</summary>
public enum ConsentAnswer
{
    /// <summary>Nothing agreed; nothing is sent to the server either.</summary>
    NotNow,

    /// <summary>The assistant, and not the lookups: <c>/me/assistant-consent</c> only.</summary>
    Agree,

    /// <summary>Both: <c>/me/assistant-consent</c> (when not already given), then <c>/me/assistant-lookup-consent</c>.</summary>
    AgreeWithLookups,
}

/// <summary>
/// Looking things up for the assistant, from a client's side (docs/protocol.md, "Looking things up"; issue #72): which
/// providers there are to name, what the consent screen, the owner's switch and Settings say about them, the two writes
/// an answer turns into, and which answers keep their links out of preview cards.
/// </summary>
/// <remarks>
/// <para>
/// <b>THREE KEYS, AND THIS CLIENT HOLDS NONE OF THEM.</b> A lookup happens only where the server has a source
/// (<c>assistant.lookups</c>), the owner has turned <c>ai_lookups</c> on, and the asking member has given BOTH consents —
/// and the server decides all three. A client's part is to say who would receive what, to ask, and to send the answer;
/// it never assumes one. Without the lookup consent nothing is refused: the question is answered as before.
/// </para>
/// <para>
/// <b>A CLIENT THAT CANNOT NAME THE PROVIDERS DOES NOT ASK</b> (docs/protocol.md, "What a client must say before it
/// asks"). <see cref="Providers"/> is null for an absent, empty or blank list, and every surface here keys off it.
/// </para>
/// </remarks>
public static class Lookups
{
    /// <summary>
    /// The providers to name, in the server's order, or null where there is nothing to name — an absent array (an older
    /// server, or one with no source), an empty one (which the protocol forbids, and which names nobody), or one of blank
    /// names. Blank names inside a list are dropped rather than named as nothing.
    /// </summary>
    public static IReadOnlyList<string>? Providers(AssistantDto? assistant)
    {
        if (assistant?.Lookups is not { Length: > 0 } names)
        {
            return null;
        }
        var named = names.Where(name => !string.IsNullOrWhiteSpace(name)).Select(name => name.Trim()).ToList();
        return named.Count > 0 ? named : null;
    }

    /// <summary>
    /// Whether this server offers lookups at all: providers to name, on an assistant this client may offer — one whose
    /// processor is named, since the lookup consent stands on top of the assistant's.
    /// </summary>
    public static bool Offered(AssistantDto? assistant) =>
        AssistantConsent.IsAvailable(assistant?.Processor) && Providers(assistant) is not null;

    /// <summary>
    /// The providers as one phrase for a <c>%@</c>: one on its own, two as "A and B", three as "A, B and C" — and the
    /// languages that join them differently (、と, 和) do so in their own translation of those two keys. More than three
    /// (the server sends at most three) folds the head into the first slot, so nobody goes unnamed.
    /// </summary>
    public static string Names(IReadOnlyList<string> names, IStringCatalog say)
    {
        ArgumentNullException.ThrowIfNull(names);
        ArgumentNullException.ThrowIfNull(say);
        return names.Count switch
        {
            0 => string.Empty,
            1 => names[0],
            2 => say.Format("%@ and %@", names[0], names[1]),
            _ => say.Format("%@, %@ and %@", string.Join(", ", names.Take(names.Count - 2)), names[^2], names[^1]),
        };
    }

    /// <summary>
    /// The consent screen's line about lookups — the one more line protocol.md asks for. Which of the two follows the
    /// owner's <c>ai_history</c>, the way the two family-chat lines already do: with it on, a mention's query may be shaped
    /// by recent messages too, and saying otherwise would be asking permission for something other than what happens.
    /// </summary>
    public static string ConsentLine(IReadOnlyList<string> names, bool familyHistory, IStringCatalog say) =>
        familyHistory
            ? say.Format(
                "If your family's owner turns on lookups, the assistant may send a short search query or place name it writes from your question — in the family chat, possibly from recent messages too — to %@, and its answer then lists its sources.",
                Names(names, say))
            : say.Format(
                "If your family's owner turns on lookups, the assistant may send a short search query or place name it writes from your question to %@, and its answer then lists its sources.",
                Names(names, say));

    /// <summary>
    /// What a member who has ALREADY agreed to the assistant reads before allowing lookups too: the lookup line, and the
    /// way back out. The rest of the assistant's disclosure they have read and agreed to.
    /// </summary>
    public static IReadOnlyList<string> Disclosure(IReadOnlyList<string> names, bool familyHistory, IStringCatalog say) =>
    [
        ConsentLine(names, familyHistory, say),
        say.Get("You can stop this at any time in Settings. What has already been sent cannot be taken back."),
    ];

    /// <summary>The footnote under the owner's <c>ai_lookups</c> switch, naming the providers.</summary>
    public static string SwitchFootnote(IReadOnlyList<string> names, IStringCatalog say) =>
        say.Format(
            "With this on, the assistant can look things up when a question needs it — the weather, the news, a fact it isn't sure of — in %@. Only a short search query or place name the assistant writes from the question is sent to them, never the conversation itself, and only when the member asking has agreed to it. Answers then list their sources. It is off unless you turn it on.",
            Names(names, say));

    /// <summary>The member's footnote in Settings, before and after they allowed lookups.</summary>
    public static string SettingsFootnote(IReadOnlyList<string> names, bool allowed, IStringCatalog say) =>
        allowed
            ? say.Format(
                "The assistant may send a short search query or place name it writes from your questions to %@. Stopping takes effect at once; what has already been sent cannot be taken back.",
                Names(names, say))
            : say.Format(
                "Until you allow lookups, the assistant answers you from what it already knows, and nothing from your questions is sent to %@.",
                Names(names, say));

    /// <summary>
    /// Which answer a pressed button is. With lookups offered, the first button is "Agree With Lookups" and the second
    /// "Agree Without Lookups"; without them, the first is "I Agree" and there is no second. Anything else — Not Now, Escape,
    /// a dismissed sheet — is <see cref="ConsentAnswer.NotNow"/>.
    /// </summary>
    public static ConsentAnswer AnswerFor(bool first, bool second, bool lookupsOffered) =>
        (first, second) switch
        {
            (true, _) => lookupsOffered ? ConsentAnswer.AgreeWithLookups : ConsentAnswer.Agree,
            (false, true) when lookupsOffered => ConsentAnswer.Agree,
            _ => ConsentAnswer.NotNow,
        };

    /// <summary>Has this member allowed lookups, as the server last said?</summary>
    public static bool Allowed(SessionState state) => !string.IsNullOrWhiteSpace(state.AssistantLookupConsentAt);

    /// <summary>
    /// Whether Settings offers this member the lookup agreement: a server that names its providers, and a member who has
    /// agreed to the assistant — the lookup consent can only be granted on top of it, and the assistant question itself
    /// already asks about lookups for a member who has not.
    /// </summary>
    public static bool InSettings(SessionState state) =>
        Offered(state.Assistant) && !string.IsNullOrWhiteSpace(state.AssistantConsentAt);

    /// <summary>
    /// Turn an answer into the writes it means, in the order the server needs them: the assistant consent first — unless
    /// <paramref name="assistantAgreed"/> says it is already held — and the lookup consent only on top of it, never after
    /// a failed first write. Null when everything asked for was written (or nothing was asked for); the first refusal
    /// otherwise. The caller re-reads <c>/me</c> either way: a first write that landed before a second failed is the truth.
    /// </summary>
    public static async Task<ApiError?> RecordAsync(
        ApiClient api, ConsentAnswer answer, bool assistantAgreed, CancellationToken ct = default)
    {
        ArgumentNullException.ThrowIfNull(api);
        if (answer == ConsentAnswer.NotNow)
        {
            return null;
        }
        if (!assistantAgreed)
        {
            var first = await api.SetAssistantConsent(true, ct).ConfigureAwait(false);
            if (!first.Ok)
            {
                return first.Error ?? ApiError.Transport("no answer");
            }
        }
        if (answer == ConsentAnswer.AgreeWithLookups)
        {
            var second = await api.SetAssistantLookupConsent(true, ct).ConfigureAwait(false);
            if (!second.Ok)
            {
                return second.Error ?? ApiError.Transport("no answer");
            }
        }
        return null;
    }

    /// <summary>
    /// May this bubble draw a preview card under its first web link? Everything may, as before, EXCEPT an assistant
    /// answer that is still being written, and one that ends with the server's sources footer (design decision 7).
    /// </summary>
    /// <remarks>
    /// <para>
    /// A finished lookup answer's links are its sources and its providers' credits — the server removed every other link
    /// the model wrote before adding the footer — so a card under any of them would have every device showing the answer
    /// contact a cited page.
    /// </para>
    /// <para>
    /// An answer still STREAMING is the model's raw words: the server filters links out of the finished body, not out of
    /// the deltas, so a link a web page talked the model into writing is in the stream until the finished row replaces it.
    /// A card fetched for it in the meantime is exactly the request the filter exists to prevent. The card waits for the
    /// finished row — and an answer that stopped part-way, whose words are never replaced, never gets one.
    /// </para>
    /// </remarks>
    /// <param name="stillWriting">Streamed text not yet replaced by the finished row (<c>AssistantAnswers.IsWriting</c>).</param>
    public static bool MayPreview(
        MessageDto message, string body, long me, bool assistantChat, long? assistantUserId, bool stillWriting)
    {
        ArgumentNullException.ThrowIfNull(message);
        if (!BubbleRules.IsAssistant(message, me, assistantChat, assistantUserId))
        {
            return true;
        }
        return !stillWriting && !SourcesFooter.EndsWithFooter(body);
    }
}
