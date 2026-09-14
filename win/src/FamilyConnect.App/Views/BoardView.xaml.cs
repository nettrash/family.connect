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
    private readonly AppServices services;
    private readonly Connection connection;
    private readonly BoardModel board;
    private readonly Action shown;
    private readonly Action<NoteDto> onNote;
    private readonly Action<long, bool> onBlock;
    private readonly Action onRoster;
    private readonly Action<Resync.Report> onResync;
    private readonly StickerFace face;
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
        face = new StickerFace(services, connection);
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
        Wall.Background = new SolidColorBrush(StickerFace.Hex(dark ? "#5b4a36" : "#cbb391"));
        EmptyTitle.Foreground = EmptyText.Foreground = new SolidColorBrush(dark ? Colors.White : StickerFace.Ink(0xD9));

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
            var holder = Holder(sticker, face.Build(sticker, me, revealed), hidden, wall);
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
            var compact = BoardWall.IsCompact(BoardModel.WallSize(Scroller.ActualWidth, Scroller.ActualHeight).Width);
            await NoteSheet.ShowAsync(
                XamlRoot, services, connection, board, face, compact, note, kind,
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

    /// <summary>Two soft washes over the cork: light from the top, a little shade at the bottom.</summary>
    private static LinearGradientBrush Sheen()
    {
        var sheen = new LinearGradientBrush { StartPoint = new Point(0.15, 0), EndPoint = new Point(0.85, 1) };
        sheen.GradientStops.Add(new GradientStop { Color = ColorHelper.FromArgb(0x42, 0xFF, 0xFF, 0xFF), Offset = 0 });
        sheen.GradientStops.Add(new GradientStop { Color = ColorHelper.FromArgb(0x00, 0xFF, 0xFF, 0xFF), Offset = 0.55 });
        sheen.GradientStops.Add(new GradientStop { Color = ColorHelper.FromArgb(0x24, 0x00, 0x00, 0x00), Offset = 1 });
        return sheen;
    }
}
