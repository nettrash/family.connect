using FamilyConnect.App.Logic;
using FamilyConnect.Core;

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
            AttachMenu.Groups(assistantChat: false, offersPictureAttach: false, familyChat: true, roundAvailable: true),
            [AttachItem.PhotoOrVideo, AttachItem.File, AttachItem.Paste],
            [AttachItem.RecordVoice, AttachItem.RecordVideo],
            [AttachItem.Location, AttachItem.Poll]);
    }

    [Fact]
    public void ADirectChatHasNoPollAndNoVideoMessageWhereNoneCanBeRecorded()
    {
        Menu(
            AttachMenu.Groups(assistantChat: false, offersPictureAttach: false, familyChat: false, roundAvailable: false),
            [AttachItem.PhotoOrVideo, AttachItem.File, AttachItem.Paste],
            [AttachItem.RecordVoice],
            [AttachItem.Location]);
    }

    [Fact]
    public void TheAssistantsPhotoStandsInPhotoOrVideosPlaceAndNothingIsRecorded()
    {
        // Never both lines, and no recording group — so no empty group for a separator to sit around.
        Menu(
            AttachMenu.Groups(assistantChat: true, offersPictureAttach: true, familyChat: false, roundAvailable: true),
            [AttachItem.AssistantPhoto, AttachItem.File, AttachItem.Paste],
            [AttachItem.Location]);
    }

    [Fact]
    public void AnAssistantThatCannotSeeOffersNoPhotoLineAtAll()
    {
        Menu(
            AttachMenu.Groups(assistantChat: true, offersPictureAttach: false, familyChat: false, roundAvailable: false),
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
        {
            var groups = AttachMenu.Groups(assistant, pictures, family, round);
            Assert.All(groups, group => Assert.NotEmpty(group));
            var lines = groups.SelectMany(group => group).ToList();
            Assert.Equal(lines.Count, lines.Distinct().Count());
            Assert.False(lines.Contains(AttachItem.PhotoOrVideo) && lines.Contains(AttachItem.AssistantPhoto));
            Assert.Contains(AttachItem.File, lines);
            Assert.Contains(AttachItem.Paste, lines);
            Assert.Contains(AttachItem.Location, lines);
        }
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
