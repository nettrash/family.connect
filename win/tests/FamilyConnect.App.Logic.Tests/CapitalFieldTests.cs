namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// A prose field's capital letters as a TextBox drives them (issue #97; <see cref="CapitalField"/>): a fake box
/// (<see cref="TypingBox"/>) that raises the same calls in the same order the window makes them — the key, the change before it
/// lands, the turn that applies the capital, the Backspace — so the wiring the Mac cannot run is held to its sequence here. The
/// same field's emoji (#101) are <see cref="EmojiTypingTests"/>'.
/// </summary>
public sealed class CapitalFieldTests
{
    private const int VkA = 0x41;
    private const int VkBack = 0x08;
    private const int VkShift = 0x10;
    private const int VkV = 0x56;
    private const int VkLeft = 0x25;

    [Fact]
    public void ASentenceStartsWithACapital()
    {
        var box = new TypingBox();
        box.TypeAll("hi. how are you? fine");
        Assert.Equal("Hi. How are you? Fine", box.Text);
        Assert.Equal(box.Text.Length, box.Start);
    }

    /// <summary>The letter goes in as typed and is then replaced: two undo units, so Ctrl+Z gives the lower-case letter back.</summary>
    [Fact]
    public void TheCapitalIsItsOwnUndoStep()
    {
        var box = new TypingBox();
        box.Type("h");
        Assert.Equal("H", box.Text);
        Assert.Equal(2, box.UndoUnits);
    }

    [Fact]
    public void BackspaceStraightAfterTakesItBack()
    {
        var box = new TypingBox();
        box.Type("h");
        box.Backspace();
        Assert.Equal("h", box.Text);
        Assert.Equal(1, box.Start);
        // Once: the next Backspace is an ordinary one.
        box.Backspace();
        Assert.Equal(string.Empty, box.Text);
    }

    [Fact]
    public void AnythingElseForgetsTheTakeBack()
    {
        var box = new TypingBox();
        box.Type("h");
        box.Type("i");
        box.Backspace();
        box.Backspace();
        Assert.Equal(string.Empty, box.Text);

        // An arrow.
        box = new TypingBox();
        box.Type("h");
        box.Field.Key(CapitalField.Classify(VkLeft, false, false));
        Assert.Null(box.Field.Backspace(box.Text, box.Start, box.Length));

        // A click elsewhere.
        box = new TypingBox();
        box.Type("h");
        box.Field.SelectionMoved(0, 0);
        Assert.Null(box.Field.TakeBack);

        // Losing the focus.
        box = new TypingBox();
        box.Type("h");
        box.Field.FocusLost();
        Assert.Null(box.Field.TakeBack);
    }

    /// <summary>Shift on its own is not "another key": Shift+Backspace still takes the capital back.</summary>
    [Fact]
    public void AModifierForgetsNothing()
    {
        var box = new TypingBox();
        box.Type("h");
        box.Field.Key(CapitalField.Classify(VkShift, false, false));
        box.Field.SelectionMoved(1, 0);
        box.Backspace();
        Assert.Equal("h", box.Text);
    }

    /// <summary>The window's own <c>Text =</c> — a draft put back, a message opened for editing — is never capitalised.</summary>
    [Fact]
    public void OnlyTypingIsJudged()
    {
        var box = new TypingBox();
        box.Change("h", 1);
        box.Turn();
        Assert.Equal("h", box.Text);
    }

    [Fact]
    public void APasteIsNotTyping()
    {
        var box = new TypingBox();
        box.Field.Key(CapitalField.Classify(VkV, control: true, alt: false));
        box.Field.Pasting();
        box.Change("h", 1);
        box.Turn();
        Assert.Equal("h", box.Text);
    }

    /// <summary>AltGr is Ctrl+Alt and TYPES: <c>ą</c> on a Polish keyboard starts a sentence like any letter.</summary>
    [Fact]
    public void AltGrTypes()
    {
        var box = new TypingBox();
        box.Type("ą", VkA, control: true, alt: true);
        Assert.Equal("Ą", box.Text);
        Assert.Equal(CapitalKey.Chord, CapitalField.Classify(VkV, control: true, alt: false));
        Assert.Equal(CapitalKey.Chord, CapitalField.Classify(VkBack, control: true, alt: false));
        Assert.Equal(CapitalKey.Backspace, CapitalField.Classify(VkBack, control: false, alt: false));
        Assert.Equal(CapitalKey.Modifier, CapitalField.Classify(0xA2, control: true, alt: false));
        Assert.Equal(CapitalKey.Moving, CapitalField.Classify(VkLeft, control: false, alt: false));
        Assert.Equal(CapitalKey.Moving, CapitalField.Classify(0x09, control: false, alt: false));
        Assert.Equal(CapitalKey.Typing, CapitalField.Classify(0x0D, control: false, alt: false));
        Assert.Equal(CapitalKey.Typing, CapitalField.Classify(0x20, control: false, alt: false));
    }

    /// <summary>A key that types nothing makes nothing after it typing: a name accepted on Tab, a draft put back on an arrow.</summary>
    [Fact]
    public void AKeyThatTypesNothingTypesNothing()
    {
        var box = new TypingBox();
        box.Field.Key(CapitalField.Classify(0x09, false, false));
        box.Change("h", 1);
        box.Turn();
        Assert.Equal("h", box.Text);
    }

    [Fact]
    public void ACompositionIsLeftAlone()
    {
        var box = new TypingBox();
        box.Field.CompositionStarted();
        box.Type("k");
        Assert.Equal("k", box.Text);
        box.Field.CompositionEnded();
        box.Text = string.Empty;
        box.Start = 0;
        box.Type("k");
        Assert.Equal("K", box.Text);
    }

    /// <summary>A change that went somewhere else before the turn — the control refused it, or more was typed — is left as typed.</summary>
    [Fact]
    public void AStaleCapitalIsNotApplied()
    {
        var box = new TypingBox();
        box.Field.Key(CapitalKey.Typing);
        var plan = box.Field.BeforeChange(string.Empty, 0, 0, "h");
        Assert.NotNull(plan);
        Assert.False(box.Field.StillApplies(plan, "hx", 2, 0));
        Assert.False(box.Field.StillApplies(plan, "h", 0, 0));
        Assert.False(box.Field.StillApplies(plan, "h", 1, 1));
        Assert.True(box.Field.StillApplies(plan, "h", 1, 0));
    }

    /// <summary>A letter typed over a selection is typed: the rule reads the text before the selection's start.</summary>
    [Fact]
    public void TypingOverASelection()
    {
        var box = new TypingBox { Text = "Ok. word", Start = 4, Length = 4 };
        box.Type("n");
        Assert.Equal("Ok. N", box.Text);
    }

    /// <summary>Enter's new line is typing too (a CR, as the TextBox writes one), and the next letter starts a line.</summary>
    [Fact]
    public void ANewLineStartsASentence()
    {
        var box = new TypingBox();
        box.TypeAll("ok");
        box.Type("\r", key: 0x0D);
        box.Type("n");
        Assert.Equal("Ok\rN", box.Text);
    }
}
