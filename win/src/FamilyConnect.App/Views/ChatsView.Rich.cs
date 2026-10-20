using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;

namespace FamilyConnect.App.Views;

// ---- formulas, diagrams and graphs (issue #97) ------------------------------------------------------------------------------
//
// What a chat bubble DRAWS rather than sets in type (docs/protocol.md, "Rich blocks: formulas, diagrams and graphs";
// docs/capitals-and-rich-markdown-2026-10-17.md, "In the bubble"). The drawing is RichRenderer's; this is where it lands.
//
// Until a block is drawn — and for good when it cannot be — the bubble shows its source in the code face, exactly as a code
// block looks (a display formula with the delimiters it was typed with), and an inline formula its typed text, monospaced. A
// drawing held in memory is put in at once, so a bubble rebuilt by the next redraw never flickers back through its source; one
// that has to be drawn replaces the stand-in when it lands, and the chat keeps its place round the bubble that grew.

public sealed partial class ChatsView
{
    /// <summary>The body text size, DIPs: a formula is set at it (WinUI's 14-pixel body, which the emoji ladder also scales from).</summary>
    private const double BodyTextSize = 14;

    /// <summary>The widest an inline formula stands in a line before it is scaled down whole: a bubble's 576 DIPs, less some.</summary>
    private const double InlineFormulaWidth = 520;

    /// <summary>A diagram's card is rounded like every other picture in a bubble.</summary>
    private const double CardCorner = 8;

    /// <summary>The code face a stand-in is set in — a code block's. A new one each time, as the runs always made theirs.</summary>
    private static FontFamily CodeFace => new("Cascadia Mono, Consolas");

    /// <summary>The drawing the viewer is showing instead of an attachment, or null.</summary>
    private RichDrawn? viewerDrawing;

    /// <summary>
    /// How this bubble's drawings look (the design doc's job table): the theme, the bubble's own ink — white on an own bubble,
    /// the theme's primary text on anybody else's — and the body text size.
    /// </summary>
    private RichLook LookFor(bool mine)
    {
        var ink = (mine ? Palette.Ink() : Application.Current.Resources["TextFillColorPrimaryBrush"]) as SolidColorBrush;
        var colour = ink?.Color ?? (ActualTheme == ElementTheme.Dark ? Microsoft.UI.Colors.White : Microsoft.UI.Colors.Black);
        return new RichLook(ActualTheme == ElementTheme.Dark, RichJob.CssColor(colour.A, colour.R, colour.G, colour.B), BodyTextSize);
    }

    private static bool HasFormula(IEnumerable<BodyRun> runs) => runs.Any(run => run.Math is not null);

    /// <summary>
    /// A text block that carries an inline formula: a RichTextBlock, because only it can hold a picture in a run of text. The
    /// same runs as <see cref="FillRuns"/> draws, with each formula a picture on the baseline once drawn.
    /// </summary>
    private RichTextBlock FormulaText(IReadOnlyList<BodyRun> runs, bool mine, Brush ink)
    {
        var paragraph = new Paragraph();
        var text = new RichTextBlock { TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true, Foreground = ink };
        text.Blocks.Add(paragraph);
        var look = LookFor(mine);
        AddRuns(paragraph.Inlines, runs, mine, run => FormulaInline(run.Text, run.Math!, run.Style, look, paragraph, text));
        return text;
    }

    /// <summary>A table cell that carries an inline formula: as <see cref="FormulaText"/>, with the cell's own alignment.</summary>
    private RichTextBlock FormulaCell(MarkdownText cell, bool header, TextAlignment alignment, Brush ink, RichLook look)
    {
        var paragraph = new Paragraph();
        var text = new RichTextBlock
        {
            TextWrapping = TextWrapping.Wrap,
            IsTextSelectionEnabled = true,
            Foreground = ink,
            TextAlignment = alignment,
            FontWeight = header ? FontWeights.SemiBold : FontWeights.Normal,
        };
        text.Blocks.Add(paragraph);
        foreach (var run in cell.Runs)
        {
            paragraph.Inlines.Add(run.Style.Math is { } source
                ? FormulaInline(run.Text, source, run.Style, look, paragraph, text)
                : StyledRun(run.Text, run.Style, marked: false));
        }
        return text;
    }

    /// <summary>
    /// One inline formula: its picture when it is already held, else the typed text, monospaced (<see cref="StyledRun"/>), which
    /// the picture replaces when it lands.
    /// </summary>
    private Inline FormulaInline(string typed, string source, MarkdownStyle style, RichLook look, Paragraph paragraph, FrameworkElement owner)
    {
        var job = RichJob.ForInline(source, look);
        var held = services.Rich?.Held(job);
        if (held is { Picture: not null })
        {
            return FormulaPicture(held, source);
        }
        var stand = StyledRun(typed, style, marked: false);
        if (held is null && services.Rich is { } rich)
        {
            _ = PutFormulaAsync(rich, job, paragraph, stand, owner, source);
            // Held — drawn, or refused for good — or the next redraw builds the row again and asks again.
            SettlesWhen(() => rich.Held(job) is not null);
        }
        return stand;
    }

    private async Task PutFormulaAsync(RichRenderer rich, RichJob job, Paragraph paragraph, Run stand, FrameworkElement owner, string source)
    {
        var drawn = await rich.DrawAsync(job);
        if (gone || drawn.Picture is null)
        {
            return;
        }
        // A formula never carries a link or a mark (MessageBody), so its run is always the paragraph's own.
        var at = paragraph.Inlines.IndexOf(stand);
        if (at < 0)
        {
            return;
        }
        KeepPlace(owner, () =>
        {
            paragraph.Inlines.RemoveAt(at);
            paragraph.Inlines.Insert(at, FormulaPicture(drawn, source));
        });
    }

    /// <summary>
    /// A drawn inline formula: its picture at its natural size (scaled down whole when it is wider than a line can take), dropped
    /// by what hangs below its own baseline — a picture in a line stands ON the baseline, and a fraction's denominator or a
    /// subscript belongs under it (<see cref="RichFit.Inline"/>). Its name is its source.
    /// </summary>
    private static InlineUIContainer FormulaPicture(RichDrawn drawn, string source)
    {
        var (width, height, drop) = RichFit.Inline(drawn.Width, drawn.Height, drawn.Baseline, InlineFormulaWidth);
        var image = new Image
        {
            Source = drawn.Picture,
            Width = width,
            Height = height,
            Stretch = Stretch.Fill,
            RenderTransform = new TranslateTransform { Y = drop },
        };
        AutomationProperties.SetName(image, source);
        return new InlineUIContainer { Child = image };
    }

    /// <summary>
    /// A rich block in a bubble: its drawing when it is already held, the source as a code block shows it when it cannot be
    /// drawn, and otherwise that stand-in until the drawing lands in its place.
    /// </summary>
    private FrameworkElement RichBlockElement(BodyRichBlock block, Brush ink, bool mine)
    {
        var stand = new TextBlock
        {
            Text = block.Shown,
            TextWrapping = TextWrapping.Wrap,
            IsTextSelectionEnabled = true,
            Foreground = ink,
            FontFamily = CodeFace,
        };
        if (services.Rich is not { } rich)
        {
            return stand;
        }
        var job = RichJob.ForBlock(block.Kind, block.Engine, block.Source, LookFor(mine));
        switch (rich.Held(job))
        {
            case { Picture: not null } drawn:
                return DrawnBlock(drawn, block.Source, job.IsFormula);
            case not null:
                return stand;
        }
        var holder = new Border { Child = stand };
        _ = PutBlockAsync(rich, job, block.Source, holder);
        SettlesWhen(() => rich.Held(job) is not null);
        return holder;
    }

    private async Task PutBlockAsync(RichRenderer rich, RichJob job, string source, Border holder)
    {
        var drawn = await rich.DrawAsync(job);
        if (gone || drawn.Picture is null)
        {
            return;
        }
        KeepPlace(holder, () => holder.Child = DrawnBlock(drawn, source, job.IsFormula));
    }

    /// <summary>
    /// A drawn block: at its natural size, scaled down — never up — to the bubble's width. A display formula is the picture
    /// alone, in the bubble's ink. A diagram or a graph is its card — the picture painted as the rounded border's own ground,
    /// since a corner radius rounds a ground and never a child — over a transparent copy of the picture that gives it its size;
    /// a click opens it full size in the viewer.
    /// </summary>
    private FrameworkElement DrawnBlock(RichDrawn drawn, string source, bool formula)
    {
        var image = new Image
        {
            Source = drawn.Picture,
            Stretch = Stretch.Uniform,
            MaxWidth = drawn.Width,
            MaxHeight = drawn.Height,
            HorizontalAlignment = HorizontalAlignment.Left,
        };
        if (formula)
        {
            AutomationProperties.SetName(image, source);
            return image;
        }
        image.Opacity = 0;
        AutomationProperties.SetAccessibilityView(image, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        var card = new Border
        {
            Child = image,
            CornerRadius = new CornerRadius(CardCorner),
            HorizontalAlignment = HorizontalAlignment.Left,
            Background = new ImageBrush { ImageSource = drawn.Picture, Stretch = Stretch.Fill },
        };
        AutomationProperties.SetName(card, source);
        card.Tapped += (_, e) =>
        {
            // The picture's own gesture: not the balloon's, nor the start of a link's wait.
            e.Handled = true;
            OpenDrawing(drawn);
        };
        return card;
    }

    /// <summary>
    /// A drawing landed in a bubble that is showing: the change, with the chat kept where it was — at the newest when it was
    /// there, and with what the reader is looking at left still when the bubble that grew is above it — as a photo's does.
    /// </summary>
    private static void KeepPlace(FrameworkElement grown, Action change)
    {
        if (grown.XamlRoot is null || Scroller(grown) is not { } scroller)
        {
            change();
            return;
        }
        var atEnd = scroller.VerticalOffset >= scroller.ScrollableHeight - 24;
        var fromBottom = scroller.ExtentHeight - scroller.VerticalOffset;
        double top;
        try
        {
            top = grown.TransformToVisual(scroller).TransformPoint(new Windows.Foundation.Point(0, 0)).Y;
        }
        catch (ArgumentException)
        {
            top = 0;
        }
        change();
        scroller.UpdateLayout();
        if (atEnd)
        {
            scroller.ChangeView(null, scroller.ScrollableHeight, null, disableAnimation: true);
        }
        else if (top < 0)
        {
            scroller.ChangeView(null, scroller.ExtentHeight - fromBottom, null, disableAnimation: true);
        }
    }

    /// <summary>The scroller a bubble sits in: the conversation's or the thread's.</summary>
    private static ScrollViewer? Scroller(DependencyObject element)
    {
        for (var parent = VisualTreeHelper.GetParent(element); parent is not null; parent = VisualTreeHelper.GetParent(parent))
        {
            if (parent is ScrollViewer scroller)
            {
                return scroller;
            }
        }
        return null;
    }

    /// <summary>
    /// A diagram or a graph full size, in the photo viewer — zoom, Esc and Close as for a photo; no Save or Share, which hand over
    /// an attachment's bytes, and no title, so nothing new needs words.
    /// </summary>
    private void OpenDrawing(RichDrawn drawn)
    {
        YieldRound();
        viewerDrawing = drawn;
        viewing = new MediaAlbum([new AttachmentDto(0, "photo")], 0);
        ViewerOverlay.Visibility = Visibility.Visible;
        ShowViewerItem();
        ViewerClose.Focus(FocusState.Programmatic);
    }

    /// <summary>
    /// The viewer's page for a drawing (from <see cref="ShowViewerItem"/>): the picture at most at its natural size — the viewer
    /// fits a photo to the stage, and a small graph blown up to fill a screen is not "full size" — zoomable from there.
    /// </summary>
    private void ShowViewerDrawing(RichDrawn drawn)
    {
        ViewerTitle.Text = string.Empty;
        ViewerSave.Visibility = Visibility.Collapsed;
        ViewerShare.Visibility = Visibility.Collapsed;
        ViewerImage.MaxWidth = drawn.Width;
        ViewerImage.MaxHeight = drawn.Height;
        ViewerImage.Source = drawn.Picture;
        ViewerLoading.IsActive = false;
        FitViewerImage();
    }

    /// <summary>The viewer back to an attachment's page: its buttons back, and no cap on the picture's size.</summary>
    private void ShowViewerAttachment()
    {
        ViewerSave.Visibility = Visibility.Visible;
        ViewerShare.Visibility = Visibility.Visible;
        ViewerImage.MaxWidth = double.PositiveInfinity;
        ViewerImage.MaxHeight = double.PositiveInfinity;
    }
}
