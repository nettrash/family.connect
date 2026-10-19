namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// A TextBox with an undo count, driven the way <c>Views/CapitalLetters.cs</c> drives <see cref="CapitalField"/>: the same calls
/// in the same order the window makes them — PreviewKeyDown, the change before it lands (<c>BeforeTextChanging</c>), the
/// selection the change leaves, the high-priority turn that applies the capital or the emoji, the Backspace — so the wiring the
/// Mac cannot run is held to its sequence here (issues #97, #101).
/// </summary>
internal sealed class TypingBox(bool capitals = true)
{
    public const int VkA = 0x41;
    public const int VkBack = 0x08;
    public const int VkEnter = 0x0D;
    public const int VkLeft = 0x25;
    public const int VkRight = 0x27;
    public const int VkV = 0x56;

    public string Text = string.Empty;
    public int Start;
    public int Length;
    public int UndoUnits;

    /// <summary>The box's <c>MaxLength</c>: 0 is none, and over it the box refuses what is typed.</summary>
    public int MaxLength;

    public readonly CapitalField Field = new(capitals);
    private CapitalPlan? pending;

    /// <summary>A key typed on a hardware keyboard: PreviewKeyDown, then the change it makes, then the high-priority turn.</summary>
    public void Type(string typed, int key = VkA, bool control = false, bool alt = false)
    {
        Field.Key(CapitalField.Classify(key, control, alt));
        if (MaxLength > 0 && Text.Length - Length + typed.Length > MaxLength)
        {
            // The box refuses the character: no change at all.
            return;
        }
        Change(Text[..Start] + typed + Text[(Start + Length)..], Start + typed.Length);
        Turn();
    }

    /// <summary>A key that types nothing in this box (Enter in a single-line one, a dead key): only PreviewKeyDown.</summary>
    public void TypeNothing(int key = VkEnter) => Field.Key(CapitalField.Classify(key, false, false));

    /// <summary>The control's own edit: BeforeTextChanging with the old text and selection, then the new text and caret.</summary>
    public void Change(string newText, int caret)
    {
        pending = Field.BeforeChange(Text, Start, Length, newText) ?? pending;
        Text = newText;
        Start = caret;
        Length = 0;
        UndoUnits++;
        // The caret the change leaves raises SelectionChanged like any other move — not inside the window's own edit.
        Field.SelectionMoved(Start, Length);
    }

    /// <summary>The high-priority turn after a change: the replacement through the selection — one more undo unit.</summary>
    public void Turn()
    {
        if (pending is not { } plan)
        {
            return;
        }
        pending = null;
        if (!Field.StillApplies(plan, Text, Start, Length, MaxLength))
        {
            return;
        }
        Field.Applying = true;
        Text = Text[..plan.At] + plan.Replacement + Text[(plan.At + plan.Length)..];
        Start = plan.Caret;
        UndoUnits++;
        Field.SelectionMoved(Start, Length);
        Field.Applying = false;
        Field.Applied(plan);
    }

    /// <summary>Backspace: the take-back, or the control's own deletion of one character (a surrogate pair as a whole).</summary>
    public void Backspace()
    {
        if (Field.Backspace(Text, Start, Length) is { } restore)
        {
            Field.Applying = true;
            Text = Text[..restore.At] + restore.Replacement + Text[(restore.At + restore.Length)..];
            Start = restore.Caret;
            Field.SelectionMoved(Start, Length);
            Field.Applying = false;
            return;
        }
        Field.Key(CapitalField.Classify(VkBack, false, false));
        if (Length > 0)
        {
            Change(Text[..Start] + Text[(Start + Length)..], Start);
        }
        else if (Start > 0)
        {
            var one = Start >= 2 && char.IsSurrogatePair(Text[Start - 2], Text[Start - 1]) ? 2 : 1;
            Change(Text[..(Start - one)] + Text[Start..], Start - one);
        }
    }

    /// <summary>A click that puts the caret at <paramref name="caret"/>.</summary>
    public void Click(int caret, int length = 0)
    {
        Start = caret;
        Length = length;
        Field.SelectionMoved(Start, Length);
    }

    /// <summary>Ctrl+V: the chord, the paste event, then the change.</summary>
    public void Paste(string pasted)
    {
        Field.Key(CapitalField.Classify(VkV, control: true, alt: false));
        Field.Pasting();
        Change(Text[..Start] + pasted + Text[(Start + Length)..], Start + pasted.Length);
        Turn();
    }

    /// <summary>
    /// Every character of <paramref name="keys"/> typed (a surrogate pair as one), with <c>⌫</c> a Backspace, <c>→</c> an
    /// arrow key, and <c>\r</c> or <c>\n</c> Enter — the vectors' alphabet, through the window's sequence.
    /// </summary>
    public void TypeAll(string keys)
    {
        foreach (var rune in keys.EnumerateRunes())
        {
            switch (rune.Value)
            {
                case '⌫':
                    Backspace();
                    break;
                case '→':
                    // An arrow that, at the end of the text, moves nothing: the key alone is "something else happened".
                    Field.Key(CapitalField.Classify(VkRight, false, false));
                    break;
                case '\r' or '\n':
                    Type(rune.ToString(), VkEnter);
                    break;
                default:
                    Type(rune.ToString());
                    break;
            }
        }
    }
}
