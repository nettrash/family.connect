using System.Runtime.InteropServices.WindowsRuntime;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using Windows.Graphics.Imaging;
using Windows.Media.Core;
using Windows.Media.MediaProperties;
using Windows.Media.Transcoding;
using Windows.Storage;
using Windows.Storage.Streams;

namespace FamilyConnect.App.Services;

/// <summary>
/// An animated GIF, WebP or PNG made into the protocol's silent looping video (issue #96; docs/protocol.md, "Animations",
/// "The file") — the Windows half: decoding what this client does not decode itself, and encoding. What becomes one, the
/// timing, the compositing, the colour conversion and the sample table are App.Logic's, and tested there.
/// </summary>
/// <remarks>
/// <para>
/// <b>WHERE THE FRAMES COME FROM.</b> A GIF is decoded by this client (<see cref="GifPicture"/>): Windows Imaging's GIF decoder
/// hands out each image as the raw rectangle the file stores, so compositing was ours anyway, and its own decode makes the
/// whole picture a function of its bytes. An APNG is cut into standalone PNGs (<see cref="ApngPicture"/>) — the PNG decoder
/// has never read APNG's frames — and each is decoded by Windows Imaging as the still it is. An animated WebP is cut the
/// same way (<see cref="WebPAnimation"/>) and each still decoded by the WebP extension's still-picture path, the one stickers
/// already use; if that path refuses the first frame, the extension's own animation frames are asked for instead — WHOLE
/// pictures when each is the canvas's size, raw rectangles composited from the file's <c>ANMF</c>s when each is its frame's —
/// and anything else is rule C. Nothing here has been run against the extension.
/// </para>
/// <para>
/// <b>HOW IT IS ENCODED.</b> A <see cref="MediaStreamSource"/> hands Media Foundation one uncompressed NV12 frame per sample of
/// the plan (<see cref="AnimationRender.Nv12"/>, converted here in BT.709 so the video processor's resolution-based choice of
/// matrix never runs), each stamped at its tick and lasting its ticks
/// (<see cref="MediaStreamSample.CreateFromBuffer"/>, <see cref="MediaStreamSample.Duration"/>), and
/// <see cref="MediaTranscoder.PrepareMediaStreamSourceTranscodeAsync"/> encodes them into an MP4 with the profile's H.264 —
/// High, then Main — at the plan's size and bitrate, and NO audio track (<see cref="MediaEncodingProfile.Audio"/> null). The
/// result is read back (H.264, the plan's sides, no sound), its sample table made exact on the 30-a-second grid
/// (<see cref="AnimationMp4.ExactTicks"/>), and its index moved to the front (<see cref="Faststart"/>). Its poster is the
/// first frame as a JPEG.
/// </para>
/// <para>
/// <b>RULE C.</b> Anything that does not come out — a picture that will not decode, an encoder that refuses or throws, a result
/// that is not what was asked, one over the ceiling — answers null, and the caller sends it exactly as it went before #96: a
/// GIF or a WebP as its original bytes, as a file; an animated PNG down the photo route. Only the person's cancel is not a failure: it comes out as <see cref="OperationCanceledException"/>.
/// </para>
/// </remarks>
internal static class AnimationPreparing
{
    // Media Foundation's colour attributes (mfapi.h), and their BT.709 values (mfobjects.h) — what the frames ARE.
    private static readonly Guid TransferFunction = new("5fb0fce9-be5c-4935-a811-ec838f8eed93");
    private static readonly Guid VideoPrimaries = new("dbfbe4d7-0740-4ee0-8192-850ab0e21935");
    private static readonly Guid YuvMatrix = new("3e23d450-2c75-4d25-a00e-b91670d12327");
    private static readonly Guid NominalRange = new("c21b8ee5-b956-4071-8daf-325edf5cab11");
    private const uint Bt709Transfer = 5;
    private const uint Bt709Primaries = 2;
    private const uint Bt709Matrix = 1;

    // MFNominalRange_16_235: the limited range the frames are converted into.
    private const uint LimitedRange = 2;

    /// <summary>One way of asking for the video: the H.264 profile, High first.</summary>
    private sealed record Attempt(H264Profile Profile);

    /// <summary>The made video, or null for rule C.</summary>
    /// <exception cref="OperationCanceledException"><paramref name="cancel"/> was cancelled.</exception>
    public static async Task<StagedMedia?> PrepareAsync(byte[] bytes, CancellationToken cancel)
    {
        if (AnimationPrep.Layout(bytes) is not { } layout)
        {
            Diagnostics.Write("an animated picture's frames could not be walked; it goes as a file");
            return null;
        }
        if (AnimationTiming.Plan(layout.Width, layout.Height, layout.StatedMs) is not { } plan)
        {
            Diagnostics.Write($"an animated picture of {layout.Width}x{layout.Height} has no video size; it goes as a file");
            return null;
        }
        Diagnostics.Write(
            $"making a {layout.Format} animation of {layout.Width}x{layout.Height}, {layout.Frames.Count} frame(s) into "
            + $"{plan.Width}x{plan.Height}, {plan.Samples.Count} video frame(s), {plan.DurationMs} ms, {plan.VideoBitrate} bit/s");
        try
        {
            if (await SourceAsync(bytes, layout, cancel) is not { } source)
            {
                return null;
            }
            // The poster first: it is frame zero, and a picture whose first frame will not decode goes no further.
            var poster = await PosterAsync(source.Layout, plan, source.Pixels, cancel);
            if (TranscodeFolder() is not { } folder)
            {
                return null;
            }
            var made = await TranscodeAttempts.FirstTakenAsync<Attempt, byte[]>(
                [new Attempt(H264Profile.High), new Attempt(H264Profile.Main)],
                (attempt, token) => AttemptAsync(
                    Path.Combine(folder, $"{Guid.NewGuid():N}.mp4"), source.Layout, plan, source.Pixels, attempt, token),
                MediaEncoding.TranscodeCeiling(plan.DurationMs),
                Diagnostics.Write,
                cancel);
            if (made is null)
            {
                return null;
            }
            return new StagedMedia(
                "video", "video/mp4", made, plan.Width, plan.Height, (int)Math.Min(plan.DurationMs, int.MaxValue),
                Preview: poster, Animation: true);
        }
        catch (OperationCanceledException) when (cancel.IsCancellationRequested)
        {
            throw;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"making an animation: {e.GetType().Name} 0x{e.HResult:X8}");
            return null;
        }
    }

    /// <summary>Where each frame's pixels come from, and the layout they are composited by.</summary>
    internal sealed record FrameSource(AnimationLayout Layout, Func<int, CancellationToken, Task<byte[]?>> Pixels);

    /// <remarks>Also how an animated STICKER's frames are had (<see cref="StickerImaging"/>).</remarks>
    internal static async Task<FrameSource?> SourceAsync(byte[] bytes, AnimationLayout layout, CancellationToken cancel)
    {
        switch (layout.Format)
        {
            case AnimationFormat.Gif when GifPicture.Read(bytes) is { } gif:
                return new FrameSource(layout, (index, _) => Task.FromResult<byte[]?>(gif.Pixels(index)));
            case AnimationFormat.Apng when ApngPicture.Read(bytes) is { } apng:
                return new FrameSource(layout, (index, _) => StillAsync(apng.Standalone(index), layout.Frames[index]));
            case AnimationFormat.WebP when WebPAnimation.Read(bytes) is { } webp:
                var cut = new FrameSource(layout, (index, _) =>
                    webp.Standalone(index) is { } still ? StillAsync(still, layout.Frames[index]) : Task.FromResult<byte[]?>(null));
                if (await cut.Pixels(0, cancel) is not null)
                {
                    return cut;
                }
                Diagnostics.Write("a WebP animation's first frame would not decode as a still; asking the codec for its frames");
                return await WebPFramesAsync(bytes, layout);
            default:
                return null;
        }
    }

    /// <summary>
    /// A standalone still decoded by Windows Imaging, as straight BGRA at exactly the frame's size; null when it will not decode
    /// or comes out another size.
    /// </summary>
    private static async Task<byte[]?> StillAsync(byte[] still, AnimationFrame frame)
    {
        try
        {
            using var stream = await StreamAsync(still);
            var decoder = await BitmapDecoder.CreateAsync(stream);
            if (decoder.PixelWidth != frame.Width || decoder.PixelHeight != frame.Height)
            {
                Diagnostics.Write($"an animation frame decoded at {decoder.PixelWidth}x{decoder.PixelHeight}, not {frame.Width}x{frame.Height}");
                return null;
            }
            return await PixelsAsync(await decoder.GetFrameAsync(0));
        }
        catch (Exception e)
        {
            Diagnostics.Write($"decoding an animation frame: {e.GetType().Name} 0x{e.HResult:X8}");
            return null;
        }
    }

    /// <summary>
    /// The WebP extension's own frames, when it gives one per <c>ANMF</c>: whole pictures go through the compositor as
    /// replacements (<see cref="AnimationLayout.Whole"/>), raw rectangles as the file's frames. Anything else is null.
    /// </summary>
    private static async Task<FrameSource?> WebPFramesAsync(byte[] bytes, AnimationLayout layout)
    {
        try
        {
            var stream = await StreamAsync(bytes);
            var decoder = await BitmapDecoder.CreateAsync(stream);
            if (decoder.FrameCount != layout.Frames.Count)
            {
                Diagnostics.Write($"the WebP codec gave {decoder.FrameCount} frame(s) of an animation of {layout.Frames.Count}");
                stream.Dispose();
                return null;
            }
            var sizes = new List<(uint Width, uint Height)>();
            for (var index = 0u; index < decoder.FrameCount; index++)
            {
                var frame = await decoder.GetFrameAsync(index);
                sizes.Add((frame.PixelWidth, frame.PixelHeight));
            }
            AnimationLayout? used = null;
            if (sizes.All(size => size.Width == layout.Width && size.Height == layout.Height))
            {
                used = layout.Whole();
            }
            else if (sizes.Select((size, index) => size.Width == layout.Frames[index].Width && size.Height == layout.Frames[index].Height).All(same => same))
            {
                used = layout;
            }
            if (used is null)
            {
                Diagnostics.Write("the WebP codec's frames are neither whole pictures nor the file's rectangles");
                stream.Dispose();
                return null;
            }
            Diagnostics.Write(ReferenceEquals(used, layout)
                ? "the WebP codec gives an animation's raw rectangles; compositing them"
                : "the WebP codec gives an animation's whole pictures");
            // The stream lives as long as the decoder reading it, which is the transcode's: it is collected with the closure.
            return new FrameSource(used, async (index, _) =>
            {
                try
                {
                    return await PixelsAsync(await decoder.GetFrameAsync((uint)index));
                }
                catch (Exception e)
                {
                    Diagnostics.Write($"reading a WebP codec frame: {e.GetType().Name} 0x{e.HResult:X8}");
                    return null;
                }
            });
        }
        catch (Exception e)
        {
            Diagnostics.Write($"asking the WebP codec for an animation's frames: {e.GetType().Name} 0x{e.HResult:X8}");
            return null;
        }
    }

    private static async Task<byte[]> PixelsAsync(BitmapFrame frame)
    {
        // Straight alpha, as the compositor blends; the file's own colours, as a browser draws an animation.
        var pixels = await frame.GetPixelDataAsync(
            BitmapPixelFormat.Bgra8,
            BitmapAlphaMode.Straight,
            new BitmapTransform(),
            ExifOrientationMode.IgnoreExifOrientation,
            ColorManagementMode.DoNotColorManage);
        return pixels.DetachPixelData();
    }

    /// <summary>The first frame, on white, fitted within the preview's 600, as a JPEG — or null, and the video goes without.</summary>
    private static async Task<ReadOnlyMemory<byte>?> PosterAsync(
        AnimationLayout layout, AnimationPlan plan, Func<int, CancellationToken, Task<byte[]?>> pixels, CancellationToken cancel)
    {
        var first = await new AnimationFrames(layout, plan, pixels).NextAsync(cancel)
            ?? throw new InvalidDataException("an animation with no first frame");
        try
        {
            var (width, height) = MediaPrep.FitWithin((uint)plan.Width, (uint)plan.Height, MediaPrep.PreviewEdge);
            var fitted = AnimationRender.Scale(first, plan.Width, plan.Height, (int)width, (int)height);
            return await MediaPreparing.EncodeAsync(fitted, width, height, MediaPrep.PreviewQuality);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"an animation's poster: {e.GetType().Name} 0x{e.HResult:X8}");
            return null;
        }
    }

    /// <summary>
    /// One way of asking, into a file of its own that is gone again when this returns. Whatever throws is the loop's to catch,
    /// and means "try the next".
    /// </summary>
    private static async Task<Attempted<byte[]>> AttemptAsync(
        string path,
        AnimationLayout layout,
        AnimationPlan plan,
        Func<int, CancellationToken, Task<byte[]?>> pixels,
        Attempt attempt,
        CancellationToken cancel)
    {
        Exception? failed = null;
        try
        {
            await File.WriteAllBytesAsync(path, [], cancel);
            var output = await StorageFile.GetFileFromPathAsync(path);
            var frames = new AnimationFrames(layout, plan, pixels);
            var starts = plan.StartTicks;
            var source = new MediaStreamSource(new VideoStreamDescriptor(InputProperties(plan)))
            {
                CanSeek = false,
                BufferTime = TimeSpan.Zero,
                Duration = TimeSpan.FromTicks(AnimationTiming.Hundreds(plan.TotalTicks)),
            };
            source.Starting += (_, args) => args.Request.SetActualStartPosition(TimeSpan.Zero);
            source.SampleRequested += async (_, args) =>
            {
                var request = args.Request;
                var deferral = request.GetDeferral();
                try
                {
                    var index = frames.Given;
                    if (failed is not null || await frames.NextAsync(cancel) is not { } frame)
                    {
                        // No sample is the end of the stream.
                        request.Sample = null;
                        return;
                    }
                    var start = AnimationTiming.Hundreds(starts[index]);
                    var end = AnimationTiming.Hundreds(starts[index] + plan.Samples[index].Ticks);
                    var sample = MediaStreamSample.CreateFromBuffer(
                        AnimationRender.Nv12(frame, plan.Width, plan.Height).AsBuffer(), TimeSpan.FromTicks(start));
                    sample.Duration = TimeSpan.FromTicks(end - start);
                    request.Sample = sample;
                }
                catch (Exception e)
                {
                    // A frame that would not decode: the stream ends here, and the result is thrown away below.
                    failed = e;
                    request.Sample = null;
                }
                finally
                {
                    deferral.Complete();
                }
            };
            var transcoder = new MediaTranscoder { HardwareAccelerationEnabled = true };
            IRandomAccessStream? written = null;
            try
            {
                written = await output.OpenAsync(FileAccessMode.ReadWrite).AsTask(cancel);
                var prepared = await transcoder.PrepareMediaStreamSourceTranscodeAsync(source, written, Profile(plan, attempt)).AsTask(cancel);
                if (!prepared.CanTranscode)
                {
                    Diagnostics.Write($"an animation's encode was refused: {prepared.FailureReason} ({attempt.Profile})");
                    return Attempted<byte[]>.Refused;
                }
                await prepared.TranscodeAsync().AsTask(cancel);
            }
            finally
            {
                written?.Dispose();
            }
            GC.KeepAlive(transcoder);
            GC.KeepAlive(source);
            cancel.ThrowIfCancellationRequested();
            if (failed is not null)
            {
                Diagnostics.Write($"an animation frame failed mid-encode: {failed.GetType().Name} {failed.Message}");
                return Attempted<byte[]>.Wrong;
            }
            if (frames.Given != plan.Samples.Count)
            {
                Diagnostics.Write($"an animation's encode took {frames.Given} of {plan.Samples.Count} frames");
                return Attempted<byte[]>.Wrong;
            }
            if (!await CameOutAsync(output, plan))
            {
                return Attempted<byte[]>.Wrong;
            }
            return Finished(await File.ReadAllBytesAsync(path, cancel), plan);
        }
        finally
        {
            try
            {
                File.Delete(path);
            }
            catch (Exception e)
            {
                Diagnostics.Write($"removing an animation's encode: {e.GetType().Name}");
            }
        }
    }

    /// <summary>
    /// What the frames handed over ARE: uncompressed NV12 at the plan's size, BT.709 limited range, at a nominal 30 a second
    /// — the output's own rate, so no frame-rate conversion is put between them and every sample keeps its own length.
    /// </summary>
    private static VideoEncodingProperties InputProperties(AnimationPlan plan)
    {
        var input = VideoEncodingProperties.CreateUncompressed(MediaEncodingSubtypes.Nv12, (uint)plan.Width, (uint)plan.Height);
        input.FrameRate.Numerator = AnimationTiming.TicksPerSecond;
        input.FrameRate.Denominator = 1;
        input.PixelAspectRatio.Numerator = 1;
        input.PixelAspectRatio.Denominator = 1;
        input.Properties[TransferFunction] = Bt709Transfer;
        input.Properties[VideoPrimaries] = Bt709Primaries;
        input.Properties[YuvMatrix] = Bt709Matrix;
        input.Properties[NominalRange] = LimitedRange;
        return input;
    }

    /// <summary>The profile's video at the plan's numbers, and no audio track at all.</summary>
    private static MediaEncodingProfile Profile(AnimationPlan plan, Attempt attempt)
    {
        var profile = MediaEncodingProfile.CreateMp4(VideoEncodingQuality.HD720p);
        var video = profile.Video;
        video.Subtype = MediaEncodingSubtypes.H264;
        video.ProfileId = attempt.Profile == H264Profile.High ? H264ProfileIds.High : H264ProfileIds.Main;
        video.Width = (uint)plan.Width;
        video.Height = (uint)plan.Height;
        video.Bitrate = (uint)Math.Clamp(plan.VideoBitrate, 0, uint.MaxValue);
        video.FrameRate.Numerator = AnimationTiming.TicksPerSecond;
        video.FrameRate.Denominator = 1;
        video.PixelAspectRatio.Numerator = 1;
        video.PixelAspectRatio.Denominator = 1;
        video.Properties[TransferFunction] = Bt709Transfer;
        video.Properties[VideoPrimaries] = Bt709Primaries;
        video.Properties[YuvMatrix] = Bt709Matrix;
        // "No audio track": not a silent one — none.
        profile.Audio = null;
        return profile;
    }

    /// <summary>Whether the encode made what was asked: H.264, the plan's sides, and no sound track.</summary>
    private static async Task<bool> CameOutAsync(StorageFile output, AnimationPlan plan)
    {
        var profile = await MediaEncodingProfile.CreateFromFileAsync(output);
        if (profile.Video is not { } video || !string.Equals(video.Subtype, MediaEncodingSubtypes.H264, StringComparison.OrdinalIgnoreCase))
        {
            Diagnostics.Write($"an animation came out as {profile.Video?.Subtype ?? "no video"}, not H.264");
            return false;
        }
        if (profile.Audio is not null)
        {
            Diagnostics.Write("an animation came out with a sound track");
            return false;
        }
        if (video.Width != plan.Width || video.Height != plan.Height)
        {
            Diagnostics.Write($"an animation came out {video.Width}x{video.Height}; asked {plan.Width}x{plan.Height}");
            return false;
        }
        return true;
    }

    /// <summary>
    /// The encode taken in: an MP4 by its bytes, within the ceiling, its timing made exact where it can be (the encoder's own,
    /// within 100 ns a frame, otherwise), and its index in front — or Wrong, and rule C.
    /// </summary>
    private static Attempted<byte[]> Finished(byte[] bytes, AnimationPlan plan)
    {
        if (!MediaPrep.MatchesMagic("video/mp4", bytes))
        {
            Diagnostics.Write("an animation's bytes are not an MP4's");
            return Attempted<byte[]>.Wrong;
        }
        var timed = AnimationMp4.ExactTicks(bytes, [.. plan.Samples.Select(sample => sample.Ticks)]);
        if (timed is null)
        {
            Diagnostics.Write("an animation's sample table could not be made exact; it keeps the encoder's timing");
        }
        if (Faststart.MoovFirst(timed ?? bytes) is not { } arranged)
        {
            Diagnostics.Write("an animation's index could not be moved to the front");
            return Attempted<byte[]>.Wrong;
        }
        if (arranged.LongLength > MediaPrep.SizeLimit)
        {
            Diagnostics.Write($"an animation came out over the ceiling ({arranged.LongLength} bytes)");
            return Attempted<byte[]>.Wrong;
        }
        return Attempted<byte[]>.Taken(arranged);
    }

    /// <summary>The folder every encode is written into — the transcodes' own; null when it cannot be made.</summary>
    private static string? TranscodeFolder()
    {
        var folder = Path.Combine(Path.GetTempPath(), "FamilyConnect", "prepared");
        try
        {
            Directory.CreateDirectory(folder);
            return folder;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"making room for an animation's encode: {e.GetType().Name}");
            return null;
        }
    }

    private static async Task<InMemoryRandomAccessStream> StreamAsync(byte[] bytes)
    {
        var stream = new InMemoryRandomAccessStream();
        try
        {
            await stream.WriteAsync(bytes.AsBuffer());
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
