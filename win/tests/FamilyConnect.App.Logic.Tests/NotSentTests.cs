using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// "Voice message not sent" (docs/audio-video-messages-2026-10-04.md, S2.8) and the voice columns of S4's table, row by
/// row: what each ending does to a recording that is running, and to one in review.
/// </summary>
public sealed class NotSentTests
{
    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    private const int Readable = 8 * 1024;

    private static readonly TimeSpan Long = TimeSpan.FromSeconds(42);

    /// <summary>Every way something other than the person stops a recording, but a failure and a sign-out.</summary>
    public static TheoryData<RecordingEnd> Interruptions =>
    [
        RecordingEnd.LeftChat, RecordingEnd.Call, RecordingEnd.WindowHidden, RecordingEnd.WindowClosed, RecordingEnd.SessionLocked,
    ];

    [Fact]
    public void TheirStopGoesToReview()
    {
        Assert.Equal(new RecordingOutcome(RecordingFate.Review, null), NotSent.Ended(RecordingEnd.Stopped, Long, Readable, Say));
        Assert.Equal(new RecordingOutcome(RecordingFate.Review, null),
            NotSent.Ended(RecordingEnd.Stopped, TimeSpan.FromSeconds(1), Readable, Say));
    }

    [Fact]
    public void TheirStopUnderASecondIsTooShort()
    {
        var tooShort = new RecordingOutcome(RecordingFate.Discard, "That recording was too short.");
        Assert.Equal(tooShort, NotSent.Ended(RecordingEnd.Stopped, TimeSpan.FromMilliseconds(999), Readable, Say));
        Assert.Equal(tooShort, NotSent.Ended(RecordingEnd.Stopped, TimeSpan.FromMilliseconds(400), null, Say));
    }

    /// <summary>
    /// The slot's Send in row 2 SENDS — the one ending that does (S2.5) — with a second or more; shorter is "too short" and
    /// gone, never a blip in the family chat; and what cannot be read back is said to have stopped, never sent empty.
    /// </summary>
    [Fact]
    public void TheirSendSendsASecondOrMore()
    {
        Assert.Equal(new RecordingOutcome(RecordingFate.Send, null), NotSent.Ended(RecordingEnd.Sent, Long, Readable, Say));
        Assert.Equal(new RecordingOutcome(RecordingFate.Send, null),
            NotSent.Ended(RecordingEnd.Sent, TimeSpan.FromSeconds(1), Readable, Say));
        var tooShort = new RecordingOutcome(RecordingFate.Discard, "That recording was too short.");
        Assert.Equal(tooShort, NotSent.Ended(RecordingEnd.Sent, TimeSpan.FromMilliseconds(999), Readable, Say));
        Assert.Equal(tooShort, NotSent.Ended(RecordingEnd.Sent, TimeSpan.FromMilliseconds(150), null, Say));
        Assert.Equal(
            new RecordingOutcome(RecordingFate.Discard, "The recording stopped unexpectedly."),
            NotSent.Ended(RecordingEnd.Sent, Long, null, Say));
        // A Send keeps nothing back for review, and a note in review is not parked by one.
        Assert.False(NotSent.ParksReview(RecordingEnd.Sent));
    }

    /// <summary>
    /// What a screen reader is told once a recording went where it went (S6): its Send sent, its Stop in review with the
    /// length, its Delete gone — and nothing for an interruption, whose row is right there, or for an ending that did not go
    /// where it was meant to (a send kept in review is said by the notice line instead).
    /// </summary>
    [Fact]
    public void WhatTheyDidIsSaid()
    {
        Assert.Equal("Voice message sent", NotSent.Said(RecordingEnd.Sent, RecordingFate.Send, 42_000, Say));
        Assert.Equal("Ready to review, 0:42", NotSent.Said(RecordingEnd.Stopped, RecordingFate.Review, 42_000, Say));
        Assert.Equal("Ready to review, 0:43", NotSent.Said(RecordingEnd.Stopped, RecordingFate.Review, 42_500, Say));
        Assert.Equal("Recording deleted", NotSent.Said(RecordingEnd.Deleted, RecordingFate.Discard, 3_000, Say));
        Assert.Null(NotSent.Said(RecordingEnd.Sent, RecordingFate.Review, 42_000, Say));
        Assert.Null(NotSent.Said(RecordingEnd.Sent, RecordingFate.Discard, 400, Say));
        Assert.Null(NotSent.Said(RecordingEnd.Stopped, RecordingFate.Park, 42_000, Say));
        Assert.Null(NotSent.Said(RecordingEnd.Stopped, RecordingFate.Discard, 400, Say));
        Assert.Null(NotSent.Said(RecordingEnd.Capped, RecordingFate.Review, 300_000, Say));
        // "Recording deleted" only for what really went: a Delete whose recording was kept anywhere is never said gone.
        Assert.Null(NotSent.Said(RecordingEnd.Deleted, RecordingFate.Park, 42_000, Say));
        Assert.Null(NotSent.Said(RecordingEnd.Deleted, RecordingFate.Review, 42_000, Say));
        foreach (var why in new[] { RecordingEnd.LeftChat, RecordingEnd.Call, RecordingEnd.WindowHidden, RecordingEnd.SessionLocked })
        {
            Assert.Null(NotSent.Said(why, RecordingFate.Park, 42_000, Say));
        }
        Assert.NotEqual("Voice message sent", NotSent.Said(RecordingEnd.Sent, RecordingFate.Send, 42_000, JsonCatalog.For("ru")));
    }

    /// <summary>
    /// The person's own Send, Stop and Delete give the keyboard back to the field (S2.4) — so a second Enter cannot open the
    /// microphone again — and an interruption leaves focus where it was.
    /// </summary>
    [Fact]
    public void TheirOwnEndingGivesTheKeyboardBackToTheField()
    {
        Assert.True(NotSent.GivesFocusBack(RecordingEnd.Sent));
        Assert.True(NotSent.GivesFocusBack(RecordingEnd.Stopped));
        Assert.True(NotSent.GivesFocusBack(RecordingEnd.Deleted));
        foreach (var why in new[]
                 {
                     RecordingEnd.Capped, RecordingEnd.LeftChat, RecordingEnd.Call, RecordingEnd.WindowHidden, RecordingEnd.WindowClosed,
                     RecordingEnd.SessionLocked, RecordingEnd.RecorderFailed, RecordingEnd.SignedOut,
                 })
        {
            Assert.False(NotSent.GivesFocusBack(why), why.ToString());
        }
    }

    /// <summary>Nothing but the person's Send sends: no interruption, no limit, no Stop, no Delete (S1.7, decision 12).</summary>
    [Fact]
    public void NothingButTheirSendSends()
    {
        foreach (var why in Enum.GetValues<RecordingEnd>().Where(why => why != RecordingEnd.Sent))
        {
            Assert.NotEqual(RecordingFate.Send, NotSent.Ended(why, Long, Readable, Say).Fate);
            Assert.NotEqual(RecordingFate.Send, NotSent.Ended(why, VoiceNotes.Longest, Readable, Say).Fate);
        }
    }

    /// <summary>Forty seconds that cannot be read back are not "too short": they were lost, and the person is told so.</summary>
    [Fact]
    public void ARecordingThatCannotBeReadBackIsSaidToHaveStopped()
    {
        var lost = new RecordingOutcome(RecordingFate.Discard, "The recording stopped unexpectedly.");
        Assert.Equal(lost, NotSent.Ended(RecordingEnd.Stopped, Long, null, Say));
        Assert.Equal(lost, NotSent.Ended(RecordingEnd.Stopped, Long, VoiceNotes.NothingAtOrBelowBytes, Say));
        Assert.Equal(lost, NotSent.Ended(RecordingEnd.Call, Long, null, Say));
        Assert.Equal(lost, NotSent.Ended(RecordingEnd.Capped, VoiceNotes.Longest, null, Say));
    }

    /// <summary>Five minutes stops into REVIEW and says so (S2.5) — never a send, and never a not-sent row.</summary>
    [Fact]
    public void FiveMinutesStopsIntoReviewAndSaysSo()
    {
        Assert.Equal(
            new RecordingOutcome(RecordingFate.Review, "Recording stopped at five minutes."),
            NotSent.Ended(RecordingEnd.Capped, VoiceNotes.Longest, Readable, Say));
        Assert.True(VoiceNotes.IsDone(VoiceNotes.Longest));
    }

    /// <summary>S4: a call, leaving the chat, a hidden, minimised or closed window and a locked session stop and KEEP.</summary>
    [Theory]
    [MemberData(nameof(Interruptions))]
    public void AnInterruptionStopsAndKeeps(RecordingEnd why)
    {
        Assert.Equal(new RecordingOutcome(RecordingFate.Park, null), NotSent.Ended(why, Long, Readable, Say));
        Assert.Equal(new RecordingOutcome(RecordingFate.Park, null), NotSent.Ended(why, TimeSpan.FromSeconds(1), Readable, Say));
    }

    /// <summary>S4: "Under 1.0 s, 'not sent' below means deleted instead — there is nothing worth keeping" — and nothing is said.</summary>
    [Theory]
    [MemberData(nameof(Interruptions))]
    public void AnInterruptionUnderASecondDeletesWithoutAWord(RecordingEnd why)
    {
        Assert.Equal(new RecordingOutcome(RecordingFate.Discard, null), NotSent.Ended(why, TimeSpan.FromMilliseconds(999), Readable, Say));
        Assert.Equal(new RecordingOutcome(RecordingFate.Discard, null), NotSent.Ended(why, TimeSpan.Zero, null, Say));
    }

    /// <summary>S4: the recorder failing keeps what is readable as not sent, with the sentence — and says it even when nothing is.</summary>
    [Fact]
    public void AFailedRecorderKeepsWhatIsReadableAndSaysSo()
    {
        const string Sentence = "The recording stopped unexpectedly.";
        Assert.Equal(new RecordingOutcome(RecordingFate.Park, Sentence), NotSent.Ended(RecordingEnd.RecorderFailed, Long, Readable, Say));
        Assert.Equal(new RecordingOutcome(RecordingFate.Discard, Sentence), NotSent.Ended(RecordingEnd.RecorderFailed, Long, null, Say));
        Assert.Equal(new RecordingOutcome(RecordingFate.Discard, Sentence),
            NotSent.Ended(RecordingEnd.RecorderFailed, TimeSpan.FromMilliseconds(300), Readable, Say));
    }

    /// <summary>S4's last row: a sign-out takes everything recorded and not sent with it. The person's Cancel deletes too.</summary>
    [Fact]
    public void ASignOutAndTheirCancelDiscard()
    {
        Assert.Equal(new RecordingOutcome(RecordingFate.Discard, null), NotSent.Ended(RecordingEnd.SignedOut, Long, Readable, Say));
        Assert.Equal(new RecordingOutcome(RecordingFate.Discard, null), NotSent.Ended(RecordingEnd.Deleted, Long, Readable, Say));
        Assert.False(NotSent.ParksReview(RecordingEnd.SignedOut));
    }

    /// <summary>
    /// A note in REVIEW becomes not sent only when the person leaves the chat or the window really closes; a call, a hidden
    /// window, a locked session and a failed recorder keep it in review (S4's "Review / not sent" column).
    /// </summary>
    [Fact]
    public void OnlyLeavingTheChatOrClosingTheWindowParksANoteInReview()
    {
        Assert.True(NotSent.ParksReview(RecordingEnd.LeftChat));
        Assert.True(NotSent.ParksReview(RecordingEnd.WindowClosed));
        foreach (var kept in new[]
                 {
                     RecordingEnd.Call, RecordingEnd.WindowHidden, RecordingEnd.SessionLocked, RecordingEnd.RecorderFailed,
                     RecordingEnd.Stopped, RecordingEnd.Capped, RecordingEnd.Deleted,
                 })
        {
            Assert.False(NotSent.ParksReview(kept), kept.ToString());
        }
    }

    /// <summary>
    /// A note in review takes the words as its caption and the reply with them, and leaves the composer empty (S2.8's
    /// <c>takeComposer</c>); trailing white space is not part of a caption, as it is not part of a sent body.
    /// </summary>
    [Fact]
    public void ANoteInReviewTakesTheWordsAndTheReply()
    {
        Assert.Equal(new ComposerTake("Happy birthday 🎂", 7, ClearsWords: true, ClearsReply: true),
            NotSent.ForReview("Happy birthday 🎂  \n", 7, editing: false));
        Assert.Equal(new ComposerTake("  indented", null, ClearsWords: true, ClearsReply: false),
            NotSent.ForReview("  indented", null, editing: false));
        Assert.Equal(new ComposerTake(null, 7, ClearsWords: false, ClearsReply: true),
            NotSent.ForReview("   ", 7, editing: false));
        Assert.Equal(ComposerTake.Nothing, NotSent.ForReview(string.Empty, null, editing: false));
    }

    /// <summary>An edit's words are the edit's: never a caption, and an edit has no reply to give.</summary>
    [Fact]
    public void AnEditGivesNothing()
    {
        Assert.Equal(ComposerTake.Nothing, NotSent.ForReview("the corrected words", 7, editing: true));
        Assert.Equal(ComposerTake.Nothing, NotSent.ForRecording("the corrected words", 7, editing: true, staged: false));
    }

    /// <summary>
    /// A RUNNING recording takes the reply it was recorded under, and never the words; the reply leaves the composer only
    /// when nothing else there would carry it.
    /// </summary>
    [Fact]
    public void ARunningRecordingTakesTheReplyAndLeavesTheWords()
    {
        Assert.Equal(new ComposerTake(null, 7, ClearsWords: false, ClearsReply: true),
            NotSent.ForRecording(string.Empty, 7, editing: false, staged: false));
        Assert.Equal(new ComposerTake(null, 7, ClearsWords: false, ClearsReply: true),
            NotSent.ForRecording(" \n", 7, editing: false, staged: false));
        Assert.Equal(new ComposerTake(null, 7, ClearsWords: false, ClearsReply: false),
            NotSent.ForRecording("look at this", 7, editing: false, staged: false));
        Assert.Equal(new ComposerTake(null, 7, ClearsWords: false, ClearsReply: false),
            NotSent.ForRecording(string.Empty, 7, editing: false, staged: true));
        Assert.Equal(ComposerTake.Nothing, NotSent.ForRecording("look at this", null, editing: false, staged: false));
    }

    [Fact]
    public void TheRowSaysHowLongTheRecordingIs()
    {
        Assert.Equal("Voice message not sent · 0:42", NotSent.Line(42_000, Say));
        Assert.Equal("Voice message not sent · 0:01", NotSent.Line(1_000, Say));
        Assert.Equal("Voice message not sent · 0:43", NotSent.Line(42_500, Say));
        Assert.Equal("Voice message not sent · 5:00", NotSent.Line(300_000, Say));
    }

    /// <summary>The row reads in the reader's language, its time where the translation puts it.</summary>
    [Fact]
    public void TheRowIsTranslated()
    {
        var russian = JsonCatalog.For("ru");
        Assert.NotEqual("Voice message not sent · 0:42", NotSent.Line(42_000, russian));
        Assert.Contains("0:42", NotSent.Line(42_000, russian));
    }

    /// <summary>
    /// A NOT-SENT ROW'S OWN BUTTONS GO WITH IT (S2.8, S6): its Send and its ✕ each give the keyboard back to the field — never
    /// nowhere, which loses a screen reader's place (WCAG 2.4.3) — and say what happened.
    /// </summary>
    [Fact]
    public void ARowsSendOrDeleteGivesTheKeyboardBackAndSaysSo()
    {
        Assert.Equal((true, "Voice message sent"), NotSent.RowEnded(sent: true, 42_000, Say));
        Assert.Equal((true, "Recording deleted"), NotSent.RowEnded(sent: false, 42_000, Say));
        Assert.NotEqual("Voice message sent", NotSent.RowEnded(sent: true, 42_000, JsonCatalog.For("ru")).Said);
    }
}
