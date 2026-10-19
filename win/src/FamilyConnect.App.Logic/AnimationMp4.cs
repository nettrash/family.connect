using System.Buffers.Binary;
using System.Text;

namespace FamilyConnect.App.Logic;

/// <summary>
/// An animation's MP4 given its EXACT timing after Media Foundation has written it (docs/protocol.md, "Animations", "The
/// file": "Each video frame lasts its ticks exactly — a timescale that is a multiple of 30").
/// </summary>
/// <remarks>
/// <para>
/// <b>WHY AFTERWARDS.</b> Media Foundation counts time in hundreds of nanoseconds, and a thirtieth of a second is not a whole
/// number of them: each sample is handed over at its tick rounded to the nearest 100 ns, so its stated length can be a
/// hundred nanoseconds off, and the timescale its MP4 sink chooses is its own. The sample table is ours to write, though:
/// the video track's <c>mdhd</c> timescale becomes one that is a multiple of 30 (kept when it already is, 30 000
/// otherwise), its <c>stts</c> says each sample's ticks in it, and every duration that follows from them — the track's,
/// the movie's, the edit list's — is said again. Composition offsets (<c>ctts</c>, which only B-frames need) and an edit's
/// media time are carried over into the new timescale.
/// </para>
/// <para>
/// <b>BEFORE THE INDEX MOVES.</b> This runs on the file as the sink wrote it, <c>moov</c> LAST: a <c>moov</c> that grows or
/// shrinks there moves no media data, so no chunk offset changes, and <see cref="Faststart"/> then moves the finished index to
/// the front as it does every video's.
/// </para>
/// <para>
/// <b>ANYTHING IT CANNOT VOUCH FOR, IT LEAVES ALONE</b> — null, and the file goes with the encoder's own timing, which plays
/// the same to within a hundred nanoseconds a frame: a <c>moov</c> before media data, more than one track, a sample count
/// that is not the plan's (an encoder that dropped or doubled a frame), a box it cannot walk.
/// </para>
/// </remarks>
public static class AnimationMp4
{
    /// <summary>The timescale a video track is given when the encoder's own is not a multiple of 30.</summary>
    public const uint Timescale = 30_000;

    private static readonly HashSet<string> Containers = ["moov", "trak", "mdia", "minf", "stbl", "edts"];

    private sealed class Node(string type, byte[]? payload, List<Node>? children)
    {
        public string Type { get; } = type;

        public byte[]? Payload { get; set; } = payload;

        public List<Node>? Children { get; } = children;

        public Node? Child(string type) => Children?.FirstOrDefault(child => child.Type == type);

        public IEnumerable<Node> All(string type) => Children?.Where(child => child.Type == type) ?? [];
    }

    /// <summary>
    /// <paramref name="file"/> with its one video track timed by <paramref name="ticks"/> — one entry per sample, in 1/30 s —
    /// or null when that cannot be done safely.
    /// </summary>
    public static byte[]? ExactTicks(byte[] file, IReadOnlyList<int> ticks)
    {
        if (ticks.Count == 0 || ticks.Any(tick => tick <= 0))
        {
            return null;
        }
        var top = Walk(file, 0, file.Length);
        if (top is null)
        {
            return null;
        }
        var moovs = top.Where(box => box.Type == "moov").ToList();
        if (moovs.Count != 1)
        {
            return null;
        }
        var moovBox = moovs[0];
        // Every byte of media data before the index: then a moov of another size moves nothing a chunk offset names.
        if (top.Any(box => box.Type == "mdat" && box.Start > moovBox.Start))
        {
            return null;
        }
        var moov = Parse("moov", file.AsSpan(moovBox.Start + moovBox.Header, moovBox.Size - moovBox.Header));
        if (moov?.Child("mvhd") is not { Payload: { } mvhd } || moov.All("trak").Count() != 1)
        {
            return null;
        }
        var trak = moov.Child("trak")!;
        var mdia = trak.Child("mdia");
        var stbl = mdia?.Child("minf")?.Child("stbl");
        if (trak.Child("tkhd") is not { Payload: { } tkhd }
            || mdia?.Child("mdhd") is not { Payload: { } mdhd }
            || mdia.Child("hdlr") is not { Payload: { Length: >= 12 } hdlr }
            || Encoding.Latin1.GetString(hdlr, 8, 4) != "vide"
            || stbl?.Child("stts") is not { } stts
            || stbl.Child("stsz") is not { Payload: { Length: >= 12 } stsz })
        {
            return null;
        }
        if (BinaryPrimitives.ReadUInt32BigEndian(stsz.AsSpan(8)) != ticks.Count)
        {
            return null;
        }
        var movieScale = Field(mvhd, TimescaleAt(mvhd), 4);
        var oldScale = Field(mdhd, TimescaleAt(mdhd), 4);
        if (movieScale is not > 0 || oldScale is not > 0)
        {
            return null;
        }
        var newScale = oldScale.Value % 30 == 0 ? (uint)oldScale.Value : Timescale;
        ulong per = newScale / 30;
        ulong total = 0;
        foreach (var tick in ticks)
        {
            total += (ulong)tick;
        }
        var mediaDuration = total * per;
        var movieDuration = Rescale(total, movieScale.Value, 30);
        // The sample table: each sample's ticks, run-length coded.
        var runs = new List<(uint Count, uint Delta)>();
        foreach (var tick in ticks)
        {
            var delta = (ulong)tick * per;
            if (delta > uint.MaxValue)
            {
                return null;
            }
            if (runs.Count > 0 && runs[^1].Delta == (uint)delta)
            {
                runs[^1] = (runs[^1].Count + 1, (uint)delta);
            }
            else
            {
                runs.Add((1, (uint)delta));
            }
        }
        var table = new byte[8 + (8 * runs.Count)];
        BinaryPrimitives.WriteUInt32BigEndian(table.AsSpan(4), (uint)runs.Count);
        for (var run = 0; run < runs.Count; run++)
        {
            BinaryPrimitives.WriteUInt32BigEndian(table.AsSpan(8 + (8 * run)), runs[run].Count);
            BinaryPrimitives.WriteUInt32BigEndian(table.AsSpan(12 + (8 * run)), runs[run].Delta);
        }
        stts.Payload = table;
        if (!SetTime(mdhd, TimescaleAt(mdhd), newScale) || !SetTime(mdhd, TimescaleAt(mdhd) + 4, mediaDuration)
            || !SetTime(tkhd, TkhdDurationAt(tkhd), movieDuration)
            || !SetTime(mvhd, TimescaleAt(mvhd) + 4, movieDuration))
        {
            return null;
        }
        if (newScale != oldScale.Value && stbl.Child("ctts") is { Payload: { } ctts } && !RescaleOffsets(ctts, (uint)oldScale.Value, newScale))
        {
            return null;
        }
        if (trak.Child("edts")?.Child("elst") is { Payload: { } elst } && !Edit(elst, (uint)oldScale.Value, newScale, movieDuration))
        {
            return null;
        }
        var written = Serialize(moov);
        var result = new byte[file.Length - moovBox.Size + written.Length];
        file.AsSpan(0, moovBox.Start).CopyTo(result);
        written.CopyTo(result, moovBox.Start);
        file.AsSpan(moovBox.Start + moovBox.Size).CopyTo(result.AsSpan(moovBox.Start + written.Length));
        return result;
    }

    /// <summary>
    /// The video track's timescale and its samples' durations as a file states them — what a test, or a diagnostics line,
    /// reads back. Null when there is no single video track to read.
    /// </summary>
    public static (uint Timescale, IReadOnlyList<uint> Durations)? Timing(byte[] file)
    {
        if (Walk(file, 0, file.Length)?.FirstOrDefault(box => box.Type == "moov") is not { Type: "moov" } box)
        {
            return null;
        }
        var moov = Parse("moov", file.AsSpan(box.Start + box.Header, box.Size - box.Header));
        var mdia = moov?.Child("trak")?.Child("mdia");
        if (mdia?.Child("mdhd") is not { Payload: { } mdhd }
            || mdia.Child("minf")?.Child("stbl")?.Child("stts") is not { Payload: { Length: >= 8 } stts }
            || Field(mdhd, TimescaleAt(mdhd), 4) is not { } scale)
        {
            return null;
        }
        var durations = new List<uint>();
        var count = BinaryPrimitives.ReadUInt32BigEndian(stts.AsSpan(4));
        for (var entry = 0; entry < count && 16 + (8 * entry) <= stts.Length; entry++)
        {
            var samples = BinaryPrimitives.ReadUInt32BigEndian(stts.AsSpan(8 + (8 * entry)));
            var delta = BinaryPrimitives.ReadUInt32BigEndian(stts.AsSpan(12 + (8 * entry)));
            for (var sample = 0u; sample < samples && durations.Count < 1_000_000; sample++)
            {
                durations.Add(delta);
            }
        }
        return ((uint)scale, durations);
    }

    /// <summary>Where a full box's timescale sits: after version, flags and two dates of 4 bytes (version 0) or 8 (version 1).</summary>
    private static int TimescaleAt(byte[] full) => full.Length > 0 && full[0] == 1 ? 20 : 12;

    /// <summary>Where tkhd's duration sits: after the dates, the track id and a reserved word.</summary>
    private static int TkhdDurationAt(byte[] tkhd) => tkhd.Length > 0 && tkhd[0] == 1 ? 28 : 20;

    private static long? Field(byte[] full, int at, int width) =>
        at + width > full.Length ? null
        : width == 4 ? BinaryPrimitives.ReadUInt32BigEndian(full.AsSpan(at))
        : (long)Math.Min(BinaryPrimitives.ReadUInt64BigEndian(full.AsSpan(at)), long.MaxValue);

    /// <summary>
    /// A time field of a full box: 4 bytes in version 0, 8 in version 1 — except a timescale, which is 4 in both (the
    /// field right after the dates). False when it does not fit.
    /// </summary>
    private static bool SetTime(byte[] full, int at, ulong value)
    {
        var wide = full[0] == 1 && at != TimescaleAt(full);
        if (at + (wide ? 8 : 4) > full.Length)
        {
            return false;
        }
        if (wide)
        {
            BinaryPrimitives.WriteUInt64BigEndian(full.AsSpan(at), value);
            return true;
        }
        if (value > uint.MaxValue)
        {
            return false;
        }
        BinaryPrimitives.WriteUInt32BigEndian(full.AsSpan(at), (uint)value);
        return true;
    }

    private static ulong Rescale(ulong value, long to, long from) =>
        ((value * (ulong)to) + ((ulong)from / 2)) / (ulong)from;

    private static long RescaleSigned(long value, uint from, uint to)
    {
        var scaled = ((Math.Abs(value) * (long)to) + (from / 2)) / from;
        return value < 0 ? -scaled : scaled;
    }

    private static bool RescaleOffsets(byte[] ctts, uint from, uint to)
    {
        if (ctts.Length < 8)
        {
            return false;
        }
        var signed = ctts[0] == 1;
        var count = BinaryPrimitives.ReadUInt32BigEndian(ctts.AsSpan(4));
        if (count > (ctts.Length - 8) / 8)
        {
            return false;
        }
        for (var entry = 0; entry < count; entry++)
        {
            var at = ctts.AsSpan(12 + (8 * entry), 4);
            long offset = signed ? BinaryPrimitives.ReadInt32BigEndian(at) : BinaryPrimitives.ReadUInt32BigEndian(at);
            var scaled = RescaleSigned(offset, from, to);
            if (signed)
            {
                if (scaled is < int.MinValue or > int.MaxValue)
                {
                    return false;
                }
                BinaryPrimitives.WriteInt32BigEndian(at, (int)scaled);
            }
            else
            {
                if (scaled is < 0 or > uint.MaxValue)
                {
                    return false;
                }
                BinaryPrimitives.WriteUInt32BigEndian(at, (uint)scaled);
            }
        }
        return true;
    }

    /// <summary>The edit list: its media times into the new timescale, and a single edit's length the clip's.</summary>
    private static bool Edit(byte[] elst, uint from, uint to, ulong movieDuration)
    {
        if (elst.Length < 8)
        {
            return false;
        }
        var wide = elst[0] == 1;
        var size = wide ? 20 : 12;
        var count = BinaryPrimitives.ReadUInt32BigEndian(elst.AsSpan(4));
        if (count > (elst.Length - 8) / size)
        {
            return false;
        }
        for (var entry = 0; entry < count; entry++)
        {
            var at = 8 + (size * entry);
            if (count == 1)
            {
                if (wide)
                {
                    BinaryPrimitives.WriteUInt64BigEndian(elst.AsSpan(at), movieDuration);
                }
                else if (movieDuration <= uint.MaxValue)
                {
                    BinaryPrimitives.WriteUInt32BigEndian(elst.AsSpan(at), (uint)movieDuration);
                }
                else
                {
                    return false;
                }
            }
            var mediaAt = at + (wide ? 8 : 4);
            long media = wide ? BinaryPrimitives.ReadInt64BigEndian(elst.AsSpan(mediaAt)) : BinaryPrimitives.ReadInt32BigEndian(elst.AsSpan(mediaAt));
            if (media < 0 || from == to)
            {
                // −1 is an empty edit: no media time to carry.
                continue;
            }
            var scaled = RescaleSigned(media, from, to);
            if (wide)
            {
                BinaryPrimitives.WriteInt64BigEndian(elst.AsSpan(mediaAt), scaled);
            }
            else if (scaled <= int.MaxValue)
            {
                BinaryPrimitives.WriteInt32BigEndian(elst.AsSpan(mediaAt), (int)scaled);
            }
            else
            {
                return false;
            }
        }
        return true;
    }

    private readonly record struct Box(int Start, int Size, int Header, string Type);

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
            }
            if (size < header || size > end - at)
            {
                return null;
            }
            boxes.Add(new Box(at, (int)size, header, type));
            at += (int)size;
        }
        return boxes;
    }

    private static Node? Parse(string type, ReadOnlySpan<byte> payload)
    {
        if (!Containers.Contains(type))
        {
            return new Node(type, payload.ToArray(), null);
        }
        if (Walk(payload, 0, payload.Length) is not { } boxes)
        {
            return null;
        }
        var children = new List<Node>();
        foreach (var box in boxes)
        {
            if (Parse(box.Type, payload.Slice(box.Start + box.Header, box.Size - box.Header)) is not { } child)
            {
                return null;
            }
            children.Add(child);
        }
        return new Node(type, null, children);
    }

    private static byte[] Serialize(Node node)
    {
        using var output = new MemoryStream();
        Write(output, node);
        return output.ToArray();

        static void Write(MemoryStream output, Node node)
        {
            byte[] body;
            if (node.Children is { } children)
            {
                using var inner = new MemoryStream();
                foreach (var child in children)
                {
                    Write(inner, child);
                }
                body = inner.ToArray();
            }
            else
            {
                body = node.Payload ?? [];
            }
            Span<byte> header = stackalloc byte[8];
            BinaryPrimitives.WriteUInt32BigEndian(header, (uint)(body.Length + 8));
            Encoding.Latin1.GetBytes(node.Type, header[4..]);
            output.Write(header);
            output.Write(body);
        }
    }
}
