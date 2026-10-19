using System.Buffers.Binary;
using System.IO.Compression;
using System.Security.Cryptography;
using System.Text;
using FamilyConnect.Core;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The three animated pictures taken apart (issue #96; docs/protocol.md, "Animations"): which picked files are animations,
/// a GIF decoded and composited by this client, an APNG and an animated WebP cut into standalone stills for the platform's
/// decoder — and the compositor every one of them goes through.
/// </summary>
/// <remarks>
/// The files here are written by this test's own writers (an LZW stream of literals and clears, PNG chunks with their own
/// CRC, RIFF chunks), so the production readers are never what says they read correctly — and one GIF was written by
/// Pillow, with real LZW, sub-rectangles, local colour tables and transparency, and is checked against Pillow's own frames.
/// </remarks>
public sealed class AnimatedPicturesTests
{
    // ---- writers ------------------------------------------------------------------------------

    private sealed record GifImage(
        int Left,
        int Top,
        int Width,
        int Height,
        byte[] Indexes,
        int Disposal = 1,
        int? Transparent = null,
        ushort DelayCs = 10,
        bool Interlaced = false,
        int? StopAfter = null);

    private static readonly (byte R, byte G, byte B)[] Colours =
    [
        (0, 0, 0), (255, 0, 0), (0, 255, 0), (0, 0, 255), (255, 255, 255), (255, 255, 0), (0, 255, 255), (255, 0, 255),
    ];

    /// <summary>
    /// A GIF of literals: a clear code every <c>clear − 2</c> indexes, so the code size never grows — the "uncompressed GIF"
    /// every decoder must read, and one whose every index this test knows.
    /// </summary>
    private static byte[] Gif(int width, int height, params GifImage[] images)
    {
        using var output = new MemoryStream();
        output.Write("GIF89a"u8);
        Span<byte> word = stackalloc byte[2];
        BinaryPrimitives.WriteUInt16LittleEndian(word, (ushort)width);
        output.Write(word);
        BinaryPrimitives.WriteUInt16LittleEndian(word, (ushort)height);
        output.Write(word);
        // A global table of eight colours: 2^(2+1).
        output.WriteByte(0x80 | 0x02);
        output.WriteByte(0);
        output.WriteByte(0);
        foreach (var (r, g, b) in Colours)
        {
            output.WriteByte(r);
            output.WriteByte(g);
            output.WriteByte(b);
        }
        foreach (var image in images)
        {
            output.Write([0x21, 0xF9, 0x04, (byte)((image.Disposal << 2) | (image.Transparent is null ? 0 : 1))]);
            BinaryPrimitives.WriteUInt16LittleEndian(word, image.DelayCs);
            output.Write(word);
            output.WriteByte((byte)(image.Transparent ?? 0));
            output.WriteByte(0);
            output.WriteByte(0x2C);
            foreach (var value in new[] { image.Left, image.Top, image.Width, image.Height })
            {
                BinaryPrimitives.WriteUInt16LittleEndian(word, (ushort)value);
                output.Write(word);
            }
            output.WriteByte(image.Interlaced ? (byte)0x40 : (byte)0);
            const int Minimum = 3;
            output.WriteByte(Minimum);
            var order = image.Interlaced ? InterlaceOrder(image.Height) : [.. Enumerable.Range(0, image.Height)];
            var indexes = order.SelectMany(row => image.Indexes.Skip(row * image.Width).Take(image.Width))
                .Take(image.StopAfter ?? int.MaxValue).ToArray();
            var codes = new List<int>();
            const int Clear = 1 << Minimum;
            codes.Add(Clear);
            for (var at = 0; at < indexes.Length; at++)
            {
                if (at > 0 && at % (Clear - 2) == 0)
                {
                    codes.Add(Clear);
                }
                codes.Add(indexes[at]);
            }
            if (image.StopAfter is null)
            {
                codes.Add(Clear + 1);
            }
            var packed = new List<byte>();
            int bits = 0, held = 0;
            foreach (var code in codes)
            {
                bits |= code << held;
                held += Minimum + 1;
                while (held >= 8)
                {
                    packed.Add((byte)bits);
                    bits >>= 8;
                    held -= 8;
                }
            }
            if (held > 0)
            {
                packed.Add((byte)bits);
            }
            for (var at = 0; at < packed.Count; at += 255)
            {
                var block = packed.Skip(at).Take(255).ToArray();
                output.WriteByte((byte)block.Length);
                output.Write(block);
            }
            output.WriteByte(0);
        }
        output.WriteByte(0x3B);
        return output.ToArray();
    }

    /// <summary>The order an interlaced image's rows are stored in, worked out here independently of the reader.</summary>
    private static int[] InterlaceOrder(int height) =>
    [
        .. Enumerable.Range(0, height).Where(row => row % 8 == 0),
        .. Enumerable.Range(0, height).Where(row => row % 8 == 4),
        .. Enumerable.Range(0, height).Where(row => row % 4 == 2),
        .. Enumerable.Range(0, height).Where(row => row % 2 == 1),
    ];

    private static byte[] Fill(int count, byte index) => [.. Enumerable.Repeat(index, count)];

    /// <summary>A pixel of a straight-BGRA canvas, as (r, g, b, a).</summary>
    private static (byte R, byte G, byte B, byte A) At(byte[] canvas, int width, int x, int y)
    {
        var at = ((y * width) + x) * 4;
        return (canvas[at + 2], canvas[at + 1], canvas[at], canvas[at + 3]);
    }

    private static readonly (byte, byte, byte, byte) Red = (255, 0, 0, 255);
    private static readonly (byte, byte, byte, byte) Green = (0, 255, 0, 255);
    private static readonly (byte, byte, byte, byte) Blue = (0, 0, 255, 255);
    private static readonly (byte, byte, byte, byte) Empty = (0, 0, 0, 0);

    /// <summary>Every frame of a GIF composited, as the canvas stands after each.</summary>
    private static List<byte[]> Composited(byte[] gif)
    {
        var picture = GifPicture.Read(gif)!;
        var canvas = new FrameCanvas(picture.Layout.Width, picture.Layout.Height);
        var shown = new List<byte[]>();
        for (var index = 0; index < picture.Layout.Frames.Count; index++)
        {
            canvas.Draw(picture.Layout.Frames[index], picture.Pixels(index));
            shown.Add((byte[])canvas.Pixels.Clone());
        }
        return shown;
    }

    // ---- which files are animations ------------------------------------------------------------

    [Fact]
    public void AnAnimationIsWhatItsOwnBytesSay()
    {
        var two = Gif(2, 2, new GifImage(0, 0, 2, 2, Fill(4, 1)), new GifImage(0, 0, 2, 2, Fill(4, 2)));
        var one = Gif(2, 2, new GifImage(0, 0, 2, 2, Fill(4, 1)));
        Assert.Equal(AnimationFormat.Gif, AnimationPrep.Animated(two));
        Assert.Null(AnimationPrep.Animated(one));
        Assert.Equal(AnimationFormat.Apng, AnimationPrep.Animated(Apng()));
        Assert.Equal(AnimationFormat.WebP, AnimationPrep.Animated(WebP()));
        Assert.Null(AnimationPrep.Animated(StillPng()));
        Assert.Null(AnimationPrep.Animated(StillWebP()));
        Assert.Null(AnimationPrep.Animated("BM\0\0\0\0"u8));
        Assert.True(AnimationPrep.MayBe(two));
        Assert.True(AnimationPrep.MayBe(Apng()));
        Assert.True(AnimationPrep.MayBe(WebP()));
        Assert.False(AnimationPrep.MayBe("BM\0\0\0\0\0\0\0\0\0\0"u8));
        Assert.False(AnimationPrep.MayBe([0xFF, 0xD8, 0xFF, 0xE0]));
    }

    /// <summary>
    /// ANIMATED → AN ANIMATION, STILL → EXACTLY AS BEFORE: a still GIF or WebP is a file and a still PNG a photo (the router
    /// it always went through), and an animation is one only where a chat takes them and within the ceiling.
    /// </summary>
    [Fact]
    public void AnAnimatedPictureIsRoutedAsAnAnimationAndAStillOneAsBefore()
    {
        var animated = Gif(2, 2, new GifImage(0, 0, 2, 2, Fill(4, 1)), new GifImage(0, 0, 2, 2, Fill(4, 2)));
        var still = Gif(2, 2, new GifImage(0, 0, 2, 2, Fill(4, 1)));
        Assert.Equal(AnimationFormat.Gif, AnimationPrep.Route(true, animated.Length, animated));
        Assert.Equal(AnimationFormat.Apng, AnimationPrep.Route(true, Apng().Length, Apng()));
        Assert.Equal(AnimationFormat.WebP, AnimationPrep.Route(true, WebP().Length, WebP()));
        // The assistant's chat, the board, a profile picture: not offered.
        Assert.Null(AnimationPrep.Route(false, animated.Length, animated));
        // Over the ceiling, the route it always had: refused, as a file.
        Assert.Null(AnimationPrep.Route(true, MediaPrep.SizeLimit + 1, animated));
        Assert.Null(AnimationPrep.Route(true, still.Length, still));
        Assert.Equal(MediaRoute.File, MediaPrep.Route("image/gif", "still.gif", still).Route);
        Assert.Null(AnimationPrep.Route(true, StillWebP().Length, StillWebP()));
        Assert.Equal(MediaRoute.File, MediaPrep.Route("image/webp", "still.webp", StillWebP()).Route);
        Assert.Null(AnimationPrep.Route(true, StillPng().Length, StillPng()));
        Assert.Equal(MediaRoute.Photo, MediaPrep.Route("image/png", "still.png", StillPng()).Route);
        // A layout for each animation, none for a still.
        Assert.Equal(AnimationFormat.Gif, AnimationPrep.Layout(animated)!.Format);
        Assert.Equal(AnimationFormat.Apng, AnimationPrep.Layout(Apng())!.Format);
        Assert.Equal(AnimationFormat.WebP, AnimationPrep.Layout(WebP())!.Format);
        Assert.Null(AnimationPrep.Layout(still));
    }

    // ---- a GIF ----------------------------------------------------------------------------------

    [Fact]
    public void AGifsFramesAndDelaysAreReadFromItsBlocks()
    {
        var gif = Gif(
            4, 3,
            new GifImage(0, 0, 4, 3, Fill(12, 1), Disposal: 0, DelayCs: 7),
            new GifImage(1, 1, 2, 1, Fill(2, 2), Disposal: 2, Transparent: 0, DelayCs: 0),
            new GifImage(0, 0, 1, 1, Fill(1, 3), Disposal: 3, DelayCs: 100),
            new GifImage(0, 0, 1, 1, Fill(1, 3), Disposal: 6, DelayCs: 2));
        var layout = GifPicture.Read(gif)!.Layout;
        Assert.Equal((4, 3), (layout.Width, layout.Height));
        Assert.Equal([70u, 0u, 1000u, 20u], layout.StatedMs);
        Assert.Equal(
            [FrameDisposal.None, FrameDisposal.Background, FrameDisposal.Previous, FrameDisposal.None],
            layout.Frames.Select(frame => frame.Disposal));
        Assert.All(layout.Frames, frame => Assert.Equal(FrameBlend.Over, frame.Blend));
        Assert.Equal(new AnimationFrame(1, 1, 2, 1, 0, FrameBlend.Over, FrameDisposal.Background), layout.Frames[1]);
    }

    [Fact]
    public void ATransparentIndexDrawsNothingAndTheRestOver()
    {
        // Frame 1 is green with its two left pixels transparent (index 0, named transparent).
        var gif = Gif(
            3, 2,
            new GifImage(0, 0, 3, 2, Fill(6, 1)),
            new GifImage(0, 0, 3, 2, [0, 2, 2, 0, 2, 2], Transparent: 0));
        var shown = Composited(gif);
        Assert.Equal(Red, At(shown[1], 3, 0, 0));
        Assert.Equal(Green, At(shown[1], 3, 1, 0));
        Assert.Equal(Red, At(shown[1], 3, 0, 1));
        Assert.Equal(Green, At(shown[1], 3, 2, 1));
    }

    [Fact]
    public void DisposalNoneLeavesAFrameWhereItWas()
    {
        var gif = Gif(
            3, 1,
            new GifImage(0, 0, 3, 1, Fill(3, 1), Disposal: 1),
            new GifImage(1, 0, 1, 1, [2], Disposal: 1),
            new GifImage(2, 0, 1, 1, [3], Disposal: 1));
        var shown = Composited(gif);
        Assert.Equal(Red, At(shown[2], 3, 0, 0));
        Assert.Equal(Green, At(shown[2], 3, 1, 0));
        Assert.Equal(Blue, At(shown[2], 3, 2, 0));
    }

    /// <summary>"Restore to background" clears to EMPTY — what browsers draw, whatever background colour the file names.</summary>
    [Fact]
    public void DisposalBackgroundClearsTheFramesRectangleToEmpty()
    {
        var gif = Gif(
            3, 1,
            new GifImage(0, 0, 3, 1, Fill(3, 1), Disposal: 1),
            new GifImage(1, 0, 2, 1, Fill(2, 2), Disposal: 2),
            new GifImage(0, 0, 1, 1, [3], Disposal: 1));
        var shown = Composited(gif);
        Assert.Equal(Green, At(shown[1], 3, 1, 0));
        // Frame 1's rectangle is gone once frame 2 is drawn; frame 0 outside it stays under frame 2.
        Assert.Equal(Blue, At(shown[2], 3, 0, 0));
        Assert.Equal(Empty, At(shown[2], 3, 1, 0));
        Assert.Equal(Empty, At(shown[2], 3, 2, 0));
        // And on white, which is what the video shows.
        var white = AnimationRender.OnWhite(shown[2]);
        Assert.Equal((255, 255, 255, 255), At(white, 3, 1, 0));
    }

    [Fact]
    public void DisposalPreviousPutsBackWhatTheFrameCovered()
    {
        var gif = Gif(
            3, 1,
            new GifImage(0, 0, 3, 1, Fill(3, 1), Disposal: 1),
            new GifImage(0, 0, 2, 1, Fill(2, 2), Disposal: 3),
            new GifImage(2, 0, 1, 1, [3], Disposal: 1));
        var shown = Composited(gif);
        Assert.Equal(Green, At(shown[1], 3, 0, 0));
        Assert.Equal(Red, At(shown[2], 3, 0, 0));
        Assert.Equal(Red, At(shown[2], 3, 1, 0));
        Assert.Equal(Blue, At(shown[2], 3, 2, 0));
    }

    [Fact]
    public void AnInterlacedImageIsPutBackInRowOrder()
    {
        var rows = Enumerable.Range(0, 10).Select(row => (byte)(row % 8)).ToArray();
        var gif = Gif(1, 10, new GifImage(0, 0, 1, 10, rows, Interlaced: true), new GifImage(0, 0, 1, 1, [0]));
        var pixels = GifPicture.Read(gif)!.Pixels(0);
        for (var row = 0; row < 10; row++)
        {
            var (r, g, b) = Colours[row % 8];
            Assert.Equal((r, g, b, (byte)255), At(pixels, 1, 0, row));
        }
        Assert.Equal([0, 8, 4, 2, 6, 1, 3, 5, 7, 9], Enumerable.Range(0, 10).Select(written => GifPicture.InterlacedRow(written, 10)));
    }

    /// <summary>Data that stops early leaves the rest of the rectangle undrawn, as a browser leaves it.</summary>
    [Fact]
    public void AnImageWhoseDataStopsEarlyLeavesTheRestUndrawn()
    {
        var gif = Gif(
            4, 1,
            new GifImage(0, 0, 4, 1, Fill(4, 1)),
            // A clear and three indexes are sixteen bits: the data ends on a byte, with no padding to read as a code.
            new GifImage(0, 0, 4, 1, Fill(4, 2), StopAfter: 3));
        var shown = Composited(gif);
        Assert.Equal(Green, At(shown[1], 4, 2, 0));
        Assert.Equal(Red, At(shown[1], 4, 3, 0));
    }

    /// <summary>A frame that reaches past the logical screen is clipped to it, never a crash.</summary>
    [Fact]
    public void AFramePastTheCanvasIsClipped()
    {
        var gif = Gif(2, 2, new GifImage(0, 0, 2, 2, Fill(4, 1)), new GifImage(1, 1, 3, 3, Fill(9, 2)));
        var shown = Composited(gif);
        Assert.Equal(Green, At(shown[1], 2, 1, 1));
        Assert.Equal(Red, At(shown[1], 2, 0, 0));
    }

    [Fact]
    public void BytesThatAreNotAGifAreNoPicture()
    {
        Assert.Null(GifPicture.Read("GIF89a"u8.ToArray()));
        Assert.Null(GifPicture.Read(Apng()));
        // A header and nothing after it: no image.
        Assert.Null(GifPicture.Read(Gif(2, 2)));
        // Cut off in the middle of a block: what was read before it stands.
        var whole = Gif(2, 2, new GifImage(0, 0, 2, 2, Fill(4, 1)), new GifImage(0, 0, 2, 2, Fill(4, 2)));
        Assert.Single(GifPicture.Read(whole[..^20])!.Layout.Frames);
    }

    /// <summary>
    /// A GIF Pillow wrote — real LZW at an 8-bit code size, frames 1 and 2 as sub-rectangles with local colour tables and a
    /// transparent index for every pixel that did not change — composited exactly into Pillow's own three frames (SHA-256 of
    /// each composited canvas as straight BGRA, from <c>Image.convert("RGB")</c> of the frames Pillow was given).
    /// </summary>
    [Fact]
    public void AGifPillowWroteIsCompositedIntoPillowsOwnFrames()
    {
        var gif = Convert.FromBase64String(PillowGif);
        var layout = GifPicture.Read(gif)!.Layout;
        Assert.Equal((32, 24), (layout.Width, layout.Height));
        Assert.Equal([70u, 20u, 1000u], layout.StatedMs);
        Assert.Equal(new AnimationFrame(9, 5, 6, 7, 20, FrameBlend.Over, FrameDisposal.None), layout.Frames[1]);
        var shown = Composited(gif);
        Assert.Equal(
            [
                "cab041f8fa5a527187a4c1406fb0c6e00881462d6b86961a318ae60c6e3f288a",
                "9795d1a04e272008c8b850793e00c572f3752185de731fb3f55a861b5b156171",
                "589a3295fdb8b1b9b480cdd24c3628e6d8538e5addb8c3373770a20d6f0e7e24",
            ],
            shown.Select(canvas => Convert.ToHexStringLower(SHA256.HashData(canvas))));
    }

    private const string PillowGif =
        "R0lGODlhIAAYAIMAAL2hymkxrhcLVdChLIKtto1DBfT346J8saUOVB8/4Gy+IkQ6moSBY93RGI222dAvZyH/C05FVFNDQVBFMi4wAwEAAAAh+QQEBwAAACwAAAAAIAAYAAAI/wAdKDgQIAGBAwUSNADwgEABAQAcIABQIMCChwgMOAQwwICCBAI+CkDoIEEBBQQCDHigoEFKAwAaNBCQ4AEAAAIONkBAIIFFAwYQLGDAwAGBAQkUOMj5oMGAAg8QhnygkIABBw97Ijig8SCCBgaLJhjA4EDMAweGBgCAoACDtg8MCFhg1QFLBQwAGDig8AFLBA4OKKDa8QCDkQQUCCgg8+HctgUMUEWQYOiCAwQuB9g7YEDPBwEIAFjgs8ACnoAHBGCwM4CDAQuiFoDaNgDgAq+F5u3Y1MGCpw9gJxCMEwFZwwhca2ygwLNTA2QfzKW5QICAtgsWoFSAQMFVBgYsjv8F/vppgAA0CQowYLCAzpYMOq683KAgUQREBbj+WDAy1wIDHNAAV8wd4AAAZkGEQHIEIEATe84VcJMDDCTgQH2HrfUAA7aJllNZmSWm1FU7AdjAQwqcZlcDG7Z0gE0WLSQXVgUdMFFR9rE0l0FRGVDURQE84ABQJzGgXWAHdDZkYgyAptqGrAnQkIgGAIgUaSlKOEBISS3VAF0LBGCaaVsSsGF2VD2wAIY5ZXZgQV+6d1FnYK0UH4tfdbbAazVhGJmDswUAlgIFbNUAUWBZJ2ZiQgJQ1E0H0YTVaZQJYOSCPw4wEGvnNXAggowVltRA3HW3EGZoRaYAQ3zNZeSBgMVMthIB6wGQgE8hnXZiRg0EVRYDD4G5npEWRjWaUh/N1hZH3IGXU0QUUTXYYrAFdpNqtwaAF2xCJhDTaAMY1ytUBEhEFa12pVQWXQMEBAAh+QQFAgAQACwJAAUABgAHAIO9ocppMa4XC1XQoSyCrbaNQwX09+OifLGlDlQfP+BsviJEOpqEgWPd0RiNttnQL2cILwANHCjwwACCBA8cCBAAYMFBAAkcBAAAIICCAgUcElDQoIACBQkOLBhAAOOBAAEBACH5BAVkABAALAkABQAMAA4Ag72hymkxrhcLVdChLIKtto1DBfT346J8saUOVB8/4Gy+IkQ6moSBY93RGI222dAvZwigAAUoeBBgQYEGBQMAIIDgQAMHAwQYQAgAAgMCBh4YVIAAggMBEAgsYAABQoIEBQIIMLjgAIICChYAMLCgAYEEDBIccMngQQEGAgI8YOAAwACUR1saUBBgJ4MBAwIUKDCgAQIGCBkcAJBggcwGAwAgQECAgE8DBwwwdNBSAdChCSAEwHiywAEIX7s2SNs0gVoFK3d6TVlgwQMHDhIocGAgIAA7";

    // ---- an APNG --------------------------------------------------------------------------------

    private static uint Crc(ReadOnlySpan<byte> bytes)
    {
        var crc = 0xFFFFFFFFu;
        foreach (var value in bytes)
        {
            crc ^= value;
            for (var bit = 0; bit < 8; bit++)
            {
                crc = (crc & 1) != 0 ? (crc >> 1) ^ 0xEDB88320u : crc >> 1;
            }
        }
        return ~crc;
    }

    private static byte[] Chunk(string type, byte[] payload)
    {
        var chunk = new byte[12 + payload.Length];
        BinaryPrimitives.WriteUInt32BigEndian(chunk, (uint)payload.Length);
        Encoding.ASCII.GetBytes(type).CopyTo(chunk, 4);
        payload.CopyTo(chunk, 8);
        BinaryPrimitives.WriteUInt32BigEndian(chunk.AsSpan(8 + payload.Length), Crc(chunk.AsSpan(4, 4 + payload.Length)));
        return chunk;
    }

    private static readonly byte[] PngSignature = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    private static byte[] Ihdr(int width, int height)
    {
        var ihdr = new byte[13];
        BinaryPrimitives.WriteUInt32BigEndian(ihdr, (uint)width);
        BinaryPrimitives.WriteUInt32BigEndian(ihdr.AsSpan(4), (uint)height);
        ihdr[8] = 8;
        ihdr[9] = 6;
        return Chunk("IHDR", ihdr);
    }

    /// <summary>RGBA rows, each behind a filter byte of 0, deflated as PNG's image data is.</summary>
    private static byte[] ImageData(int width, int height, byte seed)
    {
        var raw = new byte[height * (1 + (width * 4))];
        for (var at = 0; at < raw.Length; at++)
        {
            raw[at] = at % (1 + (width * 4)) == 0 ? (byte)0 : (byte)(seed + at);
        }
        using var output = new MemoryStream();
        using (var zlib = new ZLibStream(output, CompressionLevel.Optimal, leaveOpen: true))
        {
            zlib.Write(raw);
        }
        return output.ToArray();
    }

    private static byte[] Inflate(byte[] data)
    {
        using var input = new ZLibStream(new MemoryStream(data), CompressionMode.Decompress);
        using var output = new MemoryStream();
        input.CopyTo(output);
        return output.ToArray();
    }

    private static byte[] Fctl(uint sequence, int width, int height, int x, int y, ushort num, ushort den, byte dispose, byte blend)
    {
        var fctl = new byte[26];
        BinaryPrimitives.WriteUInt32BigEndian(fctl, sequence);
        BinaryPrimitives.WriteUInt32BigEndian(fctl.AsSpan(4), (uint)width);
        BinaryPrimitives.WriteUInt32BigEndian(fctl.AsSpan(8), (uint)height);
        BinaryPrimitives.WriteUInt32BigEndian(fctl.AsSpan(12), (uint)x);
        BinaryPrimitives.WriteUInt32BigEndian(fctl.AsSpan(16), (uint)y);
        BinaryPrimitives.WriteUInt16BigEndian(fctl.AsSpan(20), num);
        BinaryPrimitives.WriteUInt16BigEndian(fctl.AsSpan(22), den);
        fctl[24] = dispose;
        fctl[25] = blend;
        return Chunk("fcTL", fctl);
    }

    private static byte[] Fdat(uint sequence, byte[] data)
    {
        var fdat = new byte[4 + data.Length];
        BinaryPrimitives.WriteUInt32BigEndian(fdat, sequence);
        data.CopyTo(fdat, 4);
        return Chunk("fdAT", fdat);
    }

    private static byte[] Actl(int frames)
    {
        var actl = new byte[8];
        BinaryPrimitives.WriteUInt32BigEndian(actl, (uint)frames);
        return Chunk("acTL", actl);
    }

    private static byte[] Concat(params byte[][] parts) => [.. parts.SelectMany(part => part)];

    /// <summary>
    /// A 4 × 3 APNG: frame 0 is the default image (its fcTL before the IDAT), frame 1 a 2 × 2 rectangle at (1, 1) in two
    /// fdATs, frame 2 a 1 × 1 at (3, 2). A gAMA between the header and the image data is every frame's.
    /// </summary>
    private static byte[] Apng() => Concat(
        PngSignature,
        Ihdr(4, 3),
        Actl(3),
        Chunk("gAMA", [0, 0, 0xB1, 0x8F]),
        Fctl(0, 4, 3, 0, 0, 1, 10, 2, 0),
        Chunk("IDAT", ImageData(4, 3, 1)),
        Fctl(1, 2, 2, 1, 1, 1, 3, 2, 1),
        Fdat(2, ImageData(2, 2, 2)[..5]),
        Fdat(3, ImageData(2, 2, 2)[5..]),
        Fctl(4, 1, 1, 3, 2, 0, 0, 1, 0),
        Fdat(5, ImageData(1, 1, 3)),
        Chunk("tEXt", "Comment\0after"u8.ToArray()),
        Chunk("IEND", []));

    private static byte[] StillPng() => Concat(PngSignature, Ihdr(2, 2), Chunk("IDAT", ImageData(2, 2, 9)), Chunk("IEND", []));

    /// <summary>The chunks of a PNG by this test's own walk: type, payload, and whether its CRC is right.</summary>
    private static List<(string Type, byte[] Payload, bool Sound)> Chunks(byte[] png)
    {
        Assert.Equal(PngSignature, png[..8]);
        var chunks = new List<(string, byte[], bool)>();
        for (var at = 8; at < png.Length;)
        {
            var length = (int)BinaryPrimitives.ReadUInt32BigEndian(png.AsSpan(at));
            var crc = BinaryPrimitives.ReadUInt32BigEndian(png.AsSpan(at + 8 + length));
            chunks.Add((Encoding.ASCII.GetString(png, at + 4, 4), png[(at + 8)..(at + 8 + length)], crc == Crc(png.AsSpan(at + 4, 4 + length))));
            at += 12 + length;
        }
        return chunks;
    }

    [Fact]
    public void AnApngsFramesAreItsFctls()
    {
        var layout = ApngPicture.Read(Apng())!.Layout;
        Assert.Equal((4, 3), (layout.Width, layout.Height));
        Assert.Equal(
            [
                // A "previous" disposal on the FIRST frame is a background one (the APNG specification's words).
                new AnimationFrame(0, 0, 4, 3, 100, FrameBlend.Source, FrameDisposal.Background),
                new AnimationFrame(1, 1, 2, 2, 333, FrameBlend.Over, FrameDisposal.Previous),
                // 0/0: a den of 0 is hundredths, so 0 ms — which the timing then shows for 100.
                new AnimationFrame(3, 2, 1, 1, 0, FrameBlend.Source, FrameDisposal.Background),
            ],
            layout.Frames);
    }

    /// <summary>
    /// Each frame rebuilt as a PNG of its own that any decoder reads: the signature, the header at the FRAME's size with a
    /// right CRC, every chunk that came before the image data (but never the animation's own), the frame's data — the fdATs'
    /// joined, their sequence numbers gone — as one IDAT, and the end.
    /// </summary>
    [Fact]
    public void EachApngFrameBecomesAStandalonePng()
    {
        var picture = ApngPicture.Read(Apng())!;
        for (var index = 0; index < 3; index++)
        {
            var chunks = Chunks(picture.Standalone(index));
            Assert.All(chunks, chunk => Assert.True(chunk.Sound, $"frame {index}: {chunk.Type}'s CRC"));
            Assert.Equal(["IHDR", "gAMA", "IDAT", "IEND"], chunks.Select(chunk => chunk.Type));
            var frame = picture.Layout.Frames[index];
            Assert.Equal((uint)frame.Width, BinaryPrimitives.ReadUInt32BigEndian(chunks[0].Payload));
            Assert.Equal((uint)frame.Height, BinaryPrimitives.ReadUInt32BigEndian(chunks[0].Payload.AsSpan(4)));
            // Bit depth, colour type and the rest are the file's own.
            Assert.Equal([8, 6, 0, 0, 0], chunks[0].Payload[8..]);
            Assert.Equal(Inflate(ImageData(frame.Width, frame.Height, (byte)(index + 1))), Inflate(chunks[2].Payload));
        }
    }

    /// <summary>A default image with no fcTL before it is not part of the animation (the specification's "hidden" image).</summary>
    [Fact]
    public void AHiddenDefaultImageIsNoFrame()
    {
        var hidden = Concat(
            PngSignature, Ihdr(2, 2), Actl(1), Chunk("IDAT", ImageData(2, 2, 9)),
            Fctl(0, 2, 2, 0, 0, 1, 10, 0, 0), Fdat(1, ImageData(2, 2, 4)), Chunk("IEND", []));
        var picture = ApngPicture.Read(hidden)!;
        Assert.Single(picture.Layout.Frames);
        Assert.Equal(Inflate(ImageData(2, 2, 4)), Inflate(Chunks(picture.Standalone(0))[1].Payload));
    }

    [Fact]
    public void AnApngThisCannotTakeApartIsNone()
    {
        // A frame reaching past the canvas.
        Assert.Null(ApngPicture.Read(Concat(
            PngSignature, Ihdr(2, 2), Actl(1), Fctl(0, 2, 2, 1, 0, 1, 10, 0, 0), Chunk("IDAT", ImageData(2, 2, 9)), Chunk("IEND", []))));
        // A frame with no data.
        Assert.Null(ApngPicture.Read(Concat(
            PngSignature, Ihdr(2, 2), Actl(2), Fctl(0, 2, 2, 0, 0, 1, 10, 0, 0), Chunk("IDAT", ImageData(2, 2, 9)),
            Fctl(1, 2, 2, 0, 0, 1, 10, 0, 0), Chunk("IEND", []))));
        // A length that runs past the end.
        Assert.Null(ApngPicture.Read(Apng()[..^40]));
        Assert.Null(ApngPicture.Read(StillPng()));
    }

    // ---- an animated WebP -----------------------------------------------------------------------

    private static byte[] Riff(string type, byte[] payload)
    {
        var chunk = new byte[8 + payload.Length + (payload.Length & 1)];
        Encoding.ASCII.GetBytes(type).CopyTo(chunk, 0);
        BinaryPrimitives.WriteUInt32LittleEndian(chunk.AsSpan(4), (uint)payload.Length);
        payload.CopyTo(chunk, 8);
        return chunk;
    }

    private static byte[] U24(int value) => [(byte)value, (byte)(value >> 8), (byte)(value >> 16)];

    private static byte[] Anmf(int x, int y, int width, int height, int duration, byte flags, params byte[][] data) =>
        Riff("ANMF", Concat(U24(x / 2), U24(y / 2), U24(width - 1), U24(height - 1), U24(duration), [flags], Concat(data)));

    private static byte[] WebPFile(params byte[][] chunks)
    {
        var body = Concat("WEBP"u8.ToArray(), Concat(chunks));
        var file = new byte[8 + body.Length];
        "RIFF"u8.CopyTo(file);
        BinaryPrimitives.WriteUInt32LittleEndian(file.AsSpan(4), (uint)body.Length);
        body.CopyTo(file, 8);
        return file;
    }

    private static readonly byte[] Lossless = Riff("VP8L", [0x2F, 1, 2, 3, 4, 5, 6]);
    private static readonly byte[] Alpha = Riff("ALPH", [0, 9, 9]);
    private static readonly byte[] Lossy = Riff("VP8 ", [7, 7, 7, 0x9D, 0x01, 0x2A, 1, 0, 1, 0]);

    /// <summary>
    /// An 8 × 6 animated WebP: a whole-canvas lossless frame, a lossy frame with alpha at (2, 4) that is not blended and is
    /// disposed to background, and a lossy frame with no alpha.
    /// </summary>
    private static byte[] WebP() => WebPFile(
        Riff("VP8X", Concat([0x12, 0, 0, 0], U24(7), U24(5))),
        Riff("ANIM", [255, 255, 255, 255, 0, 0]),
        Anmf(0, 0, 8, 6, 80, 0x00, Lossless),
        Anmf(2, 4, 3, 2, 0, 0x03, Alpha, Lossy),
        Anmf(4, 0, 4, 6, 16_777_215, 0x00, Lossy));

    private static byte[] StillWebP() => WebPFile(Lossless);

    [Fact]
    public void AnAnimatedWebPsFramesAreItsAnmfs()
    {
        var layout = WebPAnimation.Read(WebP())!.Layout;
        Assert.Equal((8, 6), (layout.Width, layout.Height));
        Assert.Equal(
            [
                new AnimationFrame(0, 0, 8, 6, 80, FrameBlend.Over, FrameDisposal.None),
                new AnimationFrame(2, 4, 3, 2, 0, FrameBlend.Source, FrameDisposal.Background),
                new AnimationFrame(4, 0, 4, 6, 16_777_215, FrameBlend.Over, FrameDisposal.None),
            ],
            layout.Frames);
        Assert.Null(WebPAnimation.Read(StillWebP()));
        Assert.Null(WebPAnimation.Read(Apng()));
    }

    /// <summary>
    /// Each frame rewrapped as a still WebP: a lossless one as a simple file, a lossy one with alpha as an extended file
    /// whose VP8X says alpha at the frame's size, a lossy one without alpha as a simple file — each RIFF the size it says.
    /// </summary>
    [Fact]
    public void EachWebPFrameBecomesAStandaloneStill()
    {
        var animation = WebPAnimation.Read(WebP())!;
        var lossless = animation.Standalone(0)!;
        Assert.Equal(WebPFile(Lossless), lossless);
        var withAlpha = animation.Standalone(1)!;
        Assert.Equal(WebPFile(Riff("VP8X", Concat([0x10, 0, 0, 0], U24(2), U24(1))), Alpha, Lossy), withAlpha);
        Assert.Equal((3, 2), StickerFile.Size(withAlpha));
        Assert.False(StickerFile.IsAnimated(withAlpha));
        Assert.Equal(WebPFile(Lossy), animation.Standalone(2)!);
        Assert.All(new[] { lossless, withAlpha }, file =>
            Assert.Equal(file.Length - 8, (int)BinaryPrimitives.ReadUInt32LittleEndian(file.AsSpan(4))));
    }

    [Fact]
    public void AWebPFrameOutsideItsCanvasIsNone()
    {
        Assert.Null(WebPAnimation.Read(WebPFile(
            Riff("VP8X", Concat([0x02, 0, 0, 0], U24(3), U24(3))),
            Anmf(2, 2, 4, 4, 100, 0, Lossless))));
    }

    // ---- the compositor -------------------------------------------------------------------------

    /// <summary>OVER in straight alpha, rounded to the nearest: half-transparent red over blue, and over nothing.</summary>
    [Fact]
    public void OverBlendsAsPorterDuffSays()
    {
        byte[] blue = [255, 0, 0, 255];
        FrameCanvas.Over([0, 0, 255, 128], blue);
        // r = 255·128/255 ≈ 128, b = 255·127/255 = 127, opaque.
        Assert.Equal([127, 0, 128, 255], blue);
        byte[] nothing = [0, 0, 0, 0];
        FrameCanvas.Over([10, 20, 30, 128], nothing);
        Assert.Equal([10, 20, 30, 128], nothing);
        byte[] kept = [1, 2, 3, 4];
        FrameCanvas.Over([9, 9, 9, 0], kept);
        Assert.Equal([1, 2, 3, 4], kept);
        // Two half-covers make three quarters.
        byte[] half = [0, 0, 255, 128];
        FrameCanvas.Over([0, 255, 0, 128], half);
        Assert.Equal(192, half[3]);
    }

    /// <summary>SOURCE replaces, transparency and all: a WebP frame marked "do not blend" punches a hole.</summary>
    [Fact]
    public void SourceReplacesEvenWithTransparency()
    {
        var canvas = new FrameCanvas(2, 1);
        canvas.Draw(new AnimationFrame(0, 0, 2, 1, 100, FrameBlend.Source, FrameDisposal.None), [0, 0, 255, 255, 0, 0, 255, 255]);
        canvas.Draw(new AnimationFrame(1, 0, 1, 1, 100, FrameBlend.Source, FrameDisposal.None), [0, 255, 0, 0]);
        Assert.Equal([0, 0, 255, 255, 0, 255, 0, 0], canvas.Pixels);
        Assert.Throws<ArgumentException>(() =>
            canvas.Draw(new AnimationFrame(0, 0, 2, 1, 100, FrameBlend.Source, FrameDisposal.None), [0, 0, 0, 0]));
    }

    /// <summary>A "previous" disposal after a "previous" one restores each to what was under IT, in turn.</summary>
    [Fact]
    public void PreviousDisposalsInARowEachRestoreTheirOwn()
    {
        var canvas = new FrameCanvas(1, 1);
        byte[] red = [0, 0, 255, 255], green = [0, 255, 0, 255], blue = [255, 0, 0, 255];
        canvas.Draw(new AnimationFrame(0, 0, 1, 1, 100, FrameBlend.Source, FrameDisposal.None), red);
        canvas.Draw(new AnimationFrame(0, 0, 1, 1, 100, FrameBlend.Source, FrameDisposal.Previous), green);
        Assert.Equal(green, canvas.Pixels);
        canvas.Draw(new AnimationFrame(0, 0, 1, 1, 100, FrameBlend.Over, FrameDisposal.Previous), [0, 0, 0, 0]);
        // Green was put back to red before this empty frame drew nothing over it.
        Assert.Equal(red, canvas.Pixels);
        canvas.Draw(new AnimationFrame(0, 0, 1, 1, 100, FrameBlend.Over, FrameDisposal.None), [0, 0, 0, 0]);
        Assert.Equal(red, canvas.Pixels);
        canvas.Draw(new AnimationFrame(0, 0, 1, 1, 100, FrameBlend.Over, FrameDisposal.None), blue);
        Assert.Equal(blue, canvas.Pixels);
    }

    /// <summary>A codec's whole, already composited frames go through the same compositor as replacements.</summary>
    [Fact]
    public void AWholeFrameLayoutReplacesTheCanvasEachTime()
    {
        var whole = WebPAnimation.Read(WebP())!.Layout.Whole();
        Assert.All(whole.Frames, frame =>
        {
            Assert.Equal((0, 0, 8, 6), (frame.X, frame.Y, frame.Width, frame.Height));
            Assert.Equal(FrameBlend.Source, frame.Blend);
            Assert.Equal(FrameDisposal.None, frame.Disposal);
        });
        Assert.Equal([80u, 0u, 16_777_215u], whole.StatedMs);
    }
}
