using FamilyConnect.App.Logic;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>The global shortcut (#80): what a press does, and the combination itself.</summary>
public sealed class GlobalHotKeyRulesTests
{
    [Theory]
    [InlineData(false, true, HotKeyAction.Show)]
    [InlineData(false, false, HotKeyAction.Show)]
    [InlineData(true, true, HotKeyAction.Hide)]
    [InlineData(true, false, HotKeyAction.Show)]
    public void FromAnywhereItShowsAndInFrontItHides(bool inFront, bool canHide, HotKeyAction action) =>
        Assert.Equal(action, GlobalHotKeyRules.Action(inFront, canHide));

    /// <summary>Never Ctrl+Alt alone: that is AltGr, and AltGr+F types "[" on Hungarian and Czech keyboards.</summary>
    [Fact]
    public void TheCombinationIsNeverAltGr()
    {
        const uint alt = 0x0001, control = 0x0002, shift = 0x0004, noRepeat = 0x4000;
        Assert.Equal(alt | control | shift | noRepeat, GlobalHotKeyRules.Modifiers);
        Assert.Equal(0x46u, GlobalHotKeyRules.Key);
        Assert.Equal("Ctrl+Alt+Shift+F", GlobalHotKeyRules.Label);
    }

    [Theory]
    [InlineData(1409, true)]
    [InlineData(0, false)]
    [InlineData(5, false)]
    public void OnlyAlreadyRegisteredIsTaken(int error, bool taken) => Assert.Equal(taken, GlobalHotKeyRules.IsTaken(error));
}
