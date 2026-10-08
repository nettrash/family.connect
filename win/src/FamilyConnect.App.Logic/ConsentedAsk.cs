using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>
/// One request that sends something of the member's to the model, asked about the way a <c>/draw</c> is
/// (docs/protocol.md, "Consenting to the assistant"): ASKED FIRST when <c>/me</c> says this member has not agreed, and
/// ASKED AFTER — then sent again, ONCE — when the server answers <c>assistant_consent_required</c> anyway, which is an
/// answer withdrawn on another device that this one has not heard about. A second refusal after a yes is the failure
/// it is, never a loop.
/// </summary>
/// <remarks>
/// The board's "Draw a backdrop" (<see cref="BackdropConsent"/>) and a recording's "Show text" (<see cref="TranscriptModel"/>)
/// are both this, which is why the order lives in one place and is pinned once.
/// </remarks>
public static class ConsentedAsk
{
    /// <summary>How one click ended.</summary>
    /// <param name="Value">The answer, when there is one.</param>
    /// <param name="Error">Why there is none, to be said — null on an answer, and null when the member said Not Now.</param>
    /// <param name="Declined">The member was asked and did not agree: nothing was sent, and there is nothing to say.</param>
    public sealed record Outcome<T>(T? Value, ApiError? Error, bool Declined) where T : class;

    /// <param name="asksFirst">Read when the click lands, from the session as it is then.</param>
    /// <param name="ask">The consent question, and the answer recorded on the server: true only on a yes it has kept.</param>
    /// <param name="request">The request itself.</param>
    public static async Task<Outcome<T>> RunAsync<T>(
        Func<bool> asksFirst,
        Func<Task<bool>> ask,
        Func<Task<(T? Value, ApiError? Error)>> request) where T : class
    {
        ArgumentNullException.ThrowIfNull(asksFirst);
        ArgumentNullException.ThrowIfNull(ask);
        ArgumentNullException.ThrowIfNull(request);
        var asked = false;
        if (asksFirst())
        {
            asked = true;
            if (!await ask().ConfigureAwait(true))
            {
                return new Outcome<T>(null, null, Declined: true);
            }
        }
        var (value, error) = await request().ConfigureAwait(true);
        if (BackdropConsent.AsksAfter(error, asked))
        {
            if (!await ask().ConfigureAwait(true))
            {
                return new Outcome<T>(null, null, Declined: true);
            }
            (value, error) = await request().ConfigureAwait(true);
        }
        return new Outcome<T>(value, error, Declined: false);
    }
}
