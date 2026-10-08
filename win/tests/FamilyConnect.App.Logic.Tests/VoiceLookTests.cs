using System.Buffers.Binary;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// A voice message as the approved design of 2026-10-05 draws it — the waveform the sender measured (or a neutral one), the
/// played bars, the time, the dot, the speed chip and what a screen reader hears — and the waveform a Windows recording is
/// given, measured from its own decoded sound.
/// </summary>
public sealed class VoiceLookTests
{
    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    private const string Wire = "0123456789abcdef0123456789abcdef0123456789abcdef";

    private static readonly AttachmentDto Note = new(40, "audio", "audio/mp4", 9000, DurationMs: 42_400, Waveform: Wire);

    [Fact]
    public void TheBubbleHoldsFortyTwoBarsOfTheSendersWaveform()
    {
        Assert.Equal(210, VoiceLook.WaveWidth);
        Assert.Equal(42, VoiceLook.BarCount(VoiceLook.WaveWidth));
        Assert.Equal(48, VoiceLook.BarCount(48 * 5 - 2));
        Assert.Equal(0, VoiceLook.BarCount(2));
        Assert.Equal(0, VoiceLook.BarCount(double.NaN));
        var bars = VoiceLook.Bars(Note.Waveform, 46);
        Assert.Equal(46, bars.Length);
        Assert.Equal(Waveform.Bars(Waveform.Parse(Wire)!, 46), bars);
    }

    /// <summary>A picked sound file, an older client's note, a malformed field: the neutral shape, never an invented one.</summary>
    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF")]
    [InlineData("0123")]
    public void NoWaveformIsTheNeutralPlaceholder(string? waveform)
    {
        var bars = VoiceLook.Bars(waveform, 46);
        Assert.Equal(46, bars.Length);
        Assert.All(bars, level => Assert.Equal(Waveform.PlaceholderLevel, level));
    }

    [Fact]
    public void ABarIsTwoSeventeenthsAtSilenceAndWholeAtFullScale()
    {
        Assert.Equal(28 * 2 / 17.0, VoiceLook.BarHeight(0, 28), 9);
        Assert.Equal(28, VoiceLook.BarHeight(15, 28), 9);
        Assert.Equal(0, VoiceLook.BarHeight(15, -3));
    }

    /// <summary>The played bars follow the shared rule on whole milliseconds; an unknown length lights none.</summary>
    [Fact]
    public void PlayedBarsAreTheSharedRule()
    {
        Assert.Equal(0, VoiceLook.PlayedBars(0, 42.4, 46));
        Assert.Equal(23, VoiceLook.PlayedBars(21.2, 42.4, 46));
        Assert.Equal(46, VoiceLook.PlayedBars(42.4, 42.4, 46));
        Assert.Equal(46, VoiceLook.PlayedBars(99, 42.4, 46));
        Assert.Equal(0, VoiceLook.PlayedBars(-3, 42.4, 46));
        Assert.Equal(0, VoiceLook.PlayedBars(5, 0, 46));
        Assert.Equal(0, VoiceLook.PlayedBars(double.NaN, 42.4, 46));
        Assert.Equal(Waveform.PlayedBars(14_199, 14_200, 44), VoiceLook.PlayedBars(14.199, 14.2, 44));
    }

    /// <summary>The time counts up while it plays (or is paused part way) and is the whole length at rest.</summary>
    [Fact]
    public void TheTimeIsWhereItIsWhilePlayingAndTheLengthAtRest()
    {
        Assert.Equal("0:42", VoiceLook.Time(active: false, 12, 42.4));
        Assert.Equal("0:12", VoiceLook.Time(active: true, 12.2, 42.4));
        Assert.Equal("0:42", VoiceLook.Time(active: true, 99, 42.4));
        Assert.Equal("0:00", VoiceLook.Time(active: true, -1, 42.4));
    }

    [Theory]
    [InlineData(false, false, true, "Not played")]
    [InlineData(false, true, false, "Played")]
    [InlineData(true, false, false, null)]
    [InlineData(true, true, false, null)]
    public void TheDotAndItsStatusAreOnlyOnSomeoneElses(bool mine, bool played, bool dot, string? status)
    {
        Assert.Equal(dot, VoiceLook.ShowsDot(mine, played));
        Assert.Equal(status, VoiceLook.Status(mine, played, Say));
    }

    [Fact]
    public void AScreenReaderHearsWhatItIsAndHowLong()
    {
        Assert.Equal("Voice message, 0:42", VoiceLook.Name(Note, Say));
        Assert.Equal("Voice message, 0:00", VoiceLook.Name(Note with { DurationMs = null }, Say));
        Assert.True(VoiceLook.ShowsSpeed(active: true));
        Assert.False(VoiceLook.ShowsSpeed(active: false));
    }

    /// <summary>1× → 1.5× → 2× → 1×, stored invariantly on the device, and anything unreadable is 1×.</summary>
    [Fact]
    public void TheSpeedGoesRoundAndIsRememberedInvariantly()
    {
        Assert.Equal(1.5, VoiceSpeed.Next(1.0));
        Assert.Equal(2.0, VoiceSpeed.Next(1.5));
        Assert.Equal(1.0, VoiceSpeed.Next(2.0));
        Assert.Equal(1.0, VoiceSpeed.Next(3.0));
        Assert.Equal("1×", VoiceSpeed.Label(1.0, Say));
        Assert.Equal("1.5×", VoiceSpeed.Label(1.5, Say));
        Assert.Equal("2×", VoiceSpeed.Label(2.0, Say));
        Assert.Equal("1×", VoiceSpeed.Label(0.75, Say));
        Assert.Equal("Playback speed, 1.5×", VoiceSpeed.Name(1.5, Say));
        Assert.Equal("1.5", VoiceSpeed.Store(1.5));
        Assert.Equal("2", VoiceSpeed.Store(2.0));
        Assert.Equal(1.5, VoiceSpeed.Parse("1.5"));
        Assert.Equal(2.0, VoiceSpeed.Parse(" 2\n"));
        Assert.Equal(1.0, VoiceSpeed.Parse("1,5"));
        Assert.Equal(1.0, VoiceSpeed.Parse("3"));
        Assert.Equal(1.0, VoiceSpeed.Parse(null));
        Assert.Equal(1.0, VoiceSpeed.Parse("fast"));
    }

    /// <summary>The live line grows in from the right, the newest bar last, and the oldest falls off the left.</summary>
    [Fact]
    public void TheLiveWaveformScrollsInFromTheRight()
    {
        var live = new LiveLevels(4);
        Assert.Equal([0, 0, 0], live.Bars(3));
        live.Push(-60);
        live.Push(0);
        Assert.Equal([0, 0, 15], live.Bars(3));
        Assert.Equal([0, 0, 0, 0, 15], live.Bars(5));
        live.Push(-30);
        live.Push(-58);
        live.Push(double.NegativeInfinity);
        Assert.Equal(5, live.Heard);
        Assert.Equal([15, 8, 1, 0], live.Bars(4));
        Assert.Equal([1, 0], live.Bars(2));
        live.Clear();
        Assert.Equal(0, live.Heard);
        Assert.Equal([0, 0], live.Bars(2));
    }

    // ---- the waveform a Windows recording is given ------------------------------------------------------------------

    private static byte[] Wav(short[] samples, int rate = 1000, int channels = 1)
    {
        var data = new byte[samples.Length * 2];
        for (var i = 0; i < samples.Length; i++)
        {
            BinaryPrimitives.WriteInt16LittleEndian(data.AsSpan(i * 2), samples[i]);
        }
        return Riff(1, 16, rate, channels, data);
    }

    private static byte[] Riff(int format, int bits, int rate, int channels, byte[] data, bool listFirst = false)
    {
        using var buffer = new MemoryStream();
        using var writer = new BinaryWriter(buffer);
        writer.Write("RIFF"u8);
        writer.Write(0);
        writer.Write("WAVE"u8);
        writer.Write("fmt "u8);
        writer.Write(16);
        writer.Write((short)format);
        writer.Write((short)channels);
        writer.Write(rate);
        writer.Write(rate * channels * bits / 8);
        writer.Write((short)(channels * bits / 8));
        writer.Write((short)bits);
        if (listFirst)
        {
            // An odd-sized chunk before the data, padded to even as RIFF requires.
            writer.Write("LIST"u8);
            writer.Write(3);
            writer.Write("abc"u8);
            writer.Write((byte)0);
        }
        writer.Write("data"u8);
        writer.Write(data.Length);
        writer.Write(data);
        writer.Flush();
        return buffer.ToArray();
    }

    /// <summary>
    /// One peak per tenth of a second, the loudest sample of either channel, in dBFS — then the shared rule. At 1 000 samples
    /// a second a block is 100 samples: full scale, half scale (−6.02 dB: x = 13.49, level 13), and silence.
    /// </summary>
    [Fact]
    public void ARecordingsPeaksAreItsLoudestSamplePerTenthOfASecond()
    {
        var samples = new short[300];
        samples[10] = short.MinValue;
        samples[150] = 16384;
        samples[151] = -100;
        var peaks = VoiceWaveform.Peaks(Wav(samples))!;
        Assert.Equal(3, peaks.Count);
        Assert.Equal(0, peaks[0], 9);
        Assert.Equal(20 * Math.Log10(0.5), peaks[1], 9);
        Assert.Equal(double.NegativeInfinity, peaks[2]);
        Assert.Equal(Waveform.FromPeaks(peaks), VoiceWaveform.FromWav(Wav(samples)));
        Assert.Equal(new string('f', 16) + new string('d', 16) + new string('0', 16), VoiceWaveform.FromWav(Wav(samples)));
    }

    /// <summary>A block is exactly a tenth of a second: the first sample of the next one is never counted in this one.</summary>
    [Fact]
    public void ABlockEndsExactlyAtATenthOfASecond()
    {
        var samples = new short[200];
        samples[100] = 16384;
        var peaks = VoiceWaveform.Peaks(Wav(samples))!;
        Assert.Equal(2, peaks.Count);
        Assert.Equal(double.NegativeInfinity, peaks[0]);
        Assert.Equal(20 * Math.Log10(0.5), peaks[1], 9);
        // At 16 kHz, the rate the decode asks for, a block is 1 600 frames.
        var sixteen = new short[3200];
        sixteen[1600] = 32767;
        Assert.Equal(double.NegativeInfinity, VoiceWaveform.Peaks(Wav(sixteen, rate: 16_000))![0]);
    }

    [Fact]
    public void ATailShorterThanABlockIsAPeakOfItsOwnAndStereoTakesEitherChannel()
    {
        // Two channels, 150 frames: a block of 100 and a tail of 50; the right channel is the loud one.
        var samples = new short[300];
        samples[2 * 120 + 1] = 32767;
        var peaks = VoiceWaveform.Peaks(Wav(samples, channels: 2))!;
        Assert.Equal(2, peaks.Count);
        Assert.Equal(double.NegativeInfinity, peaks[0]);
        Assert.Equal(20 * Math.Log10(32767 / 32768.0), peaks[1], 9);
    }

    [Fact]
    public void FloatSamplesAndChunksBeforeTheDataAreRead()
    {
        var data = new byte[200 * 4];
        BinaryPrimitives.WriteSingleLittleEndian(data.AsSpan(4 * 5), -0.25f);
        BinaryPrimitives.WriteSingleLittleEndian(data.AsSpan(4 * 150), 2.0f);
        var peaks = VoiceWaveform.Peaks(Riff(3, 32, 1000, 1, data, listFirst: true))!;
        Assert.Equal(2, peaks.Count);
        Assert.Equal(20 * Math.Log10(0.25), peaks[0], 9);
        // Above full scale is full scale.
        Assert.Equal(0, peaks[1], 9);
    }

    /// <summary>Anything this cannot read is no waveform — the note goes without one and readers draw the placeholder.</summary>
    [Fact]
    public void WhatCannotBeReadIsNoWaveform()
    {
        Assert.Null(VoiceWaveform.FromWav([]));
        Assert.Null(VoiceWaveform.FromWav("RIFF\0\0\0\0WAVE"u8));
        Assert.Null(VoiceWaveform.FromWav(Wav([])));
        Assert.Null(VoiceWaveform.FromWav(Riff(1, 8, 1000, 1, new byte[100])));
        Assert.Null(VoiceWaveform.FromWav(Riff(2, 16, 1000, 1, new byte[100])));
        var noRiff = Wav(new short[100]);
        noRiff[0] = (byte)'X';
        Assert.Null(VoiceWaveform.FromWav(noRiff));
        Assert.Equal(double.NegativeInfinity, VoiceWaveform.Dbfs(0));
        Assert.Equal(double.NegativeInfinity, VoiceWaveform.Dbfs(double.NaN));
        Assert.Equal(0, VoiceWaveform.Dbfs(3));
    }

    // ---- the not-sent chip at its width ------------------------------------------------------------------------------

    /// <summary>Room to spare: the mockup's 22 bars, "Not sent" on the line.</summary>
    [Fact]
    public void AWideNotSentChipDrawsTheMockupsBarsOnOneLine()
    {
        var fit = VoiceLook.FitNotSent(rowWidth: 342, othersWidth: 100, labelWidth: 55, spacing: 10, designBars: 22);
        Assert.Equal(new ChipFit(LabelAbove: false, Bars: 22), fit);
    }

    /// <summary>
    /// German's "Nicht gesendet" and "Senden": fewer bars, the WHOLE shape reduced to them — never 22 bars cut off at the
    /// column's edge, which showed only part of the shape.
    /// </summary>
    [Fact]
    public void ALongerLanguageGetsFewerBarsNotACutOffShape()
    {
        var fit = VoiceLook.FitNotSent(rowWidth: 342, othersWidth: 150, labelWidth: 95, spacing: 10, designBars: 22);
        Assert.False(fit.LabelAbove);
        Assert.True(fit.Bars < 22);
        Assert.True(fit.Bars >= VoiceLook.MinChipBars);
        var room = 342 - 150 - 95 - 5 * 10;
        Assert.True(fit.Bars * VoiceLook.BarWidth + (fit.Bars - 1) * VoiceLook.BarGap <= room);
        Assert.Equal(VoiceLook.BarCount(room), fit.Bars);
    }

    /// <summary>At 150–200 % text the fixed columns alone overflow: "Not sent" goes above, and ✕ is never pushed out.</summary>
    [Fact]
    public void LargeTextMovesNotSentAboveTheLine()
    {
        var fit = VoiceLook.FitNotSent(rowWidth: 342, othersWidth: 240, labelWidth: 150, spacing: 10, designBars: 22);
        Assert.True(fit.LabelAbove);
        var room = 342 - 240 - 4 * 10;
        Assert.Equal(VoiceLook.BarCount(room), fit.Bars);
        // Even with no room for a single bar, the line still holds ▶, the length, Send and ✕: the waveform gives way.
        Assert.Equal(new ChipFit(true, 0), VoiceLook.FitNotSent(342, 330, 150, 10, 22));
    }

    /// <summary>Across every width, the bars drawn always fit the room they are given, and never exceed the design's.</summary>
    [Fact]
    public void TheChipsBarsAlwaysFitTheirColumn()
    {
        for (var width = 0; width <= 400; width++)
        {
            foreach (var label in new[] { 40.0, 90.0, 160.0 })
            {
                var fit = VoiceLook.FitNotSent(width, 120, label, 10, 22);
                var room = width - 120 - (fit.LabelAbove ? 4 * 10 : label + 5 * 10);
                Assert.InRange(fit.Bars, 0, 22);
                if (fit.Bars > 0)
                {
                    Assert.True(fit.Bars * VoiceLook.BarWidth + (fit.Bars - 1) * VoiceLook.BarGap <= room, $"{width} {label}");
                }
            }
        }
    }

    // ---- targets, the idle seek, a note on its way ------------------------------------------------------------------

    /// <summary>S1.1: a 44 target round a smaller visual, reached by a negative margin — the row it sits in no taller.</summary>
    [Theory]
    [InlineData(19, -12.5)]
    [InlineData(28, -8)]
    [InlineData(32, -6)]
    [InlineData(44, 0)]
    [InlineData(64, 0)]
    public void ASmallControlReachesAFortyFourTarget(double visual, double reach)
    {
        Assert.Equal(reach, VoiceLook.Reach(visual));
        Assert.Equal(Math.Max(visual, ComposerButton.MinTargetWindowsEpx), visual - 2 * VoiceLook.Reach(visual));
    }

    /// <summary>
    /// A note SEEKED while idle starts where it was put — not from 0:00, which made the seek do nothing anyone could see —
    /// and one seeked to its very end starts from the top.
    /// </summary>
    [Fact]
    public void PlayStartsWhereAnIdleNoteWasSeeked()
    {
        Assert.Equal(12.5, VoiceLook.StartAt(12.5, 42.4));
        Assert.Equal(0, VoiceLook.StartAt(null, 42.4));
        Assert.Equal(0, VoiceLook.StartAt(0, 42.4));
        Assert.Equal(0, VoiceLook.StartAt(42.3, 42.4));
        Assert.Equal(0, VoiceLook.StartAt(-1, 42.4));
        Assert.Equal(0, VoiceLook.StartAt(double.NaN, 42.4));
        Assert.Equal(0, VoiceLook.StartAt(5, 0));
        // The bubble seeked to 12.5 of 42.4 shows that much lit and that time, as it will when it plays.
        Assert.Equal(12, VoiceLook.PlayedBars(12.5, 42.4, 42));
        Assert.Equal("0:13", VoiceLook.Time(true, 12.5, 42.4));
    }

    /// <summary>A voice note going up is drawn as the bubble it will be — but only a lone note with no words.</summary>
    [Fact]
    public void OnlyALoneWordlessNoteOnItsWayIsAPendingVoiceBubble()
    {
        Assert.True(VoiceLook.IsPendingVoice("", ["audio"]));
        Assert.True(VoiceLook.IsPendingVoice(null, ["audio"]));
        Assert.False(VoiceLook.IsPendingVoice("hello", ["audio"]));
        Assert.False(VoiceLook.IsPendingVoice("", ["audio", "photo"]));
        Assert.False(VoiceLook.IsPendingVoice("", ["photo"]));
        Assert.False(VoiceLook.IsPendingVoice("", []));
    }
}
