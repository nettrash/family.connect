using FamilyConnect.App.Logic;
using Windows.Media.Capture;
using Windows.Media.MediaProperties;
using Windows.Storage.Streams;

namespace FamilyConnect.App.Services;

/// <summary>Why a recording did not start, so the composer can say it rather than leave a dead button.</summary>
internal enum RecordingFailure
{
    MicrophoneDenied,
    CouldNotStart,
}

/// <summary>
/// A voice note in progress: the microphone into an M4A — AAC in MP4, the container <c>kind=audio</c> checks and every family
/// phone plays — held in memory until it stops (docs/protocol.md, "Audio").
/// </summary>
/// <remarks>
/// <para>
/// <b>HOWEVER IT ENDS — stopped, cancelled, or the window going — THE MICROPHONE IS LET GO OF.</b> A live microphone nothing
/// can reach is a microphone left on.
/// </para>
/// <para>
/// <b>RECORDED TO THE PROTOCOL'S VOICE NOTE, WHERE THIS MACHINE'S ENCODER TAKES IT</b> ("Preparing media before upload"):
/// AAC-LC, mono, 44.1 kHz, 64 000 bit/s — not <c>AudioEncodingQuality.Auto</c>, whose numbers were whatever Windows chose.
/// Media Foundation's AAC encoder documents 96 000 as its lowest rate, so where it refuses 64 000 the note is recorded at
/// that instead (<see cref="MediaEncoding.VoiceNoteAttempts"/>), and where it refuses both, as 1.1 recorded it: a voice
/// note that cannot start is worse than one a little larger than the protocol's. The first of the protocol's two this
/// machine takes is remembered for the run, so only the first recording pays for asking — but 1.1's way is NEVER
/// remembered: it is outside the profile, and one start that failed for a reason of its own must not pin every later
/// note of the run to it.
/// </para>
/// <para>
/// <b>WHAT WAS RECORDED IS READ BACK, AND SAID WHEN IT IS NOT WHAT WAS ASKED FOR.</b> A recording cannot be refused
/// the way a transcode can — there is no original to send instead, and the moment is gone — so a capture that took the
/// profile and wrote other numbers is sent as it is. It goes into the diagnostics log, so that it is at least known.
/// </para>
/// </remarks>
internal sealed class VoiceRecorder : IAsyncDisposable
{
    /// <summary>Where in <see cref="Encodes"/> to begin: the first of the protocol's this machine took, once one has been.</summary>
    private static int firstTaken;

    /// <summary>What this recording was asked for, to read it back against; null when it was 1.1's <c>Auto</c>.</summary>
    private AacEncoding? asked;

    private MediaCapture? capture;
    private LowLagMediaRecording? recording;
    private InMemoryRandomAccessStream? stream;
    private DateTimeOffset started;

    private VoiceRecorder()
    {
    }

    /// <summary>How long it has been going.</summary>
    public TimeSpan Elapsed => DateTimeOffset.UtcNow - started;

    /// <summary>
    /// The encodes a voice note may be recorded with, in order: the protocol's, the nearest the encoder documents, and —
    /// the null at the end — what 1.1 recorded with, whose numbers are whatever Windows chooses.
    /// </summary>
    private static IReadOnlyList<AacEncoding?> Encodes { get; } =
        [.. MediaEncoding.VoiceNoteAttempts().Select(aac => (AacEncoding?)aac), null];

    /// <summary>Ask for the microphone and start. Windows asks the person the first time; a refusal is said out loud.</summary>
    public static async Task<(VoiceRecorder? Recorder, RecordingFailure? Failure)> StartAsync()
    {
        for (var at = Math.Min(firstTaken, Encodes.Count - 1); at < Encodes.Count; at++)
        {
            var encode = Encodes[at];
            // A fresh capture for every encode tried: one that refused a profile is not trusted with the next.
            var recorder = new VoiceRecorder { capture = new MediaCapture(), stream = new InMemoryRandomAccessStream() };
            try
            {
                await recorder.capture.InitializeAsync(new MediaCaptureInitializationSettings
                {
                    StreamingCaptureMode = StreamingCaptureMode.Audio,
                    MediaCategory = MediaCategory.Speech,
                });
            }
            catch (UnauthorizedAccessException)
            {
                await recorder.DisposeAsync();
                return (null, RecordingFailure.MicrophoneDenied);
            }
            catch (Exception e)
            {
                Diagnostics.Write($"opening the microphone: {e.GetType().Name}");
                await recorder.DisposeAsync();
                return (null, RecordingFailure.CouldNotStart);
            }
            try
            {
                var profile = encode is { } aac ? MediaPreparing.M4aProfile(aac) : MediaEncodingProfile.CreateM4a(AudioEncodingQuality.Auto);
                recorder.recording = await recorder.capture.PrepareLowLagRecordToStreamAsync(profile, recorder.stream);
                await recorder.recording.StartAsync();
                recorder.started = DateTimeOffset.UtcNow;
                recorder.asked = encode;
                if (encode is not null)
                {
                    firstTaken = at;
                }
                else
                {
                    Diagnostics.Write("a voice note is being recorded as 1.1 recorded it: this machine took neither of the protocol's encodes");
                }
                return (recorder, null);
            }
            catch (Exception e)
            {
                Diagnostics.Write($"starting a recording with encode {at}: {e.GetType().Name}");
                await recorder.DisposeAsync();
            }
        }
        return (null, RecordingFailure.CouldNotStart);
    }

    /// <summary>Stop, and hand over what was recorded and how long it ran — or null when nothing could be read back.</summary>
    public async Task<(byte[] Bytes, TimeSpan Elapsed)?> StopAsync()
    {
        var elapsed = Elapsed;
        try
        {
            if (recording is { } running)
            {
                recording = null;
                await running.StopAsync();
                await running.FinishAsync();
            }
            if (stream is not { Size: > 0 } recorded)
            {
                return null;
            }
            var bytes = new byte[recorded.Size];
            using (var reader = new DataReader(recorded.GetInputStreamAt(0)))
            {
                await reader.LoadAsync((uint)recorded.Size);
                reader.ReadBytes(bytes);
            }
            await SayIfNotAsAskedAsync(recorded);
            return (bytes, elapsed);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"finishing a recording: {e.GetType().Name}");
            return null;
        }
        finally
        {
            await DisposeAsync();
        }
    }

    /// <summary>
    /// The recording read back as a transcode is: AAC, the channels asked for, the rate asked for. It is only SAID —
    /// the note is sent either way — and a recording nothing can read back is not worth losing the note over.
    /// </summary>
    private async Task SayIfNotAsAskedAsync(IRandomAccessStream recorded)
    {
        if (asked is not { } encode)
        {
            return;
        }
        try
        {
            recorded.Seek(0);
            var audio = (await MediaEncodingProfile.CreateFromStreamAsync(recorded)).Audio;
            if (audio is null
                || !MediaEncoding.AacCameOut(encode, MediaPreparing.AudioCodec(audio.Subtype), audio.ChannelCount)
                || !MediaEncoding.AudioRateCameOut(encode.Bitrate, audio.Bitrate))
            {
                Diagnostics.Write(
                    $"a voice note came out as {audio?.Subtype ?? "no audio"}, {audio?.ChannelCount} channel(s), {audio?.Bitrate} bit/s; asked {encode}");
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading a voice note back: {e.GetType().Name}");
        }
    }

    public async ValueTask DisposeAsync()
    {
        if (recording is { } running)
        {
            recording = null;
            try
            {
                await running.StopAsync();
                await running.FinishAsync();
            }
            catch (Exception e)
            {
                Diagnostics.Write($"abandoning a recording: {e.GetType().Name}");
            }
        }
        capture?.Dispose();
        capture = null;
        stream?.Dispose();
        stream = null;
    }
}
