using System.Runtime.InteropServices.WindowsRuntime;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using Windows.Graphics.Imaging;
using Windows.Media.Editing;
using Windows.Media.MediaProperties;
using Windows.Media.Transcoding;
using Windows.Storage;
using Windows.Storage.FileProperties;
using Windows.Storage.Streams;

namespace FamilyConnect.App.Services;

/// <summary>
/// A picked or dropped file made ready to send — the Windows half of <see cref="MediaPrep"/>, which
/// decides the route, and of <see cref="MediaPlan"/>, which decides what a video or a sound file becomes;
/// this only does what needs the platform: decoding, drawing, reading a frame, reading a stream, encoding.
/// </summary>
/// <remarks>
/// <para>
/// <b>A PHOTO IS RE-DRAWN</b>, at most 2048 on its longest side, as JPEG — which is also what leaves its
/// EXIF behind — with its preview beside it. Transparency is drawn on white: a JPEG has no alpha, and
/// a transparent pixel would otherwise come out black.
/// </para>
/// <para>
/// <b>A VIDEO IS BROUGHT TO THE PROTOCOL'S PROFILE, OR LEFT ALONE WHEN IT IS ALREADY THERE</b>
/// (docs/protocol.md, "Preparing media before upload"; issue #74). What Media Foundation and the shell
/// read of it goes into the planner; a transcode is <see cref="MediaTranscoder"/> with the numbers
/// <see cref="MediaEncoding"/> works out, hardware first; what comes out is read back and CHECKED, its
/// index moved to the front (<see cref="Faststart"/>), and thrown away when it is bigger than what it
/// came from (rule D). Everything read for the bubble — its size, its length, its poster — is still what
/// Windows can read of the source.
/// </para>
/// <para>
/// <b>A PICKED SOUND FILE IS RE-ENCODED ONLY WHERE THE AUDIO RULES SAY</b> — lossless, Ogg, or lossy
/// above 192 kbit/s — into an M4A. AIFF and FLAC, which the server does not take as audio and which
/// went as files before, reach those rules too, and go as audio when they come out.
/// </para>
/// <para>
/// <b>NOTHING HERE MAY LOSE A SEND THAT WORKED IN 1.1</b> (rule C). Every Windows call is guarded, and
/// anything that fails — a stream Windows cannot read, a codec it does not have (HEVC without its
/// extension, Ogg without the web media one), a transcode that throws, stalls or comes out wrong — sends
/// what 1.1 sent: the original when it is within the ceiling, refused or as a file otherwise.
/// </para>
/// <para>
/// The previews are made here and never by the server (docs/protocol.md, "Previews").
/// </para>
/// </remarks>
internal static class MediaPreparing
{
    private static readonly PrepOutcome TooLarge = PrepOutcome.Refused(PrepFailure.TooLarge);

    // Media Foundation's attribute keys (mfapi.h) — carried in a stream's Properties, and read and written there.
    private static readonly Guid VideoRotation = new("c380465d-2271-428c-9b83-ecea3b4a85c1");
    private static readonly Guid TransferFunction = new("5fb0fce9-be5c-4935-a811-ec838f8eed93");
    private static readonly Guid VideoPrimaries = new("dbfbe4d7-0740-4ee0-8192-850ab0e21935");
    private static readonly Guid YuvMatrix = new("3e23d450-2c75-4d25-a00e-b91670d12327");

    // …and their BT.709 values (mfobjects.h): MFVideoTransFunc_709, MFVideoPrimaries_BT709, MFVideoTransferMatrix_BT709.
    private const uint Bt709Transfer = 5;
    private const uint Bt709Primaries = 2;
    private const uint Bt709Matrix = 1;

    // What the shell reads a stream as stating, for when Media Foundation's own type leaves a number out.
    private const string ShellFrameRate = "System.Video.FrameRate";
    private const string ShellVideoBitrate = "System.Video.EncodingBitrate";
    private const string ShellAudioBitrate = "System.Audio.EncodingBitrate";
    private const string ShellAudioChannels = "System.Audio.ChannelCount";
    private const string ShellAudioSampleRate = "System.Audio.SampleRate";

    public static async Task<PrepOutcome> PrepareAsync(StorageFile file)
    {
        var name = file.Name;
        var size = (long)(await file.GetBasicPropertiesAsync()).Size;
        var head = await HeadAsync(file);
        var declared = MediaPrep.DeclaredType(file.ContentType ?? string.Empty, name);
        var routed = MediaPrep.Route(file.ContentType ?? string.Empty, name, head);
        // Nothing over the ceiling is refused up front any more: a 400 MB 4K clip is exactly what a transcode is for.
        return routed.Route switch
        {
            MediaRoute.Photo => await PhotoAsync(file),
            MediaRoute.Video => await VideoAsync(file, declared, size, head),
            MediaRoute.Audio => await AudioAsync(file, declared, routed.AudioMime ?? "audio/mp4", asAudio: true, size, head),
            _ when MediaEncoding.PickedAudioType(declared, name) is { } sound =>
                await AudioAsync(file, declared, sound, asAudio: false, size, head),
            _ => await AsFileAsync(file, declared, size),
        };
    }

    /// <summary>
    /// Whether a file is one the router re-draws as a photo — what a board may pin — from its first bytes alone, so a
    /// video dropped on the board is refused before anything is transcoded for nothing.
    /// </summary>
    public static async Task<bool> IsPhotoAsync(StorageFile file) =>
        MediaPrep.Route(file.ContentType ?? string.Empty, file.Name, await HeadAsync(file)).Route == MediaRoute.Photo;

    /// <summary>A file, as every file has always gone: untouched, named, and refused over the ceiling.</summary>
    private static async Task<PrepOutcome> AsFileAsync(StorageFile file, string declared, long size) =>
        size > MediaPrep.SizeLimit
            ? TooLarge
            : PrepOutcome.Staged(new StagedMedia(
                "file", declared, await BytesAsync(file), Name: MediaPrep.SanitizedName(file.Name) ?? "file"));

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

    /// <summary>What a bubble is drawn from — read of the SOURCE whichever bytes go, as it always was.</summary>
    private sealed record Shown(int? Width, int? Height, int? DurationMs, ReadOnlyMemory<byte>? Preview);

    /// <summary>What Windows read of a video's streams: the planner's input, and what a profile needs besides.</summary>
    private sealed record VideoReading(
        VideoSource Source, uint Rotation, uint RateNumerator, uint RateDenominator, long? AudioSampleRate, bool Hdr);

    /// <summary>
    /// A transcode that ran and came out as asked: how long it is, its bytes when it is within the ceiling (one over it is
    /// never sent — rule D picks the source or nothing — so it is never read into memory either), and whether its index
    /// is in front.
    /// </summary>
    private sealed record Transcoded(long Length, byte[]? Bytes, bool MoovFirst);

    private static async Task<PrepOutcome> VideoAsync(StorageFile file, string declared, long size, byte[] head)
    {
        var container = declared == "video/quicktime" ? declared : "video/mp4";
        var sendable = MediaPlan.Sendable("video", container, MediaPrep.MatchesMagic(container, head), size, MediaPrep.SizeLimit);
        VideoProperties? shell = null;
        var shown = new Shown(null, null, null, null);
        try
        {
            shell = await file.Properties.GetVideoPropertiesAsync();
            // A phone holds a video on its side and says so: the size a bubble draws is the turned one.
            var (width, height) = shell.Orientation is VideoOrientation.Rotate90 or VideoOrientation.Rotate270
                ? (shell.Height, shell.Width)
                : (shell.Width, shell.Height);
            shown = new Shown(
                width > 0 ? (int)width : null,
                height > 0 ? (int)height : null,
                Milliseconds(shell.Duration),
                await PosterAsync(file, shell.Duration, width, height));
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading a video: {e.GetType().Name}");
        }

        var reading = await ReadVideoAsync(file, shell, container, size, shown.DurationMs);
        var plan = reading is null ? VideoPlan.Fallback : MediaPlan.PlanVideo(reading.Source);
        if (plan.Kind == VideoPlanKind.Keep)
        {
            // Rule A: already within the profile. Re-encoding it would only cost it quality.
            return await VideoAsBeforeAsync(file, container, size, shown);
        }
        if (reading is not null && plan is { Kind: VideoPlanKind.Transcode, Target: { } target })
        {
            Diagnostics.Write($"transcoding a video to {target.Width}x{target.Height}, {target.FrameRate:0.###} fps, {target.VideoBitrate} bit/s");
            // Off the window's thread: the transcode is Media Foundation's, but reading it back and moving its index are ours.
            var result = await Task.Run(() => TranscodeVideoAsync(file, reading, target));
            if (result is not null)
            {
                return await ChosenAsync(
                    result, size, sendable,
                    async () => PrepOutcome.Staged(await OriginalVideoAsync(file, container, shown)),
                    bytes => new StagedMedia(
                        "video", "video/mp4", bytes, (int)target.Width, (int)target.Height, shown.DurationMs, Preview: shown.Preview));
            }
        }
        // Rule C: no size to scale to, a stream Windows cannot read, or a transcode that failed. For a video the router
        // let through, "the original" and "what 1.1 did" are the same thing: untouched within the ceiling, refused over it.
        return MediaPlan.AfterFailure(sendable) == OnFailure.Original
            ? PrepOutcome.Staged(await OriginalVideoAsync(file, container, shown))
            : await VideoAsBeforeAsync(file, container, size, shown);
    }

    /// <summary>What 1.1 did with every video: untouched, and refused over the ceiling.</summary>
    private static async Task<PrepOutcome> VideoAsBeforeAsync(StorageFile file, string container, long size, Shown shown) =>
        size > MediaPrep.SizeLimit ? TooLarge : PrepOutcome.Staged(await OriginalVideoAsync(file, container, shown));

    private static async Task<StagedMedia> OriginalVideoAsync(StorageFile file, string container, Shown shown) =>
        new("video", container, await BytesAsync(file), shown.Width, shown.Height, shown.DurationMs, Preview: shown.Preview);

    /// <summary>
    /// What the planner is given for a video: Media Foundation's own view of its streams — the transcoder's view — and the
    /// shell's numbers where that leaves one out. Null when Media Foundation cannot open it at all, which is also a source
    /// it cannot transcode.
    /// </summary>
    private static async Task<VideoReading?> ReadVideoAsync(
        StorageFile file, VideoProperties? shell, string container, long size, int? durationMs)
    {
        try
        {
            var profile = await MediaEncodingProfile.CreateFromFileAsync(file);
            if (profile.Video is not { } video)
            {
                Diagnostics.Write("a video has no stream Media Foundation reads as video");
                return null;
            }
            var stated = await StatedAsync(file);
            var (storedWidth, storedHeight, rotation) = Geometry(video, shell);
            var (width, height) = MediaEncoding.Displayed(storedWidth, storedHeight, rotation);
            var numerator = video.FrameRate?.Numerator ?? 0;
            var denominator = video.FrameRate?.Denominator ?? 0;
            var perThousand = (uint)Math.Clamp(Number(stated, ShellFrameRate) ?? 0, 0, uint.MaxValue);
            var audio = profile.Audio;
            var source = new VideoSource(
                width,
                height,
                MediaEncoding.FrameRate(numerator, denominator, perThousand),
                container,
                VideoCodec(video.Subtype),
                audio is null ? null : AudioCodec(audio.Subtype),
                audio is null ? null : MediaEncoding.FirstKnown(audio.ChannelCount, Number(stated, ShellAudioChannels)),
                MediaEncoding.FirstKnown(video.Bitrate, Number(stated, ShellVideoBitrate)),
                audio is null ? null : MediaEncoding.FirstKnown(audio.Bitrate, Number(stated, ShellAudioBitrate)),
                size,
                durationMs);
            return new VideoReading(
                source,
                rotation,
                numerator,
                denominator,
                audio is null ? null : MediaEncoding.FirstKnown(audio.SampleRate, Number(stated, ShellAudioSampleRate)),
                MediaEncoding.IsHdr(Number(video.Properties, TransferFunction), Number(video.Properties, VideoPrimaries)));
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading a video's streams: {e.GetType().Name}");
            return null;
        }
    }

    /// <summary>
    /// A video's stored sides and its turn, as ONE reader reads them — the source's and the result's alike, so checking one
    /// against the other never compares two conventions.
    /// </summary>
    private static (long Width, long Height, uint Rotation) Geometry(VideoEncodingProperties video, VideoProperties? shell) =>
        (MediaEncoding.FirstKnown(video.Width, shell?.Width) ?? 0,
         MediaEncoding.FirstKnown(video.Height, shell?.Height) ?? 0,
         MediaEncoding.Rotation(Number(video.Properties, VideoRotation), shell is null ? null : (long)shell.Orientation));

    private static Task<Transcoded?> TranscodeVideoAsync(StorageFile file, VideoReading reading, VideoTarget target)
    {
        var attempts = MediaEncoding.VideoAttempts(
            target,
            reading.Rotation,
            reading.RateNumerator,
            reading.RateDenominator,
            reading.Source.AudioChannels,
            reading.AudioSampleRate,
            reading.Source.AudioBitrate,
            reading.Hdr);
        return TranscodeAsync(file, ".mp4", "video/mp4", attempts, VideoProfile, VideoCameOutAsync, reading.Source.DurationMs);
    }

    /// <summary>
    /// The protocol's profile, at exactly the planner's numbers. CreateMp4 is only the starting point — its container and
    /// its defaults for what is not set here; every number the protocol names is set explicitly.
    /// </summary>
    private static MediaEncodingProfile VideoProfile(VideoEncoding encoding)
    {
        var profile = MediaEncodingProfile.CreateMp4(VideoEncodingQuality.HD720p);
        var video = profile.Video;
        video.Subtype = MediaEncodingSubtypes.H264;
        video.ProfileId = encoding.Profile == H264Profile.High ? H264ProfileIds.High : H264ProfileIds.Main;
        video.Width = encoding.Width;
        video.Height = encoding.Height;
        video.Bitrate = encoding.Bitrate;
        video.FrameRate.Numerator = encoding.FrameRateNumerator;
        video.FrameRate.Denominator = encoding.FrameRateDenominator;
        if (encoding.Rotation != 0)
        {
            // The turn rides along as the source's did — metadata, not pixels — so the frame is scaled, never squashed.
            video.Properties[VideoRotation] = encoding.Rotation;
        }
        if (encoding.ToSdr)
        {
            // HDR is asked for as BT.709 SDR explicitly, so it is converted rather than passed through as grey (rule 8-bit SDR).
            video.Properties[TransferFunction] = Bt709Transfer;
            video.Properties[VideoPrimaries] = Bt709Primaries;
            video.Properties[YuvMatrix] = Bt709Matrix;
        }
        // No audio track in, none out: a transcode does not invent silence.
        profile.Audio = encoding.Audio is { } aac ? AudioEncodingProperties.CreateAac(aac.SampleRate, aac.Channels, aac.Bitrate) : null;
        return profile;
    }

    /// <summary>
    /// Whether the transcode made what was asked for: H.264, its audio track kept or left out as asked, the encoded sides
    /// and the SAME turn (<see cref="MediaEncoding.Matches"/>). Anything else is a video that would play sideways, squashed
    /// or silent, and is a failed transcode.
    /// </summary>
    private static async Task<bool> VideoCameOutAsync(StorageFile output, VideoEncoding asked)
    {
        var profile = await MediaEncodingProfile.CreateFromFileAsync(output);
        if (profile.Video is not { } video || VideoCodec(video.Subtype) != "h264")
        {
            Diagnostics.Write($"a transcode came out as {profile.Video?.Subtype ?? "no video"}, not H.264");
            return false;
        }
        if (profile.Audio is null != asked.Audio is null)
        {
            Diagnostics.Write("a transcode came out with its audio track added or lost");
            return false;
        }
        VideoProperties? shell = null;
        try
        {
            shell = await output.Properties.GetVideoPropertiesAsync();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading a transcode back: {e.GetType().Name}");
        }
        var (width, height, rotation) = Geometry(video, shell);
        if (!MediaEncoding.Matches(asked, (uint)Math.Clamp(width, 0, uint.MaxValue), (uint)Math.Clamp(height, 0, uint.MaxValue), rotation))
        {
            Diagnostics.Write($"a transcode came out {width}x{height} turned {rotation}; asked {asked.Width}x{asked.Height} turned {asked.Rotation}");
            return false;
        }
        var asksFor = (double)asked.FrameRateNumerator / asked.FrameRateDenominator;
        if (MediaEncoding.FrameRate(video.FrameRate?.Numerator ?? 0, video.FrameRate?.Denominator ?? 0, 0) is { } rate
            && rate > asksFor + (MediaPlan.FrameRateTolerance - MediaPlan.MaxFrameRate))
        {
            // Said, not refused: it plays everywhere and a transcoder cannot invent frames the source never had (rule B).
            // It is outside the profile's 30, though, and worth knowing about.
            Diagnostics.Write($"a transcode came out at {rate:0.###} fps; asked {asksFor:0.###}");
        }
        return true;
    }

    private static async Task<PrepOutcome> AudioAsync(
        StorageFile file, string declared, string container, bool asAudio, long size, byte[] head)
    {
        // A track's title is worth showing; the server checks the bytes against the type named here.
        var name = MediaPrep.SanitizedName(file.Name);
        var sendable = MediaPlan.Sendable("audio", container, MediaPrep.MatchesMagic(container, head), size, MediaPrep.SizeLimit);
        int? durationMs = null;
        long? statedBitrate = null;
        try
        {
            var music = await file.Properties.GetMusicPropertiesAsync();
            durationMs = Milliseconds(music.Duration);
            statedBitrate = music.Bitrate;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading a recording: {e.GetType().Name}");
        }

        var (source, sampleRate) = await ReadAudioAsync(file, container, statedBitrate, size, durationMs);
        var plan = MediaPlan.PlanAudio(source);
        if (plan.Kind == AudioPlanKind.Transcode)
        {
            Diagnostics.Write($"re-encoding a sound file ({source.Codec}) at {plan.Bitrate} bit/s");
            var attempts = MediaEncoding.AudioAttempts(plan.Bitrate, source.Channels, sampleRate, MediaPlan.SourceAudioBitrate(source));
            var result = await Task.Run(() => TranscodeAsync(file, ".m4a", "audio/mp4", attempts, M4aProfile, AudioCameOutAsync, source.DurationMs));
            if (result is not null)
            {
                return await ChosenAsync(
                    result, size, sendable,
                    () => OriginalAudioAsync(file, container, name, durationMs),
                    bytes => new StagedMedia("audio", "audio/mp4", bytes, DurationMs: durationMs, Name: MediaEncoding.M4aName(name)));
            }
            // Rule C. A sendable source is one the router already sent as audio, so this is what 1.1 did with it too.
            if (MediaPlan.AfterFailure(sendable) == OnFailure.Original)
            {
                return await OriginalAudioAsync(file, container, name, durationMs);
            }
        }
        // Kept, or rule C's "today's path": what 1.1 did — as audio within the ceiling, as a file when it was one.
        return !asAudio ? await AsFileAsync(file, declared, size)
            : size > MediaPrep.SizeLimit ? TooLarge
            : await OriginalAudioAsync(file, container, name, durationMs);
    }

    private static async Task<PrepOutcome> OriginalAudioAsync(StorageFile file, string mime, string? name, int? durationMs) =>
        PrepOutcome.Staged(new StagedMedia("audio", mime, await BytesAsync(file), DurationMs: durationMs, Name: name));

    /// <summary>
    /// What the planner is given for a sound file. The shell's bit rate comes first — for a VBR MP3 it is the file's
    /// average, which is what "above 192 kbit/s" is about. A stream Media Foundation cannot open is still judged, as a
    /// codec nobody can name: the rules leave that alone unless it is in an Ogg, and an Ogg this machine cannot decode
    /// then fails its transcode — rule C either way.
    /// </summary>
    private static async Task<(AudioSource Source, long? SampleRate)> ReadAudioAsync(
        StorageFile file, string container, long? statedBitrate, long size, int? durationMs)
    {
        var codec = "unknown";
        long? channels = null;
        long? bitrate = null;
        long? sampleRate = null;
        try
        {
            var profile = await MediaEncodingProfile.CreateFromFileAsync(file);
            if (profile.Audio is { } audio)
            {
                codec = AudioCodec(audio.Subtype);
                channels = audio.ChannelCount;
                bitrate = audio.Bitrate;
                sampleRate = audio.SampleRate;
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading a sound file's stream: {e.GetType().Name}");
        }
        var stated = await StatedAsync(file);
        return (
            new AudioSource(
                container,
                codec,
                MediaEncoding.FirstKnown(channels, Number(stated, ShellAudioChannels)),
                MediaEncoding.FirstKnown(statedBitrate, bitrate),
                size,
                durationMs),
            MediaEncoding.FirstKnown(sampleRate, Number(stated, ShellAudioSampleRate)));
    }

    /// <summary>An M4A of AAC-LC at exactly these numbers — a picked track's re-encode, and a voice note (<see cref="VoiceRecorder"/>).</summary>
    public static MediaEncodingProfile M4aProfile(AacEncoding aac)
    {
        var profile = MediaEncodingProfile.CreateM4a(AudioEncodingQuality.Medium);
        profile.Audio = AudioEncodingProperties.CreateAac(aac.SampleRate, aac.Channels, aac.Bitrate);
        return profile;
    }

    /// <summary>Whether a re-encode made AAC with the channels asked for.</summary>
    private static async Task<bool> AudioCameOutAsync(StorageFile output, AacEncoding asked)
    {
        var profile = await MediaEncodingProfile.CreateFromFileAsync(output);
        if (profile.Audio is not { } audio
            || AudioCodec(audio.Subtype) != "aac"
            || (audio.ChannelCount > 0 && audio.ChannelCount != asked.Channels))
        {
            Diagnostics.Write($"a re-encode came out as {profile.Audio?.Subtype ?? "no audio"}, {profile.Audio?.ChannelCount} channel(s)");
            return false;
        }
        return true;
    }

    /// <summary>
    /// Rule D, and what faststart could not do. A result bigger than a sendable source is thrown away; a result whose
    /// index could not be moved to the front does not go where the source can go instead; and a result over the ceiling
    /// is refused, as the source would have been. Otherwise the result is sent — the only thing that can be.
    /// </summary>
    private static async Task<PrepOutcome> ChosenAsync(
        Transcoded result, long size, bool sendable, Func<Task<PrepOutcome>> original, Func<byte[], StagedMedia> staged)
    {
        if (MediaPlan.KeepSmaller(size, sendable, result.Length) == Upload.Source)
        {
            Diagnostics.Write($"a transcode came out bigger than its source ({result.Length} > {size} bytes); the original goes instead");
            return await original();
        }
        if (!result.MoovFirst && sendable)
        {
            Diagnostics.Write("a transcode's index could not be moved to the front; the original goes instead");
            return await original();
        }
        if (result.Bytes is not { } bytes)
        {
            Diagnostics.Write($"a transcode is still over the ceiling ({result.Length} bytes)");
            return TooLarge;
        }
        if (!result.MoovFirst)
        {
            Diagnostics.Write("a transcode goes with its index at the end: the original could not have gone at all");
        }
        return PrepOutcome.Staged(staged(bytes));
    }

    /// <summary>
    /// <paramref name="source"/> through Media Foundation's transcoder, one profile after another until one is taken, into
    /// a file of its own that is gone again when this returns. Null for anything but a result that came out as asked: no
    /// profile taken, a transcode that threw or ran past its ceiling, one <paramref name="cameOut"/> does not recognise, or
    /// bytes the server's check would refuse.
    /// </summary>
    private static async Task<Transcoded?> TranscodeAsync<T>(
        StorageFile source,
        string extension,
        string mime,
        IReadOnlyList<T> attempts,
        Func<T, MediaEncodingProfile> profileFor,
        Func<StorageFile, T, Task<bool>> cameOut,
        long? durationMs)
    {
        var folder = Path.Combine(Path.GetTempPath(), "FamilyConnect", "prepared");
        var path = Path.Combine(folder, $"{Guid.NewGuid():N}{extension}");
        try
        {
            Directory.CreateDirectory(folder);
            SweepStale(folder);
            await File.WriteAllBytesAsync(path, []);
            var output = await StorageFile.GetFileFromPathAsync(path);
            foreach (var attempt in attempts)
            {
                var transcoder = new MediaTranscoder
                {
                    HardwareAccelerationEnabled = true,
                    VideoProcessingAlgorithm = MediaVideoProcessingAlgorithm.Default,
                };
                PrepareTranscodeResult prepared;
                try
                {
                    prepared = await transcoder.PrepareFileTranscodeAsync(source, output, profileFor(attempt));
                }
                catch (Exception e)
                {
                    Diagnostics.Write($"a transcode profile was refused: {e.GetType().Name} ({attempt})");
                    continue;
                }
                if (!prepared.CanTranscode)
                {
                    Diagnostics.Write($"a transcode profile was refused: {prepared.FailureReason} ({attempt})");
                    continue;
                }
                using (var ceiling = new CancellationTokenSource(MediaEncoding.TranscodeCeiling(durationMs)))
                {
                    await prepared.TranscodeAsync().AsTask(ceiling.Token);
                }
                GC.KeepAlive(transcoder);
                if (!await cameOut(output, attempt))
                {
                    return null;
                }
                var length = new FileInfo(path).Length;
                if (length > MediaPrep.SizeLimit)
                {
                    // Where its index is, nobody needs to know: rule D sends the source, or nothing.
                    return new Transcoded(length, null, MoovFirst: false);
                }
                var bytes = await File.ReadAllBytesAsync(path);
                if (!MediaPrep.MatchesMagic(mime, bytes))
                {
                    Diagnostics.Write("a transcode's bytes are not the type they are sent as");
                    return null;
                }
                var arranged = Faststart.MoovFirst(bytes);
                return new Transcoded(length, arranged ?? bytes, arranged is not null);
            }
            Diagnostics.Write("no transcode profile was taken");
            return null;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"transcoding: {e.GetType().Name}");
            return null;
        }
        finally
        {
            try
            {
                File.Delete(path);
            }
            catch (Exception e)
            {
                Diagnostics.Write($"removing a transcode: {e.GetType().Name}");
            }
        }
    }

    /// <summary>What an earlier run could not delete — a file still held open, a crash mid-transcode — gone after a day.</summary>
    private static void SweepStale(string folder)
    {
        foreach (var stale in Directory.EnumerateFiles(folder))
        {
            try
            {
                if (File.GetLastWriteTimeUtc(stale) < DateTime.UtcNow.AddDays(-1))
                {
                    File.Delete(stale);
                }
            }
            catch (Exception e)
            {
                Diagnostics.Write($"sweeping a transcode: {e.GetType().Name}");
            }
        }
    }

    /// <summary>The shell's numbers for a file's streams, or null when it has none to give.</summary>
    private static async Task<IDictionary<string, object>?> StatedAsync(StorageFile file)
    {
        try
        {
            return await file.Properties.RetrievePropertiesAsync(
                [ShellFrameRate, ShellVideoBitrate, ShellAudioBitrate, ShellAudioChannels, ShellAudioSampleRate]);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading a file's stated numbers: {e.GetType().Name}");
            return null;
        }
    }

    /// <summary>A video stream's codec as the planner names it. Only H.264 is ever asked about, so nothing rarer is named.</summary>
    private static string VideoCodec(string? subtype) =>
        Is(subtype, MediaEncodingSubtypes.H264) || Is(subtype, MediaEncodingSubtypes.H264Es) ? "h264"
        : Is(subtype, MediaEncodingSubtypes.Hevc) || Is(subtype, MediaEncodingSubtypes.HevcEs) ? "hevc"
        : "unknown";

    /// <summary>A sound stream's codec as the planner names it: WAV and AIFF are both PCM, integer or float.</summary>
    private static string AudioCodec(string? subtype) =>
        Is(subtype, MediaEncodingSubtypes.Aac) || Is(subtype, MediaEncodingSubtypes.AacAdts) ? "aac"
        : Is(subtype, MediaEncodingSubtypes.Mp3) ? "mp3"
        : Is(subtype, MediaEncodingSubtypes.Pcm) || Is(subtype, MediaEncodingSubtypes.Float) ? "pcm"
        : Is(subtype, MediaEncodingSubtypes.Flac) ? "flac"
        : Is(subtype, MediaEncodingSubtypes.Alac) ? "alac"
        : "unknown";

    private static bool Is(string? subtype, string name) => string.Equals(subtype, name, StringComparison.OrdinalIgnoreCase);

    /// <summary>A number from a property bag, whatever integer it was boxed as; null when absent or not a whole number.</summary>
    private static long? Number(IDictionary<Guid, object>? bag, Guid key) =>
        bag is not null && bag.TryGetValue(key, out var value) ? Number(value) : null;

    private static long? Number(IDictionary<string, object>? bag, string key) =>
        bag is not null && bag.TryGetValue(key, out var value) ? Number(value) : null;

    private static long? Number(object? value) => value switch
    {
        uint number => number,
        int number => number,
        ulong number => number <= long.MaxValue ? (long)number : null,
        long number => number,
        ushort number => number,
        short number => number,
        byte number => number,
        _ => null,
    };

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

    /// <summary>
    /// The picture, fitted within <paramref name="edge"/>, turned upright, drawn on white, as JPEG.
    /// </summary>
    private static async Task<(byte[] Bytes, int Width, int Height)> JpegAsync(BitmapDecoder decoder, uint edge, double quality)
    {
        var (width, height) = MediaPrep.FitWithin(decoder.OrientedPixelWidth, decoder.OrientedPixelHeight, edge);
        using var bitmap = await UprightAsync(decoder, width, height);
        var pixels = OnWhite(bitmap);
        var bytes = await EncodeAsync(pixels, (uint)bitmap.PixelWidth, (uint)bitmap.PixelHeight, quality);
        return (bytes, bitmap.PixelWidth, bitmap.PixelHeight);
    }

    /// <summary>
    /// A profile picture, as every client uploads one (<see cref="AvatarPrep"/>): the largest CENTRED
    /// square, at most 512 across, as the first JPEG quality that fits the byte budget — and the last
    /// one tried when none does, because a larger upload the server may still take beats no picture.
    /// Null when the file cannot be read as a picture.
    /// </summary>
    public static async Task<byte[]?> AvatarAsync(StorageFile file)
    {
        try
        {
            using var stream = await file.OpenReadAsync();
            var decoder = await BitmapDecoder.CreateAsync(stream);
            var shortest = Math.Min(decoder.OrientedPixelWidth, decoder.OrientedPixelHeight);
            if (shortest == 0)
            {
                return null;
            }
            // Scaled so the square's side lands on the edge — and never up.
            var scale = Math.Min(1.0, (double)AvatarPrep.Edge / shortest);
            var width = Math.Max(1u, (uint)Math.Round(decoder.OrientedPixelWidth * scale));
            var height = Math.Max(1u, (uint)Math.Round(decoder.OrientedPixelHeight * scale));
            using var bitmap = await UprightAsync(decoder, width, height);
            if (AvatarPrep.Square((uint)bitmap.PixelWidth, (uint)bitmap.PixelHeight) is not { } square)
            {
                return null;
            }
            var pixels = OnWhite(bitmap);
            // Rounding can leave the short side a pixel over the edge: the centre of it is kept.
            var side = square.Edge;
            var left = square.X + (square.Side - side) / 2;
            var top = square.Y + (square.Side - side) / 2;
            var cropped = new byte[4 * side * side];
            for (var row = 0u; row < side; row++)
            {
                System.Buffer.BlockCopy(
                    pixels, (int)(4 * (((top + row) * (uint)bitmap.PixelWidth) + left)),
                    cropped, (int)(4 * row * side), (int)(4 * side));
            }
            byte[]? last = null;
            foreach (var quality in AvatarPrep.Qualities)
            {
                var jpeg = await EncodeAsync(cropped, side, side, quality);
                if (jpeg.Length <= AvatarPrep.MaxBytes)
                {
                    return jpeg;
                }
                last = jpeg;
            }
            return last;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"preparing a profile picture: {e.GetType().Name}");
            return null;
        }
    }

    /// <summary>
    /// The picture at <paramref name="width"/> by <paramref name="height"/>, turned upright. Whether
    /// the scaler runs before the EXIF turn or after it, the answer's own size says which: asked in
    /// stored sides first, and asked again in upright sides if that came out turned.
    /// </summary>
    private static async Task<SoftwareBitmap> UprightAsync(BitmapDecoder decoder, uint width, uint height)
    {
        var turned = decoder.OrientedPixelWidth != decoder.PixelWidth;
        var bitmap = await DecodeAsync(decoder, turned ? height : width, turned ? width : height);
        if (bitmap.PixelWidth != width || bitmap.PixelHeight != height)
        {
            bitmap.Dispose();
            bitmap = await DecodeAsync(decoder, width, height);
        }
        return bitmap;
    }

    /// <summary>
    /// The pixels, drawn on white: a JPEG has no alpha, and a transparent pixel would otherwise come
    /// out black. Premultiplied, so "over white" is adding what the alpha left uncovered.
    /// </summary>
    private static byte[] OnWhite(SoftwareBitmap bitmap)
    {
        var pixels = new byte[4 * bitmap.PixelWidth * bitmap.PixelHeight];
        bitmap.CopyToBuffer(pixels.AsBuffer());
        for (var at = 0; at < pixels.Length; at += 4)
        {
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
        return pixels;
    }

    /// <summary>A new encoder, fed pixels only: nothing of the original's metadata comes along.</summary>
    private static async Task<byte[]> EncodeAsync(byte[] pixels, uint width, uint height, double quality)
    {
        using var output = new InMemoryRandomAccessStream();
        var options = new BitmapPropertySet
        {
            ["ImageQuality"] = new BitmapTypedValue((float)quality, Windows.Foundation.PropertyType.Single),
        };
        var encoder = await BitmapEncoder.CreateAsync(BitmapEncoder.JpegEncoderId, output, options);
        encoder.SetPixelData(BitmapPixelFormat.Bgra8, BitmapAlphaMode.Ignore, width, height, 96, 96, pixels);
        await encoder.FlushAsync();
        return await ReadAllAsync(output);
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
