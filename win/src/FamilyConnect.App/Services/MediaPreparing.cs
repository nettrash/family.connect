using System.Runtime.InteropServices.WindowsRuntime;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using Windows.Graphics.Imaging;
using Windows.Media.Editing;
using Windows.Storage;
using Windows.Storage.FileProperties;
using Windows.Storage.Streams;

namespace FamilyConnect.App.Services;

/// <summary>
/// A picked or dropped file made ready to send — the Windows half of <see cref="MediaPrep"/>, which
/// decides the route; this only does what needs the platform: decoding, drawing, reading a frame.
/// </summary>
/// <remarks>
/// <para>
/// <b>A PHOTO IS RE-DRAWN</b>, at most 2048 on its longest side, as JPEG — which is also what leaves its
/// EXIF behind — with its preview beside it. Transparency is drawn on white: a JPEG has no alpha, and
/// a transparent pixel would otherwise come out black.
/// </para>
/// <para>
/// <b>A VIDEO GOES AS IT IS.</b> Everything read of it — its size, its length, its poster — is what
/// Windows can read; a codec it cannot play still goes, because the server checks the container and
/// not the codec, just without those facts.
/// </para>
/// <para>
/// The previews are made here and never by the server (docs/protocol.md, "Previews").
/// </para>
/// </remarks>
internal static class MediaPreparing
{
    private static readonly PrepOutcome TooLarge = PrepOutcome.Refused(PrepFailure.TooLarge);

    public static async Task<PrepOutcome> PrepareAsync(StorageFile file)
    {
        var name = file.Name;
        var size = (long)(await file.GetBasicPropertiesAsync()).Size;
        var routed = MediaPrep.Route(file.ContentType ?? string.Empty, name, await HeadAsync(file));
        if (routed.Route != MediaRoute.Photo && size > MediaPrep.SizeLimit)
        {
            return TooLarge;
        }
        return routed.Route switch
        {
            MediaRoute.Photo => await PhotoAsync(file),
            MediaRoute.Video => await VideoAsync(file),
            MediaRoute.Audio => await AudioAsync(file, routed.AudioMime ?? "audio/mp4"),
            _ => PrepOutcome.Staged(new StagedMedia(
                "file",
                MediaPrep.DeclaredType(file.ContentType ?? string.Empty, name),
                await BytesAsync(file),
                Name: MediaPrep.SanitizedName(name) ?? "file")),
        };
    }

    private static async Task<PrepOutcome> PhotoAsync(StorageFile file)
    {
        try
        {
            using var stream = await file.OpenReadAsync();
            var decoder = await BitmapDecoder.CreateAsync(stream);
            var photo = await JpegAsync(decoder, MediaPrep.PhotoEdge, MediaPrep.PhotoQuality);
            // Far under any ceiling once downscaled — but a pathological one is better refused here
            // than by the server.
            if (photo.Bytes.LongLength > MediaPrep.SizeLimit)
            {
                return TooLarge;
            }
            ReadOnlyMemory<byte>? preview = null;
            try
            {
                preview = (await JpegAsync(decoder, MediaPrep.PreviewEdge, MediaPrep.PreviewQuality)).Bytes;
            }
            catch (Exception e)
            {
                // A photo may go without its preview; a recipient then draws the photo itself.
                Diagnostics.Write($"drawing a photo's preview: {e.GetType().Name}");
            }
            return PrepOutcome.Staged(new StagedMedia(
                "photo", "image/jpeg", photo.Bytes, photo.Width, photo.Height, Preview: preview));
        }
        catch (Exception e)
        {
            Diagnostics.Write($"preparing a photo: {e.GetType().Name}");
            return PrepOutcome.Refused(PrepFailure.Unreadable);
        }
    }

    private static async Task<PrepOutcome> VideoAsync(StorageFile file)
    {
        var declared = MediaPrep.DeclaredType(file.ContentType ?? string.Empty, file.Name);
        var media = new StagedMedia(
            "video", declared == "video/quicktime" ? declared : "video/mp4", await BytesAsync(file));
        try
        {
            var video = await file.Properties.GetVideoPropertiesAsync();
            // A phone holds a video on its side and says so: the size a bubble draws is the turned one.
            var (width, height) = video.Orientation is VideoOrientation.Rotate90 or VideoOrientation.Rotate270
                ? (video.Height, video.Width)
                : (video.Width, video.Height);
            media = media with
            {
                Width = width > 0 ? (int)width : null,
                Height = height > 0 ? (int)height : null,
                DurationMs = Milliseconds(video.Duration),
                Preview = await PosterAsync(file, video.Duration, width, height),
            };
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading a video: {e.GetType().Name}");
        }
        return PrepOutcome.Staged(media);
    }

    /// <summary>
    /// A frame worth drawing: past a fade-in first, then the very start, then a little later — the
    /// seek points every client uses.
    /// </summary>
    private static async Task<ReadOnlyMemory<byte>?> PosterAsync(StorageFile file, TimeSpan duration, uint width, uint height)
    {
        var clip = await MediaClip.CreateFromFileAsync(file);
        if (width == 0 || height == 0)
        {
            var encoding = clip.GetVideoEncodingProperties();
            (width, height) = (encoding.Width, encoding.Height);
        }
        var composition = new MediaComposition();
        composition.Clips.Add(clip);
        var (frameWidth, frameHeight) = MediaPrep.FitWithin(Math.Max(width, 1), Math.Max(height, 1), MediaPrep.PreviewEdge);
        foreach (var seconds in MediaPrep.PosterSeekSeconds)
        {
            var at = duration > TimeSpan.Zero
                ? Math.Min(seconds, Math.Max(duration.TotalSeconds - 0.05, 0))
                : seconds;
            try
            {
                using var frame = await composition.GetThumbnailAsync(
                    TimeSpan.FromSeconds(at), (int)frameWidth, (int)frameHeight, VideoFramePrecision.NearestFrame);
                var decoder = await BitmapDecoder.CreateAsync(frame);
                return (await JpegAsync(decoder, MediaPrep.PreviewEdge, MediaPrep.PreviewQuality)).Bytes;
            }
            catch (Exception e)
            {
                Diagnostics.Write($"reading a poster frame at {at:0.##}s: {e.GetType().Name}");
            }
        }
        Diagnostics.Write("no poster frame could be read from a video; it goes without one");
        return null;
    }

    private static async Task<PrepOutcome> AudioAsync(StorageFile file, string mime)
    {
        // A track's title is worth showing; the server checks the bytes against the type named here.
        var media = new StagedMedia("audio", mime, await BytesAsync(file), Name: MediaPrep.SanitizedName(file.Name));
        try
        {
            var music = await file.Properties.GetMusicPropertiesAsync();
            media = media with { DurationMs = Milliseconds(music.Duration) };
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading a recording: {e.GetType().Name}");
        }
        return PrepOutcome.Staged(media);
    }

    /// <summary>
    /// The picture, fitted within <paramref name="edge"/>, turned upright, drawn on white, as JPEG.
    /// </summary>
    private static async Task<(byte[] Bytes, int Width, int Height)> JpegAsync(BitmapDecoder decoder, uint edge, double quality)
    {
        var (width, height) = MediaPrep.FitWithin(decoder.OrientedPixelWidth, decoder.OrientedPixelHeight, edge);
        // Whether the scaler runs before the EXIF turn or after it, the answer's own size says which:
        // asked in stored sides first, and asked again in upright sides if that came out turned.
        var turned = decoder.OrientedPixelWidth != decoder.PixelWidth;
        var bitmap = await DecodeAsync(decoder, turned ? height : width, turned ? width : height);
        if (bitmap.PixelWidth != width || bitmap.PixelHeight != height)
        {
            bitmap.Dispose();
            bitmap = await DecodeAsync(decoder, width, height);
        }
        using (bitmap)
        {
            var pixels = new byte[4 * bitmap.PixelWidth * bitmap.PixelHeight];
            bitmap.CopyToBuffer(pixels.AsBuffer());
            for (var at = 0; at < pixels.Length; at += 4)
            {
                // Premultiplied, so "over white" is adding what the alpha left uncovered.
                var uncovered = 255 - pixels[at + 3];
                if (uncovered == 0)
                {
                    continue;
                }
                pixels[at] = (byte)Math.Min(255, pixels[at] + uncovered);
                pixels[at + 1] = (byte)Math.Min(255, pixels[at + 1] + uncovered);
                pixels[at + 2] = (byte)Math.Min(255, pixels[at + 2] + uncovered);
                pixels[at + 3] = 255;
            }
            using var output = new InMemoryRandomAccessStream();
            var options = new BitmapPropertySet
            {
                ["ImageQuality"] = new BitmapTypedValue((float)quality, Windows.Foundation.PropertyType.Single),
            };
            // A new encoder, fed pixels only: nothing of the original's metadata comes along.
            var encoder = await BitmapEncoder.CreateAsync(BitmapEncoder.JpegEncoderId, output, options);
            encoder.SetPixelData(
                BitmapPixelFormat.Bgra8, BitmapAlphaMode.Ignore,
                (uint)bitmap.PixelWidth, (uint)bitmap.PixelHeight, 96, 96, pixels);
            await encoder.FlushAsync();
            return (await ReadAllAsync(output), bitmap.PixelWidth, bitmap.PixelHeight);
        }
    }

    private static async Task<SoftwareBitmap> DecodeAsync(BitmapDecoder decoder, uint width, uint height) =>
        await decoder.GetSoftwareBitmapAsync(
            BitmapPixelFormat.Bgra8,
            BitmapAlphaMode.Premultiplied,
            new BitmapTransform { ScaledWidth = width, ScaledHeight = height, InterpolationMode = BitmapInterpolationMode.Fant },
            ExifOrientationMode.RespectExifOrientation,
            ColorManagementMode.ColorManageToSRgb);

    /// <summary>The first bytes — what the server's magic-number check reads.</summary>
    private static async Task<byte[]> HeadAsync(StorageFile file)
    {
        using var stream = await file.OpenReadAsync();
        var count = (uint)Math.Min(12UL, stream.Size);
        if (count == 0)
        {
            return [];
        }
        using var reader = new DataReader(stream.GetInputStreamAt(0));
        var loaded = await reader.LoadAsync(count);
        var head = new byte[loaded];
        reader.ReadBytes(head);
        return head;
    }

    private static async Task<byte[]> BytesAsync(StorageFile file)
    {
        var buffer = await FileIO.ReadBufferAsync(file);
        var bytes = new byte[buffer.Length];
        using var reader = DataReader.FromBuffer(buffer);
        reader.ReadBytes(bytes);
        return bytes;
    }

    private static async Task<byte[]> ReadAllAsync(IRandomAccessStream stream)
    {
        var bytes = new byte[stream.Size];
        using var reader = new DataReader(stream.GetInputStreamAt(0));
        await reader.LoadAsync((uint)stream.Size);
        reader.ReadBytes(bytes);
        return bytes;
    }

    private static int? Milliseconds(TimeSpan duration) =>
        duration > TimeSpan.Zero ? (int)Math.Min(int.MaxValue, Math.Round(duration.TotalMilliseconds)) : null;
}
