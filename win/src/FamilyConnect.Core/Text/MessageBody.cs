using System.Text;

namespace FamilyConnect.Core;

/// <summary>What covers one piece of a text block: a member's <c>@Name</c>, or the assistant's <c>@ai</c> or leading <c>/draw</c>.</summary>
public readonly record struct BodyMark(long? MemberId)
{
    public static readonly BodyMark Assistant = new((long?)null);

    public static BodyMark Member(long userId) => new(userId);

    public bool IsAssistant => MemberId is null;
}

/// <summary>One piece of a text block with everything that applies to it — the unit a bubble draws.</summary>
/// <param name="Link">What a click opens: always http(s), mailto or tel.</param>
public sealed record BodyPiece(string Text, MarkdownStyle Style, string? Link, BodyMark? Mark);

/// <summary>
/// A message body composed for drawing, in exactly the order the Apple app composes it (web <c>views/body.rs</c>,
/// <c>fc_text::markdown</c>'s "Composition"): markdown runs; per text block the author's openable links, then detected links
/// wherever none already is; <c>@ai</c> and a leading <c>/draw</c> drawn bold, REPLACING the run's style; member mentions last,
/// skipped under any link. Table cells get none of it. All drawing — a body is plain text on the wire.
/// </summary>
public static class MessageBody
{
    /// <summary>A text block's own markdown links that can be opened, as spans — a label split into runs still one link.</summary>
    public static IReadOnlyList<LinkSpan> Declared(MarkdownText text)
    {
        var plain = Encoding.UTF8.GetBytes(text.Plain);
        var declared = new List<LinkSpan>();
        var at = 0;
        foreach (var run in text.Runs)
        {
            var (start, end) = (at, at + Encoding.UTF8.GetByteCount(run.Text));
            at = end;
            if (run.Style.Link is not { } link)
            {
                continue;
            }
            var target = Links.NormalizeDestination(link.Destination);
            if (!Links.IsOpenable(target))
            {
                continue;
            }
            if (declared.Count > 0 && declared[^1].End == start && declared[^1].Target == target)
            {
                var previous = declared[^1];
                declared[^1] = previous with { End = end, Text = Encoding.UTF8.GetString(plain, previous.Start, end - previous.Start) };
            }
            else
            {
                declared.Add(new LinkSpan(start, end, run.Text, target));
            }
        }
        return declared;
    }

    /// <summary>
    /// One text block laid out as pieces. <paramref name="drawSource"/> is the RAW body for the first block and null for the rest:
    /// <c>/draw</c> is a request only at the start of the whole body, marked only where markdown left it recognisable.
    /// </summary>
    public static IReadOnlyList<BodyPiece> Pieces(MarkdownText text, string? drawSource, IReadOnlyList<Named> named)
    {
        var plainText = text.Plain;
        var plain = Encoding.UTF8.GetBytes(plainText);
        var runs = new List<(int Start, int End, MarkdownStyle Style)>(text.Runs.Count);
        var at = 0;
        foreach (var run in text.Runs)
        {
            var end = at + Encoding.UTF8.GetByteCount(run.Text);
            runs.Add((at, end, run.Style));
            at = end;
        }

        var linked = Links.Merge(Declared(text), Links.Detect(plain));

        var marks = AssistantText.Ranges(plainText).Select(range => (range.Start, range.End, Mark: BodyMark.Assistant)).ToList();
        if (drawSource is not null && AssistantText.DrawTokenRange(drawSource) is not null && AssistantText.DrawTokenRange(plainText) is { } draw)
        {
            marks.Add((draw.Start, draw.End, BodyMark.Assistant));
        }

        foreach (var token in Mentions.Tokens(plainText, named))
        {
            var underLink = linked.Any(link => link.Start < token.End && token.Start < link.End);
            var underMark = marks.Any(mark => mark.Start < token.End && token.Start < mark.End);
            if (!underLink && !underMark)
            {
                marks.Add((token.Start, token.End, BodyMark.Member(token.Member.UserId)));
            }
        }

        // Cut the block at every boundary any of it has.
        var cuts = new SortedSet<int> { 0, plain.Length };
        foreach (var run in runs)
        {
            cuts.Add(run.Start);
            cuts.Add(run.End);
        }
        foreach (var link in linked)
        {
            cuts.Add(link.Start);
            cuts.Add(link.End);
        }
        foreach (var mark in marks)
        {
            cuts.Add(mark.Start);
            cuts.Add(mark.End);
        }
        var boundaries = cuts.Where(cut => cut == plain.Length || (cut < plain.Length && !RustChar.IsContinuation(plain[cut]))).ToArray();

        var pieces = new List<BodyPiece>(boundaries.Length);
        for (var index = 0; index + 1 < boundaries.Length; index++)
        {
            var (start, end) = (boundaries[index], boundaries[index + 1]);
            bool Covers(int from, int to) => from <= start && end <= to;
            var style = runs.FirstOrDefault(run => Covers(run.Start, run.End)).Style ?? MarkdownStyle.Plain;
            var link = linked.FirstOrDefault(span => Covers(span.Start, span.End));
            var target = Covers(link.Start, link.End) && link.Target is not null ? link.Target : null;
            BodyMark? mark = null;
            foreach (var candidate in marks)
            {
                if (Covers(candidate.Start, candidate.End))
                {
                    mark = candidate.Mark;
                    break;
                }
            }
            if (mark is not null)
            {
                // The mark REPLACES the run's inline style with bold — and keeps the face.
                style = new MarkdownStyle(Strong: true, Face: style.Face);
            }
            pieces.Add(new BodyPiece(Encoding.UTF8.GetString(plain, start, end - start), style, target, mark));
        }
        return pieces;
    }
}
