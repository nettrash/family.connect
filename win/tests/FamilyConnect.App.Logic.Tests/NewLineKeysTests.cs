using FamilyConnect.App.Logic;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>#95: Enter sends; Enter with Shift, Ctrl or Alt is a new line — and a new line goes out as a line feed.</summary>
public sealed class NewLineKeysTests
{
    [Theory]
    [InlineData(false, false, false, false)]
    [InlineData(true, false, false, true)]
    [InlineData(false, true, false, true)]
    [InlineData(false, false, true, true)]
    [InlineData(true, true, true, true)]
    public void EnterAloneSendsAndAnyModifierMakesALine(bool shift, bool control, bool alt, bool line) =>
        Assert.Equal(line, NewLineKeys.EnterMakesLine(shift, control, alt));

    [Theory]
    [InlineData("one\rtwo", "one\ntwo")]
    [InlineData("one\r\ntwo", "one\ntwo")]
    [InlineData("one\ntwo", "one\ntwo")]
    [InlineData("a\r\rb", "a\n\nb")]
    [InlineData("a\r\n\r\nb\r", "a\n\nb\n")]
    [InlineData("", "")]
    [InlineData("no lines", "no lines")]
    public void ATextBoxLineBreakGoesOutAsALineFeed(string typed, string sent) =>
        Assert.Equal(sent, NewLineKeys.ForWire(typed));

    [Fact]
    public void TextWithoutACarriageReturnIsTheSameString()
    {
        var text = "hello\nworld";
        Assert.Same(text, NewLineKeys.ForWire(text));
    }

    [Fact]
    public void ANoteDraftHoldsLineFeeds()
    {
        var draft = new NoteDraft { Text = "milk\reggs" };
        Assert.Equal("milk\neggs", draft.Text);
    }
}
