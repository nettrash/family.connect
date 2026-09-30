using System.Buffers.Binary;
using FamilyConnect.Core;

namespace FamilyConnect.Core.Tests.Text;

/// <summary>
/// What a sticker's bytes say about themselves (docs/protocol.md, "What a sticker is made of"):
/// the two accepted types by the server's own magic numbers, the pixel size, and whether it moves
/// — all from the container's headers, with no codec in sight.
/// </summary>
public class StickerFileTests
{
    // ---- bytes built the way the formats say ------------------------------------

    private static byte[] Riff(params byte[][] chunks)
    {
        var body = chunks.SelectMany(chunk => chunk).ToArray();
        var file = new byte[12 + body.Length];
        "RIFF"u8.CopyTo(file);
        BinaryPrimitives.WriteUInt32LittleEndian(file.AsSpan(4), (uint)(4 + body.Length));
        "WEBP"u8.CopyTo(file.AsSpan(8));
        body.CopyTo(file, 12);
        return file;
    }

    private static byte[] Chunk(string name, byte[] payload)
    {
        // Padded to an even length, as RIFF pads.
        var chunk = new byte[8 + payload.Length + (payload.Length & 1)];
        System.Text.Encoding.ASCII.GetBytes(name).CopyTo(chunk, 0);
        BinaryPrimitives.WriteUInt32LittleEndian(chunk.AsSpan(4), (uint)payload.Length);
        payload.CopyTo(chunk, 8);
        return chunk;
    }

    private static byte[] Vp8x(int width, int height, bool animated)
    {
        var payload = new byte[10];
        payload[0] = (byte)(0x10 | (animated ? 0x02 : 0x00));
        Write24(payload.AsSpan(4), width - 1);
        Write24(payload.AsSpan(7), height - 1);
        return Chunk("VP8X", payload);
    }

    private static byte[] Anmf(int durationMs, int frameBytes = 3)
    {
        var payload = new byte[16 + frameBytes];
        Write24(payload.AsSpan(12), durationMs);
        return Chunk("ANMF", payload);
    }

    private static byte[] LossyFrame(int width, int height)
    {
        var payload = new byte[10];
        payload[3] = 0x9D;
        payload[4] = 0x01;
        payload[5] = 0x2A;
        BinaryPrimitives.WriteUInt16LittleEndian(payload.AsSpan(6), (ushort)width);
        BinaryPrimitives.WriteUInt16LittleEndian(payload.AsSpan(8), (ushort)height);
        return Chunk("VP8 ", payload);
    }

    private static byte[] LosslessFrame(int width, int height)
    {
        var payload = new byte[5];
        payload[0] = 0x2F;
        BinaryPrimitives.WriteUInt32LittleEndian(
            payload.AsSpan(1), (uint)(width - 1) | ((uint)(height - 1) << 14));
        return Chunk("VP8L", payload);
    }

    private static void Write24(Span<byte> at, int value)
    {
        at[0] = (byte)value;
        at[1] = (byte)(value >> 8);
        at[2] = (byte)(value >> 16);
    }

    private static byte[] Png(int width, int height, params string[] chunksAfterHeader)
    {
        var bytes = new List<byte> { 0x89, (byte)'P', (byte)'N', (byte)'G', 0x0D, 0x0A, 0x1A, 0x0A };
        var header = new byte[13];
        BinaryPrimitives.WriteUInt32BigEndian(header, (uint)width);
        BinaryPrimitives.WriteUInt32BigEndian(header.AsSpan(4), (uint)height);
        bytes.AddRange(PngChunk("IHDR", header));
        foreach (var name in chunksAfterHeader)
        {
            bytes.AddRange(PngChunk(name, new byte[8]));
        }
        bytes.AddRange(PngChunk("IEND", []));
        return [.. bytes];
    }

    private static byte[] PngChunk(string name, byte[] data)
    {
        var chunk = new byte[12 + data.Length];
        BinaryPrimitives.WriteUInt32BigEndian(chunk, (uint)data.Length);
        System.Text.Encoding.ASCII.GetBytes(name).CopyTo(chunk, 4);
        data.CopyTo(chunk, 8);
        return chunk;
    }

    // ---- the type ----------------------------------------------------------------

    /// <summary>
    /// The server's own check: <c>RIFF</c> at 0 and <c>WEBP</c> at 8, and the four bytes between
    /// are the file's length and are NOT checked.
    /// </summary>
    [Fact]
    public void AWebPIsKnownByRiffAtZeroAndWebpAtEight()
    {
        Assert.Equal(StickerFile.WebP, StickerFile.Mime(Riff(LossyFrame(512, 512))));
        byte[] anyLength = [.. "RIFF"u8, 0xFF, 0xFF, 0xFF, 0xFF, .. "WEBP"u8];
        Assert.Equal(StickerFile.WebP, StickerFile.Mime(anyLength));
        // A RIFF that is something else — a WAV is one — is not a sticker.
        byte[] wave = [.. "RIFF"u8, 0, 0, 0, 0, .. "WAVE"u8];
        Assert.Null(StickerFile.Mime(wave));
    }

    [Fact]
    public void APngIsKnownByItsEightBytesAndNothingElseIsASticker()
    {
        Assert.Equal(StickerFile.Png, StickerFile.Mime(Png(512, 512)));
        // A JPEG named .png is a JPEG: read from the bytes, never from the name.
        Assert.Null(StickerFile.Mime([0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0]));
        Assert.Null(StickerFile.Mime("GIF89a"u8));
        Assert.Null(StickerFile.Mime([]));
    }

    [Theory]
    [InlineData("image/webp", true)]
    [InlineData("image/png", true)]
    [InlineData("IMAGE/WEBP; q=1", true)]
    [InlineData("image/jpeg", false)]
    [InlineData("image/gif", false)]
    [InlineData("", false)]
    [InlineData(null, false)]
    public void OnlyWebPAndPngAreStickerTypes(string? mime, bool sticker)
    {
        Assert.Equal(sticker, StickerFile.IsStickerType(mime));
    }

    // ---- the size ----------------------------------------------------------------

    [Fact]
    public void ThePixelSizeIsReadFromEachOfTheFourHeaders()
    {
        Assert.Equal((512, 384), StickerFile.Size(Png(512, 384)));
        Assert.Equal((512, 384), StickerFile.Size(Riff(LossyFrame(512, 384))));
        Assert.Equal((512, 384), StickerFile.Size(Riff(LosslessFrame(512, 384))));
        // The extended header's CANVAS, which is the picture — not whatever the first frame covers.
        Assert.Equal((512, 384), StickerFile.Size(Riff(Vp8x(512, 384, animated: true), Anmf(40), Anmf(40))));
        Assert.Equal((16383, 1), StickerFile.Size(Riff(LosslessFrame(16383, 1))));
    }

    [Fact]
    public void ASizeTheHeaderDoesNotSayIsNotGuessed()
    {
        Assert.Null(StickerFile.Size([]));
        Assert.Null(StickerFile.Size(Riff()));
        Assert.Null(StickerFile.Size(Riff(Chunk("VP8 ", new byte[10]))));
        Assert.Null(StickerFile.Size(Riff(Chunk("VP8L", [0x00, 1, 2, 3, 4]))));
        // A PNG cut off before its header finished.
        Assert.Null(StickerFile.Size(Png(512, 512).AsSpan(0, 20)));
        Assert.Null(StickerFile.Size(Png(0, 512)));
    }

    // ---- whether it moves ---------------------------------------------------------

    [Fact]
    public void AnAnimatedWebPSaysSoInItsExtendedHeader()
    {
        Assert.True(StickerFile.IsAnimated(Riff(Vp8x(512, 512, animated: true), Anmf(40), Anmf(40))));
        Assert.False(StickerFile.IsAnimated(Riff(Vp8x(512, 512, animated: false), LossyFrame(512, 512))));
        // A simple file is one frame by construction.
        Assert.False(StickerFile.IsAnimated(Riff(LossyFrame(512, 512))));
        Assert.False(StickerFile.IsAnimated(Riff(LosslessFrame(512, 512))));
    }

    /// <summary>
    /// APNG: an animation control chunk BEFORE the image data. Windows Imaging will draw its first
    /// frame, and this client must still never re-encode it.
    /// </summary>
    [Fact]
    public void AnAnimatedPngIsOneWithAnAnimationControlChunkBeforeItsData()
    {
        Assert.True(StickerFile.IsAnimated(Png(512, 512, "acTL", "IDAT")));
        Assert.False(StickerFile.IsAnimated(Png(512, 512, "IDAT")));
        // After the data it is not the chunk the format means.
        Assert.False(StickerFile.IsAnimated(Png(512, 512, "IDAT", "acTL")));
        Assert.False(StickerFile.IsAnimated([]));
    }

    [Fact]
    public void FrameDurationsAreReadInFileOrderAndUnclamped()
    {
        var file = Riff(Vp8x(512, 512, animated: true), Anmf(40), Anmf(0), Anmf(1000, frameBytes: 5), Anmf(70_000));

        Assert.Equal([40, 0, 1000, 70_000], StickerFile.FrameDurations(file));
        Assert.Empty(StickerFile.FrameDurations(Riff(LossyFrame(512, 512))));
        Assert.Empty(StickerFile.FrameDurations(Png(512, 512, "acTL", "IDAT")));
    }

    // ---- bytes that lie -----------------------------------------------------------

    /// <summary>
    /// These are bytes somebody picked from a disk or another member uploaded: a length field
    /// that lies must end the walk, not the process.
    /// </summary>
    [Fact]
    public void ALengthThatLiesEndsTheWalkAndNothingElse()
    {
        // A chunk claiming four gigabytes, with the header fields still at its front.
        var liar = Riff(Vp8x(512, 384, animated: true), Anmf(40));
        BinaryPrimitives.WriteUInt32LittleEndian(liar.AsSpan(16), uint.MaxValue);
        Assert.Equal((512, 384), StickerFile.Size(liar));
        Assert.True(StickerFile.IsAnimated(liar));
        Assert.Empty(StickerFile.FrameDurations(liar));

        // A PNG chunk whose length runs off the end.
        var png = Png(512, 512, "tEXt", "acTL", "IDAT");
        BinaryPrimitives.WriteUInt32BigEndian(png.AsSpan(33), uint.MaxValue);
        Assert.False(StickerFile.IsAnimated(png));

        // And every prefix of a good file is something this can be asked about.
        var good = Riff(Vp8x(512, 384, animated: true), Anmf(40), Anmf(60));
        for (var length = 0; length <= good.Length; length++)
        {
            var cut = good.AsSpan(0, length);
            _ = StickerFile.Mime(cut);
            _ = StickerFile.Size(cut);
            _ = StickerFile.IsAnimated(cut);
            _ = StickerFile.FrameDurations(cut);
        }
        var still = Png(512, 512, "acTL", "IDAT");
        for (var length = 0; length <= still.Length; length++)
        {
            _ = StickerFile.Size(still.AsSpan(0, length));
            _ = StickerFile.IsAnimated(still.AsSpan(0, length));
        }
    }

    // ---- a GIF: never a sticker as it is, and a moving one never made into one ------------------

    /// <summary>A GIF the way the format says: header, screen, an optional colour table, then the blocks given.</summary>
    private static byte[] Gif(bool globalTable, params byte[][] blocks)
    {
        var file = new List<byte>();
        file.AddRange("GIF89a"u8.ToArray());
        // Width 2, height 2, packed (a 4-entry table when there is one), background, aspect.
        file.AddRange([2, 0, 2, 0, (byte)(globalTable ? 0x81 : 0x00), 0, 0]);
        if (globalTable)
        {
            file.AddRange(new byte[12]);
        }
        foreach (var block in blocks)
        {
            file.AddRange(block);
        }
        file.Add(0x3B);
        return [.. file];
    }

    /// <summary>One picture: its descriptor, an optional local colour table, the code size, and its data in sub-blocks.</summary>
    private static byte[] GifImage(bool localTable = false)
    {
        var image = new List<byte> { 0x2C, 0, 0, 0, 0, 2, 0, 2, 0, (byte)(localTable ? 0x80 : 0x00) };
        if (localTable)
        {
            image.AddRange(new byte[6]);
        }
        // Data that LOOKS like an image separator, inside a sub-block: it must be stepped over, not counted.
        image.AddRange([2, 3, 0x2C, 0x2C, 0x2C, 1, 0x2C, 0]);
        return [.. image];
    }

    /// <summary>A graphic control extension, as every frame of an animation carries — and a still GIF may too.</summary>
    private static byte[] GifControl() => [0x21, 0xF9, 4, 0, 10, 0, 0, 0];

    /// <summary>The "loop for ever" application extension.</summary>
    private static byte[] GifLoop() =>
        [0x21, 0xFF, 11, .. "NETSCAPE2.0"u8, 3, 1, 0, 0, 0];

    [Fact]
    public void AGifWithMoreThanOnePictureIsAnimated()
    {
        Assert.True(StickerFile.IsAnimatedGif(Gif(true, GifLoop(), GifControl(), GifImage(), GifControl(), GifImage())));
        Assert.True(StickerFile.IsAnimatedGif(Gif(false, GifImage(localTable: true), GifImage(localTable: true))));
        // GIF87a has no extensions at all, and may still hold two pictures.
        var old = Gif(true, GifImage(), GifImage());
        "GIF87a"u8.CopyTo(old);
        Assert.True(StickerFile.IsAnimatedGif(old));
    }

    /// <summary>
    /// A STILL GIF IS A STILL PICTURE, whatever extensions it carries — a loop block and a control
    /// block do not make one picture move — and bytes inside its data are never counted as pictures.
    /// </summary>
    [Fact]
    public void AGifWithOnePictureIsStill()
    {
        Assert.False(StickerFile.IsAnimatedGif(Gif(true, GifImage())));
        Assert.False(StickerFile.IsAnimatedGif(Gif(true, GifLoop(), GifControl(), GifImage(localTable: true))));
        Assert.False(StickerFile.IsAnimatedGif(Gif(false)));
    }

    [Fact]
    public void OnlyAGifIsAskedWhetherItIsAnAnimatedGif()
    {
        Assert.True(StickerFile.IsGif("GIF89a"u8));
        Assert.True(StickerFile.IsGif("GIF87a"u8));
        Assert.False(StickerFile.IsGif("GIF90a"u8));
        Assert.False(StickerFile.IsGif("GIF"u8));
        // A GIF is never a sticker as it is: the two accepted types are read from the bytes, and it is neither.
        Assert.Null(StickerFile.Mime(Gif(true, GifImage(), GifImage())));
        Assert.False(StickerFile.IsAnimatedGif(Riff(Vp8x(512, 384, animated: true), Anmf(40), Anmf(60))));
        Assert.False(StickerFile.IsAnimatedGif(Png(512, 512, "acTL", "IDAT")));
        Assert.False(StickerFile.IsAnimatedGif([0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
    }

    /// <summary>Every prefix of an animated GIF, and one whose lengths lie, ends the walk and nothing else.</summary>
    [Fact]
    public void AGifThatIsCutShortOrLiesEndsTheWalk()
    {
        var moving = Gif(true, GifLoop(), GifControl(), GifImage(localTable: true), GifControl(), GifImage());
        for (var length = 0; length <= moving.Length; length++)
        {
            _ = StickerFile.IsAnimatedGif(moving.AsSpan(0, length));
        }
        // A sub-block that claims 255 bytes the file does not have.
        var liar = Gif(true, GifImage(), GifImage());
        liar[13 + 12 + 10 + 1] = 0xFF;
        Assert.False(StickerFile.IsAnimatedGif(liar));
        // A screen that promises a 768-byte colour table in a 20-byte file.
        var table = Gif(false, GifImage(), GifImage());
        table[10] = 0x87;
        _ = StickerFile.IsAnimatedGif(table);
        Assert.False(StickerFile.IsAnimatedGif(table.AsSpan(0, 20)));
    }
}
