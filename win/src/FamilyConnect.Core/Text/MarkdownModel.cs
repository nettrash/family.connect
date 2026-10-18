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
/// <param name="Math">
/// An inline formula (issue #97): the source KaTeX draws, without its delimiters. The run's TEXT is what was typed —
/// <c>$x^2$</c>, delimiters and all — which is what a bubble shows until the drawing lands, or when it cannot be drawn. A
/// formula is always a run of its own, and no link, <c>@ai</c> or mention is laid over it.
/// </param>
public sealed record MarkdownStyle(
    bool Emphasis = false,
    bool Strong = false,
    bool Code = false,
    bool Strikethrough = false,
    bool LineBreak = false,
    bool Html = false,
    MarkdownLink? Link = null,
    MarkdownLink? Image = null,
    MarkdownFace Face = MarkdownFace.Body,
    string? Math = null)
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

/// <summary>
/// One piece of a laid-out body: a body with no table and no rich block is exactly one <see cref="MarkdownTextBlock"/>.
/// </summary>
public abstract record MarkdownBlock;

public sealed record MarkdownTextBlock(MarkdownText Text) : MarkdownBlock;

public sealed record MarkdownTableBlock(MarkdownTable Table) : MarkdownBlock;

/// <summary>Which engine draws a <see cref="MarkdownRichBlock"/> (<c>fc_text::markdown::RichKind</c>).</summary>
public enum RichKind
{
    /// <summary>KaTeX: a <c>math</c>, <c>latex</c> or <c>tex</c> fence, or display math.</summary>
    Math,
    /// <summary>Mermaid: a <c>mermaid</c> fence.</summary>
    Mermaid,
    /// <summary>Graphviz: a <c>dot</c>, <c>graphviz</c> or <c>gv</c> fence, or one named after a layout engine.</summary>
    Graphviz,
    /// <summary>md's plotter: a <c>plot</c> fence.</summary>
    Plot,
}

/// <summary>The protocol's words for a <see cref="RichKind"/>.</summary>
public static class RichKinds
{
    /// <summary>
    /// <c>math</c>, <c>mermaid</c>, <c>graphviz</c> or <c>plot</c> — the <c>kind</c> of the job a client hands <c>fc-rich.js</c>
    /// (an inline formula's job says <c>math-inline</c>).
    /// </summary>
    public static string Word(this RichKind kind) => kind switch
    {
        RichKind.Math => "math",
        RichKind.Mermaid => "mermaid",
        RichKind.Graphviz => "graphviz",
        _ => "plot",
    };
}

/// <summary>
/// A formula, a diagram or a graph standing on its own (issue #97; docs/protocol.md, "Rich blocks: formulas, diagrams and
/// graphs"). Until it has been drawn — and for good when it cannot be — a bubble shows <see cref="Source"/> in the monospaced
/// face, as it shows a code block.
/// </summary>
/// <param name="Engine">Graphviz's layout: <c>dot</c> unless the fence named another. EMPTY for every other kind.</param>
/// <param name="Source">
/// A fence's content exactly as a code block's would be, or a display formula's content trimmed of spaces, tabs and newlines.
/// </param>
public sealed record MarkdownRichBlock(RichKind Kind, string Engine, string Source) : MarkdownBlock
{
    /// <summary>
    /// What a bubble shows until the block is drawn, and for good when it cannot be (docs/capitals-and-rich-markdown-2026-10-17.md,
    /// "In the bubble"): a fence's <see cref="Source"/>, exactly as a code block shows its content, but a display formula's
    /// lines AS TYPED — <c>$$</c> or <c>\[</c> and all, as Android's <c>Block.Rich.shown</c> keeps them. It is the fallback and
    /// never a second source of truth: the drawing and its cache key come from <see cref="Source"/> alone.
    /// </summary>
    public string Shown { get; init; } = Source;
}

/// <summary>Runs as they are built, over UTF-8 — <c>Text::push</c> and friends.</summary>
internal sealed class MarkdownRuns
{
    private readonly List<(List<byte> Text, MarkdownStyle Style)> runs = [];

    public bool IsEmpty => runs.Count == 0;

    public bool HasLink => runs.Any(run => run.Style.Link is not null);

    /// <summary>
    /// Append, merging into the last run when the style is the same — except a formula: two side by side with the same source
    /// (<c>\(a\)\(a\)</c>) are two drawings, and merged they would be one.
    /// </summary>
    public void Push(ReadOnlySpan<byte> text, MarkdownStyle style)
    {
        if (text.IsEmpty)
        {
            return;
        }
        if (runs.Count > 0 && runs[^1].Style == style && style.Math is null)
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
