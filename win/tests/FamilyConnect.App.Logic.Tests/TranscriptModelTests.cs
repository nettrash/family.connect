using System.Net;
using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// "Show text", from the click to the text under the player (docs/protocol.md, "Transcripts on request"): the consent
/// question in the backdrop's order, the request, the answer kept on this device, and what is drawn while it all runs.
/// </summary>
public sealed class TranscriptModelTests : IDisposable
{
    private const string Path = "/chats/42/messages/1338/attachments/34/transcript";
    private const string Said = """{"transcript": {"text": "Мы будем в шесть", "language": "ru"}}""";
    private const string NotAgreed = """{"error": {"code": "assistant_consent_required", "message": "agree first"}}""";

    private readonly Database cache = Database.OpenInMemory();
    private readonly TranscriptStore store;
    private readonly List<long> changed = [];

    public TranscriptModelTests() => store = new TranscriptStore(cache);

    public void Dispose() => cache.Dispose();

    private TranscriptModel Model(Server server)
    {
        var model = new TranscriptModel(
            new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken")),
            store,
            () => DateTimeOffset.UnixEpoch);
        model.Changed += changed.Add;
        return model;
    }

    private static Task ShowAsync(TranscriptModel model, bool asksFirst = false, Func<Task<bool>>? ask = null) =>
        model.ShowAsync(42, 1338, 34, () => asksFirst, ask ?? (() => Task.FromResult(true)));

    [Fact]
    public async Task TheAnswerIsShownAndKeptOnThisDevice()
    {
        var server = new Server().On(Path, Said);
        var model = Model(server);
        Assert.Equal(TranscriptLook.Closed, model.Look(34));
        Assert.False(model.Holds(34));

        await ShowAsync(model);

        Assert.Equal(new TranscriptLook(TranscriptPhase.Open, "Мы будем в шесть"), model.Look(34));
        Assert.Equal(new KeptTranscript(34, "Мы будем в шесть", "ru"), store.Find(34));
        Assert.True(model.Holds(34));
        // The stored form: no body at all.
        Assert.Equal(["/api/v1" + Path], server.Asked);
        Assert.Empty(server.Bodies);
        // "Getting the text…" was drawn before the answer, then the answer.
        Assert.Equal([34L, 34L], changed);
    }

    /// <summary>
    /// KEPT MEANS NOT ASKED AGAIN: folded away and shown again — or shown by a model built after a restart, over the
    /// same cache — the text comes from this device and no request goes anywhere.
    /// </summary>
    [Fact]
    public async Task HideAndShowAgainAsksNobody()
    {
        var server = new Server().On(Path, Said);
        var model = Model(server);
        await ShowAsync(model);

        model.Hide(34);
        Assert.Equal(TranscriptLook.Closed, model.Look(34));
        Assert.NotNull(store.Find(34));
        await ShowAsync(model);
        Assert.Equal(TranscriptPhase.Open, model.Look(34).Phase);

        var reopened = Model(server);
        Assert.True(reopened.Holds(34));
        await ShowAsync(reopened, asksFirst: true, ask: () => throw new InvalidOperationException("never asked"));
        Assert.Equal("Мы будем в шесть", reopened.Look(34).Text);
        Assert.Single(server.Asked);
    }

    /// <summary>SILENCE IS AN ANSWER: kept, and drawn as "No speech" — not as a failure, and not asked about again.</summary>
    [Fact]
    public async Task SilenceIsKeptAsAnAnswer()
    {
        var server = new Server().On(Path, """{"transcript": {"text": ""}}""");
        var model = Model(server);

        await ShowAsync(model);

        Assert.True(model.Look(34).NoSpeech);
        Assert.Equal(string.Empty, store.Find(34)!.Text);
        model.Hide(34);
        await ShowAsync(model);
        Assert.Single(server.Asked);
    }

    /// <summary>ASKED FIRST when <c>/me</c> says no — and on Not Now nothing is sent and nothing is said.</summary>
    [Fact]
    public async Task NotNowSendsNothing()
    {
        var server = new Server().On(Path, Said);
        var model = Model(server);
        var asked = 0;

        await ShowAsync(model, asksFirst: true, ask: () => { asked++; return Task.FromResult(false); });

        Assert.Equal(1, asked);
        Assert.Empty(server.Asked);
        Assert.Equal(TranscriptLook.Closed, model.Look(34));
        Assert.Null(store.Find(34));
    }

    /// <summary>ASKED FIRST, and on a yes the request goes.</summary>
    [Fact]
    public async Task AYesSendsIt()
    {
        var server = new Server().On(Path, Said);
        var model = Model(server);

        await ShowAsync(model, asksFirst: true);

        Assert.Single(server.Asked);
        Assert.Equal(TranscriptPhase.Open, model.Look(34).Phase);
    }

    /// <summary>
    /// ASKED AFTER: the server says agree first although this device thought it had — the question, then the request
    /// again, ONCE; the board's backdrop does exactly this.
    /// </summary>
    [Fact]
    public async Task TheServersAgreeFirstAsksAndThenAsksAgainOnce()
    {
        var server = new Server().Then(Path, (HttpStatusCode.Forbidden, NotAgreed), (HttpStatusCode.OK, Said));
        var model = Model(server);
        var asked = 0;

        await ShowAsync(model, ask: () => { asked++; return Task.FromResult(true); });

        Assert.Equal(1, asked);
        Assert.Equal(2, server.Asked.Count);
        Assert.Equal("Мы будем в шесть", model.Look(34).Text);
    }

    /// <summary>A second refusal after a yes is the failure it is — said, and never asked about in a loop.</summary>
    [Fact]
    public async Task ASecondAgreeFirstIsAFailureNotALoop()
    {
        var server = new Server().On(Path, NotAgreed, HttpStatusCode.Forbidden);
        var model = Model(server);
        var asked = 0;

        await ShowAsync(model, ask: () => { asked++; return Task.FromResult(true); });

        Assert.Equal(1, asked);
        Assert.Equal(2, server.Asked.Count);
        var look = model.Look(34);
        Assert.Equal(TranscriptPhase.Failed, look.Phase);
        Assert.Equal(ErrorCodes.AssistantConsentRequired, look.Error!.Code);
        Assert.Null(store.Find(34));
    }

    /// <summary>The provider refused: said, final, and nothing kept — asking again is the member's to try, not ours.</summary>
    [Fact]
    public async Task ARefusalIsSaidAndNothingIsKept()
    {
        var server = new Server().On(
            Path, """{"error": {"code": "transcript_refused", "message": "no"}}""", HttpStatusCode.BadRequest);
        var model = Model(server);

        await ShowAsync(model);

        var look = model.Look(34);
        Assert.Equal(TranscriptPhase.Failed, look.Phase);
        Assert.Equal(ErrorCodes.TranscriptRefused, look.Error!.Code);
        Assert.False(TranscriptRules.MayRetry(look.Error));
        Assert.Null(store.Find(34));
    }

    /// <summary>
    /// A transient failure may be retried — and the retry is an ordinary click, which then shows the answer the server
    /// finished and kept in the meantime.
    /// </summary>
    [Fact]
    public async Task AFailedAskCanBeAskedAgain()
    {
        var server = new Server().Then(
            Path,
            (HttpStatusCode.InternalServerError, """{"error": {"code": "internal", "message": "provider"}}"""),
            (HttpStatusCode.OK, Said));
        var model = Model(server);

        await ShowAsync(model);
        Assert.Equal(TranscriptPhase.Failed, model.Look(34).Phase);
        Assert.True(TranscriptRules.MayRetry(model.Look(34).Error!));
        // Never repeated by itself: a write.
        Assert.Single(server.Asked);

        await ShowAsync(model);
        Assert.Equal(TranscriptPhase.Open, model.Look(34).Phase);
        Assert.Equal(2, server.Asked.Count);
    }

    /// <summary>A 2xx with no transcript in it is not an answer: not kept, said as "not available".</summary>
    [Fact]
    public async Task AnAnswerWithNoTranscriptIsNotKept()
    {
        var server = new Server().On(Path, "{}");
        var model = Model(server);

        await ShowAsync(model);

        Assert.Equal(TranscriptPhase.Failed, model.Look(34).Phase);
        Assert.Equal(ErrorCodes.Validation, model.Look(34).Error!.Code);
        Assert.Null(store.Find(34));
    }

    /// <summary>
    /// ONE ASK PER RECORDING AT A TIME: a second click while the first is still waiting — on the server or on the
    /// consent question — does nothing; while it waits on the server it is drawn as "Getting the text…".
    /// </summary>
    [Fact]
    public async Task ASecondClickWhileAskingDoesNothing()
    {
        var release = new TaskCompletionSource<(HttpStatusCode, string?)>(TaskCreationOptions.RunContinuationsAsynchronously);
        var server = new Server().OnAsync(Path, () => release.Task);
        var model = Model(server);

        var first = ShowAsync(model);
        await WaitUntil(() => server.Asked.Count == 1);
        Assert.Equal(TranscriptPhase.Asking, model.Look(34).Phase);
        // Bounded: a second click that DID ask would wait on the held answer for ever, and a hung suite proves nothing.
        await ShowAsync(model).WaitAsync(TimeSpan.FromSeconds(5));
        Assert.Single(server.Asked);

        release.SetResult((HttpStatusCode.OK, Said));
        await first;
        Assert.Equal(TranscriptPhase.Open, model.Look(34).Phase);
        Assert.Single(server.Asked);
    }

    /// <summary>Two recordings are two asks, and each is drawn on its own.</summary>
    [Fact]
    public async Task EachRecordingIsItsOwn()
    {
        var server = new Server()
            .On(Path, Said)
            .On("/chats/42/messages/1338/attachments/35/transcript", """{"transcript": {"text": "and the bread"}}""");
        var model = Model(server);

        await ShowAsync(model);
        await model.ShowAsync(42, 1338, 35, () => false, () => Task.FromResult(true));

        Assert.Equal("Мы будем в шесть", model.Look(34).Text);
        Assert.Equal("and the bread", model.Look(35).Text);
        model.Hide(35);
        Assert.Equal(TranscriptPhase.Open, model.Look(34).Phase);
    }

    /// <summary>A session that ended forgets what was on screen; the cache is wiped with everything else.</summary>
    [Fact]
    public async Task ClearForgetsWhatWasShown()
    {
        var model = Model(new Server().On(Path, Said));
        await ShowAsync(model);
        model.Clear();
        Assert.Equal(TranscriptLook.Closed, model.Look(34));
    }

    /// <summary>
    /// A SESSION THAT ENDED WHILE THE ANSWER WAS ON ITS WAY keeps nothing of it: the sign-out wiped the cache, and an
    /// answer landing afterwards must not write the last account's words back for the next one to read without asking
    /// — least of all an answer made from supplied sound, which nobody else may ever be handed. The next session's
    /// own click on the same recording is not blocked by the old request either.
    /// </summary>
    [Fact]
    public async Task AnAnswerThatLandsAfterTheSessionEndedIsNotKept()
    {
        var release = new TaskCompletionSource<(HttpStatusCode, string?)>(TaskCreationOptions.RunContinuationsAsynchronously);
        var held = true;
        var server = new Server().OnAsync(
            Path,
            () => held ? release.Task : Task.FromResult((HttpStatusCode.OK, (string?)"""{"transcript": {"text": "the next account's"}}""")));
        var model = Model(server);

        var first = ShowAsync(model);
        await WaitUntil(() => server.Asked.Count == 1);
        Assert.Equal(TranscriptPhase.Asking, model.Look(34).Phase);

        // Signed out while it waits.
        model.Clear();
        held = false;
        Assert.Equal(TranscriptLook.Closed, model.Look(34));

        // The next account clicks the same recording: asked afresh, not swallowed as "already asking".
        await ShowAsync(model).WaitAsync(TimeSpan.FromSeconds(5));
        Assert.Equal(2, server.Asked.Count);
        Assert.Equal("the next account's", model.Look(34).Text);

        // The old answer lands now — and changes nothing.
        release.SetResult((HttpStatusCode.OK, Said));
        await first.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.Equal("the next account's", model.Look(34).Text);
        Assert.Equal("the next account's", store.Find(34)!.Text);
    }

    /// <summary>The same, with nothing asked afresh: the old answer is neither kept nor drawn.</summary>
    [Fact]
    public async Task AnAnswerAfterSignOutLeavesNothingBehind()
    {
        var release = new TaskCompletionSource<(HttpStatusCode, string?)>(TaskCreationOptions.RunContinuationsAsynchronously);
        var server = new Server().OnAsync(Path, () => release.Task);
        var model = Model(server);
        var maker = new Maker(() => SuppliedSound.Made(M4a));

        var first = SupplyAsync(model, maker);
        await WaitUntil(() => server.Asked.Count == 1);
        model.Clear();
        release.SetResult((HttpStatusCode.OK, Said));
        await first.WaitAsync(TimeSpan.FromSeconds(5));

        Assert.Null(store.Find(34));
        Assert.False(model.Holds(34));
        Assert.Equal(TranscriptLook.Closed, model.Look(34));
    }

    /// <summary>Hide is for open text only: it does not wipe a failure the member has not read.</summary>
    [Fact]
    public async Task HideLeavesAFailureAlone()
    {
        var model = Model(new Server().On(
            Path, """{"error": {"code": "not_transcribable", "message": "no"}}""", HttpStatusCode.BadRequest));
        await ShowAsync(model);
        model.Hide(34);
        Assert.Equal(TranscriptPhase.Failed, model.Look(34).Phase);
    }

    // ---- the device's own sound (phase 3) --------------------------------------------------------------------------

    private static readonly byte[] M4a = [0, 0, 0, 0x18, (byte)'f', (byte)'t', (byte)'y', (byte)'p', (byte)'M', (byte)'4', (byte)'A', (byte)' ', 1, 2, 3];

    /// <summary>A maker that counts its calls and hands back what it is told to.</summary>
    private sealed class Maker(Func<SuppliedSound> made, long maxBytes = 26_214_400, long? durationMs = null)
    {
        public int Calls { get; private set; }

        public SoundSource Source => new(maxBytes, _ =>
        {
            Calls++;
            return Task.FromResult(made());
        }, durationMs);
    }

    private static Task SupplyAsync(
        TranscriptModel model, Maker maker, TranscriptForm form = TranscriptForm.Supplied, bool asksFirst = false,
        Func<Task<bool>>? ask = null) =>
        model.ShowAsync(42, 1338, 34, form, maker.Source, () => asksFirst, ask ?? (() => Task.FromResult(true)));

    /// <summary>
    /// A VIDEO'S TEXT: the device's sound goes as the one multipart <c>audio</c> part, the answer is shown, and it is kept
    /// on THIS device marked as supplied — the server never kept it.
    /// </summary>
    [Fact]
    public async Task SuppliedSoundIsSentAndKeptOnThisDeviceOnly()
    {
        var server = new Server().On(Path, Said);
        var model = Model(server);
        var maker = new Maker(() => SuppliedSound.Made(M4a));

        await SupplyAsync(model, maker);

        Assert.Equal(1, maker.Calls);
        Assert.Equal(["/api/v1" + Path], server.Asked);
        var body = Assert.Single(server.Bodies);
        Assert.Contains("name=audio", body, StringComparison.Ordinal);
        Assert.Contains("filename=audio.m4a", body, StringComparison.Ordinal);
        Assert.Contains("Content-Type: audio/mp4", body, StringComparison.Ordinal);
        Assert.Contains("ftypM4A", body, StringComparison.Ordinal);
        Assert.Equal(new TranscriptLook(TranscriptPhase.Open, "Мы будем в шесть"), model.Look(34));
        Assert.Equal(new KeptTranscript(34, "Мы будем в шесть", "ru", Supplied: true), store.Find(34));

        // Kept means not made again, and not asked again.
        model.Hide(34);
        await SupplyAsync(model, maker);
        Assert.Equal(1, maker.Calls);
        Assert.Single(server.Asked);
    }

    /// <summary>
    /// THE DEVICE CANNOT READ IT: no track, a codec it lacks — "Couldn't read the sound in this file.", final, with
    /// nothing sent anywhere and nothing kept.
    /// </summary>
    [Fact]
    public async Task SoundTheDeviceCannotReadIsSaidSo()
    {
        var server = new Server().On(Path, Said);
        var model = Model(server);

        await SupplyAsync(model, new Maker(() => SuppliedSound.Failed(TranscriptSound.Unreadable)));

        var look = model.Look(34);
        Assert.Equal(TranscriptPhase.Failed, look.Phase);
        Assert.Equal(TranscriptSound.Unreadable, look.Error);
        Assert.False(TranscriptRules.MayRetry(look.Error!));
        Assert.Empty(server.Asked);
        Assert.Null(store.Find(34));
    }

    /// <summary>
    /// THE SIZE BOUND, HELD HERE TOO: sound over the ceiling ("too long"), empty or not MPEG-4 ("couldn't read") — what
    /// the server would refuse — is never uploaded; and a maker that throws is "couldn't read", never an exception at a
    /// click.
    /// </summary>
    [Fact]
    public async Task SoundTheServerWouldRefuseIsNeverUploaded()
    {
        var server = new Server().On(Path, Said);
        var model = Model(server);

        await SupplyAsync(model, new Maker(() => SuppliedSound.Made(M4a), maxBytes: M4a.Length - 1));
        Assert.Equal(TranscriptSound.TooLong, model.Look(34).Error);
        await SupplyAsync(model, new Maker(() => SuppliedSound.Made(M4a), maxBytes: M4a.Length));
        Assert.Equal(TranscriptPhase.Open, model.Look(34).Phase);
        Assert.Single(server.Asked);

        var other = Model(new Server().On("/chats/42/messages/1338/attachments/35/transcript", Said));
        var notMpeg4 = new Maker(() => SuppliedSound.Made("OggS\0\0\0\0\0\0\0\0"u8.ToArray()));
        await other.ShowAsync(42, 1338, 35, TranscriptForm.Supplied, notMpeg4.Source, () => false, () => Task.FromResult(true));
        Assert.Equal(TranscriptSound.Unreadable, other.Look(35).Error);
        var empty = new Maker(() => SuppliedSound.Made([]));
        await other.ShowAsync(42, 1338, 35, TranscriptForm.Supplied, empty.Source, () => false, () => Task.FromResult(true));
        Assert.Equal(TranscriptSound.Unreadable, other.Look(35).Error);
        var throws = new Maker(() => throw new InvalidOperationException("codec"));
        await other.ShowAsync(42, 1338, 35, TranscriptForm.Supplied, throws.Source, () => false, () => Task.FromResult(true));
        Assert.Equal(TranscriptSound.Unreadable, other.Look(35).Error);
        Assert.Null(store.Find(35));
    }

    /// <summary>
    /// TOO LONG BY ITS OWN LENGTH: a recording whose stated length could not fit the ceiling even at 64 kbit/s is told so
    /// at the press — "This recording is too long to turn into text." — with nothing downloaded, nothing made and
    /// nothing sent. The action was offered; only its answer says why there is no text.
    /// </summary>
    [Fact]
    public async Task ARecordingTooLongByItsLengthIsToldWithoutTheWork()
    {
        var server = new Server().On(Path, Said);
        var model = Model(server);
        var maker = new Maker(() => SuppliedSound.Made(M4a), durationMs: 55 * 60_000L);

        await SupplyAsync(model, maker);

        Assert.Equal(TranscriptSound.TooLong, model.Look(34).Error);
        Assert.False(TranscriptRules.MayRetry(model.Look(34).Error!));
        Assert.Equal(0, maker.Calls);
        Assert.Empty(server.Asked);

        // A length that fits is made and sent as before.
        var fits = new Maker(() => SuppliedSound.Made(M4a), durationMs: 50 * 60_000L);
        await model.ShowAsync(42, 1338, 35, TranscriptForm.Supplied, fits.Source, () => false, () => Task.FromResult(true));
        Assert.Equal(1, fits.Calls);
    }

    /// <summary>
    /// A DOWNLOAD THAT FAILED says what it was: a lost connection is "try again" — and the next click makes the sound
    /// again — while an attachment that is gone is "not available".
    /// </summary>
    [Fact]
    public async Task AFailedDownloadIsSaidAsWhatItWas()
    {
        var server = new Server().On(Path, Said);
        var model = Model(server);
        var answers = new Queue<SuppliedSound>([
            SuppliedSound.Failed(ApiError.Transport("connection reset")),
            SuppliedSound.Made(M4a),
        ]);
        var maker = new Maker(() => answers.Dequeue());

        await SupplyAsync(model, maker);
        Assert.Equal(TranscriptPhase.Failed, model.Look(34).Phase);
        Assert.True(TranscriptRules.MayRetry(model.Look(34).Error!));
        Assert.Empty(server.Asked);

        await SupplyAsync(model, maker);
        Assert.Equal(2, maker.Calls);
        Assert.Equal(TranscriptPhase.Open, model.Look(34).Phase);

        var gone = Model(new Server());
        await gone.ShowAsync(
            42, 1338, 35, TranscriptForm.Supplied,
            new Maker(() => SuppliedSound.Failed(new ApiError(ErrorCodes.AttachmentNotFound, "gone", 404))).Source,
            () => false, () => Task.FromResult(true));
        Assert.False(TranscriptRules.MayRetry(gone.Look(35).Error!));
    }

    /// <summary>NOT NOW MAKES NOTHING: the consent question comes first, so nothing is downloaded or decoded for a no.</summary>
    [Fact]
    public async Task NotNowMakesNoSound()
    {
        var server = new Server().On(Path, Said);
        var model = Model(server);
        var maker = new Maker(() => SuppliedSound.Made(M4a));

        await SupplyAsync(model, maker, asksFirst: true, ask: () => Task.FromResult(false));

        Assert.Equal(0, maker.Calls);
        Assert.Empty(server.Asked);
        Assert.Equal(TranscriptLook.Closed, model.Look(34));
    }

    /// <summary>
    /// AGREE FIRST, FROM THE SERVER: the question, then the request again — with the sound already made, which is made
    /// ONCE per click however many times it is sent.
    /// </summary>
    [Fact]
    public async Task TheSoundIsMadeOnceThoughTheRequestGoesTwice()
    {
        var server = new Server().Then(Path, (HttpStatusCode.Forbidden, NotAgreed), (HttpStatusCode.OK, Said));
        var model = Model(server);
        var maker = new Maker(() => SuppliedSound.Made(M4a));

        await SupplyAsync(model, maker);

        Assert.Equal(1, maker.Calls);
        Assert.Equal(2, server.Asked.Count);
        Assert.Equal(2, server.Bodies.Count);
        Assert.True(store.Find(34)!.Supplied);
    }

    /// <summary>
    /// THE STORED FORM REFUSED AS NOT TRANSCRIBABLE — the server's ceiling or list is not what this device read — falls
    /// back to the device's sound, once ("a client that sent no body may send the sound track instead").
    /// </summary>
    [Fact]
    public async Task TheStoredFormFallsBackToTheDevicesSound()
    {
        var server = new Server().Then(
            Path,
            (HttpStatusCode.BadRequest, """{"error": {"code": "not_transcribable", "message": "over"}}"""),
            (HttpStatusCode.OK, Said));
        var model = Model(server);
        var maker = new Maker(() => SuppliedSound.Made(M4a));

        await SupplyAsync(model, maker, TranscriptForm.Stored);

        Assert.Equal(1, maker.Calls);
        Assert.Equal(2, server.Asked.Count);
        // The first went with no body, the second with the sound.
        Assert.Single(server.Bodies);
        Assert.Equal(TranscriptPhase.Open, model.Look(34).Phase);
        Assert.True(store.Find(34)!.Supplied);
    }

    /// <summary>The stored form answered is the stored form's: no sound made, and kept as the shared answer it is.</summary>
    [Fact]
    public async Task TheStoredFormAnsweredMakesNoSound()
    {
        var server = new Server().On(Path, Said);
        var model = Model(server);
        var maker = new Maker(() => SuppliedSound.Made(M4a));

        await SupplyAsync(model, maker, TranscriptForm.Stored);

        Assert.Equal(0, maker.Calls);
        Assert.Empty(server.Bodies);
        Assert.False(store.Find(34)!.Supplied);
    }

    /// <summary>
    /// Other refusals of the stored form are final as they are — not a reason to upload anything — and a stored form with
    /// no maker cannot fall back.
    /// </summary>
    [Fact]
    public async Task OnlyNotTranscribableFallsBack()
    {
        var server = new Server().On(
            Path, """{"error": {"code": "transcript_not_allowed", "message": "no"}}""", HttpStatusCode.Forbidden);
        var model = Model(server);
        var maker = new Maker(() => SuppliedSound.Made(M4a));

        await SupplyAsync(model, maker, TranscriptForm.Stored);
        Assert.Equal(0, maker.Calls);
        Assert.Equal(ErrorCodes.TranscriptNotAllowed, model.Look(34).Error!.Code);

        var plain = Model(new Server().On(
            Path, """{"error": {"code": "not_transcribable", "message": "no"}}""", HttpStatusCode.BadRequest));
        await plain.ShowAsync(42, 1338, 34, TranscriptForm.Stored, null, () => false, () => Task.FromResult(true));
        Assert.Equal(ErrorCodes.NotTranscribable, plain.Look(34).Error!.Code);
    }

    /// <summary>
    /// NOTHING TO ASK WITH — no form, or the device's form with no maker — is "not available" and sends nothing; but text
    /// this device already holds is shown whatever the form.
    /// </summary>
    [Fact]
    public async Task NoFormSendsNothingButHeldTextIsShown()
    {
        var server = new Server();
        var model = Model(server);

        await model.ShowAsync(42, 1338, 34, TranscriptForm.None, null, () => false, () => Task.FromResult(true));
        Assert.Equal(ErrorCodes.NotTranscribable, model.Look(34).Error!.Code);
        await model.ShowAsync(42, 1338, 34, TranscriptForm.Supplied, null, () => false, () => Task.FromResult(true));
        Assert.Equal(ErrorCodes.NotTranscribable, model.Look(34).Error!.Code);
        Assert.Empty(server.Asked);

        store.Keep(new KeptTranscript(34, "Back at six", Supplied: true), DateTimeOffset.UnixEpoch);
        await model.ShowAsync(42, 1338, 34, TranscriptForm.None, null, () => true, () => throw new InvalidOperationException("never asked"));
        Assert.Equal(new TranscriptLook(TranscriptPhase.Open, "Back at six"), model.Look(34));
        Assert.Empty(server.Asked);
    }

    /// <summary>
    /// A SUPPLIED ANSWER NEVER REPLACES THE SHARED ONE: when this device already holds the answer from the stored copy, a
    /// later supplied answer for the same attachment does not overwrite it.
    /// </summary>
    [Fact]
    public async Task ASuppliedAnswerNeverReplacesTheStoredOne()
    {
        store.Keep(new KeptTranscript(34, "from the stored copy"), DateTimeOffset.UnixEpoch);
        var model = Model(new Server().On(Path, Said));
        await SupplyAsync(model, new Maker(() => SuppliedSound.Made(M4a)));
        Assert.Equal(new KeptTranscript(34, "from the stored copy"), store.Find(34));
    }

    private static async Task WaitUntil(Func<bool> condition)
    {
        var until = DateTime.UtcNow.AddSeconds(5);
        while (!condition())
        {
            Assert.True(DateTime.UtcNow < until, "timed out");
            await Task.Delay(10);
        }
    }
}
