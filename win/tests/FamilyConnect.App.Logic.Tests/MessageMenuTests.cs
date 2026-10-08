using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// A message's context menu (the approved design of 2026-10-05): a voice or video message offers what does something for a
/// recording — reactions, Reply, Show text, Playback speed, Save…, Open Full Screen — and never Copy, Edit or anything
/// about text it does not have; every other message's menu is exactly what it was.
/// </summary>
public sealed class MessageMenuTests
{
    private static readonly AttachmentDto Voice = new(40, "audio", "audio/mp4", 9000, DurationMs: 4200);
    private static readonly AttachmentDto Circle = new(91, "video", "video/mp4", 1649700, 480, 480, 23400, HasPreview: true, Round: true);
    private static readonly AttachmentDto Photo = new(50, "photo", "image/jpeg", 1000, 600, 800);

    private static MessageDto Message(string body = "", long? root = null, params AttachmentDto[] media) =>
        new(1, 42, 7, null, body, "2026-10-05T10:00:00Z", ThreadRootId: root, Attachments: media);

    [Fact]
    public void AVoiceMessageHasNothingToCopyOrEdit()
    {
        var note = Message(media: Voice);
        Assert.Equal(RecordingKind.Voice, MessageMenu.Recording(note));
        Assert.Equal(
            [MessageAction.React, MessageAction.Reply, MessageAction.ShowText, MessageAction.PlaybackSpeed, MessageAction.Save],
            MessageMenu.Actions(note, mayEdit: true, inThread: false, offersText: true));
        Assert.Equal(
            [MessageAction.React, MessageAction.Reply, MessageAction.PlaybackSpeed, MessageAction.Save],
            MessageMenu.Actions(note, mayEdit: false, inThread: false, offersText: false));
        // In a chain: the thread straight after Reply — the order iOS, the Mac, Android and the web draw.
        Assert.Equal(
            [MessageAction.React, MessageAction.Reply, MessageAction.ViewThread, MessageAction.ShowText, MessageAction.PlaybackSpeed, MessageAction.Save],
            MessageMenu.Actions(Message(root: 3, media: Voice), mayEdit: false, inThread: false, offersText: true));
    }

    [Fact]
    public void AVideoMessageOpensFullScreenAndHasNoSpeed()
    {
        var circle = Message(root: 3, media: Circle);
        Assert.Equal(RecordingKind.Round, MessageMenu.Recording(circle));
        Assert.Equal(
            [MessageAction.React, MessageAction.Reply, MessageAction.ViewThread, MessageAction.ShowText, MessageAction.Save, MessageAction.OpenFullScreen],
            MessageMenu.Actions(circle, mayEdit: false, inThread: false, offersText: true));
        Assert.DoesNotContain(MessageAction.ViewThread, MessageMenu.Actions(circle, mayEdit: false, inThread: true, offersText: true));
    }

    /// <summary>Words make it a message with words: a sound with a caption, two sounds, a sound beside a photo — as ever.</summary>
    [Fact]
    public void AnyOtherMessageKeepsItsMenu()
    {
        Assert.Null(MessageMenu.Recording(Message("Listen to this", media: Voice)));
        Assert.Null(MessageMenu.Recording(Message(media: [Voice, Voice with { Id = 41 }])));
        Assert.Null(MessageMenu.Recording(Message(media: [Voice, Photo])));
        Assert.Null(MessageMenu.Recording(Message("hi")));
        Assert.Null(MessageMenu.Recording(Message("", media: Circle with { Round = false })));
        Assert.Equal(
            [MessageAction.Reply, MessageAction.React, MessageAction.Copy, MessageAction.Edit],
            MessageMenu.Actions(Message("Listen to this", media: Voice), mayEdit: true, inThread: false, offersText: true));
        Assert.Equal(
            [MessageAction.Reply, MessageAction.React, MessageAction.Copy, MessageAction.ViewThread, MessageAction.Edit],
            MessageMenu.Actions(Message("hi", root: 3), mayEdit: true, inThread: false, offersText: false));
        Assert.Equal(
            [MessageAction.Reply, MessageAction.React, MessageAction.Copy],
            MessageMenu.Actions(Message("hi", root: 3), mayEdit: true, inThread: true, offersText: false));
        Assert.Equal(
            [MessageAction.Reply, MessageAction.React],
            MessageMenu.Actions(Message(media: Photo), mayEdit: false, inThread: false, offersText: false));
    }

    // ---- the text item, as the words under the recording offer it -------------------------------------------------

    /// <summary>Folded away: "Show text"; open: "Hide text" — the same two the row under the player shows.</summary>
    [Fact]
    public void TheTextItemIsShowOrHideAsTheRowIs()
    {
        Assert.Equal(TextItem.Show, MessageMenu.TextItemFor(TranscriptLook.Closed));
        Assert.Equal(TextItem.Hide, MessageMenu.TextItemFor(new TranscriptLook(TranscriptPhase.Open, "hello")));
        Assert.Equal(TextItem.Hide, MessageMenu.TextItemFor(new TranscriptLook(TranscriptPhase.Open, "")));
    }

    /// <summary>"Getting the text…" is already asking: a second ask from the menu would only race the first.</summary>
    [Fact]
    public void NoTextItemWhileItIsBeingAskedFor()
    {
        Assert.Equal(TextItem.None, MessageMenu.TextItemFor(new TranscriptLook(TranscriptPhase.Asking)));
    }

    /// <summary>
    /// A failure the row offers "Try Again" for may be asked again from the menu too; one it deliberately offers nothing for —
    /// a refusal, a final error — gets no menu item that would re-ask behind the row's back.
    /// </summary>
    [Fact]
    public void AFailedAskIsOfferedAgainOnlyWhereTheRowOffersTryAgain()
    {
        var transient = ApiError.Transport("offline");
        var final = new ApiError(ErrorCodes.TranscriptRefused, "no", 422);
        Assert.True(TranscriptRules.MayRetry(transient));
        Assert.False(TranscriptRules.MayRetry(final));
        Assert.Equal(TextItem.Show, MessageMenu.TextItemFor(new TranscriptLook(TranscriptPhase.Failed, Error: transient)));
        Assert.Equal(TextItem.None, MessageMenu.TextItemFor(new TranscriptLook(TranscriptPhase.Failed, Error: final)));
        // No error recorded is the row's "no answer": a transport failure, which it offers Try Again for.
        Assert.Equal(TextItem.Show, MessageMenu.TextItemFor(new TranscriptLook(TranscriptPhase.Failed)));
    }
}
