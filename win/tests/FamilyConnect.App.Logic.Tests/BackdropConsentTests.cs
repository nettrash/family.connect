using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// "Draw a backdrop" is asked about exactly as a <c>/draw</c> is (docs/protocol.md, "Consenting to the assistant",
/// amended 2026-09-30): asked first when <c>/me</c> says no, asked after when the server says
/// <c>assistant_consent_required</c>, and drawn only on a yes the server kept.
/// </summary>
public sealed class BackdropConsentTests
{
    private const string Processor = "Microsoft — Azure OpenAI";
    private const string Agreed = "2026-09-19T08:12:04Z";

    private static readonly ApiError NotAgreed = new(ErrorCodes.AssistantConsentRequired, "agree first", 403);
    private static readonly AttachmentDto Picture = new(77, "photo", Width: 1024, Height: 512);

    /// <summary>What happened, in order: "ask" for the question, "draw" for the request.</summary>
    private sealed class Script(bool asksFirst, bool[] answers, (AttachmentDto?, ApiError?)[] draws)
    {
        private int asked;
        private int drawn;

        public List<string> Steps { get; } = [];

        public Task<BackdropConsent.Outcome> RunAsync() => BackdropConsent.RunAsync(
            () => asksFirst,
            () =>
            {
                Steps.Add("ask");
                return Task.FromResult(answers[asked++]);
            },
            () =>
            {
                Steps.Add("draw");
                return Task.FromResult(draws[drawn++]);
            });
    }

    [Fact]
    public void AskedFirstOnlyWhereSomebodyIsNamedAndNobodyHasAgreed()
    {
        Assert.True(BackdropConsent.AsksFirst(Processor, null));
        Assert.True(BackdropConsent.AsksFirst(Processor, "  "));
        Assert.False(BackdropConsent.AsksFirst(Processor, Agreed));
        // Nobody named: nothing can be asked, and the button is not offered there at all (PictureHint.OffersBackdrop).
        Assert.False(BackdropConsent.AsksFirst(null, null));
        Assert.False(BackdropConsent.AsksFirst("", null));
    }

    [Fact]
    public void OnlyTheServersAgreeFirstIsAnsweredWithTheQuestion()
    {
        Assert.True(BackdropConsent.AsksAfter(NotAgreed, askedAlready: false));
        // A yes went in and the server still refused: said as a failure, not asked again in a loop.
        Assert.False(BackdropConsent.AsksAfter(NotAgreed, askedAlready: true));
        Assert.False(BackdropConsent.AsksAfter(new ApiError(ErrorCodes.PictureRefused, "x", 400), askedAlready: false));
        Assert.False(BackdropConsent.AsksAfter(new ApiError(ErrorCodes.PicturesUnavailable, "x", 403), askedAlready: false));
        Assert.False(BackdropConsent.AsksAfter(ApiError.Transport("x"), askedAlready: false));
        Assert.False(BackdropConsent.AsksAfter(null, askedAlready: false));
    }

    /// <summary>Agreed already: drawn at once, nothing asked.</summary>
    [Fact]
    public async Task AnAuthorWhoAgreedIsNotAsked()
    {
        var script = new Script(asksFirst: false, [], [(Picture, null)]);
        var outcome = await script.RunAsync();
        Assert.Equal(["draw"], script.Steps);
        Assert.Equal(77, outcome.Drawn!.Id);
        Assert.Null(outcome.Error);
        Assert.False(outcome.Declined);
    }

    /// <summary>NOTHING REACHES THE MODEL UNASKED: the question first, and the title sent only on a yes.</summary>
    [Fact]
    public async Task NotYetAgreedIsAskedBeforeAnythingIsSent()
    {
        var script = new Script(asksFirst: true, [true], [(Picture, null)]);
        var outcome = await script.RunAsync();
        Assert.Equal(["ask", "draw"], script.Steps);
        Assert.Equal(77, outcome.Drawn!.Id);
    }

    /// <summary>Not Now sends nothing, and there is nothing to be told: no request, no failure line.</summary>
    [Fact]
    public async Task NotNowSendsNothingAndSaysNothing()
    {
        var script = new Script(asksFirst: true, [false], []);
        var outcome = await script.RunAsync();
        Assert.Equal(["ask"], script.Steps);
        Assert.True(outcome.Declined);
        Assert.Null(outcome.Drawn);
        Assert.Null(outcome.Error);
    }

    /// <summary>
    /// The server says "agree first" to a member this device thought had agreed — withdrawn somewhere else: the
    /// question, then the drawing again on a yes.
    /// </summary>
    [Fact]
    public async Task TheServersAgreeFirstIsAskedAndThenDrawnAgain()
    {
        var script = new Script(asksFirst: false, [true], [(null, NotAgreed), (Picture, null)]);
        var outcome = await script.RunAsync();
        Assert.Equal(["draw", "ask", "draw"], script.Steps);
        Assert.Equal(77, outcome.Drawn!.Id);
        Assert.Null(outcome.Error);
    }

    [Fact]
    public async Task TheServersAgreeFirstAnsweredNotNowDrawsNothingMore()
    {
        var script = new Script(asksFirst: false, [false], [(null, NotAgreed)]);
        var outcome = await script.RunAsync();
        Assert.Equal(["draw", "ask"], script.Steps);
        Assert.True(outcome.Declined);
        Assert.Null(outcome.Error);
    }

    /// <summary>Asked once per click, whichever way: a refusal after a yes is the failure it is, never a loop.</summary>
    [Fact]
    public async Task AskedAtMostOncePerClick()
    {
        var first = new Script(asksFirst: true, [true], [(null, NotAgreed)]);
        var outcome = await first.RunAsync();
        Assert.Equal(["ask", "draw"], first.Steps);
        Assert.Equal(ErrorCodes.AssistantConsentRequired, outcome.Error!.Code);
        Assert.False(outcome.Declined);

        var after = new Script(asksFirst: false, [true], [(null, NotAgreed), (null, NotAgreed)]);
        outcome = await after.RunAsync();
        Assert.Equal(["draw", "ask", "draw"], after.Steps);
        Assert.Equal(ErrorCodes.AssistantConsentRequired, outcome.Error!.Code);
    }

    /// <summary>Every other failure is said as it always was, and not asked about.</summary>
    [Fact]
    public async Task OtherFailuresAreNotAskedAbout()
    {
        var refused = new ApiError(ErrorCodes.PictureRefused, "x", 400);
        var script = new Script(asksFirst: false, [], [(null, refused)]);
        var outcome = await script.RunAsync();
        Assert.Equal(["draw"], script.Steps);
        Assert.Same(refused, outcome.Error);
        Assert.False(outcome.Declined);
    }

    /// <summary>
    /// Said where the question could not be put: the plain failure a backdrop shows, which the protocol says an older
    /// client shows for this answer anyway.
    /// </summary>
    [Fact]
    public void AnUnansweredAgreeFirstIsThePlainFailure() =>
        Assert.Equal("Couldn't draw that.", NoteSheetText.BackdropFailure(NotAgreed, EnglishCatalog.Instance));
}
