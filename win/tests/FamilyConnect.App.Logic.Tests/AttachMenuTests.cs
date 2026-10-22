using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The paperclip's menu (docs/attachment-menu-2026-10-07.md, issue #78): the same lines in the same three groups on every
/// client, top to bottom, no Camera on Windows, and no separator around a group that is empty.
/// </summary>
public sealed class AttachMenuTests
{
    private static void Menu(IReadOnlyList<IReadOnlyList<AttachItem>> actual, params AttachItem[][] expected)
    {
        Assert.Equal(expected.Length, actual.Count);
        for (var group = 0; group < expected.Length; group++)
        {
            Assert.Equal(expected[group], actual[group]);
        }
    }

    [Fact]
    public void AFamilyChatHasEveryLineInThreeGroups()
    {
        Menu(
            AttachMenu.Groups(assistantChat: false, offersPictureAttach: false, sharedChat: true, roundAvailable: true, assistantAvailable: false),
            [AttachItem.PhotoOrVideo, AttachItem.File, AttachItem.Paste],
            [AttachItem.RecordVoice, AttachItem.RecordVideo],
            [AttachItem.Location, AttachItem.Poll]);
    }

    [Fact]
    public void ADirectChatHasNoPollAndNoVideoMessageWhereNoneCanBeRecorded()
    {
        Menu(
            AttachMenu.Groups(assistantChat: false, offersPictureAttach: false, sharedChat: false, roundAvailable: false, assistantAvailable: true),
            [AttachItem.PhotoOrVideo, AttachItem.File, AttachItem.Paste],
            [AttachItem.RecordVoice],
            [AttachItem.Location]);
    }

    [Fact]
    public void TheAssistantsPhotoStandsInPhotoOrVideosPlaceAndNothingIsRecorded()
    {
        // Never both lines, and no recording group — so no empty group for a separator to sit around.
        Menu(
            AttachMenu.Groups(assistantChat: true, offersPictureAttach: true, sharedChat: false, roundAvailable: true, assistantAvailable: true),
            [AttachItem.AssistantPhoto, AttachItem.File, AttachItem.Paste],
            [AttachItem.Location]);
    }

    [Fact]
    public void AnAssistantThatCannotSeeOffersNoPhotoLineAtAll()
    {
        Menu(
            AttachMenu.Groups(assistantChat: true, offersPictureAttach: false, sharedChat: false, roundAvailable: false, assistantAvailable: false),
            [AttachItem.File, AttachItem.Paste],
            [AttachItem.Location]);
    }

    [Fact]
    public void NoGroupIsEverEmptyAndNoLineAppearsTwice()
    {
        foreach (var assistant in new[] { false, true })
        foreach (var pictures in new[] { false, true })
        foreach (var family in new[] { false, true })
        foreach (var round in new[] { false, true })
        foreach (var asks in new[] { false, true })
        {
            var groups = AttachMenu.Groups(assistant, pictures, family, round, asks);
            Assert.All(groups, group => Assert.NotEmpty(group));
            var lines = groups.SelectMany(group => group).ToList();
            Assert.Equal(lines.Count, lines.Distinct().Count());
            Assert.False(lines.Contains(AttachItem.PhotoOrVideo) && lines.Contains(AttachItem.AssistantPhoto));
            Assert.Contains(AttachItem.File, lines);
            Assert.Contains(AttachItem.Paste, lines);
            Assert.Contains(AttachItem.Location, lines);
            // "Ask the Assistant" only in the family chat of a server whose assistant can be asked — and first in its group.
            Assert.Equal(family && asks, lines.Contains(AttachItem.AskAssistant));
            if (family && asks)
            {
                Assert.Equal(AttachItem.AskAssistant, groups[^1][0]);
            }
        }
    }

    /// <summary>
    /// #106: the @ai button left the composer's row, and "Ask the Assistant" opens the family chat's last group — never a
    /// direct chat's, never the assistant's own (where every message already asks it).
    /// </summary>
    [Fact]
    public void AskTheAssistantOpensTheFamilyChatsLastGroup()
    {
        Menu(
            AttachMenu.Groups(assistantChat: false, offersPictureAttach: false, sharedChat: true, roundAvailable: false, assistantAvailable: true),
            [AttachItem.PhotoOrVideo, AttachItem.File, AttachItem.Paste],
            [AttachItem.RecordVoice],
            [AttachItem.AskAssistant, AttachItem.Location, AttachItem.Poll]);
        Menu(
            AttachMenu.Groups(assistantChat: false, offersPictureAttach: false, sharedChat: false, roundAvailable: false, assistantAvailable: true),
            [AttachItem.PhotoOrVideo, AttachItem.File, AttachItem.Paste],
            [AttachItem.RecordVoice],
            [AttachItem.Location]);
        Menu(
            AttachMenu.Groups(assistantChat: true, offersPictureAttach: false, sharedChat: false, roundAvailable: false, assistantAvailable: true),
            [AttachItem.File, AttachItem.Paste],
            [AttachItem.Location]);
    }

    /// <summary>An assistant can be asked only where the server names who answers (docs/protocol.md, "Consenting to the assistant").</summary>
    [Fact]
    public void TheAssistantIsAvailableOnlyWhereItsProcessorIsNamed()
    {
        Assert.True(AttachMenu.AssistantAvailable(new AssistantDto(99, "Assistant", "@ai", Processor: "Microsoft — Azure OpenAI")));
        Assert.False(AttachMenu.AssistantAvailable(new AssistantDto(99, "Assistant", "@ai")));
        Assert.False(AttachMenu.AssistantAvailable(new AssistantDto(99, "Assistant", "@ai", Processor: "  ")));
        Assert.False(AttachMenu.AssistantAvailable(null));
    }

    [Fact]
    public void ThePhotoOrVideoPickerOffersOnlyPicturesAndVideosTheClientKnows()
    {
        Assert.All(AttachMenu.PhotoOrVideoTypes, type =>
        {
            Assert.StartsWith(".", type);
            var mime = MediaPrep.MimeFor("x" + type);
            Assert.True(mime.StartsWith("image/", StringComparison.Ordinal) || mime.StartsWith("video/", StringComparison.Ordinal), $"{type} is {mime}");
        });
        Assert.Equal(AttachMenu.PhotoOrVideoTypes.Count, AttachMenu.PhotoOrVideoTypes.Distinct().Count());
        // Every video the server takes as a video, and every photo the client re-draws.
        foreach (var type in new[] { ".mp4", ".m4v", ".mov", ".jpg", ".jpeg", ".png", ".heic", ".heif", ".tif", ".tiff", ".avif" })
        {
            Assert.Contains(type, AttachMenu.PhotoOrVideoTypes);
        }
        Assert.DoesNotContain(".svg", AttachMenu.PhotoOrVideoTypes);
    }
}
