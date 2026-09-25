using System.Text;

namespace FamilyConnect.Core;

/// <summary>The face a run is set in (<c>fc_text::markdown::Font</c>): a <c>#</c>–<c>###</c> line's heading, or a fence's code.</summary>
public enum MarkdownFace
{
    Body,
    Heading1,
    Heading2,
    Heading3,
    Monospaced,
}

/// <summary>A table column's <c>:---</c> / <c>:--:</c> / <c>---:</c>.</summary>
public enum MarkdownAlignment
{
    Leading,
    Center,
    Trailing,
}

/// <summary>A destination exactly as the parser produced it: entities and escapes resolved, NOT given a scheme.</summary>
public sealed record MarkdownLink(string Destination, string? Title);

/// <summary>Everything Apple draws over a run, in Foundation's vocabulary (<c>fc_text::markdown::Style</c>).</summary>
public sealed record MarkdownStyle(
    bool Emphasis = false,
    bool Strong = false,
    bool Code = false,
    bool Strikethrough = false,
    bool LineBreak = false,
    bool Html = false,
    MarkdownLink? Link = null,
    MarkdownLink? Image = null,
    MarkdownFace Face = MarkdownFace.Body)
{
    public static readonly MarkdownStyle Plain = new();
}

/// <summary>Some characters and what is drawn over them.</summary>
public sealed record MarkdownRun(string Text, MarkdownStyle Style);

/// <summary>A laid-out string: maximal runs, two neighbours never sharing a style.</summary>
public sealed class MarkdownText
{
    internal MarkdownText(IReadOnlyList<MarkdownRun> runs) => Runs = runs;

    public IReadOnlyList<MarkdownRun> Runs { get; }

    /// <summary>The characters as drawn, markup removed — what every offset-based pass indexes, never the raw body.</summary>
    public string Plain => string.Concat(Runs.Select(run => run.Text));

    public bool IsEmpty => Runs.Count == 0;
}

/// <summary>A GFM pipe table: every row exactly <see cref="Alignments"/> cells long.</summary>
public sealed class MarkdownTable
{
    internal MarkdownTable(IReadOnlyList<MarkdownAlignment> alignments, IReadOnlyList<MarkdownText> header, IReadOnlyList<IReadOnlyList<MarkdownText>> rows)
    {
        Alignments = alignments;
        Header = header;
        Rows = rows;
    }

    public IReadOnlyList<MarkdownAlignment> Alignments { get; }

    public IReadOnlyList<MarkdownText> Header { get; }

    public IReadOnlyList<IReadOnlyList<MarkdownText>> Rows { get; }

    public int ColumnCount => Alignments.Count;

    /// <summary>Leading for a column past the last one.</summary>
    public MarkdownAlignment Alignment(int column) => column < Alignments.Count ? Alignments[column] : MarkdownAlignment.Leading;
}

/// <summary>One piece of a laid-out body: a body with no table is exactly one <see cref="MarkdownTextBlock"/>.</summary>
public abstract record MarkdownBlock;

public sealed record MarkdownTextBlock(MarkdownText Text) : MarkdownBlock;

public sealed record MarkdownTableBlock(MarkdownTable Table) : MarkdownBlock;

/// <summary>Runs as they are built, over UTF-8 — <c>Text::push</c> and friends.</summary>
internal sealed class MarkdownRuns
{
    private readonly List<(List<byte> Text, MarkdownStyle Style)> runs = [];

    public bool IsEmpty => runs.Count == 0;

    public bool HasLink => runs.Any(run => run.Style.Link is not null);

    /// <summary>Append, merging into the last run when the style is the same.</summary>
    public void Push(ReadOnlySpan<byte> text, MarkdownStyle style)
    {
        if (text.IsEmpty)
        {
            return;
        }
        if (runs.Count > 0 && runs[^1].Style == style)
        {
            runs[^1].Text.AddRange(text);
            return;
        }
        var bytes = new List<byte>(text.Length);
        bytes.AddRange(text);
        runs.Add((bytes, style));
    }

    public void Append(MarkdownRuns other)
    {
        foreach (var (text, style) in other.runs)
        {
            Push(text.ToArray(), style);
        }
    }

    public void SetFace(MarkdownFace face) => Restyle(style => style with { Face = face });

    public void StripLinks() => Restyle(style => style with { Link = null });

    private void Restyle(Func<MarkdownStyle, MarkdownStyle> change)
    {
        var old = runs.ToArray();
        runs.Clear();
        foreach (var (text, style) in old)
        {
            Push(text.ToArray(), change(style));
        }
    }

    public MarkdownText ToText() => new([.. runs.Select(run => new MarkdownRun(Encoding.UTF8.GetString(run.Text.ToArray()), run.Style))]);
}
