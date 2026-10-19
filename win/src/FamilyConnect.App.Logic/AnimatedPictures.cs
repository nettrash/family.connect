using System.Buffers.Binary;
using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

/// <summary>The three animated pictures the protocol turns into a video (docs/protocol.md, "Animations", "What becomes one").</summary>
public enum AnimationFormat
{
    Gif,
    WebP,
    Apng,
}

/// <summary>How a frame's pixels meet what is already on the canvas.</summary>
public enum FrameBlend
{
    /// <summary>They replace it, transparency and all (APNG <c>APNG_BLEND_OP_SOURCE</c>, a WebP frame marked "do not blend").</summary>
    Source,

    /// <summary>They are drawn over it (APNG <c>OVER</c>, WebP's alpha-blending, and every GIF frame: a transparent index draws nothing).</summary>
    Over,
}

/// <summary>What happens to a frame's rectangle once its time is up, before the next frame is drawn.</summary>
public enum FrameDisposal
{
    /// <summary>It stays (GIF 0 and 1, and the values the format leaves undefined).</summary>
    None,

    /// <summary>It is cleared to transparent — what every browser does with GIF's "restore to background" too.</summary>
    Background,

    /// <summary>It goes back to what was there before the frame was drawn.</summary>
    Previous,
}

/// <summary>One frame of an animated picture: where it is drawn, how long it is stated to stand, and how it is composited.</summary>
public sealed record AnimationFrame(
    int X,
    int Y,
    int Width,
    int Height,
    uint StatedMs,
    FrameBlend Blend,
    FrameDisposal Disposal);

/// <summary>An animated picture's canvas and frames, as its own bytes state them.</summary>
public sealed record AnimationLayout(AnimationFormat Format, int Width, int Height, IReadOnlyList<AnimationFrame> Frames)
{
    /// <summary>Each frame's stated length, in order — what <see cref="AnimationTiming.Plan"/> is given.</summary>
    public IReadOnlyList<uint> StatedMs => [.. Frames.Select(frame => frame.StatedMs)];

    /// <summary>
    /// The same animation as a codec that hands out WHOLE, already composited pictures would give it: every frame the whole
    /// canvas, replacing the last.
    /// </summary>
    public AnimationLayout Whole() => this with
    {
        Frames = [.. Frames.Select(frame => frame with
        {
            X = 0,
            Y = 0,
            Width = Width,
            Height = Height,
            Blend = FrameBlend.Source,
            Disposal = FrameDisposal.None,
        })],
    };
}

/// <summary>
/// Which picked pictures are ANIMATIONS, and how one is drawn once it has arrived (docs/protocol.md, "Animations").
/// </summary>
/// <remarks>
/// "Animated" is the picture's own statement, read from its bytes with the tests "Sticker pack" already uses
/// (<see cref="StickerFile"/>): a GIF with a second image, a WebP whose <c>VP8X</c> sets the animation flag, a PNG with an
/// <c>acTL</c> before its <c>IDAT</c>. A still GIF, WebP or BMP is none of these, and goes exactly as before.
/// </remarks>
public static class AnimationPrep
{
    /// <summary>Above this an animation's tile shows its poster and plays only when clicked — the bound on what a tile downloads.</summary>
    public const long TileBytes = 8L * 1024 * 1024;

    /// <summary>Whether these first bytes could begin an animated picture, so the whole file is worth reading to find out.</summary>
    public static bool MayBe(ReadOnlySpan<byte> head) => StickerFile.IsGif(head) || StickerFile.Mime(head) is not null;

    /// <summary>Which animation these bytes are, or null for anything that is not one — a still picture included.</summary>
    public static AnimationFormat? Animated(ReadOnlySpan<byte> bytes)
    {
        if (StickerFile.IsAnimatedGif(bytes))
        {
            return AnimationFormat.Gif;
        }
        return StickerFile.Mime(bytes) switch
        {
            StickerFile.WebP when StickerFile.IsAnimated(bytes) => AnimationFormat.WebP,
            StickerFile.Png when StickerFile.IsAnimated(bytes) => AnimationFormat.Apng,
            _ => null,
        };
    }

    /// <summary>
    /// Whether a picked file goes as an animation: where the chat takes them (<paramref name="allowed"/> — not the
    /// assistant's, whose model is shown a photograph; not the board, nor a profile picture), within the attachment
    /// ceiling, and animated by its own statement. Anything else takes the route it always took (<see cref="MediaPrep.Route"/>).
    /// </summary>
    public static AnimationFormat? Route(bool allowed, long size, ReadOnlySpan<byte> bytes) =>
        allowed && size > 0 && size <= MediaPrep.SizeLimit ? Animated(bytes) : null;

    /// <summary>The canvas and frames of an animated picture, or null when its bytes cannot be walked.</summary>
    public static AnimationLayout? Layout(byte[] bytes) => Animated(bytes) switch
    {
        AnimationFormat.Gif => GifPicture.Read(bytes)?.Layout,
        AnimationFormat.WebP => WebPAnimation.Read(bytes)?.Layout,
        AnimationFormat.Apng => ApngPicture.Read(bytes)?.Layout,
        _ => null,
    };

    /// <summary>
    /// Whether an arrived animation's tile plays by itself: the system has not asked for less motion, and the video is within
    /// <see cref="TileBytes"/> — the one video a tile downloads to draw itself. A size nobody stated is not presumed small.
    /// </summary>
    public static bool PlaysInTile(bool animationsWanted, long? size) =>
        animationsWanted && size is > 0 and <= TileBytes;
}

/// <summary>
/// A GIF decoded by this client, not by Windows: its blocks walked, each image's LZW data unpacked into its own rectangle.
/// </summary>
/// <remarks>
/// <para>
/// <b>WHY NOT WINDOWS IMAGING.</b> WIC's GIF decoder hands out each image as the RAW sub-rectangle the file stores, with its
/// disposal and transparency left to the caller in metadata — so compositing was ours either way, and decoding as well
/// makes the whole picture a function of its bytes that runs, and is tested, wherever <c>dotnet</c> does.
/// </para>
/// <para>
/// <b>EVERY READ IS BOUNDED.</b> These are bytes somebody picked from a disk. A length that runs past the end ends the walk;
/// an image whose data stops early leaves the rest of its rectangle undrawn, as a browser does; a frame with no colour table
/// at all cannot be drawn, and the picture is not one this can read.
/// </para>
/// </remarks>
public sealed class GifPicture
{
    private readonly byte[] bytes;
    private readonly List<Image> images;

    private sealed record Image(
        AnimationFrame Frame,
        int PaletteAt,
        int PaletteEntries,
        int? Transparent,
        bool Interlaced,
        int DataAt);

    private GifPicture(byte[] bytes, int width, int height, List<Image> images)
    {
        this.bytes = bytes;
        this.images = images;
        Layout = new AnimationLayout(AnimationFormat.Gif, width, height, [.. images.Select(image => image.Frame)]);
    }

    public AnimationLayout Layout { get; }

    /// <summary>The picture's blocks, or null when they cannot be walked or hold no image.</summary>
    public static GifPicture? Read(byte[] bytes)
    {
        if (!StickerFile.IsGif(bytes) || bytes.Length < 13)
        {
            return null;
        }
        var width = BinaryPrimitives.ReadUInt16LittleEndian(bytes.AsSpan(6));
        var height = BinaryPrimitives.ReadUInt16LittleEndian(bytes.AsSpan(8));
        var packed = bytes[10];
        var globalAt = -1;
        var globalEntries = 0;
        var at = 13;
        if ((packed & 0x80) != 0)
        {
            globalEntries = 1 << ((packed & 0x07) + 1);
            globalAt = at;
            at += 3 * globalEntries;
        }
        var images = new List<Image>();
        // What the next image's Graphic Control Extension said, if one came.
        ushort delay = 0;
        var disposal = FrameDisposal.None;
        int? transparent = null;
        while (at < bytes.Length)
        {
            switch (bytes[at])
            {
                case 0x21:
                    if (at + 2 > bytes.Length)
                    {
                        return Finished();
                    }
                    if (bytes[at + 1] == 0xF9 && at + 8 <= bytes.Length && bytes[at + 2] >= 4)
                    {
                        var control = bytes[at + 3];
                        disposal = ((control >> 2) & 0x07) switch
                        {
                            2 => FrameDisposal.Background,
                            3 => FrameDisposal.Previous,
                            _ => FrameDisposal.None,
                        };
                        delay = BinaryPrimitives.ReadUInt16LittleEndian(bytes.AsSpan(at + 4));
                        transparent = (control & 0x01) != 0 ? bytes[at + 6] : null;
                    }
                    at = SubBlocks(bytes, at + 2);
                    break;
                case 0x2C:
                    if (at + 10 > bytes.Length)
                    {
                        return Finished();
                    }
                    var left = BinaryPrimitives.ReadUInt16LittleEndian(bytes.AsSpan(at + 1));
                    var top = BinaryPrimitives.ReadUInt16LittleEndian(bytes.AsSpan(at + 3));
                    var frameWidth = BinaryPrimitives.ReadUInt16LittleEndian(bytes.AsSpan(at + 5));
                    var frameHeight = BinaryPrimitives.ReadUInt16LittleEndian(bytes.AsSpan(at + 7));
                    var imagePacked = bytes[at + 9];
                    at += 10;
                    var paletteAt = globalAt;
                    var paletteEntries = globalEntries;
                    if ((imagePacked & 0x80) != 0)
                    {
                        paletteEntries = 1 << ((imagePacked & 0x07) + 1);
                        paletteAt = at;
                        at += 3 * paletteEntries;
                    }
                    if (at >= bytes.Length)
                    {
                        return Finished();
                    }
                    var dataAt = at;
                    at = SubBlocks(bytes, at + 1);
                    if (paletteAt < 0 || paletteAt + (3 * paletteEntries) > bytes.Length)
                    {
                        // Nothing to say what its indexes are: not a picture this can draw.
                        return null;
                    }
                    images.Add(new Image(
                        new AnimationFrame(
                            left, top, frameWidth, frameHeight, AnimationTiming.GifDelayMs(delay), FrameBlend.Over, disposal),
                        paletteAt,
                        paletteEntries,
                        transparent,
                        (imagePacked & 0x40) != 0,
                        dataAt));
                    // A control extension is for the ONE image after it.
                    delay = 0;
                    disposal = FrameDisposal.None;
                    transparent = null;
                    break;
                default:
                    // The trailer (0x3B), or bytes that are not a GIF's: either way the end.
                    return Finished();
            }
        }
        return Finished();

        GifPicture? Finished()
        {
            if (images.Count == 0)
            {
                return null;
            }
            // A logical screen of nothing is a GIF whose images say how big it is.
            var canvasWidth = width > 0 ? width : images.Max(image => image.Frame.X + image.Frame.Width);
            var canvasHeight = height > 0 ? height : images.Max(image => image.Frame.Y + image.Frame.Height);
            return canvasWidth > 0 && canvasHeight > 0 ? new GifPicture(bytes, canvasWidth, canvasHeight, images) : null;
        }

        static int SubBlocks(byte[] bytes, int at)
        {
            while (at < bytes.Length)
            {
                var length = bytes[at];
                at += 1 + length;
                if (length == 0)
                {
                    return at;
                }
            }
            return bytes.Length;
        }
    }

    /// <summary>
    /// Image <paramref name="index"/>'s own rectangle as straight BGRA, <c>Width × Height × 4</c> bytes: its transparent index,
    /// an index past its colour table and every pixel its data never reached all come out with an alpha of 0, so drawing it
    /// OVER the canvas leaves those pixels as they were.
    /// </summary>
    public byte[] Pixels(int index)
    {
        var image = images[index];
        var frame = image.Frame;
        var count = frame.Width * frame.Height;
        var pixels = new byte[count * 4];
        if (count == 0)
        {
            return pixels;
        }
        var indexes = new byte[count];
        var decoded = Lzw(bytes, image.DataAt, indexes);
        for (var at = 0; at < decoded; at++)
        {
            var row = image.Interlaced ? InterlacedRow(at / frame.Width, frame.Height) : at / frame.Width;
            var target = ((row * frame.Width) + (at % frame.Width)) * 4;
            var colour = indexes[at];
            if (colour == image.Transparent || colour >= image.PaletteEntries)
            {
                continue;
            }
            var rgb = image.PaletteAt + (3 * colour);
            pixels[target] = bytes[rgb + 2];
            pixels[target + 1] = bytes[rgb + 1];
            pixels[target + 2] = bytes[rgb];
            pixels[target + 3] = 0xFF;
        }
        return pixels;
    }

    /// <summary>
    /// The row the <paramref name="written"/>th row of an interlaced image belongs on: every 8th from 0, every 8th from 4,
    /// every 4th from 2, every 2nd from 1.
    /// </summary>
    public static int InterlacedRow(int written, int height)
    {
        var first = (height + 7) / 8;
        if (written < first)
        {
            return written * 8;
        }
        written -= first;
        var second = (height + 3) / 8;
        if (written < second)
        {
            return 4 + (written * 8);
        }
        written -= second;
        var third = (height + 1) / 4;
        if (written < third)
        {
            return 2 + (written * 4);
        }
        written -= third;
        return 1 + (written * 2);
    }

    /// <summary>
    /// The image data at <paramref name="at"/> (its minimum code size, then its sub-blocks) unpacked into
    /// <paramref name="output"/>. Answers how many indexes it held — fewer than asked for when the data stops early.
    /// </summary>
    public static int Lzw(byte[] bytes, int at, byte[] output)
    {
        if (at >= bytes.Length)
        {
            return 0;
        }
        var minimum = bytes[at];
        // The format's own range: 2 to 8 bits a colour, and room above it for the decoder's 12-bit codes.
        if (minimum is < 2 or > 11)
        {
            return 0;
        }
        const int MaxCodes = 4096;
        var prefix = new short[MaxCodes];
        var suffix = new byte[MaxCodes];
        var first = new byte[MaxCodes];
        var stack = new byte[MaxCodes + 1];
        var clear = 1 << minimum;
        var end = clear + 1;
        for (var code = 0; code < clear; code++)
        {
            prefix[code] = -1;
            suffix[code] = (byte)code;
            first[code] = (byte)code;
        }
        var size = minimum + 1;
        var next = clear + 2;
        var previous = -1;
        var written = 0;
        // The bit reader over the sub-blocks.
        var block = at + 1;
        var left = 0;
        var position = block;
        var bits = 0;
        var held = 0;
        while (written < output.Length)
        {
            while (held < size)
            {
                if (left == 0)
                {
                    if (position >= bytes.Length || bytes[position] == 0)
                    {
                        return written;
                    }
                    left = bytes[position];
                    position++;
                }
                if (position >= bytes.Length)
                {
                    return written;
                }
                bits |= bytes[position] << held;
                position++;
                left--;
                held += 8;
            }
            var code = bits & ((1 << size) - 1);
            bits >>= size;
            held -= size;
            if (code == clear)
            {
                size = minimum + 1;
                next = clear + 2;
                previous = -1;
                continue;
            }
            if (code == end)
            {
                return written;
            }
            int emit;
            if (previous < 0)
            {
                if (code >= clear)
                {
                    // The first code after a clear must be a colour; anything else is data that cannot be read on.
                    return written;
                }
                emit = code;
            }
            else if (code < next)
            {
                emit = code;
                if (next < MaxCodes)
                {
                    prefix[next] = (short)previous;
                    suffix[next] = first[code];
                    first[next] = first[previous];
                    next++;
                }
            }
            else if (code == next && next < MaxCodes)
            {
                // The one code the decoder has not made yet: the previous string and its own first byte.
                prefix[next] = (short)previous;
                suffix[next] = first[previous];
                first[next] = first[previous];
                next++;
                emit = code;
            }
            else
            {
                return written;
            }
            // The string, back to front, then out in order.
            var depth = 0;
            for (var walk = emit; walk >= 0 && depth < stack.Length; walk = prefix[walk])
            {
                stack[depth++] = suffix[walk];
            }
            while (depth > 0 && written < output.Length)
            {
                output[written++] = stack[--depth];
            }
            previous = emit;
            if (next == (1 << size) && size < 12)
            {
                size++;
            }
        }
        return written;
    }
}

/// <summary>
/// An animated PNG taken apart by this client: its <c>fcTL</c> frames, and each one rebuilt as a STANDALONE PNG — the file's
/// own header chunks around that frame's data — which any PNG decoder reads as the still picture it is.
/// </summary>
/// <remarks>
/// <para>
/// <b>WHY.</b> Windows Imaging's PNG decoder has never read APNG: it hands out the default image and nothing else. A frame's
/// data is ordinary PNG image data (zlib, filtered rows) at the frame's own size, so a PNG whose <c>IHDR</c> names that size
/// and whose <c>IDAT</c> is that data IS the frame — decoding stays the platform's, and only the cutting is ours.
/// </para>
/// <para>
/// A frame that reaches past the canvas, chunks out of order, or an animation with no frames is a picture this does not
/// read — and goes as the file it is (rule C).
/// </para>
/// </remarks>
public sealed class ApngPicture
{
    private static readonly byte[] Signature = [0x89, (byte)'P', (byte)'N', (byte)'G', 0x0D, 0x0A, 0x1A, 0x0A];

    private readonly byte[] ihdr;
    private readonly List<byte[]> shared;
    private readonly List<List<ReadOnlyMemory<byte>>> data;

    private ApngPicture(AnimationLayout layout, byte[] ihdr, List<byte[]> shared, List<List<ReadOnlyMemory<byte>>> data)
    {
        Layout = layout;
        this.ihdr = ihdr;
        this.shared = shared;
        this.data = data;
    }

    public AnimationLayout Layout { get; }

    /// <summary>The picture's frames, or null when it is not an APNG this can take apart.</summary>
    public static ApngPicture? Read(byte[] bytes)
    {
        if (!bytes.AsSpan().StartsWith(Signature))
        {
            return null;
        }
        byte[]? ihdr = null;
        var shared = new List<byte[]>();
        var frames = new List<AnimationFrame>();
        var data = new List<List<ReadOnlyMemory<byte>>>();
        var seenImage = false;
        var at = Signature.Length;
        // Whether the frame being collected takes the IDAT (an fcTL before it) or fdATs.
        var collecting = false;
        while (at + 12 <= bytes.Length)
        {
            var length = BinaryPrimitives.ReadUInt32BigEndian(bytes.AsSpan(at));
            if (length > (uint)(bytes.Length - at - 12))
            {
                return null;
            }
            var type = System.Text.Encoding.ASCII.GetString(bytes, at + 4, 4);
            var payload = new ReadOnlyMemory<byte>(bytes, at + 8, (int)length);
            var whole = bytes.AsSpan(at, (int)length + 12).ToArray();
            at += (int)length + 12;
            switch (type)
            {
                case "IHDR":
                    if (ihdr is not null || length != 13)
                    {
                        return null;
                    }
                    ihdr = payload.ToArray();
                    break;
                case "acTL":
                    break;
                case "fcTL":
                    if (ihdr is null || length < 26)
                    {
                        return null;
                    }
                    var control = payload.Span;
                    var frameWidth = BinaryPrimitives.ReadUInt32BigEndian(control[4..]);
                    var frameHeight = BinaryPrimitives.ReadUInt32BigEndian(control[8..]);
                    var x = BinaryPrimitives.ReadUInt32BigEndian(control[12..]);
                    var y = BinaryPrimitives.ReadUInt32BigEndian(control[16..]);
                    var numerator = BinaryPrimitives.ReadUInt16BigEndian(control[20..]);
                    var denominator = BinaryPrimitives.ReadUInt16BigEndian(control[22..]);
                    var dispose = control[24];
                    var blend = control[25];
                    var canvasWidth = BinaryPrimitives.ReadUInt32BigEndian(ihdr);
                    var canvasHeight = BinaryPrimitives.ReadUInt32BigEndian(ihdr.AsSpan(4));
                    if (frameWidth == 0 || frameHeight == 0 || (ulong)x + frameWidth > canvasWidth || (ulong)y + frameHeight > canvasHeight)
                    {
                        return null;
                    }
                    frames.Add(new AnimationFrame(
                        (int)x,
                        (int)y,
                        (int)frameWidth,
                        (int)frameHeight,
                        AnimationTiming.ApngDelayMs(numerator, denominator),
                        blend == 1 ? FrameBlend.Over : FrameBlend.Source,
                        dispose switch
                        {
                            1 => FrameDisposal.Background,
                            // "If the first fcTL chunk uses a dispose_op of APNG_DISPOSE_OP_PREVIOUS it should be treated as
                            // APNG_DISPOSE_OP_BACKGROUND."
                            2 => frames.Count == 0 ? FrameDisposal.Background : FrameDisposal.Previous,
                            _ => FrameDisposal.None,
                        }));
                    data.Add([]);
                    collecting = !seenImage;
                    break;
                case "IDAT":
                    seenImage = true;
                    if (collecting && data.Count == 1)
                    {
                        data[0].Add(payload);
                    }
                    break;
                case "fdAT":
                    if (data.Count == 0 || length < 4)
                    {
                        return null;
                    }
                    // A sequence number, then the frame's image data.
                    data[^1].Add(payload[4..]);
                    break;
                case "IEND":
                    at = bytes.Length;
                    break;
                default:
                    // Everything the file says about its pixels before they begin — PLTE, tRNS, gAMA, sRGB, iCCP… — is every
                    // frame's too.
                    if (!seenImage)
                    {
                        shared.Add(whole);
                    }
                    break;
            }
        }
        if (ihdr is null || frames.Count == 0 || data.Any(parts => parts.Count == 0))
        {
            return null;
        }
        var layout = new AnimationLayout(
            AnimationFormat.Apng,
            (int)Math.Min(BinaryPrimitives.ReadUInt32BigEndian(ihdr), int.MaxValue),
            (int)Math.Min(BinaryPrimitives.ReadUInt32BigEndian(ihdr.AsSpan(4)), int.MaxValue),
            frames);
        return new ApngPicture(layout, ihdr, shared, data);
    }

    /// <summary>Frame <paramref name="index"/> as a PNG of its own: the file's header with the frame's size, and its data.</summary>
    public byte[] Standalone(int index)
    {
        var frame = Layout.Frames[index];
        var header = (byte[])ihdr.Clone();
        BinaryPrimitives.WriteUInt32BigEndian(header, (uint)frame.Width);
        BinaryPrimitives.WriteUInt32BigEndian(header.AsSpan(4), (uint)frame.Height);
        using var output = new MemoryStream();
        output.Write(Signature);
        Chunk(output, "IHDR", header);
        foreach (var chunk in shared)
        {
            output.Write(chunk);
        }
        var length = data[index].Sum(part => part.Length);
        var joined = new byte[length];
        var at = 0;
        foreach (var part in data[index])
        {
            part.Span.CopyTo(joined.AsSpan(at));
            at += part.Length;
        }
        Chunk(output, "IDAT", joined);
        Chunk(output, "IEND", []);
        return output.ToArray();
    }

    private static void Chunk(MemoryStream output, string type, ReadOnlySpan<byte> payload)
    {
        Span<byte> number = stackalloc byte[4];
        BinaryPrimitives.WriteUInt32BigEndian(number, (uint)payload.Length);
        output.Write(number);
        var typed = new byte[4 + payload.Length];
        System.Text.Encoding.ASCII.GetBytes(type, typed);
        payload.CopyTo(typed.AsSpan(4));
        output.Write(typed);
        BinaryPrimitives.WriteUInt32BigEndian(number, Crc32.Of(typed));
        output.Write(number);
    }
}

/// <summary>
/// An animated WebP taken apart by this client: its <c>ANMF</c> frames, and each one rewrapped as a STANDALONE still WebP
/// that any WebP decoder reads.
/// </summary>
/// <remarks>
/// Windows decodes WebP through a Store extension, and whether that hands out an animation's frames — and whether whole or
/// as raw rectangles — is not something this port has seen. A frame's own bitstream (<c>VP8</c> or <c>VP8L</c>, with its
/// <c>ALPH</c>) is exactly a still WebP's, so wrapped in a RIFF header of its own it is one, and the extension's
/// still-picture path — the one stickers already rely on — decodes it.
/// </remarks>
public sealed class WebPAnimation
{
    private readonly List<byte[]> frameData;

    private WebPAnimation(AnimationLayout layout, List<byte[]> frameData)
    {
        Layout = layout;
        this.frameData = frameData;
    }

    public AnimationLayout Layout { get; }

    /// <summary>The animation's frames, or null when the file is not an animated WebP this can take apart.</summary>
    public static WebPAnimation? Read(byte[] bytes)
    {
        if (StickerFile.Mime(bytes) != StickerFile.WebP)
        {
            return null;
        }
        int width = 0, height = 0;
        var animated = false;
        var frames = new List<AnimationFrame>();
        var data = new List<byte[]>();
        foreach (var (type, payload) in Chunks(bytes, 12, bytes.Length))
        {
            var span = payload.Span;
            switch (type)
            {
                case "VP8X":
                    if (span.Length < 10)
                    {
                        return null;
                    }
                    animated = (span[0] & 0x02) != 0;
                    width = U24(span[4..]) + 1;
                    height = U24(span[7..]) + 1;
                    break;
                case "ANMF":
                    if (span.Length < 16 || width == 0)
                    {
                        return null;
                    }
                    var x = U24(span) * 2;
                    var y = U24(span[3..]) * 2;
                    var frameWidth = U24(span[6..]) + 1;
                    var frameHeight = U24(span[9..]) + 1;
                    var duration = (uint)U24(span[12..]);
                    var flags = span[15];
                    if ((long)x + frameWidth > width || (long)y + frameHeight > height)
                    {
                        return null;
                    }
                    frames.Add(new AnimationFrame(
                        x,
                        y,
                        frameWidth,
                        frameHeight,
                        duration,
                        (flags & 0x02) != 0 ? FrameBlend.Source : FrameBlend.Over,
                        (flags & 0x01) != 0 ? FrameDisposal.Background : FrameDisposal.None));
                    data.Add(span[16..].ToArray());
                    break;
            }
        }
        return animated && frames.Count > 0
            ? new WebPAnimation(new AnimationLayout(AnimationFormat.WebP, width, height, frames), data)
            : null;
    }

    /// <summary>
    /// Frame <paramref name="index"/> as a still WebP: its <c>VP8L</c> alone, its <c>VP8</c> alone, or — where it carries
    /// an <c>ALPH</c> — an extended file whose <c>VP8X</c> says alpha, at the frame's own size. Null for a frame whose data
    /// holds no bitstream.
    /// </summary>
    public byte[]? Standalone(int index)
    {
        var frame = Layout.Frames[index];
        var own = frameData[index];
        ReadOnlyMemory<byte>? alpha = null, lossy = null, lossless = null;
        foreach (var (type, payload) in Chunks(own, 0, own.Length))
        {
            switch (type)
            {
                case "ALPH":
                    alpha ??= payload;
                    break;
                case "VP8 ":
                    lossy ??= payload;
                    break;
                case "VP8L":
                    lossless ??= payload;
                    break;
            }
        }
        using var body = new MemoryStream();
        body.Write("WEBP"u8);
        if (lossless is { } vp8l)
        {
            Chunk(body, "VP8L", vp8l.Span);
        }
        else if (lossy is { } vp8)
        {
            if (alpha is { } alph)
            {
                Span<byte> extended = stackalloc byte[10];
                extended.Clear();
                extended[0] = 0x10;
                WriteU24(extended[4..], frame.Width - 1);
                WriteU24(extended[7..], frame.Height - 1);
                Chunk(body, "VP8X", extended);
                Chunk(body, "ALPH", alph.Span);
            }
            Chunk(body, "VP8 ", vp8.Span);
        }
        else
        {
            return null;
        }
        var inner = body.ToArray();
        var file = new byte[8 + inner.Length];
        "RIFF"u8.CopyTo(file);
        BinaryPrimitives.WriteUInt32LittleEndian(file.AsSpan(4), (uint)inner.Length);
        inner.CopyTo(file, 8);
        return file;
    }

    private static int U24(ReadOnlySpan<byte> at) => at[0] | (at[1] << 8) | (at[2] << 16);

    private static void WriteU24(Span<byte> at, int value)
    {
        at[0] = (byte)value;
        at[1] = (byte)(value >> 8);
        at[2] = (byte)(value >> 16);
    }

    private static void Chunk(MemoryStream output, string type, ReadOnlySpan<byte> payload)
    {
        output.Write(System.Text.Encoding.ASCII.GetBytes(type));
        Span<byte> size = stackalloc byte[4];
        BinaryPrimitives.WriteUInt32LittleEndian(size, (uint)payload.Length);
        output.Write(size);
        output.Write(payload);
        if ((payload.Length & 1) != 0)
        {
            output.WriteByte(0);
        }
    }

    /// <summary>The RIFF chunks in <paramref name="start"/>..<paramref name="end"/>, each bounded by what the bytes hold.</summary>
    private static List<(string Type, ReadOnlyMemory<byte> Payload)> Chunks(byte[] bytes, int start, int end)
    {
        var chunks = new List<(string, ReadOnlyMemory<byte>)>();
        var at = start;
        while (at + 8 <= end)
        {
            var size = BinaryPrimitives.ReadUInt32LittleEndian(bytes.AsSpan(at + 4));
            var held = (int)Math.Min(size, (uint)(end - at - 8));
            chunks.Add((System.Text.Encoding.ASCII.GetString(bytes, at, 4), new ReadOnlyMemory<byte>(bytes, at + 8, held)));
            var next = at + 8L + size + (size & 1);
            if (next > end)
            {
                break;
            }
            at = (int)next;
        }
        return chunks;
    }
}

/// <summary>PNG's CRC-32 (ISO 3309, the polynomial zlib uses), for the chunks a rebuilt frame is written with.</summary>
public static class Crc32
{
    private static readonly uint[] Table = Build();

    public static uint Of(ReadOnlySpan<byte> bytes)
    {
        var crc = 0xFFFFFFFFu;
        foreach (var value in bytes)
        {
            crc = Table[(crc ^ value) & 0xFF] ^ (crc >> 8);
        }
        return crc ^ 0xFFFFFFFFu;
    }

    private static uint[] Build()
    {
        var table = new uint[256];
        for (var n = 0u; n < 256; n++)
        {
            var c = n;
            for (var k = 0; k < 8; k++)
            {
                c = (c & 1) != 0 ? 0xEDB88320u ^ (c >> 1) : c >> 1;
            }
            table[n] = c;
        }
        return table;
    }
}
