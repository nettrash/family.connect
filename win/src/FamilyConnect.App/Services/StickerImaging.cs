using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;

namespace FamilyConnect.App.Services;

/// <summary>
/// A chat sticker decoded for drawing: its first frame always, and — where this machine's codec
/// gave more than one — every frame with how long each stands.
/// </summary>
/// <remarks>
/// <para>
/// <b>FRAMES ARE MEMORY THAT .NET CANNOT SEE.</b> Each is a <c>SoftwareBitmapSource</c>: a surface
/// outside the managed heap, which the collector has no reason to hurry for. So a picture that is
/// done with says so — <see cref="Release"/> when nothing draws it, <see cref="MakeStill"/> when
/// something does and the memory is wanted back — rather than waiting for a finalizer.
/// </para>
/// <para>
/// <b>FRAME ZERO IS NEVER DISPOSED BY <see cref="MakeStill"/>.</b> It is what an image goes on
/// showing, and one frame is what a still picture costs anyway.
/// </para>
/// </remarks>
/// <param name="first">Frame zero, or the whole picture when it does not move.</param>
/// <param name="frames">Every frame in order, or null for a still picture and for one drawn as its first frame.</param>
/// <param name="clock">Milliseconds per frame, the same length as <paramref name="frames"/>.</param>
/// <param name="bytes">What the frames hold between them, decoded; 0 for a still picture.</param>
internal sealed class StickerPicture(
    ImageSource first, IReadOnlyList<ImageSource>? frames = null, IReadOnlyList<int>? clock = null, long bytes = 0)
{
    public ImageSource First { get; } = first;

    public IReadOnlyList<ImageSource>? Frames { get; private set; } = frames;

    public IReadOnlyList<int>? Clock { get; private set; } = clock;

    /// <summary>What could be given back: every frame's bytes while it moves, nothing once it is still.</summary>
    public long Bytes { get; private set; } = frames is { Count: > 1 } ? Math.Max(bytes, 0) : 0;

    public bool Moves => Frames is { Count: > 1 } && Clock is { Count: > 1 };

    /// <summary>
    /// From here on this is its first frame and nothing else. <paramref name="dispose"/> gives the
    /// other frames back now — only when no image is still showing one of them.
    /// </summary>
    public void MakeStill(bool dispose)
    {
        var rest = Frames;
        Frames = null;
        Clock = null;
        Bytes = 0;
        if (dispose && rest is not null)
        {
            GiveBack(rest.Where(frame => !ReferenceEquals(frame, First)));
        }
    }

    /// <summary>Nothing draws this any more: every frame is given back, the first included.</summary>
    public void Release()
    {
        var all = Frames;
        Frames = null;
        Clock = null;
        Bytes = 0;
        if (all is not null)
        {
            GiveBack(all);
        }
    }

    /// <summary>Frames no image will draw again — a decode that stopped halfway has some and no picture to hold them.</summary>
    public static void GiveBack(IEnumerable<ImageSource> frames)
    {
        foreach (var frame in frames)
        {
            try
            {
                (frame as IDisposable)?.Dispose();
            }
            catch (Exception e)
            {
                // A surface that would not go is the collector's after all; the rest still go.
                Diagnostics.Write($"letting a sticker's frame go: {e.GetType().Name}");
            }
        }
    }
}

/// <summary>
/// The Windows half of a chat sticker: turning its bytes into something an <c>Image</c> draws, and
/// fitting a picture somebody chose into the pack's box. Decisions are App.Logic's
/// (<see cref="StickerAnimation"/>, <see cref="PackPicking"/>) — this only asks Windows Imaging.
/// </summary>
/// <remarks>
/// <para>
/// <b>ANIMATED WHERE THIS MACHINE CAN, FRAME ZERO WHERE IT CANNOT</b> (docs/protocol.md, "Animation
/// is stored, never refused and never stripped"). XAML's <c>BitmapImage</c> plays an animated GIF
/// by itself and is not documented to play anything else, so an animated WebP is asked of Windows
/// Imaging frame by frame and cycled by <c>StickerAnimator</c>. Whether the codec hands out more
/// than one frame is a fact about the machine: WebP is decoded by the "WebP Image Extension", which
/// ships with Windows 11 and can be removed, and PNG's decoder has never read APNG. Every one of
/// those outcomes lands on the same line — the first frame — and a sticker drawn still is the same
/// sticker.
/// </para>
/// <para>
/// <b>ONLY WHOLE FRAMES ARE CYCLED.</b> A frame that is not the size of the first is a codec
/// handing out the file's raw sub-rectangles, which would have to be composited with offsets and
/// blend rules this code would be guessing at. That is drawn as frame zero instead of drawn wrong.
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
                var moving = await FramesAsync(stream, bytes, box, rasterScale);
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
            var frames = new List<ImageSource>(count);
            // Whatever ends this early — a frame that is not whole, a frame that will not decode — the frames already
            // made are given back here: nobody else will ever hold them.
            var kept = false;
            try
            {
                return await ReadAsync();
            }
            finally
            {
                if (!kept)
                {
                    StickerPicture.GiveBack(frames);
                }
            }

            async Task<StickerPicture?> ReadAsync()
            {
                for (var at = 0; at < count; at++)
                {
                    var frame = await decoder.GetFrameAsync((uint)at);
                    if (frame.PixelWidth != decoder.PixelWidth || frame.PixelHeight != decoder.PixelHeight)
                    {
                        // Raw sub-rectangles, not whole pictures: not this client's to composite.
                        return null;
                    }
                    // Premultiplied BGRA is the one format a SoftwareBitmapSource takes — and it keeps the alpha,
                    // which is the point of a sticker.
                    var bitmap = await frame.GetSoftwareBitmapAsync(
                        BitmapPixelFormat.Bgra8,
                        BitmapAlphaMode.Premultiplied,
                        transform,
                        ExifOrientationMode.IgnoreExifOrientation,
                        ColorManagementMode.DoNotColorManage);
                    var source = new SoftwareBitmapSource();
                    // On the list BEFORE it is filled, so a fill that throws still leaves it where it is given back.
                    frames.Add(source);
                    await source.SetBitmapAsync(bitmap);
                }
                kept = true;
                return new StickerPicture(
                    frames[0], frames,
                    StickerAnimation.Clock(StickerFile.FrameDurations(bytes), count),
                    StickerAnimation.DecodedBytes(count, width, height));
            }
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
    public static bool AnimationsWanted()
    {
        try
        {
            return new Windows.UI.ViewManagement.UISettings().AnimationsEnabled;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading the animation setting: {e.GetType().Name}");
            return true;
        }
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
