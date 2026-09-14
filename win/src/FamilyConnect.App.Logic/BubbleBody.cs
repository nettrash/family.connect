using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>One stretch of a text block with everything the window draws over it.</summary>
/// <param name="Link">What a click opens — always http(s), mailto or tel.</param>
/// <param name="MemberId">The member a mention names, when it is one.</param>
/// <param name="Opens">A mention of somebody this reader can message: a door onto that chat.</param>
/// <param name="Marked">A mention or the assistant's <c>@ai</c> or <c>/draw</c>: drawn bold in the bubble's own ink.</param>
public sealed record BodyRun(string Text, MarkdownStyle Style, string? Link, long? MemberId, bool Opens, bool Marked);

/// <summary>A body as the window lays it out: text blocks, and tables between them.</summary>
public abstract record BodyBlock;

public sealed record BodyTextBlock(IReadOnlyList<BodyRun> Runs) : BodyBlock;

public sealed record BodyTableBlock(MarkdownTable Table) : BodyBlock;

/// <summary>
/// A message body for a bubble (ios <c>MessageBodyView</c>, web <c>views/body.rs</c>): markdown, links, the assistant's
/// tokens and member mentions, composed by <see cref="MessageBody"/>, and which mentions open a chat — the rule the plain runs
/// had (<see cref="ComposerMentions.OpensChat"/>).
/// </summary>
public static class BubbleBody
{
    /// <summary>How long a link waits before it opens, so a double-click on it can be the heart instead — the Mac's 350 ms.</summary>
    public static readonly TimeSpan LinkDelay = TimeSpan.FromMilliseconds(350);

    public static IReadOnlyList<BodyBlock> Lay(
        string body, MentionDto[]? named, IReadOnlyList<MemberDto> members, long me, Func<long, bool> blocked)
    {
        Named[] mentions = named is { Length: > 0 } ? [.. named.Select(mention => new Named(mention.UserId, mention.Name))] : [];
        var blocks = Markdown.Blocks(body);
        var laid = new List<BodyBlock>(blocks.Count);
        for (var index = 0; index < blocks.Count; index++)
        {
            switch (blocks[index])
            {
                case MarkdownTableBlock { Table: var table }:
                    laid.Add(new BodyTableBlock(table));
                    break;
                case MarkdownTextBlock { Text: var text }:
                    // Only the first block may carry `/draw`: it is a request only at the start of the whole body.
                    laid.Add(new BodyTextBlock([.. MessageBody.Pieces(text, index == 0 ? body : null, mentions).Select(piece => new BodyRun(
                        piece.Text,
                        piece.Style,
                        piece.Link,
                        piece.Mark?.MemberId,
                        piece.Mark is { MemberId: { } id } && ComposerMentions.OpensChat(id, members, me, blocked),
                        piece.Mark is not null))]));
                    break;
            }
        }
        return laid;
    }

    private static readonly Dictionary<string, string?> PreviewLinks = new(StringComparer.Ordinal);

    /// <summary>
    /// The link a bubble's preview card describes: the first https link its body draws (<see cref="MessageBody.FirstWebLink"/>),
    /// and none for a body of nothing but emoji, which draws no markup to find one in. Remembered per body, as the Mac does —
    /// every redraw asks again.
    /// </summary>
    public static string? PreviewLink(string body, bool emojiOnly)
    {
        if (emojiOnly || body.Length == 0)
        {
            return null;
        }
        lock (PreviewLinks)
        {
            if (PreviewLinks.TryGetValue(body, out var known))
            {
                return known;
            }
            if (PreviewLinks.Count >= 512)
            {
                PreviewLinks.Clear();
            }
            return PreviewLinks[body] = MessageBody.FirstWebLink(body);
        }
    }

    /// <summary>A link target as the system opens it, or null when it is not an absolute http(s), mailto or tel URI.</summary>
    public static Uri? Openable(string? target) =>
        target is not null && Links.IsOpenable(target) && Uri.TryCreate(target, UriKind.Absolute, out var uri) ? uri : null;
}
