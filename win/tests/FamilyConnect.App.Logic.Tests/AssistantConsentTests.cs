using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// Nothing a member writes reaches the model before that member has said yes (docs/protocol.md,
/// "Consenting to the assistant").
/// </summary>
/// <remarks>
/// The vectors are the ones <c>AssistantConsentTests.swift</c>,
/// <c>fc_text::assistant_consent</c>, Android's <c>AssistantConsentTest.kt</c> and the server's
/// own <c>model_surface</c> carry: a disagreement between them is either a message refused after
/// it was typed or one sent having asked nothing.
/// </remarks>
public sealed class AssistantConsentTests
{
    private const string Processor = "Microsoft — Azure OpenAI";

    [Fact]
    public void TheAssistantsOwnChatAlwaysReachesTheModel()
    {
        foreach (var body in new[] { "hello", "", "/draw a cat", "no mention here" })
        {
            Assert.True(AssistantConsent.ReachesTheModel("ai", body), body);
        }
    }

    [Fact]
    public void TheFamilyChatReachesItOnlyOnAMention()
    {
        Assert.True(AssistantConsent.ReachesTheModel("family", "@ai when is dinner?"));
        Assert.True(AssistantConsent.ReachesTheModel("family", "hey @AI"));
        // A picture request in the family chat IS a mention; a bare one asks nobody.
        Assert.True(AssistantConsent.ReachesTheModel("family", "@ai /draw a cat"));
        Assert.False(AssistantConsent.ReachesTheModel("family", "/draw a cat"));
        Assert.False(AssistantConsent.ReachesTheModel("family", "dinner at 7?"));
        Assert.False(AssistantConsent.ReachesTheModel("family", "write to anna@ai.example"));
        Assert.False(AssistantConsent.ReachesTheModel("family", "@aiden said so"));
    }

    [Fact]
    public void NowhereElseReachesIt()
    {
        foreach (var kind in new string?[] { "direct", "unknown", null })
        {
            Assert.False(AssistantConsent.ReachesTheModel(kind, "@ai hello"));
        }
    }

    [Fact]
    public void AskedOnceAndNotAgain()
    {
        Assert.True(AssistantConsent.IsRequired("ai", "hello", Processor, null));
        Assert.False(AssistantConsent.IsRequired("ai", "hello", Processor, "2026-09-19T19:34:43Z"));
        Assert.False(AssistantConsent.IsRequired("family", "dinner at 7?", Processor, null));
        Assert.False(AssistantConsent.IsRequired("direct", "@ai hello", Processor, null));
    }

    [Fact]
    public void AServerThatNamesNobodyOffersNoAssistant()
    {
        Assert.False(AssistantConsent.IsAvailable(null));
        Assert.False(AssistantConsent.IsAvailable(string.Empty));
        Assert.False(AssistantConsent.IsAvailable("   "));
        Assert.True(AssistantConsent.IsAvailable(Processor));
        Assert.False(AssistantConsent.IsRequired("ai", "hello", null, null));
    }

    /// <summary>
    /// A server that HAS an assistant but names nobody: consent cannot be asked for, so the
    /// message is held back rather than sent.
    /// </summary>
    [Fact]
    public void AnUnnamedAssistantWithholdsTheMessage()
    {
        Assert.True(AssistantConsent.IsWithheldFromAnUnnamedAssistant("ai", "hello", true, null));
        Assert.True(AssistantConsent.IsWithheldFromAnUnnamedAssistant("family", "@ai hi", true, ""));
        Assert.False(AssistantConsent.IsWithheldFromAnUnnamedAssistant("ai", "hello", true, Processor));
    }

    /// <summary>And the case that must NOT be swallowed.</summary>
    [Fact]
    public void WhereThereIsNoAssistantAtAllTheWordsAreJustWords()
    {
        Assert.False(AssistantConsent.IsWithheldFromAnUnnamedAssistant("family", "@ai hi", false, null));
    }

    [Fact]
    public void TheDisclosureNamesTheProcessorAndFollowsTheSwitches()
    {
        var say = EnglishCatalog.Instance;
        var withHistory = AssistantConsent.Disclosure(Processor, familyHistory: true, familyVision: true, say);
        Assert.Contains(withHistory, line => line.Contains(Processor, StringComparison.Ordinal));
        Assert.Contains(
            withHistory,
            line => line.Contains("30", StringComparison.Ordinal) && line.Contains("200", StringComparison.Ordinal));

        var without = AssistantConsent.Disclosure(Processor, familyHistory: false, familyVision: false, say);
        Assert.DoesNotContain(
            without,
            line => line.Contains("30", StringComparison.Ordinal) || line.Contains("200", StringComparison.Ordinal));

        // Photos are mentioned only where a photo could go.
        var withPictures = AssistantConsent.Disclosure(Processor, familyHistory: false, familyVision: true, say);
        Assert.Equal(without.Count + 1, withPictures.Count);
    }
}
