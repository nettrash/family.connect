using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>A sticker in hand — the web client's drag and keyboard tests, ported.</summary>
public sealed class NoteHandTests
{
    private static readonly (double Width, double Height) Wall = (1000, 1600);
    private static readonly (double Width, double Height) Card = (150, 110);

    private static bool Near(double a, double b) => Math.Abs(a - b) < 1e-9;

    [Fact]
    public void AFewPixelsAreAClickNotADrag()
    {
        var hand = new NoteHand();
        hand.Down(1, (100, 100), (0.1, 0.1), Card, Wall);
        Assert.False(hand.Move(1, (102, 101)));
        Assert.False(hand.Dragging);
        Assert.Equal(new Letting(HandResult.Click), hand.Up(1, (0.1, 0.1), Card, Wall));
        Assert.False(hand.Sending);
    }

    /// <summary>A drag follows the pointer, is one write on release, and is held where it was dropped until answered.</summary>
    [Fact]
    public void ADragReportsWhereItWasDroppedAndIsHeldThereUntilAnswered()
    {
        var hand = new NoteHand();
        var fraction = (0.1, 0.1);
        hand.Down(1, (100, 100), fraction, Card, Wall);
        Assert.True(hand.Move(1, (160, 130)));
        Assert.True(hand.Move(1, (190, 150)));
        Assert.True(hand.Dragging);
        Assert.Equal((190.0, 210.0), hand.Corner(fraction, Card, Wall));

        var letting = hand.Up(1, fraction, Card, Wall);
        Assert.Equal(HandResult.Drop, letting.Result);
        Assert.True(letting.SendNow);
        Assert.True(Near(0.19, letting.Target!.Value.X) && Near(210.0 / 1600, letting.Target.Value.Y));
        Assert.True(hand.Held);
        Assert.Equal((190.0, 210.0), hand.Corner(fraction, Card, Wall));

        // A move that came to nothing lets go, and the note is where the server has it.
        Assert.Null(hand.Answered());
        Assert.False(hand.Held);
        Assert.Equal((100.0, 160.0), hand.Corner(fraction, Card, Wall));
    }

    /// <summary>One move at a time: a second drop while the first is out waits, and only the LATEST waiting is sent.</summary>
    [Fact]
    public void MovesOfOneNoteGoOneAtATimeAndTheLatestWins()
    {
        var hand = new NoteHand();
        var fraction = (0.1, 0.1);
        hand.Nudge(0.01 * Wall.Width, 0, fraction, Card, Wall);
        var first = hand.PutDown(fraction, Card, Wall);
        Assert.True(first.SendNow);
        Assert.True(Near(0.11, first.Target!.Value.X));

        hand.Nudge(0.01 * Wall.Width, 0, fraction, Card, Wall);
        var second = hand.PutDown(fraction, Card, Wall);
        Assert.False(second.SendNow, "waits behind the first");
        hand.Nudge(0, 0.01 * Wall.Height, fraction, Card, Wall);
        Assert.False(hand.PutDown(fraction, Card, Wall).SendNow);
        // Drawn where it was LAST put.
        Assert.True(Near(0.12 * Wall.Width, hand.Corner(fraction, Card, Wall).X));

        var next = hand.Answered();
        Assert.True(Near(0.12, next!.Value.X) && Near(0.11, next.Value.Y));
        Assert.True(hand.Sending);
        Assert.Null(hand.Answered());
        Assert.False(hand.Held);
    }

    /// <summary>An arrow moves a hundredth of the wall a press, nothing is written while it is down, and letting go puts it down.</summary>
    [Fact]
    public void TheArrowKeysMoveANoteAndLettingGoPutsItDown()
    {
        var hand = new NoteHand();
        var fraction = (0.2, 0.3);
        var square = (1000.0, 1000.0);
        for (var press = 0; press < 3; press++)
        {
            hand.Nudge(0.01 * square.Item1, 0, fraction, Card, square);
        }
        hand.Nudge(0, -0.01 * square.Item2, fraction, Card, square);
        Assert.True(hand.Dragging);
        Assert.False(hand.Sending, "nothing written while the key is down");
        var put = hand.PutDown(fraction, Card, square);
        Assert.True(Near(0.23, put.Target!.Value.X) && Near(0.29, put.Target.Value.Y));
        // A pointer release has nothing to put down after the keyboard has.
        Assert.Equal(new Letting(HandResult.None), hand.PutDown(fraction, Card, square));
    }

    [Fact]
    public void ADragIsHeldInsideTheWallAndAPressPastTheEdgeIsNotOwedBack()
    {
        var hand = new NoteHand();
        var fraction = (0.0, 0.0);
        hand.Down(1, (10, 10), fraction, Card, Wall);
        hand.Move(1, (-500, -500));
        Assert.Equal((0.0, 0.0), hand.Corner(fraction, Card, Wall));
        hand.Cancel(1);

        hand.Nudge(-100, 0, fraction, Card, Wall);
        hand.Nudge(20, 0, fraction, Card, Wall);
        Assert.Equal((20.0, 0.0), hand.Corner(fraction, Card, Wall));
    }

    /// <summary>A cancelled drag goes back where it was; another pointer is somebody else's.</summary>
    [Fact]
    public void ACancelledDragIsPutBackAndAnotherPointerIsIgnored()
    {
        var hand = new NoteHand();
        var fraction = (0.1, 0.1);
        hand.Down(1, (100, 100), fraction, Card, Wall);
        Assert.False(hand.Move(2, (300, 300)));
        hand.Move(1, (180, 100));
        hand.Cancel(2);
        Assert.True(hand.Dragging);
        hand.Cancel(1);
        Assert.False(hand.Dragging);
        Assert.Equal((100.0, 160.0), hand.Corner(fraction, Card, Wall));
        Assert.Equal(new Letting(HandResult.None), hand.Up(1, fraction, Card, Wall));
    }

    /// <summary>Picked up again before its move landed, a note travels from where it was DROPPED.</summary>
    [Fact]
    public void APickUpBeforeTheAnswerStartsFromWhereItWasDropped()
    {
        var hand = new NoteHand();
        var fraction = (0.1, 0.1);
        hand.Down(1, (100, 100), fraction, Card, Wall);
        hand.Move(1, (200, 100));
        hand.Up(1, fraction, Card, Wall);
        hand.Down(1, (0, 0), fraction, Card, Wall);
        hand.Move(1, (50, 0));
        Assert.Equal((250.0, 160.0), hand.Corner(fraction, Card, Wall));
    }

    /// <summary>
    /// The focus going while a POINTER holds the note is not the keyboard's put-down: the pointer still has it,
    /// and its release decides where it lands.
    /// </summary>
    [Fact]
    public void LosingTheFocusDoesNotDropANoteThePointerStillHolds()
    {
        var hand = new NoteHand();
        var fraction = (0.1, 0.1);
        hand.Down(1, (100, 100), fraction, Card, Wall);
        hand.Move(1, (300, 200));

        Assert.Equal(new Letting(HandResult.None), hand.PutDown(fraction, Card, Wall));
        Assert.True(hand.Dragging);
        Assert.False(hand.Sending);
        Assert.Equal(HandResult.Drop, hand.Up(1, fraction, Card, Wall).Result);
    }
}
