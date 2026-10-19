using System.Buffers.Binary;
using System.Text;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// What a composited canvas becomes on its way into the encoder (issue #96; docs/protocol.md, "Animations", "The file"):
/// on white, at the plan's size, as BT.709 NV12 — the frames in the plan's order — and the MP4's sample table made exact
/// afterwards.
/// </summary>
public sealed class AnimationRenderTests
{
    [Fact]
    public void TransparencyIsDrawnOnWhite()
    {
        var white = AnimationRender.OnWhite([10, 20, 30, 0, 10, 20, 30, 255, 0, 0, 0, 128]);
        Assert.Equal([255, 255, 255, 255, 10, 20, 30, 255, 127, 127, 127, 255], white);
    }

    [Fact]
    public void AreaAveragingIsTheMeanOfWhatEachPixelCovers()
    {
        // 4 × 2 → 2 × 1: each target pixel is the mean of a 2 × 2 block.
        byte[] four =
        [
            0, 0, 0, 255, 100, 100, 100, 255, 10, 10, 10, 255, 30, 30, 30, 255,
            200, 200, 200, 255, 100, 100, 100, 255, 50, 50, 50, 255, 70, 70, 70, 255,
        ];
        Assert.Equal([100, 100, 100, 255, 40, 40, 40, 255], AnimationRender.Scale(four, 4, 2, 2, 1));
        // 3 → 2 across: the middle pixel is split half and half.
        byte[] three = [0, 0, 0, 255, 90, 90, 90, 255, 180, 180, 180, 255];
        Assert.Equal([30, 30, 30, 255, 150, 150, 150, 255], AnimationRender.Scale(three, 3, 1, 2, 1));
        // The same size is a copy.
        var same = AnimationRender.Scale(three, 3, 1, 3, 1);
        Assert.Equal(three, same);
        Assert.NotSame(three, same);
    }

    /// <summary>BT.709, limited range: white and black at the ends of 16–235, grey at the middle of chroma, the primaries.</summary>
    [Fact]
    public void ColoursBecomeBt709LimitedRange()
    {
        Assert.Equal(235, AnimationRender.Y(255, 255, 255));
        Assert.Equal(16, AnimationRender.Y(0, 0, 0));
        Assert.Equal(((byte)128, (byte)128), AnimationRender.CbCr(255, 255, 255, 1));
        Assert.Equal(((byte)128, (byte)128), AnimationRender.CbCr(4 * 77, 4 * 77, 4 * 77, 4));
        // Red: Y 16 + 219 × 0.2126 = 62.6, Cb 128 − 224 × 0.1146 = 102.3, Cr 240.
        Assert.Equal(63, AnimationRender.Y(255, 0, 0));
        Assert.Equal(((byte)102, (byte)240), AnimationRender.CbCr(255, 0, 0, 1));
        // Green: Y 16 + 219 × 0.7152 = 172.6; blue: 16 + 219 × 0.0722 = 31.8.
        Assert.Equal(173, AnimationRender.Y(0, 255, 0));
        Assert.Equal(32, AnimationRender.Y(0, 0, 255));
        Assert.Equal(((byte)240, (byte)118), AnimationRender.CbCr(0, 0, 255, 1));
    }

    [Fact]
    public void Nv12IsAWholeLumaPlaneThenInterleavedHalfSizeChroma()
    {
        // 2 × 2 — white, white, red, red — and 4 × 2 of white.
        byte[] pixels = [255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255];
        var nv12 = AnimationRender.Nv12(pixels, 2, 2);
        Assert.Equal(6, nv12.Length);
        Assert.Equal([235, 235, 63, 63], nv12[..4]);
        // The chroma of the four together: half white, half red.
        var (cb, cr) = AnimationRender.CbCr(4 * 255, 2 * 255, 2 * 255, 4);
        Assert.Equal([cb, cr], nv12[4..]);
        var white = AnimationRender.Nv12([.. Enumerable.Repeat((byte)255, 4 * 4 * 2)], 4, 2);
        Assert.Equal([235, 235, 235, 235, 235, 235, 235, 235, 128, 128, 128, 128], white);
    }

    /// <summary>
    /// The frames in the plan's order — and EVERY source frame drawn, sampled or not: a 20 ms flash no tick lands on is not a
    /// video frame, but what it left on the canvas is under the next one.
    /// </summary>
    [Fact]
    public async Task EveryFrameIsDrawnAndOnlyTheSampledOnesAreGiven()
    {
        // 2 × 2: frame 0 red for 510 ms; frame 1 a green left half for 20 ms, never sampled; frame 2 a blue top-right pixel.
        var layout = new AnimationLayout(AnimationFormat.Gif, 2, 2,
        [
            new AnimationFrame(0, 0, 2, 2, 510, FrameBlend.Over, FrameDisposal.None),
            new AnimationFrame(0, 0, 1, 2, 20, FrameBlend.Over, FrameDisposal.None),
            new AnimationFrame(1, 0, 1, 1, 500, FrameBlend.Over, FrameDisposal.None),
        ]);
        var plan = AnimationTiming.Plan(2, 2, layout.StatedMs)!;
        Assert.Equal([new AnimationSample(0, 16), new AnimationSample(2, 15)], plan.Samples);
        var asked = new List<int>();
        byte[] Solid(int count, byte b, byte g, byte r) => [.. Enumerable.Range(0, count).SelectMany(_ => new[] { b, g, r, (byte)255 })];
        var sources = new[] { Solid(4, 0, 0, 255), Solid(2, 0, 255, 0), Solid(1, 255, 0, 0) };
        var frames = new AnimationFrames(layout, plan, (index, _) =>
        {
            asked.Add(index);
            return Task.FromResult<byte[]?>(sources[index]);
        });
        var first = await frames.NextAsync();
        var second = await frames.NextAsync();
        Assert.Null(await frames.NextAsync());
        Assert.Equal(2, frames.Given);
        Assert.Equal([0, 1, 2], asked);
        Assert.Equal(Solid(4, 0, 0, 255), first);
        // Green left (from the unsampled flash), blue top right, red bottom right.
        Assert.Equal([0, 255, 0, 255, 255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255], second);
    }

    [Fact]
    public async Task AFrameThatWillNotDecodeEndsTheAnimation()
    {
        var layout = new AnimationLayout(AnimationFormat.WebP, 2, 2,
        [
            new AnimationFrame(0, 0, 2, 2, 100, FrameBlend.Over, FrameDisposal.None),
            new AnimationFrame(0, 0, 2, 2, 100, FrameBlend.Over, FrameDisposal.None),
        ]);
        var plan = AnimationTiming.Plan(2, 2, layout.StatedMs)!;
        var undecodable = new AnimationFrames(layout, plan, (_, _) => Task.FromResult<byte[]?>(null));
        await Assert.ThrowsAsync<InvalidDataException>(() => undecodable.NextAsync());
        // A decoder that answers at another size is not drawn either.
        var wrong = new AnimationFrames(layout, plan, (_, _) => Task.FromResult<byte[]?>(new byte[4]));
        await Assert.ThrowsAsync<InvalidDataException>(() => wrong.NextAsync());
    }

    [Fact]
    public async Task ItIsScaledToThePlansSize()
    {
        // 333 × 201 becomes 332 × 200 (even), and the frame is what the plan says.
        var layout = new AnimationLayout(AnimationFormat.Apng, 333, 201,
            [new AnimationFrame(0, 0, 333, 201, 100, FrameBlend.Source, FrameDisposal.None)]);
        var plan = AnimationTiming.Plan(333, 201, layout.StatedMs)!;
        var frames = new AnimationFrames(layout, plan, (_, _) =>
            Task.FromResult<byte[]?>([.. Enumerable.Range(0, 333 * 201).SelectMany(_ => new byte[] { 10, 20, 30, 255 })]));
        var frame = (await frames.NextAsync())!;
        Assert.Equal(332 * 200 * 4, frame.Length);
        Assert.All(frame.Chunk(4), pixel => Assert.Equal([10, 20, 30, 255], pixel));
    }

    // ---- the sample table made exact --------------------------------------------------------------

    private static byte[] Box(string type, params byte[][] children)
    {
        var body = children.SelectMany(child => child).ToArray();
        var box = new byte[8 + body.Length];
        BinaryPrimitives.WriteUInt32BigEndian(box, (uint)box.Length);
        Encoding.ASCII.GetBytes(type).CopyTo(box, 4);
        body.CopyTo(box, 8);
        return box;
    }

    private static byte[] U32(params uint[] values)
    {
        var bytes = new byte[4 * values.Length];
        for (var at = 0; at < values.Length; at++)
        {
            BinaryPrimitives.WriteUInt32BigEndian(bytes.AsSpan(4 * at), values[at]);
        }
        return bytes;
    }

    /// <summary>
    /// What Media Foundation might write: ftyp, the media data, then the index — a 1000-a-second movie, a 10 000 000-a-second
    /// track (hundreds of nanoseconds, not a multiple of 30), one sample per entry, an edit list, and a ctts.
    /// </summary>
    private static byte[] Encoded(uint trackScale, uint[] durations, bool ctts = true, string handler = "vide", int tracks = 1)
    {
        var ftyp = Box("ftyp", Encoding.ASCII.GetBytes("mp42\0\0\0\0mp42isom"));
        var mdat = Box("mdat", [.. Enumerable.Range(0, 64).Select(at => (byte)at)]);
        var total = durations.Aggregate(0u, (sum, value) => sum + value);
        var mvhd = Box("mvhd", U32(0, 0, 0, 1000, 999), new byte[80]);
        byte[] Trak() => Box("trak",
            Box("tkhd", U32(0, 0, 0, 1, 0, 999), new byte[60]),
            Box("edts", Box("elst", U32(0, 1, 999, 0, 0x00010000))),
            Box("mdia",
                Box("mdhd", U32(0, 0, 0, trackScale, total), new byte[4]),
                Box("hdlr", U32(0, 0), Encoding.ASCII.GetBytes(handler), new byte[13]),
                Box("minf", Box("stbl",
                    Box("stsd", U32(0, 0)),
                    Box("stts", U32([0, (uint)durations.Length, .. durations.SelectMany(value => new uint[] { 1, value })])),
                    ctts ? Box("ctts", U32([0, (uint)durations.Length, .. durations.SelectMany(value => new uint[] { 1, value * 2 })])) : [],
                    Box("stsz", U32(0, 0, (uint)durations.Length)),
                    Box("stco", U32(0, 1, (uint)(ftyp.Length + 8)))))));
        var moov = Box("moov", [mvhd, .. Enumerable.Range(0, tracks).Select(_ => Trak())]);
        return [.. ftyp, .. mdat, .. moov];
    }

    private static uint Read(byte[] file, string path, int offset)
    {
        var at = 0;
        var end = file.Length;
        foreach (var type in path.Split('/'))
        {
            while (at < end && Encoding.ASCII.GetString(file, at + 4, 4) != type)
            {
                at += (int)BinaryPrimitives.ReadUInt32BigEndian(file.AsSpan(at));
            }
            Assert.True(at < end, $"no {type} in {path}");
            end = at + (int)BinaryPrimitives.ReadUInt32BigEndian(file.AsSpan(at));
            at += 8;
        }
        return BinaryPrimitives.ReadUInt32BigEndian(file.AsSpan(at + offset));
    }

    [Fact]
    public void ATimescaleThatIsNotAMultipleOfThirtyBecomesThirtyThousand()
    {
        var file = Encoded(10_000_000, [333_333, 1_000_000, 333_334]);
        var exact = AnimationMp4.ExactTicks(file, [3 * 1, 30, 1])!;
        var (scale, durations) = AnimationMp4.Timing(exact)!.Value;
        Assert.Equal(30_000u, scale);
        Assert.Equal([3_000u, 30_000u, 1_000u], durations);
        Assert.Equal(34_000u, Read(exact, "moov/trak/mdia/mdhd", 16));
        // The movie's 1000-a-second durations: 34 ticks is 1133⅓ ms.
        Assert.Equal(1133u, Read(exact, "moov/mvhd", 16));
        Assert.Equal(1133u, Read(exact, "moov/trak/tkhd", 20));
        Assert.Equal(1133u, Read(exact, "moov/trak/edts/elst", 8));
        // Composition offsets carried into the new timescale: 666 666 hundreds of ns is 2 000 thirty-thousandths.
        Assert.Equal(2_000u, Read(exact, "moov/trak/mdia/minf/stbl/ctts", 12));
        // The media data did not move, and the faststart that follows re-points it.
        Assert.Equal(file[..(8 + 16 + 72)], exact[..(8 + 16 + 72)]);
        var moved = AnimationMp4.Timing(Faststart.MoovFirst(exact)!)!.Value;
        Assert.Equal(30_000u, moved.Timescale);
        Assert.Equal(durations, moved.Durations);
    }

    /// <summary>A timescale that is already a multiple of 30 is kept; runs of equal samples are one stts entry.</summary>
    [Fact]
    public void AMultipleOfThirtyIsKeptAndRunsAreOneEntry()
    {
        var exact = AnimationMp4.ExactTicks(Encoded(90_000, [9_000, 9_000, 9_000, 90_000], ctts: false), [3, 3, 3, 30])!;
        var (scale, durations) = AnimationMp4.Timing(exact)!.Value;
        Assert.Equal(90_000u, scale);
        Assert.Equal([9_000u, 9_000u, 9_000u, 90_000u], durations);
        Assert.Equal(2u, Read(exact, "moov/trak/mdia/minf/stbl/stts", 4));
        Assert.Equal(3u, Read(exact, "moov/trak/mdia/minf/stbl/stts", 8));
    }

    /// <summary>What it cannot vouch for, it leaves alone: the encoder's own timing goes, which plays the same.</summary>
    [Fact]
    public void WhatItCannotVouchForItLeavesAlone()
    {
        // A sample count that is not the plan's: a frame dropped or doubled.
        Assert.Null(AnimationMp4.ExactTicks(Encoded(10_000_000, [333_333, 333_333]), [1, 1, 1]));
        // An index already in front of the media data, whose offsets a resize would break.
        Assert.Null(AnimationMp4.ExactTicks(Faststart.MoovFirst(Encoded(10_000_000, [333_333]))!, [1]));
        // Not a video track, or more than one track.
        Assert.Null(AnimationMp4.ExactTicks(Encoded(10_000_000, [333_333], handler: "soun"), [1]));
        Assert.Null(AnimationMp4.ExactTicks(Encoded(10_000_000, [333_333], tracks: 2), [1]));
        // No ticks, or bytes that are no MP4.
        Assert.Null(AnimationMp4.ExactTicks(Encoded(10_000_000, [333_333]), []));
        Assert.Null(AnimationMp4.ExactTicks([1, 2, 3], [1]));
    }
}
