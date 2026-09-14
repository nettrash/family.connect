using FamilyConnect.Core.Board;

namespace FamilyConnect.App.Logic;

/// <summary>What letting go of a note meant.</summary>
public enum HandResult
{
    /// <summary>Nothing of this hand's was in progress.</summary>
    None,

    /// <summary>A press that did not travel: open the note (or reveal it).</summary>
    Click,

    /// <summary>A move: <c>Target</c> is the fraction of the wall it was dropped at.</summary>
    Drop,
}

/// <summary>What letting go asks for: the result, where it was dropped, and whether to send that move now.</summary>
public readonly record struct Letting(HandResult Result, (double X, double Y)? Target = null, bool SendNow = false);

/// <summary>
/// One sticker in hand — the web client's <c>Hand</c>. A drag moves the sticker at once and reports the
/// fraction on RELEASE, read back from where the sticker is DRAWN: one intent is one write, not sixty a
/// second.
/// </summary>
/// <remarks>
/// <para>
/// <b>HELD WHERE IT WAS DROPPED UNTIL THE MOVE IS ANSWERED</b>, or it would jump back for the round trip.
/// </para>
/// <para>
/// <b>ONE MOVE OF A NOTE AT A TIME, THE LATEST WAITING.</b> Moves sent side by side commit in whatever order
/// the server locks them, and the note could end at the second of three places on every device.
/// </para>
/// <para>
/// <b>PICKED UP FROM WHERE IT IS DRAWN</b> — picked up again before its last move landed, that is where it
/// was dropped, never an offset the pointer would first have to travel back.
/// </para>
/// </remarks>
public sealed class NoteHand
{
    /// <summary>How far a pointer may wander before a press is a drag rather than a click.</summary>
    public const double ClickSlop = 4.0;

    /// <summary>The arrow keys' "pointer".</summary>
    public const long Keyboard = -1;

    private Drag? drag;
    private (long Drop, (double X, double Y) Target)? held;
    private (long Drop, (double X, double Y) Target)? queued;
    private long drops;
    private long sendingDrop;

    private readonly record struct Drag(long Pointer, (double X, double Y) Down, (double X, double Y) From, (double X, double Y) Delta, bool Moved);

    /// <summary>A move of this note is on its way.</summary>
    public bool Sending { get; private set; }

    /// <summary>In hand and travelling — lifted above everything on the wall.</summary>
    public bool Dragging => drag is { Moved: true };

    /// <summary>Put down, and not yet answered.</summary>
    public bool Held => held is not null;

    /// <summary>
    /// Where the card is drawn right now, in wall pixels: in hand, where the drag has it; dropped, where it
    /// was dropped; otherwise where the server has it.
    /// </summary>
    public (double X, double Y) Corner((double X, double Y) fraction, (double Width, double Height) card, (double Width, double Height) wall) =>
        drag is { Moved: true } moving ? BoardWall.Clamp(moving.From.X + moving.Delta.X, moving.From.Y + moving.Delta.Y, card, wall)
        : held is { } put ? BoardWall.Origin(put.Target, card, wall)
        : BoardWall.Origin(fraction, card, wall);

    public void Down(long pointer, (double X, double Y) at, (double X, double Y) fraction, (double Width, double Height) card, (double Width, double Height) wall) =>
        drag = new Drag(pointer, at, Corner(fraction, card, wall), (0, 0), false);

    /// <summary>The pointer travelled. Answers whether the card is now being dragged, and so has to be drawn again.</summary>
    public bool Move(long pointer, (double X, double Y) at)
    {
        if (drag is not { } current || current.Pointer != pointer)
        {
            return false;
        }
        var delta = (at.X - current.Down.X, at.Y - current.Down.Y);
        var moved = current.Moved || Math.Sqrt((delta.Item1 * delta.Item1) + (delta.Item2 * delta.Item2)) > ClickSlop;
        drag = current with { Moved = moved, Delta = moved ? delta : current.Delta };
        return moved;
    }

    public Letting Up(long pointer, (double X, double Y) fraction, (double Width, double Height) card, (double Width, double Height) wall)
    {
        if (drag is not { } current || current.Pointer != pointer)
        {
            return new Letting(HandResult.None);
        }
        var drawn = Corner(fraction, card, wall);
        drag = null;
        return current.Moved ? Commit(drawn, wall) : new Letting(HandResult.Click);
    }

    /// <summary>A drag the system took away — cancelled, or its capture lost — is put back where it was, not dropped.</summary>
    public void Cancel(long pointer)
    {
        if (drag is { } current && current.Pointer == pointer)
        {
            drag = null;
        }
    }

    /// <summary>An arrow key: a step of the wall from where it is drawn, held to where it CAN be drawn.</summary>
    public void Nudge(double dx, double dy, (double X, double Y) fraction, (double Width, double Height) card, (double Width, double Height) wall)
    {
        var now = Corner(fraction, card, wall);
        drag = new Drag(Keyboard, (0, 0), BoardWall.Clamp(now.X + dx, now.Y + dy, card, wall), (0, 0), true);
    }

    /// <summary>The arrow let go of, or the focus gone mid-move: put down where it is drawn.</summary>
    public Letting PutDown((double X, double Y) fraction, (double Width, double Height) card, (double Width, double Height) wall)
    {
        if (drag is not { Pointer: Keyboard })
        {
            return new Letting(HandResult.None);
        }
        var drawn = Corner(fraction, card, wall);
        drag = null;
        return Commit(drawn, wall);
    }

    /// <summary>
    /// The move on its way was answered. Answers the drop that waited behind it — send that now — or null,
    /// and then lets go of where the note was held, so it is drawn where the server has it.
    /// </summary>
    public (double X, double Y)? Answered()
    {
        Sending = false;
        if (queued is { } next)
        {
            queued = null;
            Sending = true;
            sendingDrop = next.Drop;
            return next.Target;
        }
        if (held is { } put && put.Drop == sendingDrop)
        {
            held = null;
        }
        return null;
    }

    private Letting Commit((double X, double Y) drawn, (double Width, double Height) wall)
    {
        var target = BoardWall.FractionOf(drawn, wall);
        drops++;
        held = (drops, target);
        if (Sending)
        {
            queued = (drops, target);
            return new Letting(HandResult.Drop, target, SendNow: false);
        }
        Sending = true;
        sendingDrop = drops;
        return new Letting(HandResult.Drop, target, SendNow: true);
    }
}
