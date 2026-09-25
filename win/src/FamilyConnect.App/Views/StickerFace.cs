using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using FamilyConnect.Core;
using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;
using Microsoft.UI;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Xaml.Shapes;
using Windows.Foundation;
using Windows.UI;
using Windows.UI.Text;
using static FamilyConnect.App.Views.Dialogs;

namespace FamilyConnect.App.Views;

/// <summary>
/// One sticker's face — the card, its picture or backdrop, the event's calendar block, the fitted words and the
/// pin — drawn the same for the wall and for the note sheet's preview, which is why it is its own class: a
/// second renderer would be a second place for the board's rules to drift.
/// </summary>
/// <remarks>
/// It holds the picture cache, keyed by ATTACHMENT (issue #70), so the wall and the sheet opened from it share
/// one.
/// </remarks>
internal sealed class StickerFace(AppServices services, Connection connection)
{
    private const double CardPadding = 10;
    private const double Gap = 4;
    private const double LineHeight = 1.2;

    private readonly Dictionary<string, BitmapImage> pictures = [];

    public Grid Build(Sticker sticker, long me, HashSet<long> revealed)
    {
        var say = services.Say;
        var note = sticker.Note;
        var hidden = WallText.IsHidden(note, me, connection.Chats.IsBlocked, revealed);
        var caption = WallText.CaptionOf(note);
        var bare = sticker.IsPhoto && caption.Length == 0 && !hidden && sticker.Picture is not null;
        var author = WallText.AuthorName(note, me, id => connection.Chats.Member(id)?.DisplayName, say);

        var root = new Grid
        {
            // Transparent, not absent: an element with no background takes no pointer where nothing is drawn.
            Background = new SolidColorBrush(Colors.Transparent),
            Width = sticker.Width,
            Height = sticker.Height,
            RenderTransformOrigin = new Point(0.5, 0.5),
            RenderTransform = new RotateTransform { Angle = sticker.TiltDegrees },
        };
        AutomationProperties.SetName(root, WallText.Label(hidden, sticker.Mine, author, WhatOf(sticker, caption), say));

        root.Children.Add(new Border
        {
            CornerRadius = new CornerRadius(bare ? 4 : 8),
            Background = bare ? null : new SolidColorBrush(Hex(Notes.ColorHex(note.Color))),
            BorderBrush = bare ? null : new SolidColorBrush(Ink(0x22)),
            BorderThickness = new Thickness(bare ? 0 : 1),
        });
        // AN EVENT'S PICTURE IS ITS BACKDROP: the ground the card is drawn on, under a scrim of its own.
        if (!hidden && sticker.IsEvent && sticker.Picture is { } backdrop)
        {
            root.Children.Add(Backdrop(backdrop));
        }

        var inner = (Width: sticker.Width - (bare ? 0 : 2 * CardPadding), Height: sticker.Height - (bare ? 0 : 2 * CardPadding));
        var content = new Grid { Padding = new Thickness(bare ? 0 : CardPadding), RowSpacing = Gap };
        content.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        content.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        content.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        var used = 0.0;

        if (!hidden && sticker.IsPhoto && sticker.Picture is { } photo)
        {
            // A PHOTO IS DRAWN WHOLE: fitted, never cropped to fill.
            var image = new Image { Stretch = Enum.Parse<Stretch>(BoardPicture.Stretch) };
            _ = ShowPictureAsync(image, photo);
            var frame = new Border { Child = image, CornerRadius = new CornerRadius(bare ? 3 : 6) };
            if (bare)
            {
                Grid.SetRowSpan(frame, 3);
            }
            else
            {
                frame.Height = inner.Height * 0.55;
                used += frame.Height + Gap;
            }
            content.Children.Add(frame);
        }
        if (!hidden && sticker.IsEvent && note.StartsAt is { } starts)
        {
            var block = EventBlock(note, starts);
            block.Measure(new Size(inner.Width, double.PositiveInfinity));
            used += block.DesiredSize.Height + Gap;
            content.Children.Add(block);
        }
        if (WallText.ShowsAuthor(sticker.Kind, caption, hidden))
        {
            var signed = new TextBlock
            {
                Text = author,
                FontSize = 11,
                Foreground = new SolidColorBrush(Ink(0x80)),
                TextWrapping = TextWrapping.NoWrap,
                TextTrimming = TextTrimming.CharacterEllipsis,
            };
            signed.Measure(new Size(inner.Width, double.PositiveInfinity));
            used += signed.DesiredSize.Height + Gap;
            Grid.SetRow(signed, 2);
            content.Children.Add(signed);
        }
        if (WallText.ShowsText(sticker.Kind, caption, hidden))
        {
            var words = hidden ? say.Get("Hidden — blocked member") : note.Text ?? string.Empty;
            var floorLine = BoardWall.TypePx(sticker.Size) * NoteFitting.MinTextScale * LineHeight;
            var text = FittedText(sticker, words, hidden, inner.Width, Math.Max(floorLine, inner.Height - used));
            Grid.SetRow(text, 1);
            content.Children.Add(text);
        }
        root.Children.Add(content);
        root.Children.Add(Pin());

        return root;
    }

    private string WhatOf(Sticker sticker, string caption)
    {
        var note = sticker.Note;
        if (!sticker.IsEvent || note.StartsAt is not { } starts)
        {
            return WallText.What(sticker.Kind, caption, null, null, null, services.Say);
        }
        return WallText.What(
            sticker.Kind, caption,
            EventText.When(starts, note.EndsAt, services.Culture, TimeZoneInfo.Local),
            note.Place,
            Notes.GoingLine(note.Count("going"), note.Count("maybe"), services.Say),
            services.Say);
    }

    /// <summary>
    /// The words — and a list's first lines under them, in the same box, so the two scale together and
    /// the whole note is inside its card.
    /// </summary>
    private StackPanel FittedText(Sticker sticker, string words, bool hidden, double width, double room)
    {
        var basePx = BoardWall.TypePx(sticker.Size);
        var family = new FontFamily(Notes.FontFamily(sticker.Font));
        var panel = new StackPanel { VerticalAlignment = VerticalAlignment.Top };
        var title = new TextBlock
        {
            Text = words,
            FontFamily = family,
            TextWrapping = TextWrapping.Wrap,
            LineStackingStrategy = LineStackingStrategy.BlockLineHeight,
            FontStyle = hidden ? FontStyle.Italic : FontStyle.Normal,
            Foreground = new SolidColorBrush(Ink(hidden ? (byte)0x73 : (byte)0xD9)),
        };
        panel.Children.Add(title);
        // The names the note says, BOLD in the note's own ink — and no door on the wall: a sticker's whole face is a drag handle.
        if (!hidden && sticker.Note.Mentions is { Length: > 0 } named)
        {
            title.Text = string.Empty;
            title.Inlines.Clear();
            foreach (var (text, userId) in Mentions.Runs(words, [.. named.Select(mention => new Named(mention.UserId, mention.Name))]))
            {
                title.Inlines.Add(new Run { Text = text, FontWeight = userId is null ? FontWeights.Normal : FontWeights.SemiBold });
            }
        }

        var lines = new List<TextBlock>();
        if (!hidden && sticker.IsTasks)
        {
            var items = sticker.Note.TaskList;
            var (shownLines, left) = BoardTasks.Drawn(items.Count);
            foreach (var item in items.Take(shownLines))
            {
                var line = new TextBlock { FontFamily = family, TextWrapping = TextWrapping.NoWrap, TextTrimming = TextTrimming.CharacterEllipsis };
                line.Inlines.Add(new Run { Text = item.Done ? "☑ " : "☐ ", Foreground = new SolidColorBrush(Ink(0xD9)) });
                line.Inlines.Add(new Run
                {
                    Text = item.Text,
                    TextDecorations = item.Done ? TextDecorations.Strikethrough : TextDecorations.None,
                    Foreground = new SolidColorBrush(Ink(item.Done ? (byte)0x75 : (byte)0xD9)),
                });
                lines.Add(line);
            }
            if (left > 0)
            {
                lines.Add(new TextBlock
                {
                    Text = WallText.MoreLine(left, services.Say),
                    FontFamily = family,
                    FontStyle = FontStyle.Italic,
                    Foreground = new SolidColorBrush(Ink(0x80)),
                });
            }
            if (lines.Count > 0)
            {
                lines[0].Margin = new Thickness(0, Gap, 0, 0);
            }
            foreach (var line in lines)
            {
                panel.Children.Add(line);
            }
        }

        void At(double scale)
        {
            title.FontSize = basePx * scale;
            title.LineHeight = basePx * scale * LineHeight;
            foreach (var line in lines)
            {
                line.FontSize = basePx * scale * 0.85;
            }
        }
        bool Fits(double scale)
        {
            At(scale);
            panel.Measure(new Size(width, double.PositiveInfinity));
            return panel.DesiredSize.Height <= room + 1;
        }
        if (NoteFitting.FittedScale(Fits, NoteFitting.MinTextScale, NoteFitting.FitSteps) is { } fitted)
        {
            At(fitted);
        }
        else
        {
            // Past the floor: cut, with an ellipsis, at the lines it holds.
            At(NoteFitting.MinTextScale);
            title.MaxLines = NoteFitting.LinesThatFit(room, basePx * NoteFitting.MinTextScale * LineHeight);
            title.TextTrimming = TextTrimming.CharacterEllipsis;
        }
        panel.MaxHeight = room;
        return panel;
    }

    /// <summary>
    /// When, where and who is coming — above the title, because the date is the reason it is on the wall —
    /// drawn as a calendar entry: the day over its short month on paper of its own, the time beside it.
    /// </summary>
    private StackPanel EventBlock(NoteDto note, string starts)
    {
        var culture = services.Culture;
        var past = EventText.IsPast(starts, note.EndsAt, DateTimeOffset.Now);
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        if (EventText.DateBlock(starts, culture, TimeZoneInfo.Local) is { } date)
        {
            var page = new StackPanel();
            page.Children.Add(new TextBlock
            {
                Text = date.Day,
                FontSize = 15,
                FontWeight = FontWeights.Bold,
                HorizontalAlignment = HorizontalAlignment.Center,
                Foreground = new SolidColorBrush(Ink(0xC7)),
            });
            page.Children.Add(new TextBlock
            {
                Text = date.Month.ToUpper(culture),
                FontSize = 9,
                HorizontalAlignment = HorizontalAlignment.Center,
                Foreground = new SolidColorBrush(ColorHelper.FromArgb(0xD9, 0xB2, 0x26, 0x1E)),
            });
            row.Children.Add(new Border
            {
                Child = page,
                Padding = new Thickness(5, 2, 5, 3),
                CornerRadius = new CornerRadius(5),
                Background = new SolidColorBrush(ColorHelper.FromArgb(0x8C, 0xFF, 0xFF, 0xFF)),
                Opacity = past ? 0.55 : 1,
            });
        }
        var lines = new StackPanel { Spacing = 1 };
        lines.Children.Add(new TextBlock
        {
            Text = EventText.Clock(starts, note.EndsAt, culture, TimeZoneInfo.Local),
            FontSize = 11,
            FontWeight = FontWeights.SemiBold,
            TextWrapping = TextWrapping.Wrap,
            MaxLines = 2,
            TextTrimming = TextTrimming.CharacterEllipsis,
            Foreground = new SolidColorBrush(Ink(past ? (byte)0x66 : (byte)0xBF)),
        });
        if (note.Place is { Length: > 0 } place)
        {
            lines.Children.Add(Small(place));
        }
        if (Notes.GoingLine(note.Count("going"), note.Count("maybe"), services.Say) is { } going)
        {
            lines.Children.Add(Small(going));
        }
        row.Children.Add(lines);
        return row;
    }

    private static TextBlock Small(string text) => new()
    {
        Text = text,
        FontSize = 11,
        TextWrapping = TextWrapping.NoWrap,
        TextTrimming = TextTrimming.CharacterEllipsis,
        Foreground = new SolidColorBrush(Ink(0x8C)),
    };

    /// <summary>
    /// The one picture on the wall NOT drawn whole: it stands in for the paper, so it covers the card —
    /// lighter at the top, heavier under the words, so the fixed dark ink stays readable.
    /// </summary>
    private Border Backdrop(AttachmentDto attachment)
    {
        var image = new Image { Stretch = Stretch.UniformToFill };
        _ = ShowPictureAsync(image, attachment);
        var ground = new Grid();
        ground.Children.Add(image);
        var scrim = new LinearGradientBrush { StartPoint = new Point(0, 0), EndPoint = new Point(0, 1) };
        scrim.GradientStops.Add(new GradientStop { Color = ColorHelper.FromArgb(0x73, 0xFF, 0xFF, 0xFF), Offset = 0 });
        scrim.GradientStops.Add(new GradientStop { Color = ColorHelper.FromArgb(0xC7, 0xFF, 0xFF, 0xFF), Offset = 1 });
        ground.Children.Add(new Rectangle { Fill = scrim });
        return new Border { Child = ground, CornerRadius = new CornerRadius(8) };
    }

    /// <summary>The pin: decoration, nowhere on the wire, and no hit area of its own.</summary>
    private static Ellipse Pin() => new()
    {
        Width = 9,
        Height = 9,
        Margin = new Thickness(0, -4, 0, 0),
        HorizontalAlignment = HorizontalAlignment.Center,
        VerticalAlignment = VerticalAlignment.Top,
        IsHitTestVisible = false,
        Fill = new SolidColorBrush(Hex("#b3261e")),
        Stroke = new SolidColorBrush(Hex("#7a1a15")),
        StrokeThickness = 1,
    };

    /// <summary>
    /// A picture through the same cache a message's comes from — cached by ATTACHMENT, never by note: a
    /// redrawn backdrop is a new attachment, and a picture cached by note is shown for ever (issue #70).
    /// </summary>
    private async Task ShowPictureAsync(Image image, AttachmentDto attachment)
    {
        var source = AttachmentFiles.SourceFor(attachment);
        if (source == AttachmentFiles.TileSource.None)
        {
            return;
        }
        var preview = source == AttachmentFiles.TileSource.Preview;
        var key = AttachmentCache.KeyFor(attachment.Id, preview && attachment.HasPreview);
        try
        {
            if (!pictures.TryGetValue(key, out var picture))
            {
                var (bytes, error) = await connection.Attachments.BytesAsync(attachment, preview);
                if (bytes is null)
                {
                    if (error is not null)
                    {
                        Diagnostics.Write($"a board picture: {error.Code} {error.Status}");
                    }
                    return;
                }
                picture = await BitmapAsync(bytes);
                pictures[key] = picture;
            }
            image.Source = picture;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"drawing a board picture: {e.GetType().Name}");
        }
    }

    internal static Color Ink(byte alpha) => ColorHelper.FromArgb(alpha, 0, 0, 0);

    internal static Color Hex(string hex) => ColorHelper.FromArgb(
        0xFF,
        Convert.ToByte(hex.Substring(1, 2), 16),
        Convert.ToByte(hex.Substring(3, 2), 16),
        Convert.ToByte(hex.Substring(5, 2), 16));
}
