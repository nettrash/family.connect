using System.Diagnostics;
using FamilyConnect.App.Logic;
using Windows.Media.Capture;
using Windows.Media.MediaProperties;
using Windows.Security.Authorization.AppCapabilityAccess;
using Windows.Storage.Streams;

namespace FamilyConnect.App.Services;

/// <summary>Why a recording did not start, so the composer can say it rather than leave a dead button.</summary>
internal enum RecordingFailure
{
    MicrophoneDenied,
    CouldNotStart,
}

/// <summary>What a stopped recording handed over: how long it ran, and its bytes — null when nothing could be read back.</summary>
internal sealed record Recorded(byte[]? Bytes, TimeSpan Elapsed);

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
/// <b>ITS CLOCK IS MONOTONIC</b> (docs/audio-video-messages-2026-10-04.md, S2.9): a wall clock moved while somebody
/// talks would move the length a recording is kept, sent and asked about with.
/// </para>
/// <para>
/// <b>A RECORDER THAT FAILS SAYS SO</b> (<see cref="Failed"/>): the microphone pulled out or taken, the system's own
/// limit — and the composer keeps what can be read back as a voice message that was not sent (S4).
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
    private readonly Stopwatch clock = new();

    private VoiceRecorder()
    {
    }

    /// <summary>
    /// The recording stopped on its own: Windows' <c>MediaCapture</c> failed or reached its own limit. Raised on whatever
    /// thread Windows raised it on, at most once.
    /// </summary>
    public event Action? Failed;

    /// <summary>How long it has been going, by a monotonic clock.</summary>
    public TimeSpan Elapsed => clock.Elapsed;

    /// <summary>
    /// What Windows says about this app and the microphone (<c>AppCapability.CheckAccess</c>, S2.2) — asked after a
    /// refusal, to know whether the Settings page is worth offering.
    /// </summary>
    public static CapabilityAccess MicrophoneAccess()
    {
        try
        {
            // The manifest's own spelling of the capability: <DeviceCapability Name="microphone" />.
            return AppCapability.Create("microphone").CheckAccess() switch
            {
                AppCapabilityAccessStatus.Allowed => CapabilityAccess.Allowed,
                AppCapabilityAccessStatus.UserPromptRequired => CapabilityAccess.UserPromptRequired,
                AppCapabilityAccessStatus.DeniedByUser => CapabilityAccess.DeniedByUser,
                AppCapabilityAccessStatus.DeniedBySystem => CapabilityAccess.DeniedBySystem,
                AppCapabilityAccessStatus.NotDeclaredByApp => CapabilityAccess.NotDeclaredByApp,
                _ => CapabilityAccess.Unknown,
            };
        }
        catch (Exception e)
        {
            Diagnostics.Write($"asking about the microphone: {e.GetType().Name} 0x{e.HResult:X8}");
            return CapabilityAccess.Unknown;
        }
    }

    /// <summary>
    /// The encodes a voice note may be recorded with, in order: the protocol's, the nearest the encoder documents, and —
    /// the null at the end — what 1.1 recorded with, whose numbers are whatever Windows chooses.
    /// </summary>
    private static IReadOnlyList<AacEncoding?> Encodes { get; } =
        [.. MediaEncoding.VoiceNoteAttempts().Select(aac => (AacEncoding?)aac), null];

    /// <summary>Ask for the microphone and start. Windows asks the person the first time; a refusal is said out loud.</summary>
    /// <param name="speakFirst">
    /// Run once the microphone is open and BEFORE anything is recorded, and awaited: with a screen reader running, "Recording"
    /// is said and a second passes (docs/audio-video-messages-2026-10-04.md, S6), so the app's own voice is not the first
    /// thing in the note and the clock starts with the recording. Windows' own question, the first time, comes before it.
    /// </param>
    public static async Task<(VoiceRecorder? Recorder, RecordingFailure? Failure)> StartAsync(Func<Task>? speakFirst = null)
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
            if (speakFirst is { } speak)
            {
                // Once, however many encodes are tried after it.
                speakFirst = null;
                try
                {
                    await speak();
                }
                catch (Exception e)
                {
                    Diagnostics.Write($"before recording: {e.GetType().Name}");
                }
            }
            try
            {
                var profile = encode is { } aac ? MediaPreparing.M4aProfile(aac) : MediaEncodingProfile.CreateM4a(AudioEncodingQuality.Auto);
                recorder.recording = await recorder.capture.PrepareLowLagRecordToStreamAsync(profile, recorder.stream);
                await recorder.recording.StartAsync();
                recorder.clock.Start();
                recorder.asked = encode;
                recorder.capture.Failed += recorder.OnCaptureFailed;
                recorder.capture.RecordLimitationExceeded += recorder.OnLimitReached;
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

    /// <summary>
    /// Stop, and hand over how long it ran and what was recorded — the bytes null when nothing could be read back. The
    /// length is ALWAYS there: it is what decides whether a recording that could not be read back was worth saying so.
    /// </summary>
    public async Task<Recorded> StopAsync()
    {
        var elapsed = Elapsed;
        clock.Stop();
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
                return new Recorded(null, elapsed);
            }
            var bytes = new byte[recorded.Size];
            using (var reader = new DataReader(recorded.GetInputStreamAt(0)))
            {
                await reader.LoadAsync((uint)recorded.Size);
                reader.ReadBytes(bytes);
            }
            await SayIfNotAsAskedAsync(recorded);
            return new Recorded(bytes, elapsed);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"finishing a recording: {e.GetType().Name}");
            return new Recorded(null, elapsed);
        }
        finally
        {
            await DisposeAsync();
        }
    }

    private void OnCaptureFailed(MediaCapture sender, MediaCaptureFailedEventArgs args)
    {
        // The code only: the message is Windows' words, and may name the device.
        Diagnostics.Write($"the microphone failed while recording: 0x{args.Code:X8}");
        RaiseFailed();
    }

    private void OnLimitReached(MediaCapture sender)
    {
        Diagnostics.Write("a recording reached Windows' own limit");
        RaiseFailed();
    }

    private void RaiseFailed()
    {
        // Once: a failure followed by the limit, or the other way round, is one recording that stopped.
        if (Interlocked.Exchange(ref Failed, null) is { } failed)
        {
            failed();
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
        clock.Stop();
        Failed = null;
        if (capture is { } owned)
        {
            owned.Failed -= OnCaptureFailed;
            owned.RecordLimitationExceeded -= OnLimitReached;
        }
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
