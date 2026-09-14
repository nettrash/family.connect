using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using FamilyConnect.Core;
using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;
using Microsoft.UI;
using Microsoft.UI.Input;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Xaml.Shapes;
using Windows.ApplicationModel.DataTransfer;
using Windows.Foundation;
using Windows.Storage;
using Windows.Storage.Pickers;
using Windows.System;
using Windows.UI;
using Windows.UI.Core;
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
    private bool sheetOpen;

    private static readonly string[] PictureTypes =
        [".jpg", ".jpeg", ".png", ".heic", ".heif", ".bmp", ".gif", ".tif", ".tiff", ".webp"];

    private readonly PhotoPinning pinning;
    private readonly Dictionary<long, NoteHand> hands = [];
    private bool preparingPhoto;
    private bool redrawWhenPutDown;

    internal BoardView(AppServices services, Connection connection, Action close, Action shown)
    {
        this.services = services;
        this.connection = connection;
        this.shown = shown;
        board = new BoardModel(connection.Board, connection.Chats, connection.Api);
        pinning = new PhotoPinning(connection.Api, connection.Board, Task.Delay);
        InitializeComponent();
        var say = services.Say;

        Heading.Text = say.Get("Board");
        DoneButton.Content = say.Get("Done");
        EmptyTitle.Text = say.Get("The board is empty");
        EmptyText.Text = say.Get("Add a note — everyone in the family sees it.");
        AddNoteButton.Content = say.Get("Add Note");
        AddEventButton.Content = say.Get("Add Event");
        AddListButton.Content = say.Get("Add List");
        PinPhotoButton.Content = say.Get("Pin a Photo");
        PinPhotoButton.Click += (_, _) => _ = PickPhotoAsync();
        Scroller.DragOver += OnDragOver;
        Scroller.Drop += OnDrop;
        DoneButton.Click += (_, _) => close();
        AddNoteButton.Click += (_, _) => _ = SheetAsync(null, NoteKind.Text);
        AddEventButton.Click += (_, _) => _ = SheetAsync(null, NoteKind.Event);
        AddListButton.Click += (_, _) => _ = SheetAsync(null, NoteKind.Tasks);
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
        // A note in hand keeps its element: rebuilding the wall under it would take the pointer it holds.
        if (hands.Values.Any(hand => hand.Dragging))
        {
            redrawWhenPutDown = true;
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
        var wall = (wallWidth, wallHeight);
        foreach (var sticker in stickers.OrderBy(sticker => sticker.Note.Id))
        {
            var hand = Hand(sticker.Note.Id);
            var hidden = WallText.IsHidden(sticker.Note, me, connection.Chats.IsBlocked, revealed);
            var holder = Holder(sticker, StickerElement(sticker, me), hidden, wall);
            var (left, top) = hand.Corner((sticker.Note.X, sticker.Note.Y), (sticker.Width, sticker.Height), wall);
            Canvas.SetLeft(holder, left);
            Canvas.SetTop(holder, top);
            // Still above everything once put down, until the move is answered.
            Canvas.SetZIndex(holder, hand.Held ? 99_999 : layers[sticker.Note.Id]);
            Wall.Children.Add(holder);
        }
        foreach (var gone in hands.Where(pair => !pair.Value.Sending && stickers.All(sticker => sticker.Note.Id != pair.Key)).Select(pair => pair.Key).ToList())
        {
            hands.Remove(gone);
        }

        // Only a wall actually on screen moves the marks: the badge counts what has not been SEEN.
        if (services.Foreground)
        {
            board.Shown();
            shown();
        }
    }

    // ---- a note in hand --------------------------------------------------------------------------

    private NoteHand Hand(long noteId)
    {
        if (!hands.TryGetValue(noteId, out var hand))
        {
            hand = new NoteHand();
            hands[noteId] = hand;
        }
        return hand;
    }

    /// <summary>
    /// The sticker, made something a pointer and a keyboard can hold. ANYONE MAY DRAG ANY NOTE; a press that
    /// does not travel OPENS it — or, for a note a block hides, reveals it and does nothing else.
    /// </summary>
    private ContentControl Holder(Sticker sticker, Grid card, bool hidden, (double Width, double Height) wall)
    {
        var id = sticker.Note.Id;
        var fraction = (sticker.Note.X, sticker.Note.Y);
        var size = (sticker.Width, sticker.Height);
        var holder = new ContentControl
        {
            Content = card,
            Width = sticker.Width,
            Height = sticker.Height,
            IsTabStop = true,
            UseSystemFocusVisuals = true,
        };
        AutomationProperties.SetName(holder, AutomationProperties.GetName(card));

        void Place()
        {
            var (x, y) = Hand(id).Corner(fraction, size, wall);
            Canvas.SetLeft(holder, x);
            Canvas.SetTop(holder, y);
        }
        void Click()
        {
            if (hidden)
            {
                // Falling through would open the very text it hides.
                revealed.Add(id);
                Draw();
            }
            else
            {
                _ = SheetAsync(id, sticker.Kind);
            }
        }

        holder.PointerPressed += (_, e) =>
        {
            if (!e.GetCurrentPoint(holder).Properties.IsLeftButtonPressed)
            {
                return;
            }
            holder.CapturePointer(e.Pointer);
            var at = e.GetCurrentPoint(Wall).Position;
            Hand(id).Down(e.Pointer.PointerId, (at.X, at.Y), fraction, size, wall);
            e.Handled = true;
        };
        holder.PointerMoved += (_, e) =>
        {
            var at = e.GetCurrentPoint(Wall).Position;
            if (Hand(id).Move(e.Pointer.PointerId, (at.X, at.Y)))
            {
                Canvas.SetZIndex(holder, 100_000);
                Place();
            }
        };
        holder.PointerReleased += (_, e) =>
        {
            var letting = Hand(id).Up(e.Pointer.PointerId, fraction, size, wall);
            holder.ReleasePointerCapture(e.Pointer);
            Settle(id, letting, Click);
        };
        // A drag the system took away is put back where it was, not dropped where it happened to be. A release has
        // already ended the drag by the time its capture goes, so this does nothing after an ordinary drop.
        holder.PointerCanceled += (_, e) => PutBack(e.Pointer.PointerId);
        holder.PointerCaptureLost += (_, e) => PutBack(e.Pointer.PointerId);
        void PutBack(uint pointer)
        {
            Hand(id).Cancel(pointer);
            Place();
            AfterPutDown();
        }

        // The keyboard's way to do all of it: Enter or Space opens, the arrows move — a hundredth of the wall a
        // press, a twentieth with Shift — and letting go of an arrow, or the focus going, puts it down.
        holder.KeyDown += (_, e) =>
        {
            if (e.Key is VirtualKey.Enter or VirtualKey.Space)
            {
                e.Handled = true;
                Click();
                return;
            }
            var shift = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Shift).HasFlag(CoreVirtualKeyStates.Down);
            var step = shift ? 0.05 : 0.01;
            var (dx, dy) = e.Key switch
            {
                VirtualKey.Left => (-step * wall.Width, 0.0),
                VirtualKey.Right => (step * wall.Width, 0.0),
                VirtualKey.Up => (0.0, -step * wall.Height),
                VirtualKey.Down => (0.0, step * wall.Height),
                _ => (0.0, 0.0),
            };
            if (dx == 0 && dy == 0)
            {
                return;
            }
            e.Handled = true;
            Hand(id).Nudge(dx, dy, fraction, size, wall);
            Canvas.SetZIndex(holder, 100_000);
            Place();
        };
        holder.KeyUp += (_, e) =>
        {
            if (e.Key is VirtualKey.Left or VirtualKey.Right or VirtualKey.Up or VirtualKey.Down)
            {
                Settle(id, Hand(id).PutDown(fraction, size, wall), Click);
            }
        };
        holder.LostFocus += (_, _) => Settle(id, Hand(id).PutDown(fraction, size, wall), Click);
        return holder;
    }

    private void Settle(long noteId, Letting letting, Action click)
    {
        switch (letting.Result)
        {
            case HandResult.Click:
                click();
                break;
            case HandResult.Drop:
                if (letting.SendNow && letting.Target is { } target)
                {
                    _ = SendMovesAsync(noteId, target);
                }
                redrawWhenPutDown = false;
                Draw();
                break;
        }
        AfterPutDown();
    }

    private void AfterPutDown()
    {
        if (redrawWhenPutDown && !hands.Values.Any(hand => hand.Dragging))
        {
            redrawWhenPutDown = false;
            Draw();
        }
    }

    /// <summary>One move of a note at a time; when it is answered, the drop that waited behind it goes next.</summary>
    private async Task SendMovesAsync(long noteId, (double X, double Y) target)
    {
        var next = target;
        while (true)
        {
            ApiError? error;
            try
            {
                error = await board.PatchAsync(noteId, new NotePatch(X: next.X, Y: next.Y));
            }
            catch (Exception e)
            {
                Diagnostics.Write($"moving a note: {e.GetType().Name}");
                error = ApiError.Transport(e.GetType().Name);
            }
            // A note taken down meanwhile has nowhere to be moved to, and nothing to be sorry about.
            if (error is not null && error.Code != ErrorCodes.NoteNotFound)
            {
                ShowStatus(services.Say.Get("Couldn't move the note."));
            }
            if (Hand(noteId).Answered() is not { } waiting)
            {
                break;
            }
            next = waiting;
        }
        Draw();
    }

    // ---- a photo onto the wall -------------------------------------------------------------------

    private async Task PickPhotoAsync()
    {
        StorageFile? file;
        try
        {
            var picker = new FileOpenPicker { SuggestedStartLocation = PickerLocationId.PicturesLibrary, ViewMode = PickerViewMode.Thumbnail };
            foreach (var type in PictureTypes)
            {
                picker.FileTypeFilter.Add(type);
            }
            WinRT.Interop.InitializeWithWindow.Initialize(picker, services.WindowHandle);
            file = await picker.PickSingleFileAsync();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"picking a photo to pin: {e.GetType().Name}");
            ShowStatus(services.Say.Get("Something went wrong. Try again."));
            return;
        }
        if (file is not null)
        {
            await PinAsync(file, PhotoPinning.Scattered(Random.Shared.NextDouble));
        }
    }

    private void OnDragOver(object sender, DragEventArgs e)
    {
        if (e.DataView.Contains(StandardDataFormats.StorageItems))
        {
            e.AcceptedOperation = DataPackageOperation.Copy;
        }
    }

    /// <summary>A photo dropped on the wall lands where the pointer is, the card centred under it.</summary>
    private async void OnDrop(object sender, DragEventArgs e)
    {
        if (!e.DataView.Contains(StandardDataFormats.StorageItems))
        {
            return;
        }
        var point = e.GetPosition(Wall);
        List<StorageFile> files;
        var deferral = e.GetDeferral();
        try
        {
            files = (await e.DataView.GetStorageItemsAsync()).OfType<StorageFile>().ToList();
        }
        catch (Exception exception)
        {
            Diagnostics.Write($"reading a drop on the board: {exception.GetType().Name}");
            return;
        }
        finally
        {
            deferral.Complete();
        }
        if (files.Count == 0)
        {
            return;
        }
        try
        {
            await PinAsync(files[0], PhotoPinning.DroppedAt((point.X, point.Y), (Wall.Width, Wall.Height)));
            if (files.Count > 1)
            {
                ShowStatus(PhotoPinning.OneAtATimeText(services.Say));
            }
        }
        catch (Exception exception)
        {
            Diagnostics.Write($"pinning a dropped photo: {exception.GetType().Name}");
        }
    }

    /// <summary>
    /// Prepared as a message's photo is — re-drawn, EXIF left behind, a preview beside it — and refused when it
    /// is anything but a photo: a wall pins pictures. One at a time, and said so.
    /// </summary>
    private async Task PinAsync(StorageFile file, (double X, double Y) at)
    {
        var say = services.Say;
        if (preparingPhoto || pinning.Pinning)
        {
            ShowStatus(PhotoPinning.StillPinningText(say));
            return;
        }
        preparingPhoto = true;
        StatusText.Visibility = Visibility.Collapsed;
        PinPhotoButton.IsEnabled = false;
        PinPhotoButton.Content = say.Get("Pinning…");
        try
        {
            PrepOutcome prepared;
            try
            {
                prepared = await MediaPreparing.PrepareAsync(file);
            }
            catch (Exception e)
            {
                Diagnostics.Write($"preparing a photo to pin: {e.GetType().Name}");
                prepared = PrepOutcome.Refused(PrepFailure.Unreadable);
            }
            if (prepared.Media is not { } media)
            {
                ShowStatus($"{say.Get("Couldn't pin that photo.")} {ComposerStaging.Sentence(prepared.Failure ?? PrepFailure.Unreadable, say)}");
                return;
            }
            if (media.Kind != "photo")
            {
                ShowStatus($"{say.Get("Couldn't pin that photo.")} {PhotoPinning.PhotosOnlyText(say)}");
                return;
            }
            var outcome = await pinning.PinAsync(media, at, Notes.Colors[Random.Shared.Next(Notes.Colors.Length)]);
            if (outcome.Busy)
            {
                ShowStatus(PhotoPinning.StillPinningText(say));
            }
            else if (outcome.Error is { } error)
            {
                ShowStatus(PhotoPinning.FailureText(error, say));
            }
        }
        finally
        {
            preparingPhoto = false;
            PinPhotoButton.IsEnabled = true;
            PinPhotoButton.Content = say.Get("Pin a Photo");
            Draw();
        }
    }

    private void ShowStatus(string sentence)
    {
        StatusText.Text = sentence;
        StatusText.Visibility = Visibility.Visible;
    }

    /// <summary>One sheet at a time: an existing note, or a blank of <paramref name="kind"/>. The wall is drawn again when it closes.</summary>
    private async Task SheetAsync(long? noteId, NoteKind kind)
    {
        if (sheetOpen)
        {
            return;
        }
        var note = noteId is { } id ? connection.Board.Note(id) : null;
        if (noteId is not null && note is null)
        {
            return;
        }
        sheetOpen = true;
        try
        {
            await NoteSheet.ShowAsync(
                XamlRoot, services, connection, board, note, kind,
                mine: note is null || note.AuthorId == connection.Chats.Reader);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"a note sheet: {e.GetType().Name}");
        }
        finally
        {
            sheetOpen = false;
            Draw();
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
