using FamilyConnect.App.Logic;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;

namespace FamilyConnect.App.Views;

/// <summary>
/// The typing aids of a prose field: capital letters at the start of a sentence in a MULTI-LINE one (issue #97;
/// docs/capitals-and-rich-markdown-2026-10-17.md, §1) — the composer, the thread reply, a note, a report's note — and emoji as you
/// type in EVERY one, single-line ones included (issue #101; docs/emoji-as-you-type-2026-10-18.md, §3) — a list's line, a poll's
/// question and options too. Always on, as no Windows keyboard does either. One call per field, <see cref="Attach"/> or
/// <see cref="AttachEmoji"/>; every decision and every piece of state is <see cref="CapitalField"/>'s, tested off Windows, and
/// this only reads the box at each event and applies the answer.
/// </summary>
/// <remarks>
/// <para>
/// <b>The typed character</b> is judged in <c>BeforeTextChanging</c>, where the box still holds the old text and selection
/// (<see cref="Capitals.TypedCharacter"/> needs both), goes into the box as typed, and is then replaced — by its capital, or with
/// what was typed before it by an emoji — through <c>Select</c> + <c>SelectedText</c> on a HIGH dispatcher turn right after,
/// ahead of the next input. Never cancelled and re-applied inside the event: whatever a real control does with an edit made
/// during its own change notification, the keystroke itself cannot be lost, and the worst surprise is a missing capital or
/// emoji. The selection path, never <c>Text =</c>, keeps the box's undo: the typed character is one step and the replacement a
/// second, so Ctrl+Z straight after gives the typed text back (md.win's smart typing, verified on real Windows, works the same
/// way — and found that an edit made inside <c>TextChanging</c> reaches the screen but not <c>TextBox.Text</c>).
/// </para>
/// <para>
/// <b>Taking it back</b> is Backspace in <c>PreviewKeyDown</c>, before the box deletes anything: right behind a capital or an
/// emoji the rule just made, nothing selected, it puts back what was typed instead. Registered with <c>handledEventsToo</c>, so
/// the field's own key handler — the composer's Enter (#95) and mention list — is untouched and every key is still seen.
/// </para>
/// <para>
/// <b>Never judged:</b> an IME composition (<c>TextCompositionStarted</c> … <c>Ended</c>), a paste (the box's <c>Paste</c>
/// event), a Ctrl chord, and any change no key went down for — every <c>box.Text = …</c> the window makes itself. Nothing here
/// is logged: what a field holds is a message.
/// </para>
/// </remarks>
internal sealed class CapitalLetters
{
    private readonly TextBox box;
    private readonly CapitalField field;

    private CapitalLetters(TextBox box, bool capitals)
    {
        this.box = box;
        field = new CapitalField(capitals);
        box.AddHandler(UIElement.PreviewKeyDownEvent, new KeyEventHandler(OnKey), handledEventsToo: true);
        box.BeforeTextChanging += OnBeforeTextChanging;
        box.SelectionChanged += (_, _) => field.SelectionMoved(box.SelectionStart, box.SelectionLength);
        box.Paste += (_, _) => field.Pasting();
        box.TextCompositionStarted += (_, _) => field.CompositionStarted();
        box.TextCompositionEnded += (_, _) => field.CompositionEnded();
        box.LostFocus += (_, _) => field.FocusLost();
    }

    /// <summary>
    /// A MULTI-LINE prose field: capital letters at a sentence's start, and emoji as you type. The box keeps what it needs alive
    /// through its own event handlers.
    /// </summary>
    public static void Attach(TextBox box) => _ = new CapitalLetters(box, capitals: true);

    /// <summary>A SINGLE-LINE prose field: emoji as you type, and no capitals (#97 gives those to the multi-line fields only).</summary>
    public static void AttachEmoji(TextBox box) => _ = new CapitalLetters(box, capitals: false);

    private void OnKey(object sender, KeyRoutedEventArgs e)
    {
        var control = IsHeld(Windows.System.VirtualKey.Control);
        var alt = e.KeyStatus.IsMenuKeyDown || IsHeld(Windows.System.VirtualKey.Menu);
        var key = CapitalField.Classify((int)e.Key, control, alt);
        // A key some other handler already acted on (the composer's Enter, a name accepted from the list) is still a key, and
        // still forgets the take-back — but only an unhandled Backspace may use it.
        if (key == CapitalKey.Backspace && !e.Handled && field.Backspace(box.Text, box.SelectionStart, box.SelectionLength) is { } restore)
        {
            e.Handled = true;
            Apply(restore.At, restore.Length, restore.Replacement, restore.Caret);
            return;
        }
        field.Key(key);
    }

    private void OnBeforeTextChanging(TextBox sender, TextBoxBeforeTextChangingEventArgs args)
    {
        if (field.BeforeChange(box.Text, box.SelectionStart, box.SelectionLength, args.NewText) is not { } plan)
        {
            return;
        }
        // Applied once the change is in, and ahead of the next keystroke.
        box.DispatcherQueue.TryEnqueue(DispatcherQueuePriority.High, () =>
        {
            if (!field.StillApplies(plan, box.Text, box.SelectionStart, box.SelectionLength, box.MaxLength))
            {
                return;
            }
            Apply(plan.At, plan.Length, plan.Replacement, plan.Caret);
            field.Applied(plan);
        });
    }

    /// <summary>One replacement through the selection — an undo step of its own, never <c>Text =</c> — whose events are not judged.</summary>
    private void Apply(int at, int length, string replacement, int caret)
    {
        field.Applying = true;
        try
        {
            box.Select(at, length);
            box.SelectedText = replacement;
            box.Select(caret, 0);
        }
        finally
        {
            field.Applying = false;
        }
    }

    private static bool IsHeld(Windows.System.VirtualKey key) =>
        InputKeyboardSource.GetKeyStateForCurrentThread(key).HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down);
}
