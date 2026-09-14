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
/// The family's wall: every note where the other clients put it, the size they draw it, in the colour
/// and hand its author chose (docs/protocol.md, "Board") — the web client's board pane, drawn from the
/// rules <c>Core/Board</c> shares with it.
/// </summary>
/// <remarks>
/// <para>
/// <b>POSITIONS ARE FRACTIONS OF THE WALL</b>, which is taller than the window and scrolls, so a note
/// sits in the same place on every screen. <see cref="BoardModel.Stickers"/> does the arithmetic; this
/// only draws what it answers.
/// </para>
/// <para>
/// <b>THE TEXT FITS THE NOTE</b>: drawn at its size's own type and scaled down until all of it is inside
/// the card — and only past the floor cut, at the lines there is room for (<see cref="NoteFitting"/>).
/// </para>
/// <para>
/// <b>THE PASTELS ARE FIXED LIGHT COLOURS IN BOTH THEMES</b>, so the ink on a note is dark in both — a
/// dark theme's white would be unreadable on yellow.
/// </para>
/// <para>
/// This is the wall to read. Opening a note — to edit it, tick it or answer it — moving it, and adding
/// one are the next slices of issue #64.
/// </para>
/// </remarks>
public sealed partial class BoardView : UserControl
{
    private const double CardPadding = 10;
    private const double Gap = 4;
    private const double LineHeight = 1.2;

    private readonly AppServices services;
    private readonly Connection connection;
    private readonly BoardModel board;
    private readonly Action shown;
    private readonly Action<NoteDto> onNote;
    private readonly Action<long, bool> onBlock;
    private readonly Action onRoster;
    private readonly Action<Resync.Report> onResync;
    private readonly Dictionary<string, BitmapImage> pictures = [];
    private readonly HashSet<long> revealed = [];
    private int redrawQueued;

    internal BoardView(AppServices services, Connection connection, Action close, Action shown)
    {
        this.services = services;
        this.connection = connection;
        this.shown = shown;
        board = new BoardModel(connection.Board, connection.Chats, connection.Api);
        InitializeComponent();
        var say = services.Say;

        Heading.Text = say.Get("Board");
        DoneButton.Content = say.Get("Done");
        EmptyTitle.Text = say.Get("The board is empty");
        EmptyText.Text = say.Get("Add a note — everyone in the family sees it.");
        DoneButton.Click += (_, _) => close();
        Scroller.SizeChanged += (_, _) => Draw();
        ActualThemeChanged += (_, _) => Draw();

        onNote = _ => QueueRedraw();
        onBlock = (_, _) => QueueRedraw();
        onRoster = QueueRedraw;
        onResync = _ => QueueRedraw();
        connection.Router.BoardChanged += onNote;
        connection.Router.BlockChanged += onBlock;
        connection.Router.RosterChanged += onRoster;
        connection.Live.Resynced += onResync;
        Unloaded += (_, _) =>
        {
            connection.Router.BoardChanged -= onNote;
            connection.Router.BlockChanged -= onBlock;
            connection.Router.RosterChanged -= onRoster;
            connection.Live.Resynced -= onResync;
        };
    }

    /// <summary>The window came back to the front with the wall on it: what is on it has now been seen.</summary>
    internal void ReaderReturned() => Draw();

    /// <summary>A frame lands on the socket's thread: one redraw, on this one, however many arrive.</summary>
    private void QueueRedraw()
    {
        if (Interlocked.Exchange(ref redrawQueued, 1) == 1)
        {
            return;
        }
        DispatcherQueue.TryEnqueue(() =>
        {
            Interlocked.Exchange(ref redrawQueued, 0);
            Draw();
        });
    }

    private void Draw()
    {
        var (visibleWidth, visibleHeight) = (Scroller.ActualWidth, Scroller.ActualHeight);
        if (visibleWidth <= 0 || visibleHeight <= 0)
        {
            return;
        }
        var (wallWidth, wallHeight) = BoardModel.WallSize(visibleWidth, visibleHeight);
        Wall.Width = wallWidth;
        Wall.Height = wallHeight;
        // Cork, and the same cork with the lamp off.
        var dark = ActualTheme == ElementTheme.Dark;
        Wall.Background = new SolidColorBrush(Hex(dark ? "#5b4a36" : "#cbb391"));
        EmptyTitle.Foreground = EmptyText.Foreground = new SolidColorBrush(dark ? Colors.White : Ink(0xD9));

        var stickers = board.Stickers(visibleWidth, visibleHeight);
        Empty.Visibility = stickers.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        var layers = WallText.Layers(stickers);
        var me = connection.Chats.Reader;
        Wall.Children.Clear();
        Wall.Children.Add(new Rectangle
        {
            Width = wallWidth,
            Height = wallHeight,
            IsHitTestVisible = false,
            Fill = Sheen(),
        });
        foreach (var sticker in stickers.OrderBy(sticker => sticker.Note.Id))
        {
            var element = StickerElement(sticker, me);
            Canvas.SetLeft(element, sticker.X);
            Canvas.SetTop(element, sticker.Y);
            Canvas.SetZIndex(element, layers[sticker.Note.Id]);
            Wall.Children.Add(element);
        }

        // Only a wall actually on screen moves the marks: the badge counts what has not been SEEN.
        if (services.Foreground)
        {
            board.Shown();
            shown();
        }
    }

    // ---- one sticker ---------------------------------------------------------------------------

    private Grid StickerElement(Sticker sticker, long me)
    {
        var say = services.Say;
        var note = sticker.Note;
        var hidden = WallText.IsHidden(note, me, connection.Chats.IsBlocked, revealed);
        var caption = WallText.CaptionOf(note);
        var bare = sticker.IsPhoto && caption.Length == 0 && !hidden && sticker.Picture is not null;
        var author = WallText.AuthorName(note, me, id => connection.Chats.Member(id)?.DisplayName, say);

        var root = new Grid
        {
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

        if (hidden)
        {
            // The first click reveals and does nothing else: falling through would open the very text it hides.
            root.Tapped += (_, _) =>
            {
                revealed.Add(note.Id);
                Draw();
            };
        }
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

    /// <summary>Two soft washes over the cork: light from the top, a little shade at the bottom.</summary>
    private static LinearGradientBrush Sheen()
    {
        var sheen = new LinearGradientBrush { StartPoint = new Point(0.15, 0), EndPoint = new Point(0.85, 1) };
        sheen.GradientStops.Add(new GradientStop { Color = ColorHelper.FromArgb(0x42, 0xFF, 0xFF, 0xFF), Offset = 0 });
        sheen.GradientStops.Add(new GradientStop { Color = ColorHelper.FromArgb(0x00, 0xFF, 0xFF, 0xFF), Offset = 0.55 });
        sheen.GradientStops.Add(new GradientStop { Color = ColorHelper.FromArgb(0x24, 0x00, 0x00, 0x00), Offset = 1 });
        return sheen;
    }

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

    private static Color Ink(byte alpha) => ColorHelper.FromArgb(alpha, 0, 0, 0);

    private static Color Hex(string hex) => ColorHelper.FromArgb(
        0xFF,
        Convert.ToByte(hex.Substring(1, 2), 16),
        Convert.ToByte(hex.Substring(3, 2), 16),
        Convert.ToByte(hex.Substring(5, 2), 16));
}
