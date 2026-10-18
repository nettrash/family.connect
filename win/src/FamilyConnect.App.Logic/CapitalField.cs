namespace FamilyConnect.App.Logic;

/// <summary>What a key press is, to <see cref="CapitalField"/>.</summary>
public enum CapitalKey
{
    /// <summary>Shift, Ctrl, Alt, the Windows key, Caps Lock: on their own they change nothing, and forget nothing.</summary>
    Modifier,

    /// <summary>Backspace with nothing but Shift held: the one key that may take a capital back.</summary>
    Backspace,

    /// <summary>
    /// A Ctrl chord without Alt — Ctrl+V, Ctrl+Z, Ctrl+Backspace: a paste, a step through history or a word deleted, never a letter
    /// typed. (Ctrl with Alt is AltGr, which TYPES on most European layouts: <c>ą</c> is AltGr+A in Polish.)
    /// </summary>
    Chord,

    /// <summary>
    /// A key that types nothing: an arrow, Home, End, Page Up or Down, Tab, Esc, Insert, Delete, a function key, the menu key.
    /// It forgets the take-back, and whatever changes the text after it — a name accepted from the mention list on Tab, a draft
    /// put back — was not typed.
    /// </summary>
    Moving,

    /// <summary>Anything else: a letter, a digit, a space, punctuation, Enter.</summary>
    Typing,
}

/// <summary>
/// One multi-line prose field's capital letters (issue #97; <see cref="Capitals"/>): the state a <c>TextBox</c> keeps between its
/// events, and every decision about them, so the window's part is to read the box, ask, and apply. The order the events come in
/// is a hardware keyboard's — the key, then the text change it makes — and what this class assumes is written next to each.
/// </summary>
/// <remarks>
/// <para>
/// <b>Only typing is judged.</b> A change is a candidate only when a key went down just before it (<see cref="Key"/>), the field
/// is not composing (an IME's candidate text is not typing), and it is not a paste (<see cref="Pasting"/>). Every
/// <c>box.Text = …</c> the window does — a draft put back, a message opened for editing, a name accepted from the mention list —
/// is therefore left exactly as it is, even when it happens to be one lower-case letter. The touch keyboard, which raises no key
/// events, is left alone too, and it capitalises sentences itself.
/// </para>
/// <para>
/// <b>Applied after, never instead.</b> The letter the keyboard typed goes into the box as typed — one undo unit — and is then
/// replaced by its capital through the selection — a second — on the next high-priority turn, ahead of the next input. So the
/// keystroke can never be lost, and Ctrl+Z straight after a capital gives the lower-case letter back. md.win's smart typing
/// learnt both halves on real Windows (2026-09-27): an edit made inside <c>TextChanging</c> reached the screen but not
/// <c>TextBox.Text</c>, and its capital is applied the same way as this one.
/// </para>
/// </remarks>
public sealed class CapitalField
{
    private bool keyed;
    private bool pasting;

    /// <summary>The capital the field may still take back with a Backspace — kept from the moment it went in until anything else happens.</summary>
    public CapitalTakeBack? TakeBack { get; private set; }

    /// <summary>Between an IME's <c>TextCompositionStarted</c> and <c>…Ended</c>: nothing is judged.</summary>
    public bool Composing { get; private set; }

    /// <summary>Set by the window around its own edits, whose change events must not be judged or forget the take-back.</summary>
    public bool Applying { get; set; }

    /// <summary>
    /// What a key is, from its virtual-key code and the modifiers held: Windows' own codes (<c>VK_SHIFT</c> 0x10 …), read as
    /// numbers so this can be decided — and tested — without Windows.
    /// </summary>
    public static CapitalKey Classify(int virtualKey, bool control, bool alt) => virtualKey switch
    {
        // Shift, Ctrl, Alt; Caps Lock; the left and right Windows keys; the left and right Shift, Ctrl and Alt.
        0x10 or 0x11 or 0x12 or 0x14 or 0x5B or 0x5C or >= 0xA0 and <= 0xA5 => CapitalKey.Modifier,
        _ when control && !alt => CapitalKey.Chord,
        0x08 when !control && !alt => CapitalKey.Backspace,
        // Tab, Esc; Page Up … Down (the arrows, Home and End among them); Insert, Delete; the menu key; F1 … F24.
        0x09 or 0x1B or >= 0x21 and <= 0x28 or 0x2D or 0x2E or 0x5D or >= 0x70 and <= 0x87 => CapitalKey.Moving,
        _ => CapitalKey.Typing,
    };

    public void CompositionStarted()
    {
        Composing = true;
        keyed = false;
        TakeBack = null;
    }

    public void CompositionEnded() => Composing = false;

    /// <summary>
    /// A key went down (before the box acts on it). Anything but a modifier forgets the take-back — the Backspace that uses it
    /// goes through <see cref="Backspace"/> first — and typing makes the change that follows a candidate.
    /// </summary>
    public void Key(CapitalKey key)
    {
        if (key == CapitalKey.Modifier)
        {
            return;
        }
        TakeBack = null;
        keyed = key is CapitalKey.Typing or CapitalKey.Backspace && !Composing;
    }

    /// <summary>A paste is about to change the text: whatever it brings is not a typed letter.</summary>
    public void Pasting()
    {
        pasting = true;
        TakeBack = null;
    }

    /// <summary>
    /// Backspace, before the box acts on it, with its text and selection as they stand: the edit that puts the lower-case letter
    /// back — replace <c>Length</c> units at <c>At</c> with <c>Replacement</c> and put the caret at <c>Caret</c> — or null, and
    /// the Backspace is an ordinary one. Either way the take-back is used up.
    /// </summary>
    public (int At, int Length, string Replacement, int Caret)? Backspace(string text, int selectionStart, int selectionLength)
    {
        var takeBack = TakeBack;
        TakeBack = null;
        if (Composing || takeBack?.Backspace(text, selectionStart, selectionLength) is not { } restored)
        {
            return null;
        }
        return (takeBack.At, takeBack.Capital.Length, takeBack.Typed, restored.Caret);
    }

    /// <summary>
    /// The text is about to change (<c>BeforeTextChanging</c>: the box still holds the OLD text and selection). The plan to
    /// capitalise what was typed, or null. A change the window is making itself is ignored; any other forgets the take-back and
    /// uses up the key and the paste that came before it.
    /// </summary>
    public CapitalPlan? BeforeChange(string oldText, int selectionStart, int selectionLength, string newText)
    {
        if (Applying)
        {
            return null;
        }
        var judged = keyed && !pasting && !Composing;
        keyed = false;
        pasting = false;
        TakeBack = null;
        return judged && Capitals.AfterTyping(oldText, selectionStart, selectionLength, newText) is { } edit
            ? new CapitalPlan(newText, edit)
            : null;
    }

    /// <summary>
    /// Whether <paramref name="plan"/> still fits the box on the turn it is applied: the box holds exactly the text the typing
    /// made, with the caret right behind the typed letter and nothing selected. Anything else — the change was refused, or more
    /// happened first — and the letter stays as typed.
    /// </summary>
    public bool StillApplies(CapitalPlan plan, string text, int selectionStart, int selectionLength) =>
        !Composing
        && selectionLength == 0
        && selectionStart == plan.At + plan.Typed.Length
        && string.Equals(text, plan.Expected, StringComparison.Ordinal);

    /// <summary>The capital is in: the next Backspace, straight after, may take it back.</summary>
    public void Applied(CapitalPlan plan) => TakeBack = plan.Edit.TakeBack;

    /// <summary>
    /// The selection moved (a click, an arrow, a drag). Unless it is still the bare caret right behind the capital, the take-back
    /// is forgotten.
    /// </summary>
    public void SelectionMoved(int selectionStart, int selectionLength)
    {
        if (Applying || TakeBack is not { } takeBack)
        {
            return;
        }
        if (selectionLength != 0 || selectionStart != takeBack.At + takeBack.Capital.Length)
        {
            TakeBack = null;
        }
    }

    /// <summary>Focus left the field: whatever was typed last is no longer "just" typed.</summary>
    public void FocusLost()
    {
        keyed = false;
        pasting = false;
        TakeBack = null;
    }
}

/// <summary>
/// A letter the rule capitalises, between the change that typed it and the turn that replaces it: the box should then hold
/// <see cref="Expected"/>, and <see cref="Edit"/> is the capitalised text, caret and take-back.
/// </summary>
public sealed record CapitalPlan(string Expected, CapitalEdit Edit)
{
    /// <summary>Where the typed letter starts, in UTF-16 units.</summary>
    public int At => Edit.TakeBack.At;

    /// <summary>The letter as typed: what is replaced.</summary>
    public string Typed => Edit.TakeBack.Typed;

    /// <summary>Its capital: what replaces it.</summary>
    public string Capital => Edit.TakeBack.Capital;
}
