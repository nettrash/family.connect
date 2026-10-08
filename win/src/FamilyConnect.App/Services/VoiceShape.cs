using FamilyConnect.App.Logic;
using Windows.Media.MediaProperties;
using Windows.Media.Transcoding;
using Windows.Storage;

namespace FamilyConnect.App.Services;

/// <summary>
/// A voice note's waveform (docs/protocol.md, "A voice note's waveform"), measured from the note ITSELF: Windows'
/// <c>MediaCapture</c> exposes no level while it records, so the M4A is decoded to 16-bit mono PCM by Media Foundation —
/// the same transcoder a send's video goes through — and <see cref="VoiceWaveform"/> takes its peak every tenth of a
/// second. The same bytes give the same 48 digits however often they are measured, so a note sent at once, one sent from
/// review and one sent from its not-sent row all go up with the same shape.
/// </summary>
/// <remarks>
/// <para>
/// <b>NEVER WORTH A NOTE.</b> Anything that goes wrong — a codec this machine lacks, a transcode refused, too long — is a note
/// WITHOUT a waveform, which every reader draws as the neutral placeholder; it is never a note that does not go.
/// </para>
/// <para>
/// <b>A COPY WITH A NAME MEDIA FOUNDATION READS</b>, as <see cref="SoundTracks"/> writes one: it picks its reader by the
/// extension. Both files live beside the transcoder's own and are gone when this returns. Nothing but an exception's type
/// is logged — the bytes are somebody's voice.
/// </para>
/// <para>
/// <b>NOT RUN.</b> Nothing on the Mac decodes through Media Foundation: that the WAV sink takes this profile, and that the
/// waveform it gives looks like the recording, are to be seen on Windows. The arithmetic after the decode is tested.
/// </para>
/// </remarks>
internal static class VoiceShape
{
    /// <summary>How long a measure may take before the note goes without one: a five-minute note decodes in well under it.</summary>
    private static readonly TimeSpan Ceiling = TimeSpan.FromSeconds(15);

    /// <summary>The 48 digits for these M4A bytes, or null — never an exception.</summary>
    public static async Task<string?> MeasureAsync(ReadOnlyMemory<byte> m4a, CancellationToken cancel = default)
    {
        if (m4a.IsEmpty)
        {
            return null;
        }
        var folder = Path.Combine(Path.GetTempPath(), "FamilyConnect", "prepared");
        var name = Guid.NewGuid().ToString("N");
        var input = Path.Combine(folder, $"{name}.m4a");
        var output = Path.Combine(folder, $"{name}.wav");
        using var limit = CancellationTokenSource.CreateLinkedTokenSource(cancel);
        limit.CancelAfter(Ceiling);
        try
        {
            Directory.CreateDirectory(folder);
            await File.WriteAllBytesAsync(input, m4a.ToArray(), limit.Token);
            await File.WriteAllBytesAsync(output, [], limit.Token);
            var source = await StorageFile.GetFileFromPathAsync(input);
            var target = await StorageFile.GetFileFromPathAsync(output);
            var profile = MediaEncodingProfile.CreateWav(AudioEncodingQuality.Low);
            // Mono, 16 kHz, 16-bit: a peak per tenth of a second needs nothing finer, and it keeps the file small.
            profile.Audio = AudioEncodingProperties.CreatePcm(16_000, 1, 16);
            profile.Video = null;
            var transcoder = new MediaTranscoder();
            var prepared = await transcoder.PrepareFileTranscodeAsync(source, target, profile).AsTask(limit.Token);
            if (!prepared.CanTranscode)
            {
                Diagnostics.Write($"a voice note's waveform: the decode was refused ({prepared.FailureReason})");
                return null;
            }
            await prepared.TranscodeAsync().AsTask(limit.Token);
            GC.KeepAlive(transcoder);
            var wav = await File.ReadAllBytesAsync(output, limit.Token);
            return VoiceWaveform.FromWav(wav);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"measuring a voice note's waveform: {e.GetType().Name}");
            return null;
        }
        finally
        {
            foreach (var path in new[] { input, output })
            {
                try
                {
                    File.Delete(path);
                }
                catch (Exception e)
                {
                    Diagnostics.Write($"removing a voice note's decode: {e.GetType().Name}");
                }
            }
        }
    }

    /// <summary>The note with its shape when one could be measured, and as it was otherwise.</summary>
    public static async Task<StagedMedia> ShapedAsync(StagedMedia note, CancellationToken cancel = default) =>
        note.Kind != "audio" || note.Waveform is not null
            ? note
            : await MeasureAsync(note.Bytes, cancel) is { } waveform ? note with { Waveform = waveform } : note;
}
