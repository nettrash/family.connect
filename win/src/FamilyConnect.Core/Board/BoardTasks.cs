namespace FamilyConnect.Core.Board;

/// <summary>
/// How much of a task list a STICKER draws (docs/protocol.md, "Board").
/// </summary>
/// <remarks>
/// One number for all four clients, like <see cref="BoardWall.Screens"/> and for the same reason:
/// a list that ran to a different point on the phone and in this window would be a different list.
/// The note itself always has them all.
///
/// Web counterpart: <c>fc_text::board::WALL_TASK_LINES</c>. Apple: <c>BoardTasks</c>. Android:
/// <c>NoteTasks</c>.
/// </remarks>
public static class BoardTasks
{
    public const int OnWall = 5;

    /// <summary>
    /// The lines a sticker draws, and how many it had to leave — <c>Left</c> is zero on a list
    /// that fits.
    /// </summary>
    public static (int Shown, int Left) Drawn(int total)
    {
        var shown = Math.Min(Math.Max(total, 0), OnWall);
        return (shown, Math.Max(total, 0) - shown);
    }
}
