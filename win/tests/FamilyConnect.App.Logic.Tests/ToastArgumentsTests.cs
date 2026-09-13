using System.Globalization;
using FamilyConnect.App.Logic;
using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>What a notification carries back when it is clicked, and when a note is worth one.</summary>
public class ToastArgumentsTests
{
    private static IReadOnlyDictionary<string, string> Args(params (string Key, string Value)[] pairs) =>
        pairs.ToDictionary(pair => pair.Key, pair => pair.Value);

    [Fact]
    public void AChatNotificationOpensItsChatAndTheWallsOpensTheWall()
    {
        var chat = ToastArguments.For(new Toast("chat-42", "The Smiths", "New message", 42));
        Assert.Equal(new ToastTarget(42), ToastArguments.Parse(chat.ToDictionary()));

        var wall = ToastArguments.For(new Toast("board", "The Smiths", "New note"));
        Assert.True(ToastArguments.Parse(wall.ToDictionary())!.IsBoard);
    }

    /// <summary>AN ARGUMENT IS UNTRUSTED INPUT: anything but a positive id opens nothing.</summary>
    [Fact]
    public void WhatTheAppDidNotWriteOpensNothing()
    {
        Assert.Null(ToastArguments.Parse(Args()));
        Assert.Null(ToastArguments.Parse(Args(("chat", "abc"))));
        Assert.Null(ToastArguments.Parse(Args(("chat", "-5"))));
        Assert.Null(ToastArguments.Parse(Args(("chat", "0"))));
        Assert.Null(ToastArguments.Parse(Args(("chat", " 42"))));
        Assert.Null(ToastArguments.Parse(Args(("chat", "4 2"))));
        Assert.Null(ToastArguments.Parse(Args(("elsewhere", "42"))));
    }

    [Fact]
    public void TheIdIsWrittenTheSameWayInEveryLanguage()
    {
        var was = CultureInfo.CurrentCulture;
        try
        {
            CultureInfo.CurrentCulture = CultureInfo.GetCultureInfo("fi-FI");
            var big = ToastArguments.For(new Toast("chat-1234567", "t", "b", 1234567));
            Assert.Equal("1234567", big[0].Value);
            Assert.Equal("chat-1234567", NotificationRules.ChatTag(1234567));
        }
        finally
        {
            CultureInfo.CurrentCulture = was;
        }
    }

    /// <summary>A NOTE IS NEWS BY THE BADGE'S RULE, and a tombstone never is.</summary>
    [Fact]
    public void ANoteIsNewsExactlyWhenTheBadgeWouldCountIt()
    {
        var marks = new BoardMarks(NoteId: 5, ContentSeq: 10);
        var note = new NoteDto(7, 11, "text", "Milk", BoardSeq: 12, ContentSeq: 12);

        Assert.Equal(BoardBadge.IsUnread(7, 12, marks), ToastArguments.IsNews(note, marks));
        Assert.True(ToastArguments.IsNews(note, marks));
        Assert.False(ToastArguments.IsNews(note with { Deleted = true }, marks));
        Assert.False(ToastArguments.IsNews(note, new BoardMarks(NoteId: 7, ContentSeq: 12)));
    }
}
