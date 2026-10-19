namespace FamilyConnect.App.Logic;

/// <summary>What a key press is, to <see cref="CapitalField"/>.</summary>
public enum CapitalKey
{
    /// <summary>Shift, Ctrl, Alt, the Windows key, Caps Lock: on their own they change nothing, and forget nothing.</summary>
    Modifier,

    /// <summary>Backspace with nothing but Shift held: the one key that may take a capital or an emoji back.</summary>
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
/// One prose field's typing aids — the state a <c>TextBox</c> keeps between its events, and every decision about them, so the
/// window's part is to read the box, ask, and apply. Two rules run here: capital letters at a sentence's start (issue #97;
/// <see cref="Capitals"/>), in the MULTI-LINE prose fields only, and emoji as you type (issue #101; <see cref="EmojiTyping"/>), in
/// EVERY prose field. On a field that has both, capitals decide the typed character first and the emoji rule sees what they
/// decided — so <c>xD</c> + space at a sentence's start is <c>XD </c>, and 😆. The order the events come in is a hardware
/// keyboard's — the key, then the text change it makes — and what this class assumes is written next to each.
/// </summary>
/// <remarks>
/// <para>
/// <b>Only typing is judged.</b> A change is a candidate only when a key went down just before it (<see cref="Key"/>), the field
/// is not composing (an IME's candidate text is not typing), and it is not a paste (<see cref="Pasting"/>). Every
/// <c>box.Text = …</c> the window does — a draft put back, a message opened for editing, a name accepted from the mention list —
/// is therefore left exactly as it is, even when it happens to be one lower-case letter or the colon that closes a name. The
/// touch keyboard, which raises no key events, is left alone too, and it capitalises sentences itself.
/// </para>
/// <para>
/// <b>Applied after, never instead.</b> The character the keyboard typed goes into the box as typed — one undo unit — and is
/// then replaced through the selection — a second — on the next high-priority turn, ahead of the next input: by its capital, or
/// (with what was typed before it) by an emoji. So the keystroke can never be lost, and Ctrl+Z straight after gives the typed
/// text back. md.win's smart typing learnt both halves on real Windows (2026-09-27): an edit made inside <c>TextChanging</c>
/// reached the screen but not <c>TextBox.Text</c>, and its replacements are applied the same way as these.
/// </para>
/// <para>
/// <b>One live replacement</b> at a time: the capital (<see cref="TakeBack"/>) or the emoji (<see cref="Swap"/>) the rule just
/// made, which a Backspace straight after takes back. The capital is forgotten by the next key; the emoji survives the next
/// TYPING key until its change is judged, because that character may grow it (<c>:(</c> then <c>(</c> is <c>:((</c>), and is
/// forgotten by anything else — another key, a click, a selection, a paste, the focus leaving.
/// </para>
/// </remarks>
public sealed class CapitalField
{
    private bool keyed;
    private bool pasting;

    /// <summary>A field with emoji as you type, and — when <paramref name="capitals"/> — capital letters at a sentence's start.</summary>
    /// <param name="capitals">True for a multi-line prose field (#97); a single-line one gets emoji only (#101).</param>
    public CapitalField(bool capitals = true) => HasCapitals = capitals;

    /// <summary>Whether this field capitalises a sentence's first letter (#97: multi-line prose fields only).</summary>
    public bool HasCapitals { get; }

    /// <summary>The capital the field may still take back with a Backspace — kept from the moment it went in until anything else happens.</summary>
    public CapitalTakeBack? TakeBack { get; private set; }

    /// <summary>The emoji the field may still take back with a Backspace, or grow with the next typed character.</summary>
    public EmojiSwap? Swap { get; private set; }

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
        Forget();
    }

    public void CompositionEnded() => Composing = false;

    /// <summary>
    /// A key went down (before the box acts on it). Anything but a modifier forgets the capital — the Backspace that uses it goes
    /// through <see cref="Backspace"/> first — and anything but typing forgets the emoji. Typing makes the change that follows a
    /// candidate; and a typing key that comes while the one before it is still waiting for its change (it typed nothing: Enter
    /// in a single-line box, a dead key, a character the box refused) is "something else happened", so the emoji is forgotten.
    /// </summary>
    public void Key(CapitalKey key)
    {
        if (key == CapitalKey.Modifier)
        {
            return;
        }
        TakeBack = null;
        if (key != CapitalKey.Typing || keyed)
        {
            Swap = null;
        }
        keyed = key is CapitalKey.Typing or CapitalKey.Backspace && !Composing;
    }

    /// <summary>A paste is about to change the text: whatever it brings is not typed, and nothing is taken back after it.</summary>
    public void Pasting()
    {
        pasting = true;
        Forget();
    }

    /// <summary>
    /// Backspace, before the box acts on it, with its text and selection as they stand: the edit that puts back what was typed —
    /// the lower-case letter, or every character the emoji replaced — as "replace <c>Length</c> units at <c>At</c> with
    /// <c>Replacement</c> and put the caret at <c>Caret</c>"; or null, and the Backspace is an ordinary one. Either way the
    /// take-back is used up.
    /// </summary>
    public (int At, int Length, string Replacement, int Caret)? Backspace(string text, int selectionStart, int selectionLength)
    {
        var takeBack = TakeBack;
        var swap = Swap;
        // A typing key since that changed nothing: the replacement is no longer the last thing that happened.
        var stale = keyed;
        Forget();
        if (Composing || stale)
        {
            return null;
        }
        if (swap is not null)
        {
            return selectionLength == 0 && swap.Backspace(text, selectionStart) is { } undone
                ? (swap.At, swap.Put.Length, swap.Typed, undone.Caret)
                : null;
        }
        if (takeBack?.Backspace(text, selectionStart, selectionLength) is not { } restored)
        {
            return null;
        }
        return (takeBack.At, takeBack.Capital.Length, takeBack.Typed, restored.Caret);
    }

    /// <summary>
    /// The text is about to change (<c>BeforeTextChanging</c>: the box still holds the OLD text and selection). The plan for
    /// what was typed — its capital, or an emoji — or null. A change the window is making itself is ignored; any other forgets
    /// the take-back and uses up the key and the paste that came before it.
    /// </summary>
    public CapitalPlan? BeforeChange(string oldText, int selectionStart, int selectionLength, string newText)
    {
        if (Applying)
        {
            return null;
        }
        var judged = keyed && !pasting && !Composing;
        // Only a bare caret can be right behind the emoji; typing over a selection grows nothing.
        var live = selectionLength == 0 ? Swap : null;
        keyed = false;
        pasting = false;
        Forget();
        if (!judged || Capitals.TypedCharacter(oldText, selectionStart, selectionLength, newText) is not { } typed)
        {
            return null;
        }
        var before = newText[..typed.Start];
        var typedEnd = typed.Start + typed.Typed.Length;
        // 1. Capitals decide the typed character (#97) …
        var capital = HasCapitals && Capitals.Capitalizes(before, typed.Typed) ? CapitalTakeBack.After(typed.Start, typed.Typed) : null;
        // 2. … and the emoji rule sees what they decided (#101). An emoji replaces everything from where it starts to the end
        // of the character as the box holds it.
        if (EmojiTyping.Replace(before, capital?.Capital ?? typed.Typed, live) is { } swap)
        {
            return new CapitalPlan(newText, typedEnd, swap.At, typedEnd - swap.At, swap.Put, swap.At + swap.Put.Length, null, swap);
        }
        return capital is null
            ? null
            : new CapitalPlan(newText, typedEnd, typed.Start, typed.Typed.Length, capital.Capital, typed.Start + capital.Capital.Length, capital, null);
    }

    /// <summary>
    /// Whether <paramref name="plan"/> still fits the box on the turn it is applied: the box holds exactly the text the typing
    /// made, with the caret right behind the typed character and nothing selected, and — when the box has a
    /// <paramref name="maxLength"/> (0 is none) — the replacement fits under it (a few shortcodes are SHORTER than their emoji:
    /// <c>:wales:</c> is 7 units, its flag 14). Anything else — the change was refused, or more happened first — and the
    /// character stays as typed.
    /// </summary>
    public bool StillApplies(CapitalPlan plan, string text, int selectionStart, int selectionLength, int maxLength = 0) =>
        !Composing
        && selectionLength == 0
        && selectionStart == plan.TypedEnd
        && string.Equals(text, plan.Expected, StringComparison.Ordinal)
        && (maxLength <= 0 || text.Length - plan.Length + plan.Replacement.Length <= maxLength);

    /// <summary>The replacement is in: the next Backspace, straight after, may take it back — and the next typed character may grow an emoji.</summary>
    public void Applied(CapitalPlan plan)
    {
        TakeBack = plan.TakeBack;
        Swap = plan.Swap;
    }

    /// <summary>
    /// The selection moved (a click, an arrow, a drag). Unless it is still the bare caret right behind the replacement, the
    /// take-back is forgotten.
    /// </summary>
    public void SelectionMoved(int selectionStart, int selectionLength)
    {
        if (Applying)
        {
            return;
        }
        if (TakeBack is { } takeBack && (selectionLength != 0 || selectionStart != takeBack.At + takeBack.Capital.Length))
        {
            TakeBack = null;
        }
        if (Swap is { } swap && (selectionLength != 0 || selectionStart != swap.At + swap.Put.Length))
        {
            Swap = null;
        }
    }

    /// <summary>Focus left the field: whatever was typed last is no longer "just" typed.</summary>
    public void FocusLost()
    {
        keyed = false;
        pasting = false;
        Forget();
    }

    private void Forget()
    {
        TakeBack = null;
        Swap = null;
    }
}

/// <summary>
/// What the field does to a typed character, between the change that typed it and the turn that rewrites it: the box should
/// then hold <see cref="Expected"/> with the caret at <see cref="TypedEnd"/>, and the plan replaces <see cref="Length"/> units at
/// <see cref="At"/> with <see cref="Replacement"/> and puts the caret at <see cref="Caret"/>. Exactly one of
/// <see cref="TakeBack"/> (a capital, #97) and <see cref="Swap"/> (an emoji, #101) is set: what the field keeps after.
/// </summary>
/// <param name="Expected">The box's text once the typed character is in.</param>
/// <param name="TypedEnd">Where the caret is then: right behind the typed character.</param>
/// <param name="At">Where the replacement starts, in UTF-16 units.</param>
/// <param name="Length">How many units it replaces: the typed character, and for an emoji what was typed before it.</param>
/// <param name="Replacement">The capital, or the emoji (and its space).</param>
/// <param name="Caret">Where the caret goes after: right behind the replacement.</param>
/// <param name="TakeBack">The capital's take-back, for a capital.</param>
/// <param name="Swap">The emoji's swap, for an emoji.</param>
public sealed record CapitalPlan(
    string Expected,
    int TypedEnd,
    int At,
    int Length,
    string Replacement,
    int Caret,
    CapitalTakeBack? TakeBack,
    EmojiSwap? Swap);
