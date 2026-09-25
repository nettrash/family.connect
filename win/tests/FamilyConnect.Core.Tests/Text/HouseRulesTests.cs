using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.Core.Tests.Text;

public class HouseRulesTests
{
    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    [Fact]
    public void TheMemberLimitFooterSaysWhichOfItsThreeStatesItIsIn()
    {
        Assert.Equal("No limit of your own. This server allows up to 50 members in a family.",
            HouseRules.CapFooter(HouseRules.Cap(null, 3, 50), Say));
        Assert.Equal("3 of 5 seats used.", HouseRules.CapFooter(HouseRules.Cap(5, 3, 50), Say));
        // The LOWER of the two binds: a cap of 40 under a ceiling of 10 is ten seats.
        Assert.Equal("3 of 10 seats used.", HouseRules.CapFooter(HouseRules.Cap(40, 3, 10), Say));
        Assert.Equal("4 members now. Nobody new can join until somebody leaves; no one is removed.",
            HouseRules.CapFooter(HouseRules.Cap(2, 4, 10), Say));
        Assert.Equal("1 member now. Nobody new can join until somebody leaves; no one is removed.",
            HouseRules.CapFooter(HouseRules.Cap(1, 1, 10), Say));
    }

    [Fact]
    public void ThePolicyCaptionFollowsThePolicy()
    {
        Assert.Equal("Anyone with the invite code joins straight away.", HouseRules.PolicyCaption("open", Say));
        Assert.Equal("Anyone with the invite code joins straight away.", HouseRules.PolicyCaption(null, Say));
        Assert.StartsWith("With approval", HouseRules.PolicyCaption("approval", Say));
        Assert.StartsWith("The invite code stops working", HouseRules.PolicyCaption("closed", Say));
        Assert.Equal(["open", "approval", "closed"], HouseRules.Policies.Select(policy => policy.Code));
    }

    private static FamilyDto Family(bool vision, bool history) => new(3, "The Smiths", AiVision: vision, AiHistory: history);

    /// <summary>
    /// A switch that rides on vision says why it does nothing — the server, then vision, then the
    /// history it draws from — and nothing once it does something.
    /// </summary>
    [Fact]
    public void ASwitchThatRidesOnVisionSaysWhyItDoesNothing()
    {
        const string historyOff = "history is off";
        Assert.StartsWith("Not available here", HouseRules.PicturesNote(false, Family(true, true), historyOff, Say));
        Assert.StartsWith("Turn on Can be shown photos first", HouseRules.PicturesNote(true, Family(false, false), historyOff, Say));
        Assert.Equal(historyOff, HouseRules.PicturesNote(true, Family(true, false), historyOff, Say));
        Assert.Null(HouseRules.PicturesNote(true, Family(true, true), historyOff, Say));
    }

    [Fact]
    public void AnAssistantChangeRefusedIsSaidForWhatItWas()
    {
        Assert.Equal("Only the family owner can change this.",
            HouseRules.AssistantFailure(new ApiError(ErrorCodes.NotFamilyOwner, "x", 403), Say));
        Assert.Equal("Couldn't save that. Try again.", HouseRules.AssistantFailure(ApiError.Transport("x"), Say));
    }
}
