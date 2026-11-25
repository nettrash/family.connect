using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>Device-local picker choices for the active account.</summary>
public sealed class RecentReactions(Database cache, Uri server, Func<long> signedInAccount)
{
    public const int Limit = 24;

    private readonly RecentReactionStore store = new(cache, server);

    public IReadOnlyList<string> Read(long accountId)
    {
        using var held = cache.Hold();
        return IsCurrent(accountId) ? Clean(store.Read(accountId)) : [];
    }

    public void Remember(long accountId, string emoji)
    {
        var picked = ReactionSearch.CanonicalEmoji(emoji);
        if (picked is null)
        {
            return;
        }

        // Keep the session check and update serialized with sign-out's cache wipe.
        using var held = cache.Hold();
        if (IsCurrent(accountId))
        {
            store.Update(accountId, saved => Clean(saved.Prepend(picked)));
        }
    }

    private bool IsCurrent(long accountId) => accountId > 0 && accountId == signedInAccount();

    private static string[] Clean(IEnumerable<string?> choices) => choices.OfType<string>()
        .Select(ReactionSearch.CanonicalEmoji)
        .OfType<string>()
        .Distinct(StringComparer.Ordinal)
        .Take(Limit)
        .ToArray();
}
