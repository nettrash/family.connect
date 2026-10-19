using System.Buffers.Binary;
using System.Text;

namespace FamilyConnect.App.Logic;

/// <summary>What Rule E says of a video about to be transcoded, from its sound tracks (issue #98).</summary>
public enum SoundBefore
{
    /// <summary>
    /// The one-track transcode, as before Rule E: one sound track that plays and Media Foundation shows, none, or a
    /// count nobody could read.
    /// </summary>
    Transcode,

    /// <summary>
    /// More than one sound track plays: they are MIXED into the one AAC-LC track the profile allows. Media Foundation's
    /// transcoder carries one audio stream, so this is a different render (<c>MediaPreparing</c>'s composition).
    /// </summary>
    Mix,

    /// <summary>A sound track plays that Media Foundation does not show: nothing here can decode it, so the original goes.</summary>
    Unheard,
}

/// <summary>
/// One <c>soun</c> track as the container lists it, in <c>moov</c> order.
/// </summary>
/// <param name="TrackId">The <c>tkhd</c> <c>track_ID</c> — for the diagnostics log; nothing else reads it.</param>
/// <param name="Enabled"><c>tkhd</c> flag <c>0x1</c>.</param>
/// <param name="AlternateGroup">The <c>tkhd</c> <c>alternate_group</c>; 0 is "no group".</param>
/// <param name="Channels">The first sample entry's channel count, or null when it cannot be read.</param>
/// <param name="SampleRate">The first sample entry's sample rate in hertz, or null when it cannot be read.</param>
public sealed record SoundTrack(uint TrackId, bool Enabled, ushort AlternateGroup, uint? Channels, uint? SampleRate);

/// <summary>What Media Foundation reads of one embedded audio track: its channels and sample rate, when it states them.</summary>
public readonly record struct DecodedTrack(uint? Channels, uint? SampleRate, uint? Bitrate);

/// <summary>What a mix is encoded from: the channel count, sample rate and total bit rate of the tracks mixed.</summary>
/// <param name="Channels">2 when any track is stereo or more, 1 when every one is mono, null when none says and none is stereo.</param>
/// <param name="SampleRate">The highest any track has, or null when none says.</param>
/// <param name="Bitrate">The tracks' bit rates added up — rule B's cap for the one track they become — or null when any is unknown.</param>
public readonly record struct MixSound(long? Channels, long? SampleRate, long? Bitrate);

/// <summary>
/// The sound tracks an MP4 or a QuickTime file carries, read from the container's own track list; which of them PLAY;
/// and what Rule E decides from that (docs/protocol.md, "Preparing media before upload", <b>Rule E — a transcode keeps
/// the sound</b>; issue #98).
/// </summary>
/// <remarks>
/// <para>
/// <b>WHY THE CONTAINER AND NOT MEDIA FOUNDATION.</b> A decoder that cannot read a track does not list it: when
/// <c>MediaEncodingProfile.CreateFromFileAsync</c> shows no audio for a clip whose sound is in a sample entry it does not
/// know, every profile asks for no audio, none comes out, and the read-back agrees with itself over a SILENT upload. The
/// shell's numbers (<see cref="MediaEncoding.HidesAudio"/>) are read through the same property handlers and are likely
/// blind to the same tracks. The <c>moov</c> is not: every <c>trak</c> whose <c>mdia</c>/<c>hdlr</c> handler type is
/// <c>soun</c> is a sound track, whatever is inside it.
/// </para>
/// <para>
/// <b>WHAT COUNTS IS WHAT PLAYS.</b> An enabled track (<c>tkhd</c> flag <c>0x1</c>) counts; of the enabled tracks that
/// share a non-zero <c>alternate_group</c> only the first does, since a player picks one of them — an iPhone's spatial
/// recording carries its stereo AAC and its spatial track as such a pair, and that is one voice, not two. A file whose
/// sound tracks are all disabled counts one — the first: a player that honours the flag plays nothing, but a transcode
/// that drops them has still lost what a player that ignores it would have played.
/// </para>
/// <para>
/// <b>MORE THAN ONE THAT PLAYS IS MIXED</b> — a screen recording's app sound and microphone, a camera's two
/// microphones. The tracks a mix is made of are found in Media Foundation's list of embedded tracks by
/// <see cref="MixMap"/>, and that mapping is the one place this has to trust an order it cannot see; see there.
/// </para>
/// <para>
/// <b>ANYTHING IT CANNOT WALK IS NULL</b> — not ISO BMFF, a box that runs past its parent, a <c>trak</c> with no
/// <c>tkhd</c> or no handler, a <c>moov</c> too big to be an index — and null means "nobody knows", which leaves the
/// transcode to decide as it did before Rule E. The walk is <see cref="Faststart"/>'s: boxes must tile their parent,
/// 64-bit sizes and size 0 ("to the end") are honoured, and the <c>moov</c> may sit anywhere at the top level. Only the
/// box headers before it are read, and then the <c>moov</c> alone — never the media data. A sample entry it cannot read
/// leaves that track's channels and rate unknown; it does not make the track unknown.
/// </para>
/// </remarks>
public static class ContainerSound
{
    /// <summary>
    /// The biggest <c>moov</c> read into memory. An hour of 4K is a few megabytes of index; anything past this is not a
    /// file this walk vouches for, and it says so with null rather than allocating it.
    /// </summary>
    public const long MaxMoovBytes = 64L * 1024 * 1024;

    private readonly record struct Box(int Start, int Size, int Header, string Type);

    // --- Rule E --------------------------------------------------------------------------------------------------------

    /// <summary>
    /// What Rule E says before a transcode: <paramref name="playing"/> is what <see cref="Playing(Stream)"/> counted,
    /// <paramref name="decoderShowsAudio"/> whether Media Foundation's view of the streams has an audio track. A count
    /// nobody could read is <see cref="SoundBefore.Transcode"/> — behaviour as before Rule E, with
    /// <see cref="MediaEncoding.HidesAudio"/> still the guard it was. Sound Media Foundation does not show at all is
    /// <see cref="SoundBefore.Unheard"/> however many tracks there are: there is nothing to mix it from.
    /// </summary>
    public static SoundBefore BeforeTranscode(int? playing, bool decoderShowsAudio) =>
        playing switch
        {
            > 0 when !decoderShowsAudio => SoundBefore.Unheard,
            > 1 => SoundBefore.Mix,
            _ => SoundBefore.Transcode,
        };

    /// <summary>
    /// Whether a transcode LOST the sound: its source had a sound track that plays and the result has none. That is a
    /// failed transcode (Rule C) however much smaller it is — Rule D never keeps a silent result over a source with
    /// sound. A count that could not be read on either side says nothing, and the read-back decides as before.
    /// </summary>
    public static bool LostSound(int? source, int? result) => source is > 0 && result == 0;

    /// <summary>
    /// Whether a MIX came out as the one sound track the profile allows: exactly one in the result's own track list. A
    /// count that cannot be read is not one — a mix is this file's own render, and one it cannot check is not sent.
    /// </summary>
    public static bool MixCameOut(int? result) => result == 1;

    /// <summary>
    /// Which of Media Foundation's embedded audio tracks (<c>MediaClip.EmbeddedAudioTracks</c>, in its order) are the
    /// container's PLAYING tracks — or null when that cannot be told, and the source goes as it is (Rule C).
    /// </summary>
    /// <remarks>
    /// <para>
    /// <b>BY POSITION, AND ONLY WHEN NOTHING IS MISSING.</b> An embedded track carries no <c>track_ID</c> — WinRT's
    /// <c>EmbeddedAudioTrack</c> has its encoding properties and nothing that names the <c>trak</c> it came from. Media
    /// Foundation's MPEG-4 source presents a stream for each track in the order the <c>moov</c> lists them, so when it
    /// lists exactly as many audio tracks as the container has <c>soun</c> tracks, the n-th of one is the n-th of the
    /// other. When it lists FEWER — a sample entry it cannot decode, the very case Rule E exists for — which one is
    /// missing cannot be known, and a mix of the wrong ones would keep fewer voices than the source has, which the
    /// protocol counts as a failed transcode.
    /// </para>
    /// <para>
    /// <b>AND CHECKED WHERE IT CAN BE.</b> Where both sides state a channel count or a sample rate and they differ, the
    /// order is not what it was assumed to be, and the answer is null too.
    /// </para>
    /// </remarks>
    public static IReadOnlyList<int>? MixMap(IReadOnlyList<SoundTrack> container, IReadOnlyList<DecodedTrack> embedded)
    {
        if (container.Count == 0 || embedded.Count != container.Count)
        {
            return null;
        }
        for (var at = 0; at < container.Count; at++)
        {
            if (Differ(container[at].Channels, embedded[at].Channels) || Differ(container[at].SampleRate, embedded[at].SampleRate))
            {
                return null;
            }
        }
        return PlayingIndexes(container);
    }

    /// <summary>
    /// What the mix of <paramref name="mixed"/> is encoded from (Rule E): stereo when any of them is, at the highest
    /// sample rate any of them has, capped by their bit rates added up. Media Foundation's numbers first, the container's
    /// where those leave one out.
    /// </summary>
    public static MixSound MixFormat(IEnumerable<(DecodedTrack Decoded, SoundTrack Container)> mixed)
    {
        var tracks = mixed.ToList();
        var channels = tracks.Select(track => Known(track.Decoded.Channels) ?? Known(track.Container.Channels)).ToList();
        var rates = tracks.Select(track => Known(track.Decoded.SampleRate) ?? Known(track.Container.SampleRate)).ToList();
        var bitrates = tracks.Select(track => Known(track.Decoded.Bitrate)).ToList();
        return new MixSound(
            channels.Any(count => count >= 2) ? 2L
                : channels.Count > 0 && channels.All(count => count == 1) ? 1L
                : null,
            rates.Max(),
            bitrates.Count > 0 && bitrates.All(rate => rate is not null) ? bitrates.Sum(rate => (long)rate!.Value) : null);
    }

    // --- counting ------------------------------------------------------------------------------------------------------

    /// <summary>
    /// The positions in <paramref name="tracks"/> of the ones that PLAY: enabled, the first of each non-zero alternate
    /// group, and the first track when none is enabled.
    /// </summary>
    public static IReadOnlyList<int> PlayingIndexes(IReadOnlyList<SoundTrack> tracks)
    {
        List<int> playing = [];
        HashSet<ushort> groups = [];
        for (var at = 0; at < tracks.Count; at++)
        {
            var track = tracks[at];
            // Group 0 is "no group": every such track plays. A non-zero group plays one of its members.
            if (track.Enabled && (track.AlternateGroup == 0 || groups.Add(track.AlternateGroup)))
            {
                playing.Add(at);
            }
        }
        return tracks.Count > 0 && playing.Count == 0 ? [0] : playing;
    }

    /// <summary>The sound tracks that play in <paramref name="file"/>, or null when it cannot be walked.</summary>
    public static int? Playing(byte[] file) => Read(file) is { } tracks ? PlayingIndexes(tracks).Count : null;

    /// <summary>The sound tracks that play in a seekable <paramref name="file"/>, or null when it cannot be walked.</summary>
    public static int? Playing(Stream file) => Read(file) is { } tracks ? PlayingIndexes(tracks).Count : null;

    /// <summary>Every <c>soun</c> track in <paramref name="file"/>, in <c>moov</c> order, or null when it cannot be walked.</summary>
    public static IReadOnlyList<SoundTrack>? Read(byte[] file)
    {
        using var stream = new MemoryStream(file, writable: false);
        return Read(stream);
    }

    /// <summary>
    /// Every <c>soun</c> track in a seekable <paramref name="file"/>, in <c>moov</c> order, or null when it cannot be
    /// walked: not seekable, no <c>moov</c>, a box that does not fit, or a <c>moov</c> over <see cref="MaxMoovBytes"/>.
    /// Reads only the top-level box headers up to the first <c>moov</c>, and that <c>moov</c>. A read that throws is the
    /// caller's to catch.
    /// </summary>
    public static IReadOnlyList<SoundTrack>? Read(Stream file)
    {
        if (!file.CanSeek || !file.CanRead)
        {
            return null;
        }
        var end = file.Length;
        Span<byte> header = stackalloc byte[16];
        for (long at = 0; at < end;)
        {
            if (end - at < 8)
            {
                return null;
            }
            file.Position = at;
            file.ReadExactly(header[..8]);
            long size = BinaryPrimitives.ReadUInt32BigEndian(header);
            var type = Encoding.Latin1.GetString(header.Slice(4, 4));
            var length = 8;
            if (size == 1)
            {
                if (end - at < 16)
                {
                    return null;
                }
                file.ReadExactly(header.Slice(8, 8));
                var large = BinaryPrimitives.ReadUInt64BigEndian(header[8..]);
                size = large > long.MaxValue ? long.MaxValue : (long)large;
                length = 16;
            }
            else if (size == 0)
            {
                size = end - at;
            }
            if (size < length || size > end - at)
            {
                return null;
            }
            if (type == "moov")
            {
                if (size - length > MaxMoovBytes)
                {
                    return null;
                }
                var moov = new byte[size - length];
                file.ReadExactly(moov);
                return ReadMoov(moov);
            }
            at += size;
        }
        return null;
    }

    /// <summary>The <c>soun</c> tracks among a <c>moov</c>'s <c>trak</c>s, given its body; null when one cannot be read.</summary>
    private static List<SoundTrack>? ReadMoov(ReadOnlySpan<byte> moov)
    {
        if (Walk(moov, 0, moov.Length) is not { } children)
        {
            return null;
        }
        List<SoundTrack> sound = [];
        foreach (var child in children)
        {
            if (child.Type != "trak")
            {
                continue;
            }
            var read = ReadTrack(Body(moov, child), out var track);
            if (!read)
            {
                return null;
            }
            if (track is not null)
            {
                sound.Add(track);
            }
        }
        return sound;
    }

    /// <summary>
    /// One <c>trak</c>, given its body: false when its <c>tkhd</c> or its <c>mdia</c>/<c>hdlr</c> is missing or short;
    /// otherwise true, with <paramref name="track"/> set when it is a sound track and null when it is anything else.
    /// </summary>
    private static bool ReadTrack(ReadOnlySpan<byte> trak, out SoundTrack? track)
    {
        track = null;
        if (Walk(trak, 0, trak.Length) is not { } children
            || children.FirstOrDefault(box => box.Type == "tkhd") is not { Size: > 0 } tkhd
            || children.FirstOrDefault(box => box.Type == "mdia") is not { Size: > 0 } mdia)
        {
            return false;
        }
        var header = Body(trak, tkhd);
        // A full box: version (1) and flags (3), then creation, modification, track_ID, reserved and duration — 32-bit
        // times and duration in version 0, 64-bit in version 1 — then reserved (8) and layer (2) before alternate_group.
        var (idAt, groupAt) = header.Length > 0 ? header[0] switch
        {
            0 => (4 + 4 + 4, 4 + 4 + 4 + 4 + 4 + 4 + 8 + 2),
            1 => (4 + 8 + 8, 4 + 8 + 8 + 4 + 4 + 8 + 8 + 2),
            _ => (-1, -1),
        } : (-1, -1);
        if (groupAt < 0 || header.Length < groupAt + 2)
        {
            return false;
        }

        var media = Body(trak, mdia);
        if (Walk(media, 0, media.Length) is not { } boxes
            || boxes.FirstOrDefault(box => box.Type == "hdlr") is not { Size: > 0 } hdlr)
        {
            return false;
        }
        // A full box: version and flags (4), pre_defined — QuickTime's component type, "mhlr" — (4), then handler_type.
        var handler = Body(media, hdlr);
        if (handler.Length < 12)
        {
            return false;
        }
        if (!handler.Slice(8, 4).SequenceEqual("soun"u8))
        {
            return true;
        }
        var (channels, rate) = boxes.FirstOrDefault(box => box.Type == "minf") is { Size: > 0 } minf
            ? SampleEntry(Body(media, minf))
            : (null, null);
        track = new SoundTrack(
            BinaryPrimitives.ReadUInt32BigEndian(header[idAt..]),
            (header[3] & 0x1) != 0,
            BinaryPrimitives.ReadUInt16BigEndian(header[groupAt..]),
            channels,
            rate);
        return true;
    }

    /// <summary>
    /// The channel count and sample rate of the first sample entry in <c>minf</c>/<c>stbl</c>/<c>stsd</c>, each null when
    /// it cannot be read. The layout is the ISO <c>AudioSampleEntry</c>, which is QuickTime's sound description versions
    /// 0 and 1 too: six reserved bytes and the data reference index, a version (QuickTime's; 0 in ISO), revision and
    /// vendor, then the channel count (2) at 16, sample size, compression id and packet size, and the rate as 16.16 at
    /// 24. QuickTime's version 2 moves both: the rate to a 64-bit float at 32 and the count to 32 bits at 40.
    /// </summary>
    private static (uint? Channels, uint? SampleRate) SampleEntry(ReadOnlySpan<byte> minf)
    {
        if (Walk(minf, 0, minf.Length)?.FirstOrDefault(box => box.Type == "stbl") is not { Size: > 0 } stbl)
        {
            return (null, null);
        }
        var table = Body(minf, stbl);
        if (Walk(table, 0, table.Length)?.FirstOrDefault(box => box.Type == "stsd") is not { Size: > 0 } stsd)
        {
            return (null, null);
        }
        // A full box: version and flags (4), entry_count (4), then the entries, each a box.
        var descriptions = Body(table, stsd);
        if (descriptions.Length < 8 || BinaryPrimitives.ReadUInt32BigEndian(descriptions[4..]) == 0
            || Walk(descriptions, 8, descriptions.Length) is not [var first, ..])
        {
            return (null, null);
        }
        var entry = Body(descriptions, first);
        if (entry.Length < 28)
        {
            return (null, null);
        }
        if (BinaryPrimitives.ReadUInt16BigEndian(entry[8..]) == 2)
        {
            if (entry.Length < 44)
            {
                return (null, null);
            }
            var hertz = BitConverter.Int64BitsToDouble(BinaryPrimitives.ReadInt64BigEndian(entry[32..]));
            return (
                Known(BinaryPrimitives.ReadUInt32BigEndian(entry[40..])),
                double.IsFinite(hertz) && hertz >= 1 && hertz <= uint.MaxValue ? (uint)Math.Round(hertz) : null);
        }
        return (
            Known(BinaryPrimitives.ReadUInt16BigEndian(entry[16..])),
            Known((uint)BinaryPrimitives.ReadUInt16BigEndian(entry[24..])));
    }

    private static ReadOnlySpan<byte> Body(ReadOnlySpan<byte> parent, Box box) =>
        parent.Slice(box.Start + box.Header, box.Size - box.Header);

    private static uint? Known(uint? value) => value is > 0 ? value : null;

    private static bool Differ(uint? container, uint? decoded) =>
        Known(container) is { } one && Known(decoded) is { } other && one != other;

    /// <summary>
    /// The boxes laid end to end in <paramref name="start"/>..<paramref name="end"/>, as <see cref="Faststart"/> walks them,
    /// or null when they do not tile it. The one slack is QuickTime's: an atom list may end in a 32-bit zero terminator,
    /// so a stub shorter than a header is let through when it is all zeros.
    /// </summary>
    private static List<Box>? Walk(ReadOnlySpan<byte> data, int start, int end)
    {
        List<Box> boxes = [];
        var at = start;
        while (at < end)
        {
            if (end - at < 8)
            {
                return data[at..end].IndexOfAnyExcept((byte)0) < 0 ? boxes : null;
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
}
