using System.Globalization;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

public sealed class SettingsTests
{
    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    private static MemberDto Member(long id, string name) => new(id, name.ToLowerInvariant(), name);

    [Fact]
    public void LeavingSaysWhatItMeansForWhoIsLeaving()
    {
        var bob = LeaveContext.For(owner: true, deletesFamily: false, Member(2, "Bob"))!;
        Assert.Equal(
            "Bob becomes the owner. You'll lose access to the family chat and your direct chats; your history returns if you rejoin.",
            SettingsText.LeaveMessage(bob, Say));
        var last = LeaveContext.For(owner: true, deletesFamily: true, null)!;
        Assert.Contains("deletes the family", SettingsText.LeaveMessage(last, Say));
        var member = LeaveContext.For(owner: false, deletesFamily: true, null)!;
        Assert.DoesNotContain("deletes", SettingsText.LeaveMessage(member, Say));
        // Deleting the family is its own question, and its own button.
        Assert.Equal("Delete the family?", SettingsText.LeaveTitle(last, Say));
        Assert.Equal("Leave and Delete", SettingsText.LeaveButton(last, Say));
        Assert.Equal("Leave Family", SettingsText.LeaveButton(bob, Say));
        Assert.Equal("Leave the family?", SettingsText.LeaveTitle(member, Say));
    }

    /// <summary>
    /// A MEMBER never gets the owner's dialogs, whatever the roster says; and an owner whose
    /// successor the fresh roster does not name gets NO dialog rather than a guess.
    /// </summary>
    [Fact]
    public void WhichDialogIsTheOwnersQuestionAndNeverAGuess()
    {
        Assert.Equal(LeaveKind.Member, LeaveContext.For(false, false, Member(2, "Bob"))!.Kind);
        Assert.Equal(LeaveKind.Member, LeaveContext.For(false, true, null)!.Kind);
        Assert.Null(LeaveContext.For(owner: true, deletesFamily: false, successor: null));
    }

    [Fact]
    public void AMembersLineCountsWhatTheySendBesidesWords()
    {
        var culture = CultureInfo.InvariantCulture;
        var member = new StatsMemberDto(1, "Anna", 3);
        Assert.Equal("Words only", SettingsText.MemberLine(member, Say, culture));

        member = member with
        {
            Attachments = new StatsMediaDto(1, Bytes: 1_234_567),
            Ai = new StatsAiDto(2, Images: 1),
        };
        Assert.Equal(
            $"1 attachment, {MediaText.DisplaySize(1_234_567, Say, culture)} · 2 questions to the assistant · 1 picture from the assistant",
            SettingsText.MemberLine(member, Say, culture));

        member = member with { Attachments = new StatsMediaDto(4, Bytes: 10), Ai = new StatsAiDto(1, Images: 3) };
        Assert.StartsWith("4 attachments, ", SettingsText.MemberLine(member, Say, culture));
        Assert.EndsWith("1 question to the assistant · 3 pictures from the assistant", SettingsText.MemberLine(member, Say, culture));
    }

    /// <summary>A translation the apps say one way whatever the count is said that way, with its own order.</summary>
    [Fact]
    public void AMembersLineReadsInTheReadersLanguage()
    {
        var russian = JsonCatalog.For("ru");
        var member = new StatsMemberDto(1, "Анна", 3, Ai: new StatsAiDto(1));

        Assert.Equal("вопросов ассистенту: 1", SettingsText.MemberLine(member, russian, CultureInfo.InvariantCulture));
    }

    [Fact]
    public void APictureRefusedIsSaidForWhatItWas()
    {
        Assert.Equal("That photo is too large for this server.",
            SettingsText.PictureFailure(new ApiError(ErrorCodes.AvatarTooLarge, "x", 413), true, Say));
        Assert.Equal("That photo is too large for this server.",
            SettingsText.PictureFailure(new ApiError("payload_too_large", "proxy", 413), true, Say));
        Assert.Equal("That file isn't a photo we can use.",
            SettingsText.PictureFailure(new ApiError(ErrorCodes.InvalidImage, "x", 400), true, Say));
        Assert.Equal("Can't reach the server. Check your connection.",
            SettingsText.PictureFailure(ApiError.Transport("reset"), false, Say));
        Assert.Equal("The server is busy. Try again in a moment.",
            SettingsText.PictureFailure(new ApiError(ErrorCodes.TooManyRequests, "slow", 429), true, Say));
        Assert.Equal("Couldn't upload the photo.",
            SettingsText.PictureFailure(new ApiError(ErrorCodes.Internal, "x", 500), true, Say));
        Assert.Equal("Couldn't remove the photo.",
            SettingsText.PictureFailure(new ApiError(ErrorCodes.Internal, "x", 500), false, Say));
    }

    /// <summary>
    /// Eight SCALARS, typed twice: seven Cyrillic letters are seven, and four family emoji are four
    /// graphemes the apps refuse but twenty-eight scalars the server takes.
    /// </summary>
    [Fact]
    public void ANewPasswordIsEightScalarsAndTypedTwice()
    {
        Assert.Equal("Use at least 8 characters.", SettingsText.PasswordProblem("1234567", "1234567", Say));
        Assert.Equal("Those two do not match.", SettingsText.PasswordProblem("12345678", "12345679", Say));
        Assert.Null(SettingsText.PasswordProblem("12345678", "12345678", Say));
        // A password is its exact characters: typed twice means typed the same, capitals included.
        Assert.Equal("Those two do not match.", SettingsText.PasswordProblem("Password1", "password1", Say));
        Assert.Equal("Use at least 8 characters.", SettingsText.PasswordProblem("пароль1", "пароль1", Say));
        var family = string.Concat(Enumerable.Repeat("👨‍👩‍👧‍👦", 4));
        Assert.Null(SettingsText.PasswordProblem(family, family, Say));
        Assert.Equal("Use at least 8 characters.", SettingsText.PasswordProblem("😀😀😀😀", "😀😀😀😀", Say));
    }

    [Fact]
    public void AccountRefusalsAreSaidAsTheWebSaysThem()
    {
        Assert.Equal("That password is not right.",
            SettingsText.DeleteFailure(new ApiError(ErrorCodes.InvalidCredentials, "x", 401), Say));
        Assert.Equal("Type your password to confirm.",
            SettingsText.DeleteFailure(new ApiError(ErrorCodes.Validation, "x", 400), Say));
        Assert.Equal("Couldn't delete your account. Try again.",
            SettingsText.DeleteFailure(ApiError.Transport("x"), Say));
        Assert.Equal("That current password is not right.",
            SettingsText.PasswordChangeFailure(new ApiError(ErrorCodes.InvalidCredentials, "x", 401), Say));
        Assert.Equal("Couldn't change your password. Try again.",
            SettingsText.PasswordChangeFailure(new ApiError(ErrorCodes.Internal, "x", 500), Say));
        Assert.Equal("That date doesn't exist.",
            SettingsText.BirthdayFailure(new ApiError(ErrorCodes.Validation, "x", 400), Say));
        Assert.Equal("Only the family owner can do that.",
            SettingsText.BirthdayFailure(new ApiError(ErrorCodes.NotFamilyOwner, "x", 403), Say));
        Assert.Equal("Couldn't save that birthday. Try again.",
            SettingsText.BirthdayFailure(ApiError.Transport("x"), Say));
    }

    /// <summary>February has 29, and a picker opens on the birthday held — inside the calendar.</summary>
    [Fact]
    public void ABirthdayHasNoYearForTheTwentyNinthToFailIn()
    {
        Assert.Equal([31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31], Enumerable.Range(1, 12).Select(BirthdayRules.DaysIn));
        Assert.True(BirthdayRules.Ok(2, 29));
        Assert.False(BirthdayRules.Ok(4, 31));
        Assert.False(BirthdayRules.Ok(13, 1));
        Assert.False(BirthdayRules.Ok(1, 0));
        Assert.Equal((1, 1), BirthdayRules.Start(null));
        Assert.Equal((4, 30), BirthdayRules.Start(new BirthdayDto(4, 31)));
        Assert.Equal((12, 1), BirthdayRules.Start(new BirthdayDto(14, -3)));
    }

    [Fact]
    public void ABirthdayIsWrittenTheReadersWay()
    {
        Assert.Equal("March 12", BirthdayRules.Text(new BirthdayDto(3, 12), Say, CultureInfo.GetCultureInfo("en-US")));
        Assert.Equal("12 марта", BirthdayRules.Text(new BirthdayDto(3, 12), Say, CultureInfo.GetCultureInfo("ru-RU")));
        Assert.Equal("February 29", BirthdayRules.Text(new BirthdayDto(2, 29), Say, CultureInfo.GetCultureInfo("en-US")));
        Assert.Equal("Not set", BirthdayRules.Text(null, Say, CultureInfo.InvariantCulture));
        Assert.Equal("Not set", BirthdayRules.Text(new BirthdayDto(2, 30), Say, CultureInfo.InvariantCulture));
    }

    [Fact]
    public void WhatDeduplicationSavedIsTheFamilysTotalsAndOnlyWhenItSavedAnything()
    {
        Assert.Null(SettingsText.Saved(null));
        Assert.Null(SettingsText.Saved(new StatsMediaDto(3, Bytes: 100)));
        Assert.Null(SettingsText.Saved(new StatsMediaDto(3, Bytes: 100, StoredBytes: 100)));
        Assert.Null(SettingsText.Saved(new StatsMediaDto(3, Bytes: 100, StoredBytes: 140)));
        Assert.Equal(60, SettingsText.Saved(new StatsMediaDto(3, Bytes: 100, StoredBytes: 40)));
    }
}
