using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>A voice note: five minutes at most, nothing when it is too short, and the lines it is drawn with.</summary>
public sealed class VoiceNotesTests
{
    [Fact]
    public void ARecordingIsDoneAtFiveMinutes()
    {
        Assert.False(VoiceNotes.IsDone(TimeSpan.FromSeconds(299.9)));
        Assert.True(VoiceNotes.IsDone(TimeSpan.FromMinutes(5)));
        Assert.True(VoiceNotes.IsDone(TimeSpan.FromMinutes(6)));
    }

    /// <summary>1024 bytes or less is nothing; past it, an MP4 audio attachment with no name, its duration its identity.</summary>
    [Fact]
    public void ARecordingIsStagedUnlessItIsNothing()
    {
        Assert.Null(VoiceNotes.Staged(new byte[1024], TimeSpan.FromSeconds(3)));
        Assert.Null(VoiceNotes.Staged(ReadOnlyMemory<byte>.Empty, TimeSpan.FromSeconds(3)));

        // Half a millisecond rounds away from zero, as the Apple apps' does — not to the even neighbour.
        var staged = VoiceNotes.Staged(new byte[1025], TimeSpan.FromMilliseconds(12_344.5))!;
        Assert.Equal(("audio", "audio/mp4", 1025, 12_345), (staged.Kind, staged.Mime, staged.Bytes.Length, staged.DurationMs));
        Assert.Null(staged.Name);
        Assert.Null(staged.Preview);

        // A recording that ran over by a tick is still five minutes long.
        Assert.Equal(300_000, VoiceNotes.Staged(new byte[4096], TimeSpan.FromSeconds(300.4))!.DurationMs);
    }

    /// <summary>A second is the floor people see (S1.1): under it nothing is staged, however many bytes it wrote.</summary>
    [Fact]
    public void ARecordingUnderASecondIsNothing()
    {
        Assert.Null(VoiceNotes.Staged(new byte[64 * 1024], TimeSpan.FromMilliseconds(999)));
        Assert.Null(VoiceNotes.Staged(new byte[64 * 1024], TimeSpan.Zero));
        Assert.Equal(1000, VoiceNotes.Staged(new byte[64 * 1024], TimeSpan.FromSeconds(1))!.DurationMs);
    }

    /// <summary>A recording is the audio with no name; a sound file picked from disk is audio WITH its name, and stays a file.</summary>
    [Fact]
    public void OnlyARecordingMadeHereIsARecording()
    {
        Assert.True(VoiceNotes.IsRecorded(VoiceNotes.Staged(new byte[2048], TimeSpan.FromSeconds(3))!));
        Assert.False(VoiceNotes.IsRecorded(new StagedMedia("audio", "audio/mp4", new byte[2048], DurationMs: 3000, Name: "song.m4a")));
        Assert.False(VoiceNotes.IsRecorded(new StagedMedia("photo", "image/jpeg", new byte[2048])));
        Assert.False(VoiceNotes.IsRecorded(new StagedMedia("file", "application/pdf", new byte[2048])));
    }

    /// <summary>Deleting ten seconds or more asks first (S1.1, S2.8): a recording cannot be made again.</summary>
    [Fact]
    public void DeletingTenSecondsOrMoreAsksFirst()
    {
        Assert.False(VoiceNotes.AsksBeforeDeleting(0));
        Assert.False(VoiceNotes.AsksBeforeDeleting(9_999));
        Assert.True(VoiceNotes.AsksBeforeDeleting(10_000));
        Assert.True(VoiceNotes.AsksBeforeDeleting(300_000));
    }

    /// <summary>
    /// A recording refused says why, in the order the Send slot's rows put the reasons (S1.3: editing, a call, an
    /// attachment on its way, a voice message not sent) — and only then the strip's cap.
    /// </summary>
    [Fact]
    public void ARecordingIsRefusedForTheSlotsReasonsInTheSlotsOrder()
    {
        var say = EnglishCatalog.Instance;
        Assert.Null(VoiceNotes.Refusal(editing: false, call: false, busy: false, notSent: false, full: false, say));
        Assert.Equal("Finish editing before attaching something.",
            VoiceNotes.Refusal(editing: true, call: true, busy: true, notSent: true, full: true, say));
        Assert.Equal("You can record a message after the call.",
            VoiceNotes.Refusal(editing: false, call: true, busy: true, notSent: true, full: true, say));
        Assert.Equal("Wait until the current attachment is done.",
            VoiceNotes.Refusal(editing: false, call: false, busy: true, notSent: true, full: true, say));
        Assert.Equal("Send or delete the voice message that wasn't sent first.",
            VoiceNotes.Refusal(editing: false, call: false, busy: false, notSent: true, full: true, say));
        Assert.Equal("You can attach up to 10 items.",
            VoiceNotes.Refusal(editing: false, call: false, busy: false, notSent: false, full: true, say));
    }

    /// <summary>
    /// Settings is offered only where a switch there can help (S2.2) — the person's, Windows' own, or when Windows could not
    /// say — never for a fault in the build, and a prompt closed without an answer says nothing at all.
    /// </summary>
    [Fact]
    public void AMicrophoneRefusalOffersSettingsOnlyWhereASwitchThereCanHelp()
    {
        var say = EnglishCatalog.Instance;
        const string Permission = "Family needs permission to use your microphone. Turn it on in Settings.";
        Assert.Equal((Permission, true), VoiceNotes.MicrophoneRefusal(CapabilityAccess.DeniedByUser, say));
        Assert.Equal((Permission, true), VoiceNotes.MicrophoneRefusal(CapabilityAccess.DeniedBySystem, say));
        Assert.Equal((Permission, true), VoiceNotes.MicrophoneRefusal(CapabilityAccess.Unknown, say));
        Assert.Equal(((string?)null, false), VoiceNotes.MicrophoneRefusal(CapabilityAccess.UserPromptRequired, say));
        Assert.Equal(("Couldn't start recording.", false), VoiceNotes.MicrophoneRefusal(CapabilityAccess.NotDeclaredByApp, say));
        Assert.Equal(("Couldn't start recording.", false), VoiceNotes.MicrophoneRefusal(CapabilityAccess.Allowed, say));
        Assert.Equal("ms-settings:privacy-microphone", VoiceNotes.MicrophoneSettingsPage);
    }

    /// <summary>
    /// The floors are the shared rules' (fc_text::record, S1.1), read from the oracle's own vectors rather than copied: a
    /// second, ten seconds to ask, five minutes.
    /// </summary>
    [Fact]
    public void TheFloorsAreTheSharedRules()
    {
        var path = Path.Combine(AppContext.BaseDirectory, "Fixtures", "record-vectors.json");
        using var vectors = System.Text.Json.JsonDocument.Parse(File.ReadAllText(path));
        var constants = vectors.RootElement.EnumerateArray()
            .Single(vector => vector.GetProperty("function").GetString() == "constants")
            .GetProperty("expected");
        Assert.Equal(constants.GetProperty("shortest_recording_ms").GetInt32(), (int)VoiceNotes.Shortest.TotalMilliseconds);
        Assert.Equal(constants.GetProperty("delete_asks_from_ms").GetInt32(), (int)VoiceNotes.DeleteAsksFrom.TotalMilliseconds);
        Assert.Equal(constants.GetProperty("voice_cap_ms").GetInt32(), (int)VoiceNotes.Longest.TotalMilliseconds);
    }

    /// <summary>
    /// The recording row's clock is "0:42" in whole seconds gone by, as a stopwatch counts (S2.4, S2.9) — 0:12 until the 13th
    /// second has run, and 5:00 only at five minutes, where the row stops.
    /// </summary>
    [Fact]
    public void TheClockCountsWholeSecondsGoneBy()
    {
        Assert.Equal("0:00", VoiceNotes.Clock(TimeSpan.Zero));
        Assert.Equal("0:00", VoiceNotes.Clock(TimeSpan.FromMilliseconds(999)));
        Assert.Equal("0:12", VoiceNotes.Clock(TimeSpan.FromSeconds(12.4)));
        Assert.Equal("0:12", VoiceNotes.Clock(TimeSpan.FromSeconds(12.9)));
        Assert.Equal("0:13", VoiceNotes.Clock(TimeSpan.FromSeconds(13)));
        Assert.Equal("4:59", VoiceNotes.Clock(TimeSpan.FromSeconds(299.9)));
        Assert.Equal("5:00", VoiceNotes.Clock(TimeSpan.FromMinutes(5)));
        Assert.Equal("0:00", VoiceNotes.Clock(TimeSpan.FromSeconds(-1)));
    }

    /// <summary>
    /// At 4:30 the clock turns orange and "30 seconds left" is shown and announced (S2.5) — the shared rule's own number, read
    /// from its vectors.
    /// </summary>
    [Fact]
    public void ThirtySecondsLeftIsSaidAtFourThirty()
    {
        Assert.False(VoiceNotes.Warns(TimeSpan.FromSeconds(269.9)));
        Assert.True(VoiceNotes.Warns(TimeSpan.FromSeconds(270)));
        Assert.True(VoiceNotes.Warns(VoiceNotes.Longest));
        var path = Path.Combine(AppContext.BaseDirectory, "Fixtures", "record-vectors.json");
        using var vectors = System.Text.Json.JsonDocument.Parse(File.ReadAllText(path));
        var constants = vectors.RootElement.EnumerateArray()
            .Single(vector => vector.GetProperty("function").GetString() == "constants")
            .GetProperty("expected");
        Assert.Equal(constants.GetProperty("voice_warning_ms").GetInt32(), (int)VoiceNotes.WarnFrom.TotalMilliseconds);
    }

    /// <summary>
    /// With a screen reader running the microphone opens a second after "Recording" is announced (S6: a fixed second where the
    /// platform cannot say when its speech ended), so the app's own voice does not open the note; otherwise at once.
    /// </summary>
    [Fact]
    public void TheMicrophoneWaitsForTheScreenReaderToSpeak()
    {
        Assert.Equal(TimeSpan.FromSeconds(1), VoiceNotes.LeadIn(screenReader: true));
        Assert.Equal(TimeSpan.Zero, VoiceNotes.LeadIn(screenReader: false));
    }

    /// <summary>A staged or not-sent note while it plays: "0:12 / 0:42" (S2.7) — never past its own length.</summary>
    [Fact]
    public void APlayingNoteSaysWhereItIsAndHowLongItIs()
    {
        Assert.Equal("0:12 / 0:42", VoiceNotes.PlayingLabel(12.2, 42));
        Assert.Equal("0:00 / 0:42", VoiceNotes.PlayingLabel(0, 42));
        Assert.Equal("0:42 / 0:42", VoiceNotes.PlayingLabel(43.5, 42));
        Assert.Equal("1:05 / 5:00", VoiceNotes.PlayingLabel(65, 300));
    }

    /// <summary>The scrubber's length comes from the attachment, and is never zero.</summary>
    [Fact]
    public void ARecordingIsAsLongAsItsAttachmentSays()
    {
        Assert.Equal(65.0, VoiceNotes.TotalSeconds(65_000));
        Assert.Equal(1.5, VoiceNotes.TotalSeconds(1_500));
        Assert.Equal(0.1, VoiceNotes.TotalSeconds(null));
        Assert.Equal(0.1, VoiceNotes.TotalSeconds(0));
        Assert.Equal(0.1, VoiceNotes.TotalSeconds(40));
    }

    [Fact]
    public void PlayStartsAgainOnlyAtTheEnd()
    {
        Assert.False(VoiceNotes.ReplaysFromStart(0, 65));
        Assert.False(VoiceNotes.ReplaysFromStart(64.7, 65));
        Assert.True(VoiceNotes.ReplaysFromStart(64.8, 65));
        Assert.True(VoiceNotes.ReplaysFromStart(65, 65));
        Assert.True(VoiceNotes.ReplaysFromStart(66, 65));
    }

    /// <summary>
    /// THE APP'S OWN SPEECH STAYS OUT OF THE NOTE (S6): while a recording runs, a sentence on the composer's notice line — a
    /// play button saying "You can play this after recording." — is shown and never said; otherwise it is said as it appears.
    /// </summary>
    [Fact]
    public void NothingOnTheNoticeLineIsSaidWhileARecordingRuns()
    {
        Assert.False(VoiceNotes.NoticeSaid(recording: true));
        Assert.True(VoiceNotes.NoticeSaid(recording: false));
    }

    /// <summary>
    /// NO APP SOUND WHILE RECORDING (S1.7), the viewer's video included: muted for as long as a recording runs, and left as it
    /// was once it ends — muted if the person had muted it, not if they had not — however often it is asked meanwhile.
    /// </summary>
    [Fact]
    public void TheViewersVideoIsMutedWhileRecordingAndGivenBackAsItWas()
    {
        var quiet = new QuietWhileRecording();
        Assert.True(quiet.Muted(recording: true, mutedNow: false));
        // A video opened in the viewer mid-recording: muted already, by this — which is not what it was before.
        Assert.True(quiet.Muted(recording: true, mutedNow: true));
        Assert.False(quiet.Muted(recording: false, mutedNow: true));
        // The person had muted it before the recording: muted it stays.
        Assert.True(quiet.Muted(recording: true, mutedNow: true));
        Assert.True(quiet.Muted(recording: false, mutedNow: true));
        // Ended twice, or with nothing recorded meanwhile: nothing is changed.
        Assert.False(quiet.Muted(recording: false, mutedNow: false));
        Assert.True(quiet.Muted(recording: false, mutedNow: true));
    }

    /// <summary>
    /// NO APP SOUND WHILE RECORDING (S1.7) STARTS WITH THE START: Windows' first-time microphone prompt and a screen reader's
    /// one-second lead-in both come before the recorder exists, and a play pressed then would run on into the note. Playback
    /// is refused from the moment a start begins, for as long as it is under way or the recording runs — and not before or
    /// after.
    /// </summary>
    [Fact]
    public void NothingPlaysWhileARecordingIsStarting()
    {
        var start = new RecordingStart();
        Assert.False(start.Starting);
        Assert.False(start.Quiet(recording: false));
        Assert.True(start.Quiet(recording: true));
        start.Begin();
        Assert.True(start.Starting);
        Assert.True(start.Quiet(recording: false));
        start.End();
        Assert.False(start.Starting);
        Assert.False(start.Quiet(recording: false));
        Assert.True(start.Quiet(recording: true));
    }

    /// <summary>
    /// AN INTERRUPTION DURING THE START IS NOT LOST (S4): a lock, the computer sleeping, a call, the window hidden or closed,
    /// leaving the chat, a sign-out — any of them arriving while Windows asks for the microphone, or during the lead-in, finds
    /// no recorder to stop, so the start remembers it and lets go of the microphone once it is granted. One heard with no
    /// start under way, or during an earlier start, says nothing about the next.
    /// </summary>
    [Fact]
    public void AnInterruptionDuringTheStartLetsGoOfTheMicrophone()
    {
        var start = new RecordingStart();
        start.Interrupted();
        start.Begin();
        Assert.False(start.WasInterrupted);
        start.Interrupted();
        Assert.True(start.WasInterrupted);
        Assert.True(start.Starting);
        start.End();
        Assert.False(start.WasInterrupted);
        start.Begin();
        Assert.False(start.WasInterrupted);
        start.End();
        start.Interrupted();
        start.Begin();
        Assert.False(start.WasInterrupted);
    }
}
