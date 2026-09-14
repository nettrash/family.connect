namespace FamilyConnect.App.Logic;

/// <summary>
/// What each chat's composer holds while the reader is somewhere else — the web client's <c>drafts</c>: half a thought
/// kept for the chat it was written in, and given back when the reader returns.
/// </summary>
/// <remarks>
/// <para>
/// <b>WORDS ONLY, AND ONLY WORDS WORTH KEEPING.</b> A draft that is nothing but white space is no draft, and saving one
/// forgets the last. Sending takes the draft with it.
/// </para>
/// <para>
/// <b>A CHAT THAT IS GONE TAKES ITS DRAFT WITH IT</b> — a direct chat deleted, a family left — so a later chat that
/// reused nothing still opens empty, and the store does not grow with chats nobody can open.
/// </para>
/// <para>
/// In memory, as the web client keeps them: a draft is this sitting's, not a document.
/// </para>
/// </remarks>
public sealed class ComposerDrafts
{
    private readonly Dictionary<long, string> drafts = [];

    /// <summary>Keep what a chat's composer holds as the reader leaves it.</summary>
    public void Save(long chatId, string text)
    {
        if (string.IsNullOrWhiteSpace(text))
        {
            drafts.Remove(chatId);
        }
        else
        {
            drafts[chatId] = text;
        }
    }

    /// <summary>What to put back in a chat's composer as the reader returns: its draft, or nothing.</summary>
    public string Of(long chatId) => drafts.GetValueOrDefault(chatId, string.Empty);

    /// <summary>A message was sent from this chat: what was being written went with it.</summary>
    public void Sent(long chatId) => drafts.Remove(chatId);

    /// <summary>Keep only the drafts of chats that still exist.</summary>
    public void Retain(IEnumerable<long> chatIds)
    {
        var living = chatIds.ToHashSet();
        foreach (var gone in drafts.Keys.Where(chatId => !living.Contains(chatId)).ToList())
        {
            drafts.Remove(gone);
        }
    }

    public int Count => drafts.Count;
}
