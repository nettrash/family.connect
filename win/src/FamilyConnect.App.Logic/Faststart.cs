using System.Buffers.Binary;
using System.Text;

namespace FamilyConnect.App.Logic;

/// <summary>
/// An MP4's <c>moov</c> box moved in front of its media data — what ffmpeg calls <c>+faststart</c> and qt-faststart
/// does on its own. The protocol's profile asks for it: "<c>moov</c> before <c>mdat</c> — so a player can start on the
/// first bytes of a <c>Range</c> read" (docs/protocol.md, "Preparing media before upload").
/// </summary>
/// <remarks>
/// <para>
/// <b>WHY IT IS DONE HERE AND NOT ASKED OF WINDOWS.</b> Media Foundation's MP4 sink writes the media data as it
/// arrives and the index when it finishes, and whether it can be told otherwise through what the WinRT transcoder
/// exposes is something the Mac this port is written on cannot find out. Rearranging the finished bytes is the same
/// answer on every Windows, and it can be tested anywhere.
/// </para>
/// <para>
/// <b>MOVING THE INDEX MOVES EVERYTHING IT POINTS AT.</b> The chunk offsets in every track's <c>stco</c> (32-bit) or
/// <c>co64</c> (64-bit) are ABSOLUTE file positions, and every byte between the first <c>mdat</c> and the old
/// <c>moov</c> slides forward by the size of the <c>moov</c>. So exactly those offsets grow by it and no others — an
/// <c>mdat</c> that was already after the <c>moov</c> stays where it was. A 32-bit offset that would no longer fit is
/// not converted to 64 bits: this gives up instead, which for a file under the 100 MB ceiling cannot happen.
/// </para>
/// <para>
/// <b>ANYTHING IT DOES NOT UNDERSTAND, IT LEAVES ALONE.</b> A top-level box it cannot vouch for — fragments
/// (<c>moof</c>, <c>sidx</c>), a <c>meta</c> with its own absolute offsets — could point into what it moves, so the answer
/// is null rather than a file that plays until somebody seeks.
/// </para>
/// </remarks>
public static class Faststart
{
    /// <summary>Top-level boxes that hold no absolute file position, and so may slide without being rewritten.</summary>
    private static readonly HashSet<string> Movable = ["ftyp", "moov", "mdat", "free", "skip", "wide", "uuid", "udta", "pdin"];

    /// <summary>The boxes a chunk-offset table can sit inside: moov → trak → mdia → minf → stbl.</summary>
    private static readonly HashSet<string> Containers = ["trak", "mdia", "minf", "stbl"];

    private readonly record struct Box(int Start, int Size, int Header, string Type, bool ToEnd);

    /// <summary>
    /// Whether the <c>moov</c> comes before the first <c>mdat</c>. Null when the top level cannot be walked, or has no
    /// <c>moov</c> or no <c>mdat</c> to compare.
    /// </summary>
    public static bool? IsMoovFirst(ReadOnlySpan<byte> file)
    {
        if (Walk(file, 0, file.Length) is not { } boxes)
        {
            return null;
        }
        var moov = boxes.FindIndex(box => box.Type == "moov");
        var mdat = boxes.FindIndex(box => box.Type == "mdat");
        return moov < 0 || mdat < 0 ? null : moov < mdat;
    }

    /// <summary>
    /// The file with its <c>moov</c> in front of its media data: the SAME array when it already is, a rearranged copy of
    /// the same length when it was not, and null when that cannot be done safely.
    /// </summary>
    public static byte[]? MoovFirst(byte[] file)
    {
        if (Walk(file, 0, file.Length) is not { } boxes)
        {
            return null;
        }
        var moovs = boxes.Where(box => box.Type == "moov").ToList();
        var first = boxes.FindIndex(box => box.Type == "mdat");
        if (moovs.Count != 1 || first < 0)
        {
            return null;
        }
        var moov = moovs[0];
        var mdat = boxes[first];
        if (moov.Start < mdat.Start)
        {
            return file;
        }
        if (boxes.Any(box => !Movable.Contains(box.Type)))
        {
            return null;
        }
        var index = file.AsSpan(moov.Start, moov.Size).ToArray();
        if (moov.ToEnd)
        {
            // "Until the end of the file" stops being true the moment it is not at the end.
            BinaryPrimitives.WriteUInt32BigEndian(index, (uint)moov.Size);
        }
        if (!Shift(index, moov.Header, index.Length, from: mdat.Start, until: moov.Start, by: moov.Size))
        {
            return null;
        }
        var arranged = new byte[file.Length];
        var at = 0;
        void Put(ReadOnlySpan<byte> bytes)
        {
            bytes.CopyTo(arranged.AsSpan(at));
            at += bytes.Length;
        }
        Put(file.AsSpan(0, mdat.Start));
        Put(index);
        Put(file.AsSpan(mdat.Start, moov.Start - mdat.Start));
        Put(file.AsSpan(moov.Start + moov.Size));
        return arranged;
    }

    /// <summary>
    /// Every chunk offset under <paramref name="start"/>..<paramref name="end"/> of <paramref name="index"/> that points at
    /// or after <paramref name="from"/> and before <paramref name="until"/>, moved on by <paramref name="by"/>. False
    /// when a box inside is malformed or a 32-bit offset would overflow.
    /// </summary>
    private static bool Shift(byte[] index, int start, int end, long from, long until, long by)
    {
        if (Walk(index, start, end) is not { } children)
        {
            return false;
        }
        foreach (var child in children)
        {
            var body = child.Start + child.Header;
            var bodyEnd = child.Start + child.Size;
            if (Containers.Contains(child.Type))
            {
                if (!Shift(index, body, bodyEnd, from, until, by))
                {
                    return false;
                }
                continue;
            }
            var width = child.Type switch
            {
                "stco" => 4,
                "co64" => 8,
                _ => 0,
            };
            if (width == 0)
            {
                continue;
            }
            // A full box: version and flags, the count, then the offsets.
            if (bodyEnd - body < 8)
            {
                return false;
            }
            var count = BinaryPrimitives.ReadUInt32BigEndian(index.AsSpan(body + 4));
            if (count > (ulong)(bodyEnd - body - 8) / (ulong)width)
            {
                return false;
            }
            for (var entry = 0; entry < (int)count; entry++)
            {
                var at = index.AsSpan(body + 8 + (entry * width), width);
                if (width == 4)
                {
                    long offset = BinaryPrimitives.ReadUInt32BigEndian(at);
                    if (offset >= from && offset < until)
                    {
                        if (offset + by > uint.MaxValue)
                        {
                            return false;
                        }
                        BinaryPrimitives.WriteUInt32BigEndian(at, (uint)(offset + by));
                    }
                }
                else
                {
                    var offset = BinaryPrimitives.ReadUInt64BigEndian(at);
                    if (offset >= (ulong)from && offset < (ulong)until)
                    {
                        BinaryPrimitives.WriteUInt64BigEndian(at, offset + (ulong)by);
                    }
                }
            }
        }
        return true;
    }

    /// <summary>
    /// The boxes laid end to end in <paramref name="start"/>..<paramref name="end"/>, or null when they do not tile it
    /// exactly: a size shorter than its own header, one running past the end, or a stub too short to be a header.
    /// </summary>
    private static List<Box>? Walk(ReadOnlySpan<byte> data, int start, int end)
    {
        List<Box> boxes = [];
        var at = start;
        while (at < end)
        {
            if (end - at < 8)
            {
                return null;
            }
            long size = BinaryPrimitives.ReadUInt32BigEndian(data[at..]);
            var type = Encoding.Latin1.GetString(data.Slice(at + 4, 4));
            var header = 8;
            var toEnd = false;
            if (size == 1)
            {
                if (end - at < 16)
                {
                    return null;
                }
                var large = BinaryPrimitives.ReadUInt64BigEndian(data[(at + 8)..]);
                size = large > long.MaxValue ? long.MaxValue : (long)large;
                header = 16;
            }
            else if (size == 0)
            {
                size = end - at;
                toEnd = true;
            }
            if (size < header || size > end - at)
            {
                return null;
            }
            boxes.Add(new Box(at, (int)size, header, type, toEnd));
            at += (int)size;
        }
        return boxes;
    }
}
