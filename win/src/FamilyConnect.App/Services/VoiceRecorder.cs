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
/// <b>HOWEVER IT ENDS — stopped, cancelled, or the window going — THE MICROPHONE IS LET GO OF.</b> A live microphone nothing
/// can reach is a microphone left on.
/// </remarks>
internal sealed class VoiceRecorder : IAsyncDisposable
{
    private MediaCapture? capture;
    private LowLagMediaRecording? recording;
    private InMemoryRandomAccessStream? stream;
    private DateTimeOffset started;

    private VoiceRecorder()
    {
    }

    /// <summary>How long it has been going.</summary>
    public TimeSpan Elapsed => DateTimeOffset.UtcNow - started;

    /// <summary>Ask for the microphone and start. Windows asks the person the first time; a refusal is said out loud.</summary>
    public static async Task<(VoiceRecorder? Recorder, RecordingFailure? Failure)> StartAsync()
    {
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
            recorder.recording = await recorder.capture.PrepareLowLagRecordToStreamAsync(
                MediaEncodingProfile.CreateM4a(AudioEncodingQuality.Auto), recorder.stream);
            await recorder.recording.StartAsync();
            recorder.started = DateTimeOffset.UtcNow;
            return (recorder, null);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"starting a recording: {e.GetType().Name}");
            await recorder.DisposeAsync();
            return (null, RecordingFailure.CouldNotStart);
        }
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
