using System.Buffers.Binary;
using System.Text;
using FamilyConnect.App.Logic;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The <c>moov</c> moved in front of the media data, with every chunk offset that moved re-pointed — checked the only
/// way that matters: every chunk the index names is the same bytes after the move as before it.
/// </summary>
/// <remarks>
/// The files here are built box by box by a writer of this test's own, so the production walker is never the thing that
/// says it walked correctly.
/// </remarks>
public sealed class FaststartTests
{
    private static byte[] Box(string type, params byte[][] children)
    {
        var body = children.SelectMany(child => child).ToArray();
        var box = new byte[8 + body.Length];
        BinaryPrimitives.WriteUInt32BigEndian(box, (uint)box.Length);
        Encoding.ASCII.GetBytes(type).CopyTo(box, 4);
        body.CopyTo(box, 8);
        return box;
    }

    private static byte[] Full(string type, byte[] body) => Box(type, new byte[4], body);

    private static byte[] Stco(params uint[] offsets)
    {
        var body = new byte[4 + (4 * offsets.Length)];
        BinaryPrimitives.WriteUInt32BigEndian(body, (uint)offsets.Length);
        for (var at = 0; at < offsets.Length; at++)
        {
            BinaryPrimitives.WriteUInt32BigEndian(body.AsSpan(4 + (4 * at)), offsets[at]);
        }
        return Full("stco", body);
    }

    private static byte[] Co64(params ulong[] offsets)
    {
        var body = new byte[4 + (8 * offsets.Length)];
        BinaryPrimitives.WriteUInt32BigEndian(body, (uint)offsets.Length);
        for (var at = 0; at < offsets.Length; at++)
        {
            BinaryPrimitives.WriteUInt64BigEndian(body.AsSpan(4 + (8 * at)), offsets[at]);
        }
        return Full("co64", body);
    }

    private static byte[] Track(byte[] table) =>
        Box("trak", Box("tkhd", new byte[84]), Box("mdia", Box("mdhd", new byte[24]), Box("minf", Box("stbl", Box("stsd", new byte[8]), table))));

    private static byte[] Moov(params byte[][] tables) =>
        Box("moov", [Box("mvhd", new byte[100]), .. tables.Select(Track)]);

    private static byte[] Ftyp() => Box("ftyp", Encoding.ASCII.GetBytes("isom\0\0\u0002\0isomiso2avc1mp41"));

    private static byte[] Payload(int length, int seed) =>
        [.. Enumerable.Range(0, length).Select(at => (byte)((at * 7) + seed))];

    private static byte[] Concat(params byte[][] parts) => [.. parts.SelectMany(part => part)];

    /// <summary>The top-level box types in order, read by this test's own walker.</summary>
    private static List<string> TopLevel(byte[] file)
    {
        List<string> types = [];
        for (var at = 0; at < file.Length;)
        {
            var size = (int)BinaryPrimitives.ReadUInt32BigEndian(file.AsSpan(at));
            types.Add(Encoding.ASCII.GetString(file, at + 4, 4));
            at += size;
        }
        return types;
    }

    /// <summary>Every offset in every table named <paramref name="type"/>, found by its four letters.</summary>
    private static List<ulong> Offsets(byte[] file, string type)
    {
        List<ulong> offsets = [];
        var name = Encoding.ASCII.GetBytes(type);
        for (var at = file.AsSpan().IndexOf(name); at >= 0;)
        {
            var count = BinaryPrimitives.ReadUInt32BigEndian(file.AsSpan(at + 8));
            for (var entry = 0; entry < count; entry++)
            {
                offsets.Add(type == "stco"
                    ? BinaryPrimitives.ReadUInt32BigEndian(file.AsSpan(at + 12 + (4 * entry)))
                    : BinaryPrimitives.ReadUInt64BigEndian(file.AsSpan(at + 12 + (8 * entry))));
            }
            var next = file.AsSpan(at + 4).IndexOf(name);
            at = next < 0 ? -1 : at + 4 + next;
        }
        return offsets;
    }

    /// <summary>What an offset points at: the sixteen bytes a decoder would read there.</summary>
    private static byte[] At(byte[] file, ulong offset) => file.AsSpan((int)offset, 16).ToArray();

    [Fact]
    public void TheIndexMovesToTheFrontAndEveryChunkIsStillWhereItPoints()
    {
        var ftyp = Ftyp();
        var payload = Payload(4096, 3);
        var mdatStart = (uint)ftyp.Length;
        var data = mdatStart + 8;
        // A video track with 32-bit offsets and an audio track with 64-bit ones, both into the one mdat.
        var moov = Moov(Stco(data, data + 100, data + 1000), Co64(data + 50, data + 3000));
        var file = Concat(ftyp, Box("mdat", payload), moov);
        Assert.False(Faststart.IsMoovFirst(file));

        var arranged = Faststart.MoovFirst(file)!;

        Assert.Equal(["ftyp", "moov", "mdat"], TopLevel(arranged));
        Assert.Equal(file.Length, arranged.Length);
        Assert.True(Faststart.IsMoovFirst(arranged));
        var before = Offsets(file, "stco").Concat(Offsets(file, "co64")).ToList();
        var after = Offsets(arranged, "stco").Concat(Offsets(arranged, "co64")).ToList();
        Assert.Equal(before.Select(offset => offset + (ulong)moov.Length), after);
        for (var at = 0; at < before.Count; at++)
        {
            Assert.Equal(At(file, before[at]), At(arranged, after[at]));
        }
        // Nothing else in the index changed: it is byte for byte the index this test would write with the new offsets.
        var by = (uint)moov.Length;
        var expected = Moov(Stco(data + by, data + 100 + by, data + 1000 + by), Co64(data + 50 + by, data + 3000 + by));
        Assert.Equal(expected, arranged.AsSpan(ftyp.Length, moov.Length).ToArray());
    }

    [Fact]
    public void AFileAlreadyInOrderIsHandedBackAsItIs()
    {
        var ftyp = Ftyp();
        var moovLength = Moov(Stco(0)).Length;
        var data = (uint)(ftyp.Length + moovLength + 8);
        var file = Concat(ftyp, Moov(Stco(data)), Box("mdat", Payload(256, 1)));
        Assert.True(Faststart.IsMoovFirst(file));
        Assert.Same(file, Faststart.MoovFirst(file));
    }

    /// <summary>Filler boxes keep their order, and slide like the data does.</summary>
    [Fact]
    public void FillerBeforeTheDataStaysBeforeIt()
    {
        var ftyp = Ftyp();
        var free = Box("free", new byte[32]);
        var data = (uint)(ftyp.Length + free.Length + 8);
        var file = Concat(ftyp, free, Box("mdat", Payload(512, 9)), Moov(Stco(data, data + 256)));
        var arranged = Faststart.MoovFirst(file)!;
        Assert.Equal(["ftyp", "free", "moov", "mdat"], TopLevel(arranged));
        var before = Offsets(file, "stco");
        var after = Offsets(arranged, "stco");
        Assert.All(before.Zip(after), pair => Assert.Equal(At(file, pair.First), At(arranged, pair.Second)));
    }

    /// <summary>
    /// Only what slid is re-pointed: an index between two data boxes moves in front of the first, the first slides by its
    /// size, and the second — before and after, behind both — stays exactly where it was.
    /// </summary>
    [Fact]
    public void DataAfterTheIndexIsNotRepointed()
    {
        var ftyp = Ftyp();
        var first = Box("mdat", Payload(300, 5));
        var moovLength = Moov(Stco(0, 0)).Length;
        var inFirst = (uint)(ftyp.Length + 8 + 10);
        var inSecond = (uint)(ftyp.Length + first.Length + moovLength + 8 + 20);
        var moov = Moov(Stco(inFirst, inSecond));
        var file = Concat(ftyp, first, moov, Box("mdat", Payload(300, 77)));
        var arranged = Faststart.MoovFirst(file)!;
        Assert.Equal(["ftyp", "moov", "mdat", "mdat"], TopLevel(arranged));
        Assert.Equal([(ulong)inFirst + (ulong)moov.Length, inSecond], Offsets(arranged, "stco"));
        Assert.Equal(At(file, inFirst), At(arranged, inFirst + (ulong)moov.Length));
        Assert.Equal(At(file, inSecond), At(arranged, inSecond));
    }

    /// <summary>A size of 0 means "to the end of the file" — which stops being true once it is not at the end.</summary>
    [Fact]
    public void AnIndexThatRanToTheEndIsGivenItsRealSize()
    {
        var ftyp = Ftyp();
        var data = (uint)(ftyp.Length + 8);
        var moov = Moov(Stco(data));
        var open = (byte[])moov.Clone();
        BinaryPrimitives.WriteUInt32BigEndian(open, 0);
        var file = Concat(ftyp, Box("mdat", Payload(128, 2)), open);
        var arranged = Faststart.MoovFirst(file)!;
        Assert.Equal((uint)moov.Length, BinaryPrimitives.ReadUInt32BigEndian(arranged.AsSpan(ftyp.Length)));
        Assert.Equal(["ftyp", "moov", "mdat"], TopLevel(arranged));
        Assert.Equal(At(file, data), At(arranged, Offsets(arranged, "stco")[0]));
    }

    /// <summary>A 64-bit box size — size 1, the real one after the type — is walked like any other.</summary>
    [Fact]
    public void ALargeSizeDataBoxIsWalked()
    {
        var ftyp = Ftyp();
        var payload = Payload(200, 4);
        var large = new byte[16 + payload.Length];
        BinaryPrimitives.WriteUInt32BigEndian(large, 1);
        Encoding.ASCII.GetBytes("mdat").CopyTo(large, 4);
        BinaryPrimitives.WriteUInt64BigEndian(large.AsSpan(8), (ulong)large.Length);
        payload.CopyTo(large, 16);
        var data = (uint)(ftyp.Length + 16);
        var file = Concat(ftyp, large, Moov(Stco(data, data + 100)));
        var arranged = Faststart.MoovFirst(file)!;
        Assert.True(Faststart.IsMoovFirst(arranged));
        Assert.All(Offsets(file, "stco").Zip(Offsets(arranged, "stco")), pair => Assert.Equal(At(file, pair.First), At(arranged, pair.Second)));
    }

    /// <summary>What it cannot vouch for, it does not touch.</summary>
    [Fact]
    public void AnythingItCannotVouchForIsLeftAlone()
    {
        var ftyp = Ftyp();
        var data = (uint)(ftyp.Length + 8);
        var mdat = Box("mdat", Payload(64, 1));
        var moov = Moov(Stco(data));

        // A top-level box that may hold absolute positions of its own, when the index would have to move past it.
        Assert.Null(Faststart.MoovFirst(Concat(ftyp, mdat, Box("meta", new byte[12]), moov)));
        Assert.Null(Faststart.MoovFirst(Concat(ftyp, mdat, moov, Box("moof", new byte[8]))));
        // …though a file already in order is fine whatever else it holds.
        var inOrder = Concat(ftyp, Moov(Stco(0)), Box("mdat", Payload(8, 1)), Box("meta", new byte[4]));
        Assert.Same(inOrder, Faststart.MoovFirst(inOrder));

        // Nothing to move, or nothing to move it in front of, or two indexes.
        Assert.Null(Faststart.MoovFirst(Concat(ftyp, mdat)));
        Assert.Null(Faststart.MoovFirst(Concat(ftyp, moov)));
        Assert.Null(Faststart.MoovFirst(Concat(ftyp, mdat, moov, moov)));
        Assert.Null(Faststart.IsMoovFirst(Concat(ftyp, mdat)));

        // Boxes that do not tile the file: one running past the end, a stub, a size smaller than its header.
        var truncated = Concat(ftyp, mdat, moov)[..^5];
        Assert.Null(Faststart.MoovFirst(truncated));
        Assert.Null(Faststart.IsMoovFirst(truncated));
        Assert.Null(Faststart.MoovFirst(Concat(ftyp, mdat, moov, new byte[3])));
        var tiny = (byte[])mdat.Clone();
        BinaryPrimitives.WriteUInt32BigEndian(tiny, 4);
        Assert.Null(Faststart.MoovFirst(Concat(ftyp, tiny, moov)));

        // A chunk table claiming more entries than it holds.
        var lying = Stco(data);
        BinaryPrimitives.WriteUInt32BigEndian(lying.AsSpan(12), 1000);
        Assert.Null(Faststart.MoovFirst(Concat(ftyp, mdat, Moov(lying))));
    }
}
