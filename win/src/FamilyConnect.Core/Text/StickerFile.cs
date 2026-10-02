using System.Buffers.Binary;

namespace FamilyConnect.Core;

/// <summary>
/// What a sticker's BYTES say about themselves: which of the two accepted types they are, how big
/// the picture is, and whether it moves (docs/protocol.md, "What a sticker is made of").
/// </summary>
/// <remarks>
/// <para>
/// <b>READ FROM THE BYTES, NEVER FROM THE NAME.</b> The server checks a magic number and not an
/// extension, so a <c>.png</c> that is really a JPEG would be refused at the claim with a request
/// already spent — and a client must refuse "where the person is choosing a picture", which means
/// knowing before anything is uploaded.
/// </para>
/// <para>
/// <b>NOTHING HERE DECODES A PIXEL.</b> The sizes and the animation flag are in the container's
/// own headers, so this runs wherever <c>dotnet</c> runs and asks nothing of a Windows codec —
/// which matters, because whether an animated sticker may be re-encoded is a decision (it may
/// not: no client can do it without losing the animation) and decisions are tested off Windows.
/// </para>
/// <para>
/// Every read is bounded: these are bytes somebody picked from a disk or another member uploaded,
/// and a length field that lies must end the walk, not the process.
/// </para>
/// </remarks>
public static class StickerFile
{
    public const string WebP = "image/webp";
    public const string Png = "image/png";

    /// <summary>The pixel box a client fits a sticker it MAKES into. A client rule; the server never decodes.</summary>
    public const int Edge = 512;

    private static ReadOnlySpan<byte> PngMagic => [0x89, (byte)'P', (byte)'N', (byte)'G', 0x0D, 0x0A, 0x1A, 0x0A];

    /// <summary>Whether a type is one a sticker may be: WebP or PNG, and nothing else.</summary>
    public static bool IsStickerType(string? mime) =>
        MediaPrep.Essence(mime ?? string.Empty) is WebP or Png;

    /// <summary>
    /// The type these bytes ARE, by the server's own check — <c>RIFF</c> at 0 and <c>WEBP</c> at 8
    /// (the four bytes between are the file's length and are not checked), or PNG's eight — or
    /// null when they are neither.
    /// </summary>
    public static string? Mime(ReadOnlySpan<byte> head)
    {
        if (head.Length >= 12 && head[..4].SequenceEqual("RIFF"u8) && head[8..12].SequenceEqual("WEBP"u8))
        {
            return WebP;
        }
        return head.StartsWith(PngMagic) ? Png : null;
    }

    /// <summary>The picture's own pixel size, or null when the header does not say.</summary>
    public static (int Width, int Height)? Size(ReadOnlySpan<byte> bytes) => Mime(bytes) switch
    {
        WebP => WebPSize(bytes),
        Png => PngSize(bytes),
        _ => null,
    };

    /// <summary>
    /// Whether the picture MOVES: a WebP whose header sets the animation flag, or a PNG carrying
    /// an animation control chunk before its first image data (APNG). An animated sticker is
    /// stored and sent as it is, always — nothing here can re-encode one and keep it moving.
    /// </summary>
    public static bool IsAnimated(ReadOnlySpan<byte> bytes)
    {
        switch (Mime(bytes))
        {
            case WebP:
                foreach (var chunk in new RiffChunks(bytes))
                {
                    if (chunk.Is("VP8X"u8))
                    {
                        return chunk.Payload.Length >= 1 && (chunk.Payload[0] & 0x02) != 0;
                    }
                    if (chunk.Is("VP8 "u8) || chunk.Is("VP8L"u8))
                    {
                        // A simple file: one frame, by construction.
                        return false;
                    }
                }
                return false;
            case Png:
                var at = PngMagic.Length;
                while (at + 8 <= bytes.Length)
                {
                    var length = BinaryPrimitives.ReadUInt32BigEndian(bytes[at..]);
                    var type = bytes.Slice(at + 4, 4);
                    if (type.SequenceEqual("acTL"u8))
                    {
                        return true;
                    }
                    if (type.SequenceEqual("IDAT"u8) || type.SequenceEqual("IEND"u8))
                    {
                        // The animation control chunk must come BEFORE the image data.
                        return false;
                    }
                    // length + type + data + crc
                    var next = at + 12L + length;
                    if (next > bytes.Length)
                    {
                        return false;
                    }
                    at = (int)next;
                }
                return false;
            default:
                return false;
        }
    }

    /// <summary>
    /// Whether these bytes are a GIF with MORE THAN ONE picture in it — an animation, which no
    /// client here can make a sticker of without flattening it to one frame. A GIF is never a
    /// sticker as it is (a sticker is a WebP or a PNG); a STILL one may be redrawn as one, and
    /// this is how the two are told apart before anything is decoded.
    /// </summary>
    /// <remarks>
    /// Walked block by block — the header, the colour tables, each image descriptor and its data
    /// sub-blocks, each extension — and stopped at the second image. A length that runs past the
    /// file ends the walk with what was counted, like every other read here.
    /// </remarks>
    public static bool IsAnimatedGif(ReadOnlySpan<byte> bytes)
    {
        if (!IsGif(bytes) || bytes.Length < 13)
        {
            return false;
        }
        // Header (6), then the logical screen: width (2), height (2), packed (1), background (1), aspect (1).
        long at = 13 + ColourTable(bytes[10]);
        var images = 0;
        while (at < bytes.Length)
        {
            var block = bytes[(int)at];
            switch (block)
            {
                case 0x2C:
                    // An image: left, top, width, height (2 each), packed (1), then its own colour
                    // table, the LZW code size (1), and the data in sub-blocks.
                    if (++images > 1)
                    {
                        return true;
                    }
                    if (at + 10 > bytes.Length)
                    {
                        return false;
                    }
                    at = SubBlocks(bytes, at + 10 + ColourTable(bytes[(int)at + 9]) + 1);
                    break;
                case 0x21:
                    // An extension: its label (1), then sub-blocks.
                    at = SubBlocks(bytes, at + 2);
                    break;
                default:
                    // The trailer (0x3B), or bytes that are not a GIF's: either way the end.
                    return false;
            }
        }
        return false;

        static int ColourTable(byte packed) => (packed & 0x80) != 0 ? 3 << ((packed & 0x07) + 1) : 0;

        static long SubBlocks(ReadOnlySpan<byte> bytes, long at)
        {
            while (at < bytes.Length)
            {
                var length = bytes[(int)at];
                at += 1 + length;
                if (length == 0)
                {
                    return at;
                }
            }
            return bytes.Length;
        }
    }

    /// <summary>Whether these bytes begin as a GIF does: <c>GIF87a</c> or <c>GIF89a</c>.</summary>
    public static bool IsGif(ReadOnlySpan<byte> head) =>
        head.StartsWith("GIF87a"u8) || head.StartsWith("GIF89a"u8);

    /// <summary>
    /// How long each frame of an animated WebP stands, in milliseconds, in file order — empty for
    /// anything that is not one. What the file says, unclamped: what a player does with a
    /// zero is the player's rule.
    /// </summary>
    public static IReadOnlyList<int> FrameDurations(ReadOnlySpan<byte> bytes)
    {
        if (Mime(bytes) != WebP)
        {
            return [];
        }
        var durations = new List<int>();
        foreach (var chunk in new RiffChunks(bytes))
        {
            // ANMF: x(3) y(3) width-1(3) height-1(3) duration(3) flags(1), then the frame's data.
            if (chunk.Is("ANMF"u8) && chunk.Payload.Length >= 16)
            {
                durations.Add(chunk.Payload[12] | (chunk.Payload[13] << 8) | (chunk.Payload[14] << 16));
            }
        }
        return durations;
    }

    private static (int Width, int Height)? PngSize(ReadOnlySpan<byte> bytes)
    {
        // Signature (8), then IHDR: length (4), "IHDR" (4), width (4), height (4), big-endian.
        if (bytes.Length < 24 || !bytes.Slice(12, 4).SequenceEqual("IHDR"u8))
        {
            return null;
        }
        var width = BinaryPrimitives.ReadUInt32BigEndian(bytes[16..]);
        var height = BinaryPrimitives.ReadUInt32BigEndian(bytes[20..]);
        return Plausible(width, height);
    }

    private static (int Width, int Height)? WebPSize(ReadOnlySpan<byte> bytes)
    {
        foreach (var chunk in new RiffChunks(bytes))
        {
            var payload = chunk.Payload;
            if (chunk.Is("VP8X"u8))
            {
                // flags (1), reserved (3), canvas width-1 (3), canvas height-1 (3), little-endian.
                if (payload.Length < 10)
                {
                    return null;
                }
                uint width = (uint)(payload[4] | (payload[5] << 8) | (payload[6] << 16)) + 1;
                uint height = (uint)(payload[7] | (payload[8] << 8) | (payload[9] << 16)) + 1;
                return Plausible(width, height);
            }
            if (chunk.Is("VP8L"u8))
            {
                // 0x2F, then width-1 and height-1 packed into 14 bits each.
                if (payload.Length < 5 || payload[0] != 0x2F)
                {
                    return null;
                }
                var bits = BinaryPrimitives.ReadUInt32LittleEndian(payload[1..]);
                return Plausible((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1);
            }
            if (chunk.Is("VP8 "u8))
            {
                // A 3-byte frame tag, the start code 9D 01 2A, then 14 bits of each dimension.
                if (payload.Length < 10 || payload[3] != 0x9D || payload[4] != 0x01 || payload[5] != 0x2A)
                {
                    return null;
                }
                return Plausible(
                    (uint)(BinaryPrimitives.ReadUInt16LittleEndian(payload[6..]) & 0x3FFF),
                    (uint)(BinaryPrimitives.ReadUInt16LittleEndian(payload[8..]) & 0x3FFF));
            }
        }
        return null;
    }

    /// <summary>A size a picture can have; a zero, or one past what an int carries, is a header that lies.</summary>
    private static (int Width, int Height)? Plausible(uint width, uint height) =>
        width is > 0 and <= int.MaxValue && height is > 0 and <= int.MaxValue
            ? ((int)width, (int)height)
            : null;

    /// <summary>The chunks of a RIFF/WEBP file after its twelve-byte header, walked without trusting a length.</summary>
    private ref struct RiffChunks(ReadOnlySpan<byte> bytes)
    {
        private readonly ReadOnlySpan<byte> bytes = bytes;
        private int at = 12;

        public RiffChunk Current { get; private set; }

        public readonly RiffChunks GetEnumerator() => this;

        public bool MoveNext()
        {
            if (at < 0 || at + 8 > bytes.Length)
            {
                return false;
            }
            var size = BinaryPrimitives.ReadUInt32LittleEndian(bytes[(at + 4)..]);
            var start = at + 8;
            // A chunk that claims more than the file holds is read as far as the file goes — the
            // header fields this class wants are at its front — and ends the walk.
            var held = (int)Math.Min(size, (uint)(bytes.Length - start));
            Current = new RiffChunk(bytes.Slice(at, 4), bytes.Slice(start, held));
            // Chunks are padded to an even length.
            var next = start + (long)size + (size & 1);
            at = next > bytes.Length ? -1 : (int)next;
            return true;
        }
    }

    private readonly ref struct RiffChunk(ReadOnlySpan<byte> name, ReadOnlySpan<byte> payload)
    {
        private readonly ReadOnlySpan<byte> name = name;

        public ReadOnlySpan<byte> Payload { get; } = payload;

        public bool Is(ReadOnlySpan<byte> fourcc) => name.SequenceEqual(fourcc);
    }
}
