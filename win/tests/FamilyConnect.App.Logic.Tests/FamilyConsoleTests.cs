using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

public sealed class FamilyConsoleTests
{
    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    private static MemberDto Member(long id, string name, string role = "member", bool left = false, bool deleted = false) =>
        new(id, name.ToLowerInvariant(), name, Role: role, HasLeft: left, Deleted: deleted);

    [Fact]
    public void TheRosterIsWhoIsStillInSortedByNameWithoutRegardToCase()
    {
        var roster = FamilyText.Roster([
            Member(1, "Zoe"), Member(2, "anna"), Member(3, "Bob", role: "owner"),
            Member(4, "Carl", deleted: true), Member(5, "Dan", left: true), Member(6, "bea"),
        ]);

        Assert.Equal(["anna", "bea", "Bob", "Zoe"], roster.Select(member => member.DisplayName));
    }

    /// <summary>
    /// No "Message" for yourself or for somebody you blocked (it would only be refused); safety for
    /// anybody but yourself; the owner's birthday tool on every row, their own included; and a reset
    /// or removal never for yourself or the owner.
    /// </summary>
    [Fact]
    public void EachRowOffersWhatThisReaderMayDo()
    {
        var owner = Member(7, "Anna", role: "owner");
        var bob = Member(11, "Bob");

        Assert.Equal(new MemberTools(false, false, true, false), FamilyText.ToolsFor(owner, me: 7, owner: true, blocked: false));
        Assert.Equal(new MemberTools(true, true, true, true), FamilyText.ToolsFor(bob, me: 7, owner: true, blocked: false));
        Assert.Equal(new MemberTools(false, true, true, true), FamilyText.ToolsFor(bob, me: 7, owner: true, blocked: true));
        Assert.Equal(new MemberTools(true, true, false, false), FamilyText.ToolsFor(owner, me: 11, owner: false, blocked: false));
        Assert.Equal(new MemberTools(false, false, false, false), FamilyText.ToolsFor(bob, me: 11, owner: false, blocked: false));
        // A roster that still names somebody else as owner — ownership moved and the frame is on its
        // way — offers no removal of them.
        Assert.False(FamilyText.ToolsFor(owner, me: 12, owner: true, blocked: false).Removable);
    }

    [Fact]
    public void TheMemberCountAgreesWithItsNumber()
    {
        Assert.Equal("1 member", FamilyText.MembersLine(1, Say));
        Assert.Equal("3 members", FamilyText.MembersLine(3, Say));
        Assert.NotEqual("5 members", FamilyText.MembersLine(5, JsonCatalog.For("ru")));
    }

    [Fact]
    public void AReportSaysWhyWhoAndWhatTheMessageCarried()
    {
        Assert.Equal("Spam", FamilyText.ReasonLabel("spam", Say));
        Assert.Equal("Something else", FamilyText.ReasonLabel("threats", Say));
        Assert.Equal(FamilyText.FirstReason, FamilyText.Reasons[1].Code);
        Assert.Equal("Anna reported Bob",
            FamilyText.Reported(new ReportDto(1, new UserDto(7, "anna", "Anna"), new UserDto(11, "bob", "Bob"), "spam"), Say));

        var photo = new AttachmentDto(1, "photo");
        Assert.Null(FamilyText.Carried(null, Say));
        Assert.Null(FamilyText.Carried([], Say));
        Assert.Equal("Photo", FamilyText.Carried([photo], Say));
        Assert.Equal("2 Photos", FamilyText.Carried([photo, photo], Say));
        Assert.Equal("Video", FamilyText.Carried([new AttachmentDto(2, "video"), photo], Say));
        Assert.Equal("Voice message", FamilyText.Carried([new AttachmentDto(3, "audio")], Say));
        Assert.Equal("Location", FamilyText.Carried([new AttachmentDto(4, "location")], Say));
        Assert.Equal("minutes.pdf", FamilyText.Carried([new AttachmentDto(5, "file", Name: "minutes.pdf")], Say));
        Assert.Equal("File", FamilyText.Carried([new AttachmentDto(6, "file")], Say));
    }

    [Fact]
    public void TheReporterIsToldWhoWillSeeIt()
    {
        Assert.Equal("Your family owner will see this message and its text.", FamilyText.ReportDisclosure(true, Say));
        Assert.Equal("Your family owner will be told you reported this member.", FamilyText.ReportDisclosure(false, Say));
    }

    /// <summary>A server that answered with trouble of its own is not a network to go and check.</summary>
    [Fact]
    public void AFailureIsSaidForWhatItWas()
    {
        Assert.Equal("The server is busy. Try again in a moment.",
            FamilyText.GenericFailure(new ApiError(ErrorCodes.TooManyRequests, "x", 429), Say));
        Assert.Equal("The server is busy. Try again in a moment.",
            FamilyText.GenericFailure(new ApiError("rate_limited_by_proxy", "x", 429), Say));
        Assert.Equal("The server had a problem. Try again in a moment.",
            FamilyText.GenericFailure(new ApiError(ErrorCodes.Internal, "x", 500), Say));
        Assert.Equal("The server had a problem. Try again in a moment.",
            FamilyText.GenericFailure(new ApiError("http_502", "bad gateway", 502), Say));
        Assert.Equal("The server had a problem. Try again in a moment.",
            FamilyText.GenericFailure(new ApiError(ErrorCodes.StorageFull, "disk", 507), Say));
        Assert.Equal("The server had a problem. Try again in a moment.",
            FamilyText.GenericFailure(new ApiError("unreadable", "html", 418), Say));
        Assert.Equal("Can't reach the server. Check your connection.",
            FamilyText.GenericFailure(ApiError.Transport("reset"), Say));
        Assert.Equal("That didn't work. Try again.",
            FamilyText.GenericFailure(new ApiError(ErrorCodes.NotFamilyOwner, "x", 403), Say));
    }

    [Fact]
    public void AFullFamilyLeavesTheRequestWaiting()
    {
        Assert.EndsWith("the request is still waiting.",
            FamilyText.RequestFailure(new ApiError(ErrorCodes.FamilyFull, "x", 409), Say));
        Assert.Equal("That didn't work. Try again.",
            FamilyText.RequestFailure(new ApiError(ErrorCodes.JoinRequestNotPending, "x", 409), Say));
    }
}
