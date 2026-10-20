using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>One row of a drawn conversation, as the redraw compares it: what it is, and everything its drawing was made from.</summary>
/// <param name="Key">Which row: a message's, a pending send's, the empty chat's line. Unique within one drawing.</param>
/// <param name="Signature">Everything the row's elements were built from. Two equal signatures draw the same elements.</param>
public readonly record struct RowMark(string Key, string Signature);

/// <summary>
/// Which of a conversation's drawn rows a redraw may KEEP, and how to turn the elements on screen into the ones wanted with
/// the fewest insertions and removals (issue: the conversation was rebuilt whole on every change).
/// </summary>
/// <remarks>
/// <para>
/// <b>WHY.</b> The conversation used to clear its stack and build every bubble again on any change it noticed — a peer's
/// read marker, a link's card landing, every chunk of an assistant answer being streamed, each page of a catch-up — and then
/// force a layout pass. A chat paged back a few hundred messages was hundreds of bubbles, their stickers, animation players
/// and pictures, made again for one tick mark. Now a row whose signature has not changed keeps the elements it has, and only
/// the rows that changed are built.
/// </para>
/// <para>
/// <b>A ROW IS KEPT ONLY WHEN NOTHING IT WAS DRAWN FROM HAS CHANGED</b> — the signature is the whole of it, and the view
/// decides what goes in (the message itself, its place in the timeline, the tick, the card, the streamed words). What is not
/// per row — the theme, the language, the block list, the roster — is not in any row's signature: a change to it rebuilds
/// every row, which is what happened before for every change.
/// </para>
/// <para>
/// <b>KEPT ROWS NEVER MOVE.</b> An element moved from one place in a panel to another is unloaded and loaded again, which
/// for a sticker, a circle or an animation is a teardown and a start — the very cost this is here to save. So the plan keeps
/// only rows that stay in the order they were in, and builds again any that would have to jump over another (which, for a
/// conversation ordered by message id, a redraw never asks for). New rows are inserted where they belong: below for what
/// arrived, above for a page of history, in place for a row that changed.
/// </para>
/// <para>
/// No window in here: the elements are a type parameter and the panel is two callbacks, so the rule is tested where no
/// window exists.
/// </para>
/// </remarks>
public static class ConversationRows
{
    /// <summary>
    /// For each row wanted, the index of the drawn row it may keep, or -1 where it must be built.
    /// </summary>
    /// <param name="drawn">The rows on screen, in order.</param>
    /// <param name="wanted">The rows the redraw wants, in order.</param>
    /// <param name="keepable">
    /// Whether a drawn row may be kept at all, by its index: false for one whose drawing never finished — a picture whose
    /// bytes did not come, a sticker that did not decode yet — which, built again, would try again, as every redraw used to.
    /// </param>
    /// <remarks>
    /// A key that appears twice on either side is never kept: which of the two a kept element belonged to could not be told.
    /// </remarks>
    public static int[] Plan(IReadOnlyList<RowMark> drawn, IReadOnlyList<RowMark> wanted, Func<int, bool>? keepable = null)
    {
        var plan = new int[wanted.Count];
        Array.Fill(plan, -1);
        var at = new Dictionary<string, int>(drawn.Count, StringComparer.Ordinal);
        var twice = new HashSet<string>(StringComparer.Ordinal);
        for (var index = 0; index < drawn.Count; index++)
        {
            if (!at.TryAdd(drawn[index].Key, index))
            {
                twice.Add(drawn[index].Key);
            }
        }
        var wantedKeys = new HashSet<string>(StringComparer.Ordinal);
        foreach (var row in wanted)
        {
            if (!wantedKeys.Add(row.Key))
            {
                twice.Add(row.Key);
            }
        }
        var last = -1;
        for (var index = 0; index < wanted.Count; index++)
        {
            var row = wanted[index];
            if (twice.Contains(row.Key)
                || !at.TryGetValue(row.Key, out var old)
                || !string.Equals(drawn[old].Signature, row.Signature, StringComparison.Ordinal)
                // Kept rows keep their order: one that would jump back over a row already kept is built again instead.
                || old <= last
                || (keepable is not null && !keepable(old)))
            {
                continue;
            }
            plan[index] = old;
            last = old;
        }
        return plan;
    }

    /// <summary>
    /// Turn what a panel holds into <paramref name="wanted"/>: every element not wanted removed, every wanted one not there
    /// inserted where it belongs, and every element kept left where it is — never removed and put back. Answers how many
    /// insertions and removals that took, or -1, having changed nothing, when the elements kept are not in the order wanted
    /// (the caller then clears the panel and adds them all, as it used to).
    /// </summary>
    /// <param name="shown">What the panel holds, in order — the caller's own copy, kept in step here.</param>
    /// <param name="wanted">What it should hold, in order; no element twice.</param>
    /// <param name="removeAt">Remove the panel's child at this index.</param>
    /// <param name="insert">Insert this element as the panel's child at this index.</param>
    public static int Merge<T>(List<T> shown, IReadOnlyList<T> wanted, Action<int> removeAt, Action<int, T> insert)
        where T : class
    {
        var wantedSet = new HashSet<T>(ReferenceEqualityComparer.Instance);
        foreach (var element in wanted)
        {
            if (!wantedSet.Add(element))
            {
                // The same element twice cannot be in a panel twice.
                return -1;
            }
        }
        // Asked BEFORE anything is touched: what stays must already be in the wanted order, or keeping it would mean moving
        // it — and inserting an element that is still in the panel elsewhere throws.
        var next = 0;
        foreach (var element in shown)
        {
            if (!wantedSet.Contains(element))
            {
                continue;
            }
            while (next < wanted.Count && !ReferenceEquals(wanted[next], element))
            {
                next++;
            }
            if (next == wanted.Count)
            {
                return -1;
            }
            next++;
        }
        var changes = 0;
        // From the end, so the indices still to visit do not shift.
        for (var index = shown.Count - 1; index >= 0; index--)
        {
            if (!wantedSet.Contains(shown[index]))
            {
                removeAt(index);
                shown.RemoveAt(index);
                changes++;
            }
        }
        for (var index = 0; index < wanted.Count; index++)
        {
            if (index < shown.Count && ReferenceEquals(shown[index], wanted[index]))
            {
                continue;
            }
            insert(index, wanted[index]);
            shown.Insert(index, wanted[index]);
            changes++;
        }
        return changes;
    }

    /// <summary>
    /// The part of a message row's signature that is the row's own: the message as the store holds it — every field, so a
    /// trimmed list preview overwritten by the full message, an edit, a reaction, a vote, a new reply count all change it —
    /// whether it is hidden, revealed or the reader's own, its place in the timeline, and what the view adds
    /// (<paramref name="drawnWith"/>: the tick, the card, the streamed words — what the view reads to draw it).
    /// </summary>
    /// <remarks>
    /// The message is compared as the WIRE writes it, because a record with arrays in it is not equal to its own copy, and
    /// a hand-picked list of fields is one field short the day the protocol grows one.
    /// </remarks>
    public static string Signature(TimelineRow row, string drawnWith)
    {
        var bubble = row.Bubble;
        return string.Join(
            Separator,
            Wire.Encode(bubble.Message),
            bubble.Hidden ? 1 : 0,
            bubble.Revealed ? 1 : 0,
            bubble.Mine ? 1 : 0,
            row.DayAbove?.DayNumber.ToString(System.Globalization.CultureInfo.InvariantCulture) ?? string.Empty,
            row.UnreadDividerAbove ? 1 : 0,
            row.ShowsSender ? 1 : 0,
            row.RunStart ? 1 : 0,
            row.RunEnd ? 1 : 0,
            drawnWith);
    }

    /// <summary>The key of a message's row.</summary>
    public static string MessageKey(long messageId) =>
        "m" + messageId.ToString(System.Globalization.CultureInfo.InvariantCulture);

    /// <summary>The key of a send still on its way.</summary>
    public static string PendingKey(string clientMsgId) => "p" + clientMsgId;

    /// <summary>The key of the line an empty chat draws.</summary>
    public const string EmptyKey = "empty";

    // A character no message's JSON contains unescaped: a control character is escaped by the writer.
    private const char Separator = (char)31;
}
