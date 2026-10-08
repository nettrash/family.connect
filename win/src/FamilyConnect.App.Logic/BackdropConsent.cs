using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>
/// "Draw a backdrop" is asked about exactly as a <c>/draw</c> is (docs/protocol.md, "Consenting to the assistant",
/// amended 2026-09-30): the event's title is the author's own words going to the model, so the server answers
/// <c>assistant_consent_required</c> (403) and sends nothing until the author has agreed.
/// </summary>
/// <remarks>
/// <para>
/// <b>ASKED FIRST</b> when <c>/me</c> says this member has not agreed, the way the Send button asks: the question, and
/// the drawing only on a yes the server has recorded. <b>ASKED AFTER</b> when the server says so anyway — an answer
/// withdrawn on another device, which this one's <c>/me</c> has not heard about — and then drawn again, ONCE: a second
/// refusal after a yes is shown as the failure it is, never asked about in a loop.
/// </para>
/// <para>
/// No XAML in here, like the rest of this assembly: the sheet hands in the question and the request, and the order they
/// run in is pinned by an ordinary unit test.
/// </para>
/// </remarks>
public static class BackdropConsent
{
    /// <summary>How a click on "Draw a backdrop" ended.</summary>
    /// <param name="Drawn">The new picture, when one was drawn.</param>
    /// <param name="Error">Why nothing was drawn, to be said — null when drawn, and null when the member said Not Now.</param>
    /// <param name="Declined">The member was asked and did not agree: nothing was sent, and there is nothing to say.</param>
    public sealed record Outcome(AttachmentDto? Drawn, ApiError? Error, bool Declined);

    /// <summary>Must the author be asked before the title goes to the model? Only where there is somebody named to agree to.</summary>
    public static bool AsksFirst(string? processor, string? agreedAt) =>
        AssistantConsent.IsAvailable(processor) && string.IsNullOrWhiteSpace(agreedAt);

    /// <summary>
    /// Does this failure call for the question — the server's own "agree first" — rather than a sentence? Not when the
    /// question was already asked for this click: the yes went in and the server still refused, and asking again would
    /// be a loop.
    /// </summary>
    public static bool AsksAfter(ApiError? error, bool askedAlready) =>
        !askedAlready && error?.Code == ErrorCodes.AssistantConsentRequired;

    /// <summary>One click, from the question (when it is needed) to the answer — in <see cref="ConsentedAsk"/>'s order.</summary>
    /// <param name="asksFirst">Read when the click lands, from the session as it is then (<see cref="AsksFirst"/>).</param>
    /// <param name="ask">The consent question, and the answer recorded on the server: true only on a yes it has kept.</param>
    /// <param name="draw">The request itself.</param>
    public static async Task<Outcome> RunAsync(
        Func<bool> asksFirst,
        Func<Task<bool>> ask,
        Func<Task<(AttachmentDto? Drawn, ApiError? Error)>> draw)
    {
        ArgumentNullException.ThrowIfNull(draw);
        var outcome = await ConsentedAsk.RunAsync<AttachmentDto>(
            asksFirst, ask, async () => await draw().ConfigureAwait(true)).ConfigureAwait(true);
        return new Outcome(outcome.Value, outcome.Error, outcome.Declined);
    }
}
