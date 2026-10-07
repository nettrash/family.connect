using System.Runtime.InteropServices.WindowsRuntime;
using FamilyConnect.App.Logic;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Graphics.DirectX;
using Windows.Graphics.Imaging;
using Windows.Media;
using Windows.Media.Core;
using Windows.Media.Playback;
using Windows.Storage.Streams;

namespace FamilyConnect.App.Services;

/// <summary>
/// One video message playing IN PLACE (docs/audio-video-messages-2026-10-04.md, S5.3): a <see cref="MediaPlayer"/> in
/// FRAME-SERVER mode — it renders nowhere by itself and plays its sound as any player does — whose frames are copied into a
/// <see cref="WriteableBitmap"/> that the circle's own <c>Ellipse</c> fills with, the same ellipse and the same brush that
/// draw the poster. The picture is round because the shape is, never because a control was trusted to clip a video:
/// <c>MediaPlayerElement</c>'s <c>CornerRadius</c> does not clip its video (microsoft-ui-xaml #8264).
/// </summary>
/// <remarks>
/// <para>
/// <b>THE COPY</b> (Microsoft Learn, "Use MediaPlayer in frame server mode"): on <see cref="MediaPlayer.VideoFrameAvailable"/>
/// — raised on a media thread, once per decoded frame — the frame is copied with
/// <see cref="MediaPlayer.CopyFrameToVideoSurface(Windows.Graphics.DirectX.Direct3D11.IDirect3DSurface)"/> into a Direct3D 11
/// surface the size of the circle on this screen (<see cref="VideoFrame.CreateAsDirect3D11SurfaceBacked(DirectXPixelFormat, int, int)"/>,
/// so no Win2D <c>CanvasDevice</c> is needed; the player scales into it), read back with
/// <see cref="SoftwareBitmap.CreateCopyFromSurfaceAsync(Windows.Graphics.DirectX.Direct3D11.IDirect3DSurface, BitmapAlphaMode)"/>
/// as premultiplied BGRA, made opaque (<see cref="RoundFrames.Opaque"/>) and written SYNCHRONOUSLY into the bitmap's pixels,
/// then <see cref="WriteableBitmap.Invalidate"/>. Everything but the throttle runs on the UI thread, as the documented sample
/// does.
/// </para>
/// <para>
/// <b>NOT A SoftwareBitmapSource</b> (2026-10-06): the first build fed one with <c>SetBitmapAsync</c>, frame after frame, under
/// the ellipse's brush, and on the owner's ARM64 machine a click on a circle ended the app inside Microsoft.UI.Xaml.dll
/// (0xc000027b) with no managed exception at all. A WriteableBitmap is the long-proven <c>ImageBrush</c> source and is
/// written with no asynchronous hand-off, and nothing of it is closed under the brush.
/// </para>
/// <para>
/// <b>THE FIRST FRAME IS LOGGED STEP BY STEP</b> ("round:" lines in diagnostics.log), so a native failure that no handler sees
/// still leaves the step it died in.
/// </para>
/// <para>
/// <b>THROTTLED</b> (<see cref="RoundFrames"/>): at the clip's own rate and never over 30, and never a second copy while one
/// is under way — a slow machine drops frames, it never queues them.
/// </para>
/// <para>
/// <b>A MACHINE WHERE IT CANNOT WORK</b> says so with <see cref="FramesFailed"/> after <see cref="RoundFrames.FailuresBeforeViewer"/>
/// failed copies in a row; the view then opens the viewer instead, which plays it in its <c>MediaPlayerElement</c> as before.
/// </para>
/// <para>
/// Every event is raised on the UI thread, and never after <see cref="Dispose"/>.
/// </para>
/// </remarks>
internal sealed class RoundFramePlayer : IDisposable
{
    private readonly DispatcherQueue ui;
    private readonly MediaPlayer player;
    private readonly int side;
    private readonly WriteableBitmap picture;
    private byte[]? pixels;
    private IRandomAccessStream? stream;
    private MediaSource? source;
    private VideoFrame? surface;

    // Read and written from the media thread as well as the UI thread.
    private int copying;
    private long lastCopy = -1;
    private long intervalTicks = BitConverter.DoubleToInt64Bits(RoundFrames.IntervalMs(null));
    private volatile bool closed;

    // The UI thread's own.
    private int failures;
    private bool framed;

    /// <param name="ui">The window's queue, where every event is raised.</param>
    /// <param name="side">The frame surface's side in physical pixels (<see cref="RoundFrames.Side"/>).</param>
    public RoundFramePlayer(DispatcherQueue ui, int side)
    {
        this.ui = ui;
        this.side = side;
        // Made here, on the UI thread, the size of the frame surface — so a frame's pixels always fill it exactly.
        picture = new WriteableBitmap(side, side);
        player = new MediaPlayer
        {
            AutoPlay = false,
            IsVideoFrameServerEnabled = true,
        };
        // A circle in a chat is not the system's "now playing": no media overlay, and no key outside the app starting it
        // under a recording (S1.7).
        player.CommandManager.IsEnabled = false;
        player.MediaOpened += OnOpened;
        player.MediaEnded += OnEnded;
        player.MediaFailed += OnFailed;
        player.PlaybackSession.PlaybackStateChanged += OnState;
        player.VideoFrameAvailable += OnFrame;
    }

    /// <summary>The frames, for the circle's brush.</summary>
    public ImageSource Picture => picture;

    /// <summary>The player has opened the clip; how long it is, in seconds (0 when it does not say).</summary>
    public event Action<double>? Opened;

    /// <summary>The first frame is in <see cref="Picture"/>.</summary>
    public event Action? FirstFrame;

    /// <summary>The player's state changed: playing, paused, or waiting for bytes.</summary>
    public event Action<MediaPlaybackState>? StateChanged;

    /// <summary>It played to its end.</summary>
    public event Action? Ended;

    /// <summary>It cannot be played.</summary>
    public event Action? Failed;

    /// <summary>The player plays but its frames cannot be copied on this machine.</summary>
    public event Action? FramesFailed;

    /// <summary>Where it is, in seconds.</summary>
    public double Position => Safe(() => player.PlaybackSession.Position.TotalSeconds);

    /// <summary>How long it is, in seconds; 0 while unknown.</summary>
    public double Duration => Safe(() => player.PlaybackSession.NaturalDuration.TotalSeconds);

    /// <summary>Open the clip — the whole MP4, fetched through the attachment cache with the session's header (S5.3).</summary>
    public void Open(byte[] bytes, string mime)
    {
        Diagnostics.Write($"round: opening {bytes.Length / 1024} KB, {side} px frames");
        var memory = new InMemoryRandomAccessStream();
        stream = memory;
        _ = FillAsync(memory, bytes, mime);
    }

    private async Task FillAsync(InMemoryRandomAccessStream memory, byte[] bytes, string mime)
    {
        try
        {
            using (var writer = new DataWriter(memory))
            {
                writer.WriteBytes(bytes);
                await writer.StoreAsync();
                await writer.FlushAsync();
                writer.DetachStream();
            }
            memory.Seek(0);
            if (closed)
            {
                return;
            }
            source = MediaSource.CreateFromStream(memory, mime);
            player.Source = new MediaPlaybackItem(source);
            Diagnostics.Write("round: handed to the player");
        }
        catch (Exception e)
        {
            Diagnostics.Write($"opening a video message: {e.GetType().Name}");
            if (!closed)
            {
                Failed?.Invoke();
            }
        }
    }

    public void Play() => Do(player.Play, "playing");

    public void Pause() => Do(player.Pause, "pausing");

    private void OnOpened(MediaPlayer sender, object args)
    {
        // The clip's own frame rate, which the copies keep to (and never more than 30).
        double? rate = null;
        try
        {
            if (sender.Source is MediaPlaybackItem { VideoTracks.Count: > 0 } item)
            {
                var ratio = item.VideoTracks[0].GetEncodingProperties().FrameRate;
                rate = RoundFrames.Rate(ratio.Numerator, ratio.Denominator);
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading a video message's frame rate: {e.GetType().Name}");
        }
        Interlocked.Exchange(ref intervalTicks, BitConverter.DoubleToInt64Bits(RoundFrames.IntervalMs(rate)));
        var total = Duration;
        Diagnostics.Write($"round: opened, {total:0.0} s at {rate?.ToString("0.##", System.Globalization.CultureInfo.InvariantCulture) ?? "an unsaid"} fps");
        Raise(() => Opened?.Invoke(total));
    }

    private void OnEnded(MediaPlayer sender, object args) => Raise(() => Ended?.Invoke());

    private void OnFailed(MediaPlayer sender, MediaPlayerFailedEventArgs args)
    {
        Diagnostics.Write($"a video message failed: {args.Error}");
        Raise(() => Failed?.Invoke());
    }

    private void OnState(MediaPlaybackSession session, object args)
    {
        MediaPlaybackState state;
        try
        {
            state = session.PlaybackState;
        }
        catch (Exception)
        {
            return;
        }
        Raise(() => StateChanged?.Invoke(state));
    }

    /// <summary>A frame is ready (on a media thread): copied if it is due and nothing is being copied already.</summary>
    private void OnFrame(MediaPlayer sender, object args)
    {
        if (closed)
        {
            return;
        }
        var now = Environment.TickCount64;
        var last = Interlocked.Read(ref lastCopy);
        var interval = BitConverter.Int64BitsToDouble(Interlocked.Read(ref intervalTicks));
        if (!RoundFrames.Due(now, last < 0 ? null : last, interval, Volatile.Read(ref copying) != 0)
            || Interlocked.CompareExchange(ref copying, 1, 0) != 0)
        {
            return;
        }
        Interlocked.Exchange(ref lastCopy, now);
        if (!ui.TryEnqueue(() => _ = CopyFrameAsync()))
        {
            Volatile.Write(ref copying, 0);
        }
    }

    private async Task CopyFrameAsync()
    {
        try
        {
            if (closed)
            {
                return;
            }
            Step("making the frame surface");
            surface ??= VideoFrame.CreateAsDirect3D11SurfaceBacked(DirectXPixelFormat.B8G8R8A8UIntNormalized, side, side);
            Step("copying a frame into it");
            player.CopyFrameToVideoSurface(surface.Direct3DSurface);
            Step("reading it back");
            var copied = await SoftwareBitmap.CreateCopyFromSurfaceAsync(surface.Direct3DSurface, BitmapAlphaMode.Premultiplied);
            try
            {
                if (closed)
                {
                    return;
                }
                Step($"a {copied.PixelWidth}×{copied.PixelHeight} {copied.BitmapPixelFormat} {copied.BitmapAlphaMode} frame");
                Draw(copied);
            }
            finally
            {
                copied.Dispose();
            }
            failures = 0;
            if (!framed && !closed)
            {
                framed = true;
                Diagnostics.Write("round: first frame drawn");
                FirstFrame?.Invoke();
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"copying a video message's frame: {e.GetType().Name} {e.HResult:X8}");
            if (++failures >= RoundFrames.FailuresBeforeViewer && !closed)
            {
                FramesFailed?.Invoke();
            }
        }
        finally
        {
            Volatile.Write(ref copying, 0);
            if (closed)
            {
                LetGoOfSurface();
            }
        }
    }

    /// <summary>
    /// One frame into the bitmap: converted to premultiplied BGRA if the read-back came as anything else, checked against the
    /// bitmap's size (the surface was made that size), made opaque, written, and the bitmap told to redraw.
    /// </summary>
    private void Draw(SoftwareBitmap copied)
    {
        SoftwareBitmap frame = copied;
        if (copied.BitmapPixelFormat != BitmapPixelFormat.Bgra8 || copied.BitmapAlphaMode != BitmapAlphaMode.Premultiplied)
        {
            frame = SoftwareBitmap.Convert(copied, BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied);
        }
        try
        {
            if (frame.PixelWidth != picture.PixelWidth || frame.PixelHeight != picture.PixelHeight)
            {
                throw new InvalidOperationException("the frame is not the bitmap's size");
            }
            var length = picture.PixelWidth * picture.PixelHeight * 4;
            if (pixels is null || pixels.Length != length)
            {
                pixels = new byte[length];
            }
            frame.CopyToBuffer(pixels.AsBuffer());
            RoundFrames.Opaque(pixels);
            Step("writing it into the circle");
            using (var target = picture.PixelBuffer.AsStream())
            {
                target.Write(pixels, 0, pixels.Length);
            }
            picture.Invalidate();
        }
        finally
        {
            if (!ReferenceEquals(frame, copied))
            {
                frame.Dispose();
            }
        }
    }

    /// <summary>A breadcrumb for the FIRST frame only — the steps a native failure could end the app in.</summary>
    private void Step(string what)
    {
        if (!framed)
        {
            Diagnostics.Write($"round: {what}");
        }
    }

    private void Raise(Action act)
    {
        if (closed)
        {
            return;
        }
        ui.TryEnqueue(() =>
        {
            if (!closed)
            {
                act();
            }
        });
    }

    private void Do(Action act, string what)
    {
        if (closed)
        {
            return;
        }
        try
        {
            act();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"{what} a video message: {e.GetType().Name}");
        }
    }

    private static double Safe(Func<double> read)
    {
        try
        {
            var value = read();
            return double.IsFinite(value) && value > 0 ? value : 0;
        }
        catch (Exception)
        {
            return 0;
        }
    }

    private void LetGoOfSurface()
    {
        surface?.Dispose();
        surface = null;
    }

    /// <summary>
    /// Let it all go — the player, its clip and the frame surface. Called on the UI thread, AFTER every circle has been given its
    /// poster back; <see cref="Picture"/> is left to the collector, never closed under a brush.
    /// </summary>
    public void Dispose()
    {
        if (closed)
        {
            return;
        }
        closed = true;
        try
        {
            player.VideoFrameAvailable -= OnFrame;
            player.MediaOpened -= OnOpened;
            player.MediaEnded -= OnEnded;
            player.MediaFailed -= OnFailed;
            player.PlaybackSession.PlaybackStateChanged -= OnState;
            player.Pause();
            player.Source = null;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"stopping a video message: {e.GetType().Name}");
        }
        try
        {
            source?.Dispose();
            stream?.Dispose();
            player.Dispose();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"letting go of a video message: {e.GetType().Name}");
        }
        source = null;
        stream = null;
        // A copy under way lets go of the surface itself when it finishes.
        if (Volatile.Read(ref copying) == 0)
        {
            LetGoOfSurface();
        }
    }
}
