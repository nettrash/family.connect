using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// Who may ask for the text of which recording, and what a failed ask says (docs/protocol.md, "Transcripts on
/// request"). The rule's vectors are the server's own <c>allowed()</c> cases, minus the one check a client cannot make
/// (whether the other sender has agreed to the assistant), which the server answers with the same refusal.
/// </summary>
public sealed class TranscriptRulesTests
{
    private const long Me = 7;
    private const long Anna = 9;
    private const long Assistant = 1;
    private const long MiB25 = 26_214_400;

    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    private static readonly AssistantDto Transcribes = new(
        Assistant, "Assistant", "@ai", Processor: "Microsoft — Azure OpenAI", Transcribe: true, TranscribeMaxBytes: MiB25);

    private static readonly FamilyDto SwitchOn = new(3, "The Smiths", AiTranscripts: true);
    private static readonly FamilyDto SwitchOff = new(3, "The Smiths");

    private static AttachmentDto Voice(string mime = "audio/mp4", long? size = 48_000, string kind = "audio") =>
        new(34, kind, mime, size, DurationMs: 8_400);

    /// <summary>YOUR OWN VOICE, WHEREVER YOU SENT IT: the family chat, a direct chat, your own assistant chat — switch or no switch.</summary>
    [Theory]
    [InlineData("family")]
    [InlineData("direct")]
    [InlineData("ai")]
    public void YourOwnRecordingNeedsNoSwitch(string kind)
    {
        Assert.True(TranscriptRules.MayAsk(kind, Me, Me, Assistant, SwitchOff));
        Assert.True(TranscriptRules.MayAsk(kind, Me, Me, Assistant, SwitchOn));
        Assert.True(TranscriptRules.MayAsk(kind, Me, Me, Assistant, null));
    }

    /// <summary>
    /// SOMEBODY ELSE'S: the family chat (and its threads, which are the same chat) while the owner allows it — never a
    /// direct chat, at any setting.
    /// </summary>
    [Fact]
    public void AnotherMembersRecordingNeedsTheFamilyChatAndTheOwnersSwitch()
    {
        Assert.True(TranscriptRules.MayAsk("family", Anna, Me, Assistant, SwitchOn));
        Assert.False(TranscriptRules.MayAsk("family", Anna, Me, Assistant, SwitchOff));
        Assert.False(TranscriptRules.MayAsk("family", Anna, Me, Assistant, null));
        Assert.False(TranscriptRules.MayAsk("direct", Anna, Me, Assistant, SwitchOn));
        Assert.False(TranscriptRules.MayAsk("ai", Anna, Me, Assistant, SwitchOn));
        Assert.False(TranscriptRules.MayAsk(null, Anna, Me, Assistant, SwitchOn));
    }

    /// <summary>The assistant's own messages are never asked about — it sends no sound, and has no consent to give.</summary>
    [Fact]
    public void TheAssistantsMessagesNever()
    {
        Assert.False(TranscriptRules.MayAsk("family", Assistant, Me, Assistant, SwitchOn));
        Assert.False(TranscriptRules.MayAsk("ai", Assistant, Me, Assistant, SwitchOn));
    }

    /// <summary>A reader not known yet is nobody's own message — never a way round the switch.</summary>
    [Fact]
    public void AnUnknownReaderOwnsNothing()
    {
        Assert.False(TranscriptRules.MayAsk("direct", 0, 0, Assistant, SwitchOn));
        Assert.False(TranscriptRules.MayAsk("family", Anna, 0, Assistant, SwitchOff));
        Assert.False(TranscriptRules.MayAsk("direct", Anna, 0, Assistant, SwitchOn));
    }

    /// <summary>
    /// THE STORED FORM: a recording of a type the provider reads, as the server spells it, within the ceiling — and
    /// nothing else (a video, Ogg, an oversized file and a size nobody stated go as the device's own sound).
    /// </summary>
    [Fact]
    public void OnlyARecordingTheServerCanSendAsItIsQualifies()
    {
        foreach (var mime in new[] { "audio/mp4", "audio/m4a", "audio/mpeg", "audio/wav" })
        {
            Assert.True(TranscriptRules.SendsStoredCopy(Voice(mime), Transcribes), mime);
        }
        Assert.False(TranscriptRules.SendsStoredCopy(Voice("audio/ogg"), Transcribes));
        Assert.False(TranscriptRules.SendsStoredCopy(Voice("audio/webm"), Transcribes));
        // Compared as the server compares: exactly.
        Assert.False(TranscriptRules.SendsStoredCopy(Voice("Audio/MP4"), Transcribes));
        Assert.False(TranscriptRules.SendsStoredCopy(Voice("video/mp4", kind: "video"), Transcribes));
        Assert.False(TranscriptRules.SendsStoredCopy(Voice("audio/mp4", kind: "file"), Transcribes));
        Assert.True(TranscriptRules.SendsStoredCopy(Voice(size: MiB25), Transcribes));
        Assert.False(TranscriptRules.SendsStoredCopy(Voice(size: MiB25 + 1), Transcribes));
        Assert.False(TranscriptRules.SendsStoredCopy(Voice(size: null), Transcribes));
        Assert.False(TranscriptRules.SendsStoredCopy(Voice(size: 0), Transcribes));
        Assert.False(TranscriptRules.SendsStoredCopy(Voice() with { Mime = null }, Transcribes));
    }

    /// <summary>The server's own ceiling where it names a lower one; never above 25 MiB, whatever it says.</summary>
    [Fact]
    public void TheCeilingIsTheServersAndNeverMoreThanTheProviders()
    {
        var lower = Transcribes with { TranscribeMaxBytes = 1_000_000 };
        Assert.Equal(1_000_000, TranscriptRules.MaxBytes(lower));
        Assert.False(TranscriptRules.SendsStoredCopy(Voice(size: 1_000_001), lower));
        Assert.Equal(MiB25, TranscriptRules.MaxBytes(Transcribes with { TranscribeMaxBytes = 100_000_000 }));
        Assert.Equal(MiB25, TranscriptRules.MaxBytes(Transcribes with { TranscribeMaxBytes = null }));
        Assert.Equal(MiB25, TranscriptRules.MaxBytes(Transcribes with { TranscribeMaxBytes = 0 }));
    }

    /// <summary>
    /// THE WHOLE DECISION: a server that transcribes and can name its processor, the rule, and the stored form — every
    /// one of them needed; text already held is shown whatever they say.
    /// </summary>
    [Fact]
    public void ShowTextIsOfferedOnlyWhereEveryPartSaysYes()
    {
        Assert.True(TranscriptRules.Offers(Transcribes, SwitchOff, "direct", Me, Me, Voice()));
        Assert.True(TranscriptRules.Offers(Transcribes, SwitchOn, "family", Anna, Me, Voice()));

        // No transcription deployment, an older server, no assistant, no processor to name.
        Assert.False(TranscriptRules.Offers(Transcribes with { Transcribe = false }, SwitchOn, "family", Me, Me, Voice()));
        Assert.False(TranscriptRules.Offers(null, SwitchOn, "family", Me, Me, Voice()));
        Assert.False(TranscriptRules.Offers(Transcribes with { Processor = null }, SwitchOn, "family", Me, Me, Voice()));
        Assert.False(TranscriptRules.Offers(Transcribes with { Processor = "  " }, SwitchOn, "family", Me, Me, Voice()));
        // The rule.
        Assert.False(TranscriptRules.Offers(Transcribes, SwitchOff, "family", Anna, Me, Voice()));
        Assert.False(TranscriptRules.Offers(Transcribes, SwitchOn, "direct", Anna, Me, Voice()));
        Assert.False(TranscriptRules.Offers(Transcribes, SwitchOn, "ai", Assistant, Me, Voice()));
        // The form, on a device that cannot make the sound itself: the stored copy or nothing.
        Assert.False(TranscriptRules.Offers(Transcribes, SwitchOn, "family", Me, Me, Voice("audio/ogg")));
        Assert.False(TranscriptRules.Offers(Transcribes, SwitchOn, "family", Me, Me, Voice("video/mp4", kind: "video")));

        // What this device was already given is shown, and showing it sends nothing.
        Assert.True(TranscriptRules.Offers(null, SwitchOff, "direct", Anna, Me, Voice("audio/ogg"), held: true));
    }

    /// <summary>
    /// THE FORM: the server's stored copy wherever it can send one; otherwise — on a device that makes the sound itself —
    /// the device's own, for a video, an Ogg file, an oversized or unsized recording; and nothing for what is no recording.
    /// </summary>
    [Fact]
    public void TheFormIsTheStoredCopyWhereverTheServerCanSendIt()
    {
        Assert.Equal(TranscriptForm.Stored, TranscriptRules.Form(Voice(), Transcribes, extracts: true));
        Assert.Equal(TranscriptForm.Stored, TranscriptRules.Form(Voice(), Transcribes, extracts: false));

        foreach (var attachment in new[]
                 {
                     Voice("audio/ogg"),
                     Voice("audio/webm"),
                     Voice(size: MiB25 + 1),
                     Voice(size: null),
                     Voice("video/mp4", size: 80_000_000, kind: "video"),
                     Voice("video/quicktime", kind: "video"),
                 })
        {
            Assert.Equal(TranscriptForm.Supplied, TranscriptRules.Form(attachment, Transcribes, extracts: true));
            // A device that cannot make the sound offers none of these.
            Assert.Equal(TranscriptForm.None, TranscriptRules.Form(attachment, Transcribes, extracts: false));
        }

        foreach (var kind in new[] { "photo", "file", "location" })
        {
            Assert.Equal(TranscriptForm.None, TranscriptRules.Form(Voice(kind: kind), Transcribes, extracts: true));
        }
    }

    /// <summary>
    /// NEVER HIDDEN FOR A REASON OF THE SOUND: a recording whose stated length is already too long for the ceiling is
    /// still offered — as on iOS and the web — and the press says "This recording is too long to turn into text."
    /// without downloading a byte (<see cref="TranscriptModel"/>). A hidden action cannot say why it is missing.
    /// </summary>
    [Fact]
    public void AVideoTooLongToFitIsStillOffered()
    {
        var video = Voice("video/mp4", size: 900_000_000, kind: "video");
        Assert.Equal(TranscriptForm.Supplied, TranscriptRules.Form(video with { DurationMs = 50 * 60_000 }, Transcribes, true));
        Assert.Equal(TranscriptForm.Supplied, TranscriptRules.Form(video with { DurationMs = 55 * 60_000 }, Transcribes, true));
        Assert.Equal(TranscriptForm.Supplied, TranscriptRules.Form(video with { DurationMs = null }, Transcribes, true));
        var lower = Transcribes with { TranscribeMaxBytes = 1_000_000 };
        Assert.Equal(TranscriptForm.Supplied, TranscriptRules.Form(video with { DurationMs = 5 * 60_000 }, lower, true));
        Assert.True(TranscriptRules.Offers(Transcribes, SwitchOn, "family", Me, Me, video with { DurationMs = 55 * 60_000 }, extracts: true));
    }

    /// <summary>
    /// WHAT THE DEVICE COULD NOT DO IS SAID IN ITS OWN WORDS — the catalogue's sentences the other three clients draw —
    /// and never as "Not available for this message.", which reads as a refusal. Both are final.
    /// </summary>
    [Fact]
    public void TheDevicesOwnFailuresSayWhatTheyWere()
    {
        Assert.Equal("This recording is too long to turn into text.", TranscriptRules.FailureSentence(TranscriptSound.TooLong, Say));
        Assert.Equal("Couldn't read the sound in this file.", TranscriptRules.FailureSentence(TranscriptSound.Unreadable, Say));
        Assert.False(TranscriptRules.MayRetry(TranscriptSound.TooLong));
        Assert.False(TranscriptRules.MayRetry(TranscriptSound.Unreadable));
    }

    /// <summary>
    /// "Show text" ON VIDEOS AND UNSUPPORTED AUDIO, under exactly the same rule as a voice note: your own anywhere;
    /// another member's in the family chat with the owner's switch on; never another's in a direct chat, never the
    /// assistant's.
    /// </summary>
    [Fact]
    public void VideosAndUnsupportedAudioFollowTheSameRule()
    {
        foreach (var attachment in new[] { Voice("video/mp4", kind: "video"), Voice("audio/ogg"), Voice(size: MiB25 + 1) })
        {
            Assert.True(TranscriptRules.Offers(Transcribes, SwitchOff, "direct", Me, Me, attachment, extracts: true));
            Assert.True(TranscriptRules.Offers(Transcribes, SwitchOn, "family", Anna, Me, attachment, extracts: true));
            Assert.False(TranscriptRules.Offers(Transcribes, SwitchOff, "family", Anna, Me, attachment, extracts: true));
            Assert.False(TranscriptRules.Offers(Transcribes, SwitchOn, "direct", Anna, Me, attachment, extracts: true));
            Assert.False(TranscriptRules.Offers(Transcribes, SwitchOn, "ai", Assistant, Me, attachment, extracts: true));
            Assert.False(TranscriptRules.Offers(Transcribes with { Transcribe = false }, SwitchOn, "family", Me, Me, attachment, extracts: true));
        }
    }

    /// <summary>
    /// What a failed ask says: the provider's refusal, final; "not available", final, for every refusal of this
    /// recording or this member; and "try again" for anything that says nothing about the request.
    /// </summary>
    [Theory]
    [InlineData(ErrorCodes.TranscriptRefused, 400, "The assistant's provider refused this recording.", false)]
    [InlineData(ErrorCodes.NotTranscribable, 400, "Not available for this message.", false)]
    [InlineData(ErrorCodes.TranscriptNotAllowed, 403, "Not available for this message.", false)]
    [InlineData(ErrorCodes.TranscriptsUnavailable, 403, "Not available for this message.", false)]
    [InlineData(ErrorCodes.MessageNotFound, 404, "Not available for this message.", false)]
    [InlineData(ErrorCodes.AttachmentNotFound, 404, "Not available for this message.", false)]
    [InlineData(ErrorCodes.Blocked, 409, "Not available for this message.", false)]
    [InlineData(ErrorCodes.Internal, 500, "Couldn't get the text. Try again.", true)]
    [InlineData(ErrorCodes.TooManyRequests, 429, "Couldn't get the text. Try again.", true)]
    [InlineData(ErrorCodes.AssistantConsentRequired, 403, "Couldn't get the text. Try again.", true)]
    public void AFailureSaysWhatItWas(string code, int status, string said, bool retry)
    {
        var error = new ApiError(code, "x", status);
        Assert.Equal(said, TranscriptRules.FailureSentence(error, Say));
        Assert.Equal(retry, TranscriptRules.MayRetry(error));
    }

    /// <summary>No connection and the deadline are the same "try again" — the server finishes and keeps it meanwhile.</summary>
    [Fact]
    public void NoAnswerAtAllMayBeAskedAgain()
    {
        var gone = ApiError.Transport("TaskCanceledException");
        Assert.Equal("Couldn't get the text. Try again.", TranscriptRules.FailureSentence(gone, Say));
        Assert.True(TranscriptRules.MayRetry(gone));
    }

    /// <summary>SILENCE IS AN ANSWER, drawn as "No speech" — never as a failure.</summary>
    [Fact]
    public void SilenceIsDrawnAsNoSpeech()
    {
        Assert.Equal("No speech", TranscriptRules.Shown("", Say));
        Assert.Equal("No speech", TranscriptRules.Shown("   ", Say));
        Assert.Equal("No speech", TranscriptRules.Shown(null, Say));
        Assert.Equal("Back at six", TranscriptRules.Shown("Back at six", Say));
        Assert.True(new TranscriptLook(TranscriptPhase.Open, "").NoSpeech);
        Assert.False(new TranscriptLook(TranscriptPhase.Open, "Back at six").NoSpeech);
        Assert.False(TranscriptLook.Closed.NoSpeech);
    }

    /// <summary>Every sentence this draws is one the apps' catalogue already says in nine languages.</summary>
    [Fact]
    public void TheSentencesAreTheCataloguesInEveryLanguage()
    {
        var russian = JsonCatalog.For("ru");
        Assert.NotEqual("No speech", TranscriptRules.Shown("", russian));
        Assert.NotEqual(
            "The assistant's provider refused this recording.",
            TranscriptRules.FailureSentence(new ApiError(ErrorCodes.TranscriptRefused, "x", 400), russian));
        Assert.NotEqual(
            "Not available for this message.",
            TranscriptRules.FailureSentence(new ApiError(ErrorCodes.NotTranscribable, "x", 400), russian));
        Assert.NotEqual(
            "Couldn't get the text. Try again.",
            TranscriptRules.FailureSentence(ApiError.Transport("x"), russian));
        Assert.NotEqual(
            "This recording is too long to turn into text.",
            TranscriptRules.FailureSentence(TranscriptSound.TooLong, russian));
        Assert.NotEqual(
            "Couldn't read the sound in this file.",
            TranscriptRules.FailureSentence(TranscriptSound.Unreadable, russian));
        foreach (var key in new[] { "Show text", "Hide text", "Getting the text…", "Text of the recording", "Voice and video as text" })
        {
            Assert.NotEqual(key, russian.Get(key));
        }
    }
}
