using System.Buffers.Binary;
using System.Text;
using FamilyConnect.App.Logic;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// Rule E's count — the sound tracks that PLAY, read from the container's own track list — and what is decided from it
/// (docs/protocol.md, "Preparing media before upload", <b>Rule E — a transcode keeps the sound</b>; issue #98).
/// </summary>
/// <remarks>
/// The files here are built box by box by a writer of this test's own, as <see cref="FaststartTests"/>' are, so the
/// production walker is never the thing that says it walked correctly. Every <c>tkhd</c> field is written at the offset
/// ISO/IEC 14496-12 gives it, and the ones before <c>alternate_group</c> are filled with non-zero bytes, so a walker
/// reading the group from the wrong version's offset reads a different number.
/// </remarks>
public sealed class ContainerSoundTests
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

    /// <summary>A box with a 64-bit size: size field 1, then the real size after the type.</summary>
    private static byte[] LargeBox(string type, byte[] body)
    {
        var box = new byte[16 + body.Length];
        BinaryPrimitives.WriteUInt32BigEndian(box, 1);
        Encoding.ASCII.GetBytes(type).CopyTo(box, 4);
        BinaryPrimitives.WriteUInt64BigEndian(box.AsSpan(8), (ulong)box.Length);
        body.CopyTo(box, 16);
        return box;
    }

    /// <summary>A box whose size field is 0: it runs to the end of the file.</summary>
    private static byte[] ToEndBox(string type, byte[] body)
    {
        var box = Box(type, body);
        BinaryPrimitives.WriteUInt32BigEndian(box, 0);
        return box;
    }

    /// <summary>
    /// A track header: version 0 (32-bit times, 84-byte body) or 1 (64-bit times, 96-byte body), with
    /// <paramref name="enabled"/> as flag 0x1 and <paramref name="group"/> as <c>alternate_group</c>. Everything before
    /// the group is non-zero filler, distinct per <paramref name="seed"/>.
    /// </summary>
    private static byte[] Tkhd(bool enabled, ushort group, byte version = 0, byte seed = 0x11, uint? id = null)
    {
        var body = new byte[version == 1 ? 96 : 84];
        var groupAt = version == 1 ? 46 : 34;
        for (var at = 4; at < groupAt; at++)
        {
            body[at] = (byte)(seed + at);
        }
        body[0] = version;
        body[3] = (byte)(enabled ? 0x7 : 0x6); // in-movie and in-preview set either way: only 0x1 is "enabled"
        BinaryPrimitives.WriteUInt16BigEndian(body.AsSpan(groupAt), group);
        if (id is { } trackId)
        {
            BinaryPrimitives.WriteUInt32BigEndian(body.AsSpan(version == 1 ? 20 : 12), trackId);
        }
        return Box("tkhd", body);
    }

    /// <summary>A handler reference: version and flags, pre_defined ("mhlr", as QuickTime writes it), the handler, reserved, a name.</summary>
    private static byte[] Hdlr(string handler) =>
        Box("hdlr", [.. new byte[4], .. "mhlr"u8.ToArray(), .. Encoding.ASCII.GetBytes(handler), .. new byte[12], .. "Core Media\0"u8.ToArray()]);

    private static byte[] Trak(string handler, bool enabled = true, ushort group = 0, byte version = 0, byte seed = 0x11) =>
        Box("trak",
            Tkhd(enabled, group, version, seed),
            Box("mdia", Box("mdhd", new byte[24]), Hdlr(handler), Box("minf", Box("stbl", Box("stsd", new byte[8])))));

    /// <summary>
    /// An ISO <c>AudioSampleEntry</c> (QuickTime's sound description version 0): reserved, data reference index, version,
    /// revision, vendor, then the channel count at 16 and the rate as 16.16 at 24 — and an <c>esds</c> after it.
    /// </summary>
    private static byte[] Mp4a(ushort channels, uint rate)
    {
        var body = new byte[28];
        BinaryPrimitives.WriteUInt16BigEndian(body.AsSpan(6), 1);
        BinaryPrimitives.WriteUInt16BigEndian(body.AsSpan(16), channels);
        BinaryPrimitives.WriteUInt16BigEndian(body.AsSpan(18), 16);
        BinaryPrimitives.WriteUInt32BigEndian(body.AsSpan(24), rate << 16);
        return Box("mp4a", body, Box("esds", new byte[27]));
    }

    /// <summary>QuickTime's sound description version 2: the rate as a 64-bit float at 32, the channel count as 32 bits at 40.</summary>
    private static byte[] Lpcm(uint channels, double rate)
    {
        var body = new byte[64];
        BinaryPrimitives.WriteUInt16BigEndian(body.AsSpan(6), 1);
        BinaryPrimitives.WriteUInt16BigEndian(body.AsSpan(8), 2);
        BinaryPrimitives.WriteUInt16BigEndian(body.AsSpan(16), 3);           // always 3 in version 2
        BinaryPrimitives.WriteUInt16BigEndian(body.AsSpan(18), 16);
        BinaryPrimitives.WriteUInt16BigEndian(body.AsSpan(20), 0xfffe);
        BinaryPrimitives.WriteUInt32BigEndian(body.AsSpan(24), 0x0001_0000); // always 1.0 in version 2
        BinaryPrimitives.WriteUInt32BigEndian(body.AsSpan(28), 72);
        BinaryPrimitives.WriteInt64BigEndian(body.AsSpan(32), BitConverter.DoubleToInt64Bits(rate));
        BinaryPrimitives.WriteUInt32BigEndian(body.AsSpan(40), channels);
        return Box("lpcm", body);
    }

    private static byte[] Stsd(byte[] entry) => Box("stsd", [0, 0, 0, 0, 0, 0, 0, 1, .. entry]);

    /// <summary>A sound track with its track id and a sample entry that states its channels and rate.</summary>
    private static byte[] Voice(uint id, byte[] entry, bool enabled = true, ushort group = 0) =>
        Box("trak",
            Tkhd(enabled, group, id: id),
            Box("mdia", Box("mdhd", new byte[24]), Hdlr("soun"), Box("minf", Box("smhd", new byte[8]), Box("stbl", Stsd(entry), Box("stts", new byte[8])))));

    private static byte[] Video() => Trak("vide", group: 0);

    private static byte[] Sound(bool enabled = true, ushort group = 0, byte version = 0, byte seed = 0x11) =>
        Trak("soun", enabled, group, version, seed);

    private static byte[] Moov(params byte[][] tracks) => Box("moov", [Box("mvhd", new byte[100]), .. tracks]);

    private static byte[] Ftyp() => Box("ftyp", Encoding.ASCII.GetBytes("isom\0\0\u0002\0isomiso2avc1mp41"));

    private static byte[] Mdat(int length = 4096) => Box("mdat", [.. Enumerable.Range(0, length).Select(at => (byte)(at * 7))]);

    private static byte[] Concat(params byte[][] parts) => [.. parts.SelectMany(part => part)];

    /// <summary>A camera's file: the index at the END, after the media data — the common case before faststart.</summary>
    private static byte[] Mp4(params byte[][] tracks) => Concat(Ftyp(), Mdat(), Moov(tracks));

    // --- the count -----------------------------------------------------------------------------------------------------

    [Fact]
    public void APictureAloneHasNoSoundTrack()
    {
        Assert.Equal(0, ContainerSound.Playing(Mp4(Video())));
        Assert.Equal(0, ContainerSound.Playing(Mp4()));
    }

    [Fact]
    public void OneEnabledSoundTrackCountsOne() =>
        Assert.Equal(1, ContainerSound.Playing(Mp4(Video(), Sound())));

    /// <summary>A screen recording's app sound and microphone: two voices, which Media Foundation cannot mix into one.</summary>
    [Fact]
    public void TwoEnabledSoundTracksCountTwo() =>
        Assert.Equal(2, ContainerSound.Playing(Mp4(Video(), Sound(), Sound())));

    /// <summary>An iPhone's spatial recording: its stereo AAC and its spatial track, of which a player picks one.</summary>
    [Fact]
    public void TracksInOneAlternateGroupCountOnce()
    {
        Assert.Equal(1, ContainerSound.Playing(Mp4(Video(), Sound(group: 1), Sound(group: 1))));
        Assert.Equal(2, ContainerSound.Playing(Mp4(Video(), Sound(group: 1), Sound(group: 1), Sound(group: 2))));
        Assert.Equal(2, ContainerSound.Playing(Mp4(Video(), Sound(group: 1), Sound(group: 0))));
        Assert.Equal(2, ContainerSound.Playing(Mp4(Video(), Sound(group: 0), Sound(group: 0))));
    }

    /// <summary>A disabled member of a group is not the group's one: the enabled one is.</summary>
    [Fact]
    public void ADisabledTrackIsNotCounted()
    {
        Assert.Equal(1, ContainerSound.Playing(Mp4(Video(), Sound(), Sound(enabled: false))));
        Assert.Equal(1, ContainerSound.Playing(Mp4(Video(), Sound(enabled: false, group: 1), Sound(group: 1))));
    }

    /// <summary>A player that ignores the flag plays them, so a transcode that drops them has lost something.</summary>
    [Fact]
    public void SoundTracksNoneOfWhichIsEnabledCountOne()
    {
        Assert.Equal(1, ContainerSound.Playing(Mp4(Video(), Sound(enabled: false))));
        Assert.Equal(1, ContainerSound.Playing(Mp4(Video(), Sound(enabled: false), Sound(enabled: false))));
    }

    /// <summary>
    /// Version 1's 64-bit times move <c>alternate_group</c> from 34 to 46. The filler at 34 differs per track, so reading
    /// version 1 at version 0's offset would see two groups — two voices — where there is one.
    /// </summary>
    [Fact]
    public void AVersionOneTrackHeaderIsReadAtItsOwnOffsets()
    {
        Assert.Equal(1, ContainerSound.Playing(Mp4(Video(), Sound(group: 5, version: 1, seed: 0x21), Sound(group: 5, version: 1, seed: 0x42))));
        Assert.Equal(2, ContainerSound.Playing(Mp4(Video(), Sound(group: 5, version: 1, seed: 0x21), Sound(group: 6, version: 1, seed: 0x21))));
        Assert.Equal(1, ContainerSound.Playing(Mp4(Video(), Sound(version: 1), Sound(enabled: false, version: 1))));
        // And the two versions side by side, as an editor's re-mux can leave them.
        Assert.Equal(1, ContainerSound.Playing(Mp4(Video(), Sound(group: 3, version: 0), Sound(group: 3, version: 1))));
    }

    [Fact]
    public void ATrackHeaderOfAnUnknownVersionCannotBeRead() =>
        Assert.Null(ContainerSound.Playing(Mp4(Video(), Sound(version: 2))));

    [Fact]
    public void SixtyFourBitBoxSizesAreHonoured()
    {
        Assert.Equal(2, ContainerSound.Playing(Concat(Ftyp(), LargeBox("mdat", new byte[512]), Moov(Video(), Sound(), Sound()))));
        Assert.Equal(1, ContainerSound.Playing(Concat(Ftyp(), LargeBox("mdat", new byte[512]), LargeBox("moov", Moov(Video(), Sound())[8..]))));
    }

    [Fact]
    public void AMoovThatRunsToTheEndIsRead() =>
        Assert.Equal(1, ContainerSound.Playing(Concat(Ftyp(), Mdat(), ToEndBox("moov", Moov(Video(), Sound())[8..]))));

    [Fact]
    public void TheMoovIsFoundWhereverItIs()
    {
        Assert.Equal(2, ContainerSound.Playing(Concat(Ftyp(), Moov(Video(), Sound(), Sound()), Mdat())));
        Assert.Equal(2, ContainerSound.Playing(Concat(Ftyp(), Box("free", new byte[16]), Mdat(), Box("wide"), Moov(Video(), Sound(), Sound()))));
    }

    /// <summary>QuickTime lets an atom list end in a 32-bit zero; that is not a malformed box.</summary>
    [Fact]
    public void AQuickTimeZeroTerminatorIsNotABox()
    {
        var moov = Box("moov", [Box("mvhd", new byte[100]), Video(), Sound(), new byte[4]]);
        Assert.Equal(1, ContainerSound.Playing(Concat(Ftyp(), Mdat(), moov)));
    }

    [Fact]
    public void WhatCannotBeWalkedIsNull()
    {
        Assert.Null(ContainerSound.Playing([]));
        Assert.Null(ContainerSound.Playing(Encoding.ASCII.GetBytes("RIFF....WEBPVP8 not an mp4 at all")));
        Assert.Null(ContainerSound.Playing([.. Enumerable.Range(0, 300).Select(at => (byte)((at * 31) + 5))]));
        Assert.Null(ContainerSound.Playing(Concat(Ftyp(), Mdat())));                    // no moov
        Assert.Null(ContainerSound.Playing(Mp4(Video(), Sound())[..^10]));           // a moov cut short
        var stub = new byte[5];
        stub[4] = 0xff;
        Assert.Null(ContainerSound.Playing(Concat(Ftyp(), Mdat(), Box("moov", [Box("mvhd", new byte[100]), Sound(), stub]))));
        var overrun = Mdat();
        BinaryPrimitives.WriteUInt32BigEndian(overrun, (uint)overrun.Length + 1000);  // mdat claims past the end
        Assert.Null(ContainerSound.Playing(Concat(Ftyp(), overrun, Moov(Sound()))));
    }

    /// <summary>A track it cannot vouch for makes the whole count unknown, never a guess that might be "none".</summary>
    [Fact]
    public void ATrackWithoutItsHeaderOrHandlerIsNull()
    {
        var noHandler = Box("trak", Tkhd(true, 0), Box("mdia", Box("mdhd", new byte[24])));
        var noHeader = Box("trak", Box("mdia", Box("mdhd", new byte[24]), Hdlr("soun")));
        var shortHeader = Box("trak", Box("tkhd", new byte[20]), Box("mdia", Hdlr("soun")));
        Assert.Null(ContainerSound.Playing(Mp4(Video(), noHandler)));
        Assert.Null(ContainerSound.Playing(Mp4(Video(), noHeader)));
        Assert.Null(ContainerSound.Playing(Mp4(Video(), shortHeader)));
    }

    /// <summary>A source can be gigabytes: only the box headers before the index and the index itself are read.</summary>
    [Fact]
    public void TheMediaDataIsNeverRead()
    {
        var mdat = Mdat(1_000_000);
        var file = Concat(Ftyp(), mdat, Moov(Video(), Sound()));
        using var counted = new CountingStream(file);
        Assert.Equal(1, ContainerSound.Playing(counted));
        Assert.True(counted.BytesRead < 4096, $"read {counted.BytesRead} bytes of a {file.Length}-byte file");
    }

    [Fact]
    public void AStreamThatCannotSeekIsNull()
    {
        using var forwardOnly = new CountingStream(Mp4(Video(), Sound()), seekable: false);
        Assert.Null(ContainerSound.Playing(forwardOnly));
    }

    // --- what Rule E decides -------------------------------------------------------------------------------------------

    [Fact]
    public void OneSoundTrackMediaFoundationShowsIsTranscoded()
    {
        Assert.Equal(SoundBefore.Transcode, ContainerSound.BeforeTranscode(1, decoderShowsAudio: true));
        Assert.Equal(SoundBefore.Transcode, ContainerSound.BeforeTranscode(0, decoderShowsAudio: false));
        // A container with no sound track and a decoder that shows one: the transcode is checked against the decoder.
        Assert.Equal(SoundBefore.Transcode, ContainerSound.BeforeTranscode(0, decoderShowsAudio: true));
    }

    /// <summary>L1 of issue #98: no audio shown, none asked for, none came out — and the read-back passed it.</summary>
    [Fact]
    public void ASoundTrackMediaFoundationDoesNotShowIsLeftAlone() =>
        Assert.Equal(SoundBefore.Unheard, ContainerSound.BeforeTranscode(1, decoderShowsAudio: false));

    /// <summary>
    /// L2 of issue #98, as the owner decided it: more than one voice is MIXED, never thinned to one — and sound Media
    /// Foundation does not show at all is still left alone however many tracks it is on: there is nothing to mix it from.
    /// </summary>
    [Fact]
    public void MoreThanOneSoundTrackThatPlaysIsMixed()
    {
        Assert.Equal(SoundBefore.Mix, ContainerSound.BeforeTranscode(2, decoderShowsAudio: true));
        Assert.Equal(SoundBefore.Mix, ContainerSound.BeforeTranscode(3, decoderShowsAudio: true));
        Assert.Equal(SoundBefore.Unheard, ContainerSound.BeforeTranscode(3, decoderShowsAudio: false));
    }

    /// <summary>A count nobody could read changes nothing: the transcode decides as it did before Rule E.</summary>
    [Fact]
    public void AnUnreadableCountDecidesNothing()
    {
        Assert.Equal(SoundBefore.Transcode, ContainerSound.BeforeTranscode(null, decoderShowsAudio: true));
        Assert.Equal(SoundBefore.Transcode, ContainerSound.BeforeTranscode(null, decoderShowsAudio: false));
    }

    [Fact]
    public void AResultWithNoSoundFromASourceWithSoundLostIt()
    {
        Assert.True(ContainerSound.LostSound(1, 0));
        Assert.True(ContainerSound.LostSound(2, 0));
        Assert.False(ContainerSound.LostSound(1, 1));
        Assert.False(ContainerSound.LostSound(0, 0), "a silent source makes a silent result");
        Assert.False(ContainerSound.LostSound(null, 0), "a source nobody could count");
        Assert.False(ContainerSound.LostSound(1, null), "a result nobody could count: the read-back decides");
    }

    /// <summary>The whole path, end to end on bytes: a source with sound, a result without, counted by the same reader.</summary>
    [Fact]
    public void ASilentTranscodeOfASourceWithSoundIsCaught()
    {
        var source = Mp4(Video(), Sound());
        var silent = Concat(Ftyp(), Moov(Video()), Mdat());
        var kept = Concat(Ftyp(), Moov(Video(), Sound()), Mdat());
        Assert.True(ContainerSound.LostSound(ContainerSound.Playing(source), ContainerSound.Playing(silent)));
        Assert.False(ContainerSound.LostSound(ContainerSound.Playing(source), ContainerSound.Playing(kept)));
    }

    // --- the track list ------------------------------------------------------------------------------------------------

    [Fact]
    public void TheTrackListIsTheSoundTracksInMoovOrderWithWhatTheirSampleEntriesSay()
    {
        var tracks = ContainerSound.Read(Mp4(Video(), Voice(2, Mp4a(2, 48_000)), Voice(7, Mp4a(1, 44_100), enabled: false, group: 3)));
        Assert.NotNull(tracks);
        Assert.Equal(
            [new SoundTrack(2, true, 0, 2, 48_000), new SoundTrack(7, false, 3, 1, 44_100)],
            tracks);
    }

    [Fact]
    public void AQuickTimeVersionTwoSoundDescriptionIsRead() =>
        Assert.Equal(
            [new SoundTrack(3, true, 0, 6, 96_000)],
            ContainerSound.Read(Mp4(Video(), Voice(3, Lpcm(6, 96_000)))));

    /// <summary>A sample entry it cannot read leaves the numbers unknown; the track is still a track.</summary>
    [Fact]
    public void AnUnreadableSampleEntryLeavesItsNumbersUnknown()
    {
        Assert.Equal([new SoundTrack(4, true, 0, null, null)], ContainerSound.Read(Mp4(Voice(4, Box("mp4a", new byte[12])))));
        Assert.Equal([null], ContainerSound.Read(Mp4(Sound()))!.Select(track => track.Channels));
    }

    [Fact]
    public void ThePlayingTracksAreFoundByPosition()
    {
        SoundTrack Track(bool enabled, ushort group = 0) => new(1, enabled, group, null, null);
        Assert.Equal([0, 2], ContainerSound.PlayingIndexes([Track(true), Track(false), Track(true)]));
        Assert.Equal([1, 3], ContainerSound.PlayingIndexes([Track(false, 1), Track(true, 1), Track(true, 1), Track(true, 2)]));
        Assert.Equal([0], ContainerSound.PlayingIndexes([Track(false), Track(false)]));
        Assert.Empty(ContainerSound.PlayingIndexes([]));
    }

    // --- the mix -------------------------------------------------------------------------------------------------------

    private static SoundTrack On(uint? channels = null, uint? rate = null, bool enabled = true, ushort group = 0) =>
        new(1, enabled, group, channels, rate);

    /// <summary>A screen recording: the app's sound and the microphone, both read by Media Foundation, in the same order.</summary>
    [Fact]
    public void EveryPlayingTrackIsMixedWhenMediaFoundationListsThemAll()
    {
        Assert.Equal([0, 1], ContainerSound.MixMap([On(2, 48_000), On(1, 48_000)], [new(2, 48_000, 128_000), new(1, 48_000, 64_000)]));
        // A disabled track is listed by both and mixed by neither.
        Assert.Equal([0, 2], ContainerSound.MixMap([On(), On(enabled: false), On()], [new(), new(), new()]));
        // Of an alternate group, its first enabled member only.
        Assert.Equal([0, 2], ContainerSound.MixMap([On(group: 1), On(group: 1), On()], [new(), new(), new()]));
    }

    /// <summary>
    /// Which one Media Foundation could not decode cannot be told from what it lists, and a mix of the wrong ones keeps
    /// fewer voices than the source has: the original goes.
    /// </summary>
    [Fact]
    public void AnEmbeddedListThatIsShortOrLongCannotBeMapped()
    {
        Assert.Null(ContainerSound.MixMap([On(), On(), On()], [new(), new()]));
        Assert.Null(ContainerSound.MixMap([On(), On()], [new(), new(), new()]));
        Assert.Null(ContainerSound.MixMap([], []));
    }

    /// <summary>Where both sides state a number and it differs, the order is not what it was assumed to be.</summary>
    [Fact]
    public void NumbersThatDisagreeMeanTheOrderIsWrong()
    {
        Assert.Null(ContainerSound.MixMap([On(2, 48_000), On(1, 44_100)], [new(1, 44_100, null), new(2, 48_000, null)]));
        Assert.Null(ContainerSound.MixMap([On(2), On(2)], [new(2, null, null), new(1, null, null)]));
        Assert.Null(ContainerSound.MixMap([On(rate: 48_000), On()], [new(null, 44_100, null), new()]));
        // A number only one side states, or one stated as 0, is not a disagreement.
        Assert.Equal([0, 1], ContainerSound.MixMap([On(2, null), On(null, 44_100)], [new(null, 48_000, null), new(0, 0, 0)]));
    }

    /// <summary>"Stereo when any of them is, at the highest sample rate any of them has" — capped by their rates together.</summary>
    [Fact]
    public void AMixIsStereoWhenAnyTrackIsAtTheHighestRateAny()
    {
        static (DecodedTrack, SoundTrack) Pair(DecodedTrack decoded, SoundTrack? container = null) => (decoded, container ?? On());
        Assert.Equal(
            new MixSound(2, 48_000, 192_000),
            ContainerSound.MixFormat([Pair(new(1, 44_100, 64_000)), Pair(new(2, 48_000, 128_000))]));
        Assert.Equal(new MixSound(1, 44_100, 128_000), ContainerSound.MixFormat([Pair(new(1, 44_100, 64_000)), Pair(new(1, 44_100, 64_000))]));
        Assert.Equal(new MixSound(2, null, null), ContainerSound.MixFormat([Pair(new(6, null, null)), Pair(new())]));
        // Mono beside a count nobody states is not known to be mono.
        Assert.Equal(new MixSound(null, 48_000, null), ContainerSound.MixFormat([Pair(new(1, 48_000, 64_000)), Pair(new(null, null, null))]));
        // The container's numbers where Media Foundation leaves one out; never its bit rate, which it does not state.
        Assert.Equal(new MixSound(2, 48_000, null), ContainerSound.MixFormat([Pair(new(0, 0, 0), On(2, 48_000)), Pair(new(1, 44_100, 64_000))]));
    }

    [Fact]
    public void AMixMustComeOutAsExactlyOneSoundTrack()
    {
        Assert.True(ContainerSound.MixCameOut(1));
        Assert.False(ContainerSound.MixCameOut(0), "the mix lost the sound");
        Assert.False(ContainerSound.MixCameOut(2), "the voices came out side by side, not mixed");
        Assert.False(ContainerSound.MixCameOut(null), "a mix nobody can check is not sent");
    }

    /// <summary>The whole mapping, end to end on bytes: what the container lists, beside what Media Foundation reads.</summary>
    [Fact]
    public void AScreenRecordingsTwoVoicesAreMappedFromItsBytes()
    {
        var tracks = ContainerSound.Read(Mp4(Video(), Voice(2, Mp4a(2, 48_000)), Voice(3, Mp4a(1, 48_000))))!;
        DecodedTrack[] decoded = [new(2, 48_000, 128_000), new(1, 48_000, 64_000)];
        var mixed = ContainerSound.MixMap(tracks, decoded);
        Assert.Equal([0, 1], mixed);
        Assert.Equal(new MixSound(2, 48_000, 192_000), ContainerSound.MixFormat(mixed!.Select(at => (decoded[at], tracks[at]))));
    }

    /// <summary>A read-only stream over bytes that counts what was read, and can refuse to seek.</summary>
    private sealed class CountingStream(byte[] bytes, bool seekable = true) : Stream
    {
        private readonly MemoryStream inner = new(bytes, writable: false);

        public long BytesRead { get; private set; }

        public override bool CanRead => true;

        public override bool CanSeek => seekable;

        public override bool CanWrite => false;

        public override long Length => seekable ? inner.Length : throw new NotSupportedException();

        public override long Position
        {
            get => inner.Position;
            set => inner.Position = seekable ? value : throw new NotSupportedException();
        }

        public override int Read(byte[] buffer, int offset, int count)
        {
            var read = inner.Read(buffer, offset, count);
            BytesRead += read;
            return read;
        }

        public override void Flush()
        {
        }

        public override long Seek(long offset, SeekOrigin origin) =>
            seekable ? inner.Seek(offset, origin) : throw new NotSupportedException();

        public override void SetLength(long value) => throw new NotSupportedException();

        public override void Write(byte[] buffer, int offset, int count) => throw new NotSupportedException();

        protected override void Dispose(bool disposing)
        {
            if (disposing)
            {
                inner.Dispose();
            }
            base.Dispose(disposing);
        }
    }
}
