using FamilyConnect.Core;
using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

public sealed class WallTextTests
{
    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    private static Sticker Sticker(long id) =>
        new(new NoteDto(id, 7), NoteKind.Text, NoteSize.Medium, NoteFont.Plain, "yellow", 0, 0, 150, 110, 0, false, false);

    [Fact]
    public void ANoteIsHiddenWhileItsAuthorIsBlockedUntilThePeek()
    {
        var bobs = new NoteDto(3, AuthorId: 11);
        var mine = new NoteDto(4, AuthorId: 7);
        bool Blocked(long id) => id is 11 or 7;

        Assert.True(WallText.IsHidden(bobs, 7, Blocked, new HashSet<long>()));
        Assert.False(WallText.IsHidden(bobs, 7, Blocked, new HashSet<long> { 3 }));
        Assert.False(WallText.IsHidden(bobs, 7, _ => false, new HashSet<long>()));
        // Never the reader's own, whatever the list says.
        Assert.False(WallText.IsHidden(mine, 7, Blocked, new HashSet<long>()));
    }

    [Fact]
    public void AStickerIsSignedByWhoWroteIt()
    {
        string? Names(long id) => id == 11 ? "Bob" : id == 12 ? "" : null;
        Assert.Equal("You", WallText.AuthorName(new NoteDto(1, 7), 7, Names, Say));
        Assert.Equal("Bob", WallText.AuthorName(new NoteDto(1, 11), 7, Names, Say));
        Assert.Equal("Someone", WallText.AuthorName(new NoteDto(1, 12), 7, Names, Say));
        Assert.Equal("Someone", WallText.AuthorName(new NoteDto(1, 99), 7, Names, Say));
    }

    /// <summary>A bare photo is the picture: no words and no author. A hidden note: words saying so, and no author.</summary>
    [Fact]
    public void WhatACardCarriesFollowsWhatItIs()
    {
        Assert.False(WallText.ShowsText(NoteKind.Photo, "", hidden: false));
        Assert.False(WallText.ShowsAuthor(NoteKind.Photo, "", hidden: false));
        Assert.True(WallText.ShowsText(NoteKind.Photo, "Beach", hidden: false));
        Assert.True(WallText.ShowsAuthor(NoteKind.Photo, "Beach", hidden: false));
        Assert.True(WallText.ShowsText(NoteKind.Photo, "", hidden: true));
        Assert.False(WallText.ShowsAuthor(NoteKind.Text, "Milk", hidden: true));
        Assert.True(WallText.ShowsText(NoteKind.Text, "", hidden: false));
        Assert.True(WallText.ShowsAuthor(NoteKind.Event, "Dinner", hidden: false));
    }

    [Fact]
    public void ALabelStandsInForTheCardsContent()
    {
        Assert.Equal("a photo", WallText.What(NoteKind.Photo, "", null, null, null, Say));
        Assert.Equal("Beach", WallText.What(NoteKind.Photo, "Beach", "ignored", "x", "y", Say));
        // Only a PHOTO with no words is "a photo": a note with none is a note with none.
        Assert.Equal(string.Empty, WallText.What(NoteKind.Text, "", null, null, null, Say));
        Assert.Equal("Dinner, Thu, 24 Dec, 16:00, Kitchen, 2 going",
            WallText.What(NoteKind.Event, "Dinner", "Thu, 24 Dec, 16:00", "Kitchen", "2 going", Say));
        Assert.Equal("Dinner, Thu, 24 Dec, 16:00", WallText.What(NoteKind.Event, "Dinner", "Thu, 24 Dec, 16:00", "", null, Say));
        Assert.Equal("Dinner", WallText.What(NoteKind.Event, "Dinner", "", "Kitchen", "2 going", Say));

        Assert.Equal("Hidden note from a blocked member", WallText.Label(true, false, "Bob", "Milk", Say));
        Assert.Equal("Your note: Milk", WallText.Label(false, true, "You", "Milk", Say));
        Assert.Equal("Note from Bob: Milk", WallText.Label(false, false, "Bob", "Milk", Say));
    }

    [Fact]
    public void TheMostRecentlyChangedNoteStacksHighest()
    {
        var layers = WallText.Layers([Sticker(9), Sticker(2), Sticker(5)]);

        Assert.Equal(3, layers[9]);
        Assert.Equal(2, layers[2]);
        Assert.Equal(1, layers[5]);
    }

    [Fact]
    public void ALongListSaysHowManyItLeftOff()
    {
        Assert.Equal("+1 more", WallText.MoreLine(1, Say));
        Assert.Equal("+3 more", WallText.MoreLine(3, Say));
    }
}
