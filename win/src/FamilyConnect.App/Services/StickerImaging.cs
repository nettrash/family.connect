using System.Runtime.InteropServices.WindowsRuntime;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;

namespace FamilyConnect.App.Services;

/// <summary>
/// A chat sticker decoded for drawing: the one picture an <c>Image</c> shows, and — where it moves —
/// every frame's pixels with how long each stands.
/// </summary>
/// <remarks>
/// <para>
/// <b>ONE SURFACE, REDRAWN — NEVER A SOURCE PER FRAME.</b> A moving sticker is one
/// <c>WriteableBitmap</c> that every image of it shows; a frame that is due is copied into it and
/// it is invalidated. Until 2026-10-21 each frame was its own <c>SoftwareBitmapSource</c> and the
/// image's <c>Source</c> was swapped to it, and WinUI draws an image EMPTY for a composition frame
/// whenever its source changes: the sticker blinked on every frame (a third of screenshots taken
/// at 60 a second were blank).
/// </para>
/// <para>
/// <b>AND SO NOTHING HERE IS DISPOSED UNDER AN IMAGE.</b> The frames are managed bytes and the
/// surface is never let go while the picture is held, so <see cref="MakeStill"/> and
/// <see cref="Release"/> drop bytes the collector takes: an image still showing the surface goes on
/// showing whichever frame was last drawn into it.
/// </para>
/// </remarks>
internal sealed class StickerPicture
{
    private readonly WriteableBitmap? canvas;
    private int drawn;

    /// <summary>A still picture.</summary>
    public StickerPicture(ImageSource still) => First = still;

    private StickerPicture(WriteableBitmap canvas, IReadOnlyList<byte[]> frames, IReadOnlyList<int> clock, long bytes)
    {
        this.canvas = canvas;
        First = canvas;
        Frames = frames;
        Clock = clock;
        Bytes = Math.Max(bytes, 0);
    }

    /// <summary>
    /// A moving picture of <paramref name="frames"/> — premultiplied BGRA, <paramref name="width"/> ×
    /// <paramref name="height"/> each — on one surface showing frame zero. The window's thread only.
    /// </summary>
    /// <param name="clock">Milliseconds per frame, the same length as <paramref name="frames"/>.</param>
    /// <param name="bytes">What the frames hold between them, decoded.</param>
    public static StickerPicture Moving(IReadOnlyList<byte[]> frames, int width, int height, IReadOnlyList<int> clock, long bytes)
    {
        var canvas = new WriteableBitmap(width, height);
        frames[0].CopyTo(canvas.PixelBuffer);
        canvas.Invalidate();
        return new StickerPicture(canvas, frames, clock, bytes);
    }

    /// <summary>What an image draws: the surface of a moving picture, or the still one.</summary>
    public ImageSource First { get; }

    /// <summary>Every frame's pixels in order, or null for a still picture and one made still.</summary>
    public IReadOnlyList<byte[]>? Frames { get; private set; }

    public IReadOnlyList<int>? Clock { get; private set; }

    /// <summary>What could be given back: every frame's bytes while it moves, nothing once it is still.</summary>
    public long Bytes { get; private set; }

    public bool Moves => Frames is { Count: > 1 } && Clock is { Count: > 1 };

    /// <summary>Frame <paramref name="index"/> into the surface every image of this picture shows — unless it is there already.</summary>
    public void Draw(int index)
    {
        if (canvas is null || Frames is not { } frames || index == drawn || index < 0 || index >= frames.Count)
        {
            return;
        }
        frames[index].CopyTo(canvas.PixelBuffer);
        canvas.Invalidate();
        drawn = index;
    }

    /// <summary>From here on this is its first frame and nothing else: frame zero is drawn, and the other frames' bytes go.</summary>
    public void MakeStill()
    {
        Draw(0);
        Release();
    }

    /// <summary>Nothing draws this any more, or nothing will move it: the frames' bytes go.</summary>
    public void Release()
    {
        Frames = null;
        Clock = null;
        Bytes = 0;
    }
}

/// <summary>
/// The Windows half of a chat sticker: turning its bytes into something an <c>Image</c> draws, and
/// fitting a picture somebody chose into the pack's box. Decisions are App.Logic's
/// (<see cref="StickerAnimation"/>, <see cref="PackPicking"/>) — this only asks Windows Imaging.
/// </summary>
/// <remarks>
/// <para>
/// <b>ANIMATED BY THIS CLIENT'S OWN FRAMES</b> (docs/protocol.md, "Animation is stored, never refused
/// and never stripped"). An animated WebP's or APNG's frames are walked from its own bytes and
/// composited by App.Logic (<see cref="StickerFrames"/>, the compositor #96 makes videos with); Windows
/// Imaging only decodes one standalone still at a time, which it does for both formats. Until
/// 2026-10-20 Windows Imaging was asked for the frames themselves — and PNG's decoder has never read
/// APNG, and the "WebP Image Extension" hands out one frame or the file's raw sub-rectangles — so on
/// Windows nearly every animated sticker was drawn as its first frame.
/// </para>
/// <para>
/// <b>WINDOWS IMAGING'S FRAMES ARE THE FALLBACK</b>, for a file whose frames cannot be walked: there
/// only whole frames are cycled. A frame that is not the size of the first is a codec handing out
/// raw sub-rectangles, drawn as frame zero instead of drawn wrong.
/// </para>
/// <para>
/// <b>EVERY CALL HERE IS GUARDED.</b> A codec that is missing, a file that lies and a stream that
/// fails all come back as null, which the caller draws as a labelled box: a WinRT exception that
/// escaped would be a whole conversation that stopped drawing.
/// </para>
/// </remarks>
internal static class StickerImaging
{
    /// <summary>
    /// Decode a sticker for a box of <paramref name="box"/> effective pixels. Null when nothing on
    /// this machine reads it.
    /// </summary>
    /// <param name="animate">Whether to look for frames at all — a panel of two hundred cells does not.</param>
    public static async Task<StickerPicture?> DecodeAsync(byte[] bytes, double box, double rasterScale, bool animate)
    {
        try
        {
            using var stream = await StreamAsync(bytes);
            if (animate && StickerFile.IsAnimated(bytes))
            {
                // This client's own frames first; Windows Imaging's only where the file's own frames cannot be walked.
                var moving = await OwnFramesAsync(bytes, box, rasterScale)
                    ?? await FramesAsync(stream, bytes, box, rasterScale);
                if (moving is not null)
                {
                    return moving;
                }
                stream.Seek(0);
            }
            // Decoded near the size it is drawn at, by height alone so the shape is the picture's own: a 2000-pixel
            // PNG in a 72-point cell is otherwise sixteen megabytes of pixels nobody sees.
            var still = new BitmapImage
            {
                DecodePixelType = DecodePixelType.Logical,
                DecodePixelHeight = (int)Math.Max(1, Math.Round(box)),
            };
            await still.SetSourceAsync(stream);
            return new StickerPicture(still);
        }
        catch (Exception e)
        {
            // No codec for it here (WebP without its extension), or bytes that are not a picture.
            Diagnostics.Write($"decoding a sticker: {e.GetType().Name}");
            return null;
        }
    }

    /// <summary>
    /// Every frame, walked and composited by this client (<see cref="StickerFrames"/>), or null when the file's frames cannot
    /// be had that way — or are not worth animating — and Windows Imaging is asked instead.
    /// </summary>
    /// <remarks>
    /// The fix for animated stickers drawn still on Windows: an APNG's frames do not exist for PNG's decoder, and an
    /// animated WebP's come out of the WebP extension as one frame or as raw rectangles, so <see cref="FramesAsync"/> alone
    /// drew nearly every animated sticker as its first frame. The work on the bytes runs off the window's thread; only the
    /// surface is made on it, as it must be.
    /// </remarks>
    private static async Task<StickerPicture?> OwnFramesAsync(byte[] bytes, double box, double rasterScale)
    {
        try
        {
            if (AnimationPrep.Layout(bytes) is not { } layout)
            {
                return null;
            }
            var (width, height) = StickerAnimation.DecodeSize(layout.Width, layout.Height, box, rasterScale);
            if (!StickerAnimation.Worth(layout.Frames.Count, width, height))
            {
                return null;
            }
            var drawn = await Task.Run(async () =>
                await AnimationPreparing.SourceAsync(bytes, layout, CancellationToken.None) is { } source
                    && source.Layout.Frames.Count == layout.Frames.Count
                    ? await StickerFrames.ComposeAsync(source.Layout, source.Pixels, width, height)
                    : null);
            if (drawn is null)
            {
                Diagnostics.Write("an animated sticker's frames could not be walked; asking Windows Imaging for them");
                return null;
            }
            return StickerPicture.Moving(
                drawn, width, height,
                // The durations the walk read, frame for frame — the same file's word the compositor drew by.
                StickerAnimation.Clock([.. layout.StatedMs.Select(ms => (int)Math.Min(ms, int.MaxValue))], drawn.Count),
                StickerAnimation.DecodedBytes(drawn.Count, width, height));
        }
        catch (Exception e)
        {
            Diagnostics.Write($"composing a sticker's frames: {e.GetType().Name}");
            return null;
        }
    }

    /// <summary>Every frame, or null when this is to be drawn as its first frame.</summary>
    private static async Task<StickerPicture?> FramesAsync(
        IRandomAccessStream stream, byte[] bytes, double box, double rasterScale)
    {
        try
        {
            var decoder = await BitmapDecoder.CreateAsync(stream);
            var count = (int)Math.Min(decoder.FrameCount, int.MaxValue);
            var (width, height) = StickerAnimation.DecodeSize(
                (int)Math.Min(decoder.PixelWidth, int.MaxValue),
                (int)Math.Min(decoder.PixelHeight, int.MaxValue),
                box, rasterScale);
            if (!StickerAnimation.Worth(count, width, height))
            {
                return null;
            }
            var transform = new BitmapTransform
            {
                ScaledWidth = (uint)width,
                ScaledHeight = (uint)height,
                InterpolationMode = BitmapInterpolationMode.Fant,
            };
            var frames = new List<byte[]>(count);
            for (var at = 0; at < count; at++)
            {
                var frame = await decoder.GetFrameAsync((uint)at);
                if (frame.PixelWidth != decoder.PixelWidth || frame.PixelHeight != decoder.PixelHeight)
                {
                    // Raw sub-rectangles, not whole pictures: not this client's to composite.
                    return null;
                }
                // Premultiplied BGRA is what a WriteableBitmap holds — and it keeps the alpha, which is the point of a sticker.
                var pixels = await frame.GetPixelDataAsync(
                    BitmapPixelFormat.Bgra8,
                    BitmapAlphaMode.Premultiplied,
                    transform,
                    ExifOrientationMode.IgnoreExifOrientation,
                    ColorManagementMode.DoNotColorManage);
                var data = pixels.DetachPixelData();
                if (data.LongLength != 4L * width * height)
                {
                    return null;
                }
                frames.Add(data);
            }
            return StickerPicture.Moving(
                frames, width, height,
                StickerAnimation.Clock(StickerFile.FrameDurations(bytes), count),
                StickerAnimation.DecodedBytes(count, width, height));
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading a sticker's frames: {e.GetType().Name}");
            return null;
        }
    }

    /// <summary>What making a sticker of a chosen picture came to.</summary>
    /// <param name="Bytes">The PNG, when one was made.</param>
    /// <param name="Animated">The decoder found a MOVING picture: refused in words, never flattened to its first frame.</param>
    public readonly record struct Made(byte[]? Bytes, bool Animated = false);

    /// <summary>
    /// A still picture — any this machine decodes: a JPEG, a HEIC, a still GIF, a WebP or PNG too
    /// big to go as it is — fitted whole into the pack's 512 × 512 box and written as PNG:
    /// proportions and transparency kept, never cropped, never scaled up, never drawn on white.
    /// Nothing at all when it cannot be read.
    /// </summary>
    /// <remarks>
    /// <para>
    /// NOT the photo path, and deliberately nothing shared with it: <see cref="MediaPreparing"/>
    /// draws a photograph on white and writes a JPEG, which is exactly what a sticker must never
    /// go through. Windows Imaging writes no WebP, so PNG is what this client makes.
    /// </para>
    /// <para>
    /// A photograph is turned the way its camera held it (EXIF), which a JPEG or a HEIC needs and
    /// a PNG never has; the size written is the size of the pixels that came back, whichever way
    /// round that is.
    /// </para>
    /// <para>
    /// A GIF WITH MORE THAN ONE FRAME IS ANSWERED AS ANIMATED, not made: App.Logic reads that from
    /// the bytes before this is ever called, and this is the decoder's own word for a file whose
    /// blocks that walk could not follow.
    /// </para>
    /// </remarks>
    public static async Task<Made> MakeAsync(byte[] bytes)
    {
        try
        {
            using var stream = await StreamAsync(bytes);
            var decoder = await BitmapDecoder.CreateAsync(stream);
            if (decoder.FrameCount > 1 && decoder.DecoderInformation?.CodecId == BitmapDecoder.GifDecoderId)
            {
                return new Made(null, Animated: true);
            }
            // The scale is asked for in the file's own orientation; the turn is applied after it.
            var (width, height) = PackPicking.Fit(
                (int)Math.Min(decoder.PixelWidth, int.MaxValue), (int)Math.Min(decoder.PixelHeight, int.MaxValue));
            var pixels = await decoder.GetPixelDataAsync(
                BitmapPixelFormat.Bgra8,
                // STRAIGHT alpha in and straight alpha out: the edge of a sticker is where a premultiplied round trip
                // would leave a dark fringe.
                BitmapAlphaMode.Straight,
                new BitmapTransform
                {
                    ScaledWidth = (uint)width,
                    ScaledHeight = (uint)height,
                    InterpolationMode = BitmapInterpolationMode.Fant,
                },
                ExifOrientationMode.RespectExifOrientation,
                ColorManagementMode.ColorManageToSRgb);
            var data = pixels.DetachPixelData();
            if (data.LongLength != 4L * width * height)
            {
                // Pixels that are not the size asked for are not something to guess a shape for.
                Diagnostics.Write("fitting a sticker into the pack's box: the pixels are not the size asked for");
                return default;
            }
            // Turned a quarter, the picture that came back is as tall as it was asked to be wide.
            if (decoder.OrientedPixelWidth == decoder.PixelHeight
                && decoder.OrientedPixelHeight == decoder.PixelWidth
                && decoder.PixelWidth != decoder.PixelHeight)
            {
                (width, height) = (height, width);
            }
            using var output = new InMemoryRandomAccessStream();
            var encoder = await BitmapEncoder.CreateAsync(BitmapEncoder.PngEncoderId, output);
            encoder.SetPixelData(
                BitmapPixelFormat.Bgra8, BitmapAlphaMode.Straight,
                (uint)width, (uint)height, 96, 96, data);
            await encoder.FlushAsync();
            var written = new byte[output.Size];
            using var reader = new DataReader(output.GetInputStreamAt(0));
            await reader.LoadAsync((uint)output.Size);
            reader.ReadBytes(written);
            return new Made(written);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"fitting a sticker into the pack's box: {e.GetType().Name}");
            return default;
        }
    }

    /// <summary>
    /// Whether Windows is set to show animations at all. Somebody who switched them off has asked
    /// for a still screen, and gets frame zero — which is a correct drawing of a sticker.
    /// </summary>
    /// <remarks>
    /// <b>ASKED OF ONE KEPT <c>UISettings</c>, NOT A NEW ONE PER CALL.</b> This is asked by every sticker decode, every
    /// animation tile's viewport change and every recording's frame, and each new <c>UISettings</c> is a WinRT object made
    /// and thrown away on the window's thread. The answer is kept for a second at most — and dropped at once when Windows
    /// says the setting changed — so switching animations off still reaches the next thing drawn.
    /// </remarks>
    public static bool AnimationsWanted()
    {
        var now = Environment.TickCount64;
        var answer = Volatile.Read(ref animationsAnswer);
        if (answer != 0 && now - Volatile.Read(ref animationsAskedAt) < AnimationsKeptMs)
        {
            return answer == 1;
        }
        try
        {
            var settings = animationSettings ??= Watched(new Windows.UI.ViewManagement.UISettings());
            var on = settings.AnimationsEnabled;
            Volatile.Write(ref animationsAskedAt, now);
            Volatile.Write(ref animationsAnswer, on ? 1 : 2);
            return on;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading the animation setting: {e.GetType().Name}");
            animationSettings = null;
            Volatile.Write(ref animationsAnswer, 0);
            return true;
        }
    }

    /// <summary>How long one reading of the animation setting is trusted, in milliseconds.</summary>
    private const long AnimationsKeptMs = 1000;

    /// <summary>The kept settings object: an event source nobody holds is collected and tells nobody.</summary>
    private static Windows.UI.ViewManagement.UISettings? animationSettings;

    /// <summary>0 while nothing is kept, 1 for animations on, 2 for off.</summary>
    private static int animationsAnswer;
    private static long animationsAskedAt;

    /// <summary>The setting changed (on whatever thread Windows says so): the kept answer goes, and the next call reads it.</summary>
    private static Windows.UI.ViewManagement.UISettings Watched(Windows.UI.ViewManagement.UISettings settings)
    {
        try
        {
            settings.AnimationsEnabledChanged += (_, _) => Volatile.Write(ref animationsAnswer, 0);
        }
        catch (Exception e)
        {
            // Without the event the answer is still only a second old.
            Diagnostics.Write($"watching the animation setting: {e.GetType().Name}");
        }
        return settings;
    }

    private static async Task<InMemoryRandomAccessStream> StreamAsync(byte[] bytes)
    {
        var stream = new InMemoryRandomAccessStream();
        try
        {
            using (var writer = new DataWriter(stream))
            {
                writer.WriteBytes(bytes);
                await writer.StoreAsync();
                await writer.FlushAsync();
                writer.DetachStream();
            }
            stream.Seek(0);
            return stream;
        }
        catch
        {
            stream.Dispose();
            throw;
        }
    }
}
