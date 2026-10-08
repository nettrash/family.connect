using System.Runtime.InteropServices;
using Windows.Media;
using Windows.Media.Audio;
using Windows.Media.Capture;
using Windows.Media.Render;
using WinRT;

namespace FamilyConnect.App.Services;

/// <summary>
/// The recording row's level, LIVE (the approved design: "a live waveform scrolls in from the right"): Windows'
/// <c>MediaCapture</c> recording exposes no level (docs/audio-video-messages-2026-10-04.md, S2.9), so a second, listen-only
/// reader of the same microphone — an <c>AudioGraph</c> in shared mode, its frames read and dropped — measures the loudest
/// sample between two reads. What it hears is drawn and forgotten: it writes nothing, keeps nothing, and the note's own
/// waveform is measured from the RECORDING (<see cref="VoiceShape"/>), never from this.
/// </summary>
/// <remarks>
/// <para>
/// <b>BEST EFFORT, AND SEPARATE.</b> It starts after the recording has, and a graph Windows will not make — no render device
/// to clock it, the microphone refused to a second reader — is no meter: the row draws the dot and the clock, as it did. It
/// is never what decides whether a recording starts, runs or is kept.
/// </para>
/// <para>
/// <b>LET GO OF WITH THE RECORDING</b>, by <see cref="VoiceRecorder"/>'s own stop and dispose, so the microphone is never kept
/// open by the meter after the note has ended.
/// </para>
/// <para>
/// <b>NOT RUN.</b> That a packaged app may open the microphone twice in shared mode beside a <c>LowLagMediaRecording</c>,
/// and that the frames come as 32-bit float, are to be seen on Windows.
/// </para>
/// </remarks>
internal sealed class VoiceMeter : IDisposable
{
    [ComImport]
    [Guid("5B0D3235-4DBA-4D44-865E-8F1D0E4FD04D")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IMemoryBufferByteAccess
    {
        void GetBuffer(out IntPtr buffer, out uint capacity);
    }

    private readonly object gate = new();
    private AudioGraph? graph;
    private AudioDeviceInputNode? input;
    private AudioFrameOutputNode? output;
    private float loudest;
    private bool heard;
    private volatile bool broken;
    private float[] samples = new float[4096];

    private VoiceMeter()
    {
    }

    /// <summary>A meter on the default microphone, running — or null where Windows will not make one.</summary>
    public static async Task<VoiceMeter?> StartAsync()
    {
        var meter = new VoiceMeter();
        try
        {
            var made = await AudioGraph.CreateAsync(new AudioGraphSettings(AudioRenderCategory.Other));
            if (made.Status != AudioGraphCreationStatus.Success)
            {
                Diagnostics.Write($"no level meter: the graph was {made.Status}");
                return null;
            }
            meter.graph = made.Graph;
            var device = await meter.graph.CreateDeviceInputNodeAsync(MediaCategory.Other);
            if (device.Status != AudioDeviceNodeCreationStatus.Success)
            {
                Diagnostics.Write($"no level meter: the microphone node was {device.Status}");
                meter.Dispose();
                return null;
            }
            meter.input = device.DeviceInputNode;
            meter.output = meter.graph.CreateFrameOutputNode();
            meter.input.AddOutgoingConnection(meter.output);
            meter.graph.QuantumStarted += meter.OnQuantum;
            meter.graph.Start();
            return meter;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"no level meter: {e.GetType().Name} 0x{e.HResult:X8}");
            meter.Dispose();
            return null;
        }
    }

    /// <summary>
    /// The loudest sample since the last read, in dBFS — −∞ for silence — or null when nothing has been heard since: the row
    /// then adds no bar rather than a silent one.
    /// </summary>
    public double? TakePeakDbfs()
    {
        lock (gate)
        {
            if (!heard)
            {
                return null;
            }
            var peak = loudest;
            loudest = 0;
            heard = false;
            return FamilyConnect.App.Logic.VoiceWaveform.Dbfs(peak);
        }
    }

    /// <summary>One quantum of the graph, on its own thread: the frame's loudest sample, and the frame dropped.</summary>
    private void OnQuantum(AudioGraph sender, object args)
    {
        try
        {
            if (broken || output is not { } node)
            {
                return;
            }
            using var frame = node.GetFrame();
            using var buffer = frame.LockBuffer(AudioBufferAccessMode.Read);
            using var reference = buffer.CreateReference();
            reference.As<IMemoryBufferByteAccess>().GetBuffer(out var data, out _);
            var count = (int)(buffer.Length / sizeof(float));
            if (count <= 0 || data == IntPtr.Zero)
            {
                return;
            }
            if (samples.Length < count)
            {
                samples = new float[count];
            }
            Marshal.Copy(data, samples, 0, count);
            var peak = 0f;
            for (var i = 0; i < count; i++)
            {
                var magnitude = Math.Abs(samples[i]);
                if (magnitude > peak)
                {
                    peak = magnitude;
                }
            }
            lock (gate)
            {
                loudest = Math.Max(loudest, peak);
                heard = true;
            }
        }
        catch (Exception e)
        {
            // A frame that cannot be read is a bar not drawn; nothing about the recording depends on it. Said once and given
            // up on: a quantum is ten milliseconds, and the same failure every one of them would bury the log.
            broken = true;
            Diagnostics.Write($"reading the level meter: {e.GetType().Name} 0x{e.HResult:X8}");
        }
    }

    public void Dispose()
    {
        try
        {
            if (graph is { } running)
            {
                running.QuantumStarted -= OnQuantum;
                running.Stop();
            }
            input?.Dispose();
            output?.Dispose();
            graph?.Dispose();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"letting go of the level meter: {e.GetType().Name}");
        }
        finally
        {
            input = null;
            output = null;
            graph = null;
        }
    }
}
