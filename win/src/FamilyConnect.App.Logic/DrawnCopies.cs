namespace FamilyConnect.App.Logic;

/// <summary>
/// Every copy DRAWN of one thing, by its key — a voice message the conversation and the thread panel both show is two
/// bubbles, and playing it, its dot going and its "Played" must reach both, not whichever was drawn last. A copy is added as
/// it is drawn (and again whenever it comes back on screen) and removed as it leaves, by reference, so a redraw's new copy
/// and the old one it replaces never stand in for each other.
/// </summary>
/// <typeparam name="TKey">What the copies are of: an attachment's id.</typeparam>
/// <typeparam name="TRow">One copy as drawn.</typeparam>
public sealed class DrawnCopies<TKey, TRow>
    where TKey : notnull
    where TRow : class
{
    private readonly Dictionary<TKey, List<TRow>> rows = new();

    /// <summary>A copy drawn; the same copy twice is still one.</summary>
    public void Add(TKey key, TRow row)
    {
        ArgumentNullException.ThrowIfNull(row);
        if (!rows.TryGetValue(key, out var copies))
        {
            rows[key] = copies = [];
        }
        if (!copies.Any(copy => ReferenceEquals(copy, row)))
        {
            copies.Add(row);
        }
    }

    /// <summary>A copy gone from the screen; the others of the same key stay. Whether it was there.</summary>
    public bool Remove(TKey key, TRow row)
    {
        if (!rows.TryGetValue(key, out var copies))
        {
            return false;
        }
        var removed = copies.RemoveAll(copy => ReferenceEquals(copy, row)) > 0;
        if (copies.Count == 0)
        {
            rows.Remove(key);
        }
        return removed;
    }

    /// <summary>Every copy of one key, in the order drawn; none when it is not on the screen.</summary>
    public IReadOnlyList<TRow> Of(TKey key) => rows.TryGetValue(key, out var copies) ? copies.ToArray() : [];

    /// <summary>Every copy of everything.</summary>
    public IReadOnlyList<TRow> All => rows.Values.SelectMany(copies => copies).ToArray();

    /// <summary>Nothing drawn any more: the chat was left.</summary>
    public void Clear() => rows.Clear();
}
