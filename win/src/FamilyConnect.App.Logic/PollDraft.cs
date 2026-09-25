using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

/// <summary>
/// The form behind "Poll": a question and between two and ten options. It opens with two empty rows,
/// because a poll with one option is not a poll and an empty list looks broken.
/// </summary>
/// <remarks>
/// Create stays off until the draft is a poll the server would take — the apps' way, which says nothing
/// it would have to translate — and <see cref="Checked"/> answers the CLEANED question and options, so
/// what was checked is exactly what leaves.
/// </remarks>
public sealed class PollDraft
{
    private readonly List<string> options = [string.Empty, string.Empty];

    public string Question { get; set; } = string.Empty;

    public IReadOnlyList<string> Options => options;

    public bool MayAdd => options.Count < Polls.MaxOptions;

    public bool MayRemove => options.Count > Polls.MinOptions;

    public void Set(int index, string text)
    {
        if (index >= 0 && index < options.Count)
        {
            options[index] = text;
        }
    }

    public bool Add()
    {
        if (!MayAdd)
        {
            return false;
        }
        options.Add(string.Empty);
        return true;
    }

    public bool Remove(int index)
    {
        if (!MayRemove || index < 0 || index >= options.Count)
        {
            return false;
        }
        options.RemoveAt(index);
        return true;
    }

    /// <summary>The question and options to send, or null while the draft is not yet a poll.</summary>
    public (string Question, string[] Options)? Checked() =>
        Polls.HasQuestion(Question) && Polls.Sanitized(options) is { } sent
            ? (Polls.Trimmed(Question), sent)
            : null;
}
