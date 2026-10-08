using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace FamilyConnect.App.Logic;

/// <summary>
/// One voice message that was not sent (docs/audio-video-messages-2026-10-04.md, S2.8): the chat it belongs to, how long
/// it is, the reply it was recorded under and its caption.
/// </summary>
public sealed record ParkedRecording(string Id, long ChatId, int DurationMs, long? ReplyToMessageId, string? Caption, DateTimeOffset ParkedAt);

/// <summary>
/// The voice messages that were not sent, kept on this device per account — the file with its length, its reply and
/// its caption — so that what an interruption stopped survives the app being closed, and can be seen, sent and deleted.
/// </summary>
/// <remarks>
/// <para>
/// <b>ONE FOLDER PER RECORDING, AND IT LANDS WHOLE OR NOT AT ALL.</b> The bytes and what is known about them are
/// written under a part name and moved into place, so a crash mid-write can never leave an entry that reads as a shorter
/// recording, or as one whose caption is missing; a part left behind is no entry, and is swept at launch with anything
/// else no entry names.
/// </para>
/// <para>
/// <b>PER ACCOUNT, PER SERVER.</b> Each account's recordings are under a folder named for the server and the account
/// together, so nobody else signed in on this device — or the same id on another server — ever sees them. They are
/// WIPED whenever the session ends, as the cache and its outbox are (<see cref="WipeAll"/>): a recording belongs to the
/// person who made it.
/// </para>
/// <para>
/// <b>AN ID IS NOT A PATH.</b> Anything but the 32 hex digits this store makes is not read, and not deleted.
/// </para>
/// <para>
/// <b>WHAT THE DISK REFUSES IS HELD IN MEMORY</b> (S4: the disk fills), for as long as this store lives — the app's run —
/// and listed, played, sent and deleted exactly as a written one (<see cref="Hold"/>): still its own row with its own reply
/// and caption, never carried by another Send (S2.8). <see cref="WriteHeld"/> tries the disk again.
/// </para>
/// <para>
/// One lock for every store, because a wipe after a sign-out runs on whatever thread noticed it, beside a park on the
/// window's.
/// </para>
/// </remarks>
public sealed class ParkedRecordings(string folder)
{
    private const string Part = ".part";
    private const string BytesFile = "voice.m4a";
    private const string MetaFile = "meta.json";

    private static readonly object Gate = new();

    /// <summary>The recordings the disk refused, by id, with their bytes.</summary>
    private readonly Dictionary<string, (ParkedRecording Entry, byte[] Bytes)> held = new(StringComparer.Ordinal);

    /// <summary>The folder this store keeps its recordings in.</summary>
    public string Folder => folder;

    /// <summary>The store for one account on one server, under <paramref name="root"/>.</summary>
    public static ParkedRecordings For(string root, Uri server, long accountId) =>
        new(Path.Combine(root, OwnerKey(server, accountId)));

    /// <summary>The folder name for one account on one server: never the address or the id themselves.</summary>
    public static string OwnerKey(Uri server, long accountId)
    {
        var owner = $"{server.AbsoluteUri}\n{accountId.ToString(System.Globalization.CultureInfo.InvariantCulture)}";
        return Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(owner)).AsSpan(0, 16));
    }

    /// <summary>
    /// Keep a recording that was not sent. Answers the entry, or throws when it could not be written — the caller keeps
    /// the recording some other way rather than lose it (<see cref="Hold"/>).
    /// </summary>
    public ParkedRecording Park(long chatId, ReadOnlyMemory<byte> bytes, int durationMs, long? replyTo, string? caption, DateTimeOffset at)
    {
        var entry = NewEntry(chatId, durationMs, replyTo, caption, at);
        lock (Gate)
        {
            Write(entry, bytes);
            return entry;
        }
    }

    /// <summary>
    /// Keep a recording the disk refused, in memory (S4: the disk fills): the same entry a park would have made — its chat,
    /// length, reply and caption — listed, played, sent and deleted like any other, until the session ends or the app does.
    /// </summary>
    public ParkedRecording Hold(long chatId, ReadOnlyMemory<byte> bytes, int durationMs, long? replyTo, string? caption, DateTimeOffset at)
    {
        var entry = NewEntry(chatId, durationMs, replyTo, caption, at);
        lock (Gate)
        {
            held[entry.Id] = (entry, bytes.ToArray());
        }
        return entry;
    }

    /// <summary>Whether a recording here is held only in memory — gone with the app unless the disk takes it (<see cref="WriteHeld"/>).</summary>
    public bool HoldsInMemory
    {
        get
        {
            lock (Gate)
            {
                return held.Count > 0;
            }
        }
    }

    /// <summary>
    /// The disk, tried again for every recording held in memory — what a real close does before it lets the app go. Each one
    /// written moves from memory to disk under the same id, so a row drawn for it is still its row; one the disk still refuses
    /// stays held. Answers how many are still held.
    /// </summary>
    public int WriteHeld()
    {
        lock (Gate)
        {
            foreach (var (id, (entry, bytes)) in held.ToList())
            {
                try
                {
                    Write(entry, bytes);
                    held.Remove(id);
                }
                catch (Exception e) when (e is IOException or UnauthorizedAccessException)
                {
                    // Still refused: it stays in memory, its row unchanged.
                }
            }
            return held.Count;
        }
    }

    /// <summary>Every recording held in memory, gone: the session ended, and they go with everything else recorded and not sent (S4).</summary>
    public void ForgetHeld()
    {
        lock (Gate)
        {
            held.Clear();
        }
    }

    /// <summary>Every recording this account has waiting, oldest first — the written ones and those held in memory.</summary>
    public IReadOnlyList<ParkedRecording> All()
    {
        lock (Gate)
        {
            var found = new List<ParkedRecording>(held.Values.Select(kept => kept.Entry));
            if (Directory.Exists(folder))
            {
                foreach (var path in Directory.EnumerateDirectories(folder))
                {
                    if (Entry(path) is { } entry)
                    {
                        found.Add(entry);
                    }
                }
            }
            return [.. found.OrderBy(entry => entry.ParkedAt).ThenBy(entry => entry.Id, StringComparer.Ordinal)];
        }
    }

    /// <summary>One chat's recordings waiting, oldest first.</summary>
    public IReadOnlyList<ParkedRecording> Of(long chatId) => [.. All().Where(entry => entry.ChatId == chatId)];

    /// <summary>Whether a chat holds a recording that was not sent — the microphone's "send or delete it first" (S1.3).</summary>
    public bool Any(long chatId) => All().Any(entry => entry.ChatId == chatId);

    /// <summary>
    /// The recording as the voice note it is, ready to stage for a send — its duration its identity, and no name, as
    /// <see cref="VoiceNotes.Staged"/> makes one — or null when it is gone or cannot be read.
    /// </summary>
    public StagedMedia? Staged(ParkedRecording entry)
    {
        if (!IsId(entry.Id))
        {
            return null;
        }
        lock (Gate)
        {
            if (held.TryGetValue(entry.Id, out var kept))
            {
                return new StagedMedia("audio", VoiceNotes.Mime, kept.Bytes, DurationMs: kept.Entry.DurationMs);
            }
            try
            {
                var bytes = File.ReadAllBytes(Path.Combine(folder, entry.Id, BytesFile));
                return new StagedMedia("audio", VoiceNotes.Mime, bytes, DurationMs: entry.DurationMs);
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                return null;
            }
        }
    }

    /// <summary>
    /// Gone — sent from its row, or deleted. What it knows goes FIRST, so a recording whose file is held open for a
    /// moment is no longer an entry, and the sweep takes the rest. Answers whether it is gone.
    /// </summary>
    public bool Remove(ParkedRecording entry)
    {
        if (!IsId(entry.Id))
        {
            return false;
        }
        var path = Path.Combine(folder, entry.Id);
        lock (Gate)
        {
            if (held.Remove(entry.Id))
            {
                return true;
            }
            try
            {
                if (Directory.Exists(path))
                {
                    File.Delete(Path.Combine(path, MetaFile));
                }
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                return false;
            }
            TryDelete(path);
            return true;
        }
    }

    /// <summary>At launch: anything in this account's folder no entry names — a part a crash left, an entry missing half of itself — goes.</summary>
    public void Sweep()
    {
        lock (Gate)
        {
            if (!Directory.Exists(folder))
            {
                return;
            }
            foreach (var path in Directory.EnumerateFileSystemEntries(folder))
            {
                if (!Directory.Exists(path) || Entry(path) is null)
                {
                    TryDelete(path);
                }
            }
        }
    }

    /// <summary>
    /// Every account's recordings, gone: what a session ending does — signed out, expired, the family left, the account
    /// deleted — and what pointing the app at another server does.
    /// </summary>
    public static void WipeAll(string root)
    {
        lock (Gate)
        {
            if (!Directory.Exists(root))
            {
                return;
            }
            try
            {
                Directory.Delete(root, recursive: true);
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                // Something held open: whatever is left is unmade as an entry, recording by recording, and swept later.
                foreach (var meta in Directory.EnumerateFiles(root, MetaFile, SearchOption.AllDirectories))
                {
                    TryDelete(meta);
                }
            }
        }
    }

    private static ParkedRecording NewEntry(long chatId, int durationMs, long? replyTo, string? caption, DateTimeOffset at) => new(
        Guid.NewGuid().ToString("N"), chatId, Math.Max(0, durationMs), replyTo, string.IsNullOrWhiteSpace(caption) ? null : caption, at);

    /// <summary>One recording onto the disk, under a part name moved into place — whole, or not at all and thrown. Under the lock.</summary>
    private void Write(ParkedRecording entry, ReadOnlyMemory<byte> bytes)
    {
        var final = Path.Combine(folder, entry.Id);
        var part = final + Part;
        try
        {
            Directory.CreateDirectory(part);
            File.WriteAllBytes(Path.Combine(part, BytesFile), bytes.ToArray());
            File.WriteAllBytes(Path.Combine(part, MetaFile), Meta(entry));
            Directory.Move(part, final);
        }
        catch
        {
            TryDelete(part);
            throw;
        }
    }

    private static ParkedRecording? Entry(string path)
    {
        var id = Path.GetFileName(path);
        if (!IsId(id) || !File.Exists(Path.Combine(path, BytesFile)))
        {
            return null;
        }
        try
        {
            using var meta = JsonDocument.Parse(File.ReadAllBytes(Path.Combine(path, MetaFile)));
            var root = meta.RootElement;
            if (root.ValueKind != JsonValueKind.Object
                || Whole(root, "chat_id") is not ({ } chatId and > 0)
                || Whole(root, "duration_ms") is not ({ } duration and >= 0 and <= int.MaxValue)
                || Whole(root, "parked_at_ms") is not { } at)
            {
                return null;
            }
            var reply = Whole(root, "reply_to_message_id");
            var caption = root.TryGetProperty("caption", out var words) && words.ValueKind == JsonValueKind.String ? words.GetString() : null;
            return new ParkedRecording(
                id, chatId, (int)duration, reply is > 0 ? reply : null,
                string.IsNullOrWhiteSpace(caption) ? null : caption, DateTimeOffset.FromUnixTimeMilliseconds(at));
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or JsonException or ArgumentOutOfRangeException)
        {
            return null;
        }
    }

    private static long? Whole(JsonElement root, string name) =>
        root.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.Number && value.TryGetInt64(out var number)
            ? number
            : null;

    /// <summary>
    /// What is known about a recording, as JSON: numbers as numbers — a time as milliseconds since 1970, never as text a
    /// reader's culture could spell differently.
    /// </summary>
    private static byte[] Meta(ParkedRecording entry)
    {
        using var buffer = new MemoryStream();
        using (var writer = new Utf8JsonWriter(buffer))
        {
            writer.WriteStartObject();
            writer.WriteNumber("chat_id", entry.ChatId);
            writer.WriteNumber("duration_ms", entry.DurationMs);
            if (entry.ReplyToMessageId is { } reply)
            {
                writer.WriteNumber("reply_to_message_id", reply);
            }
            if (entry.Caption is { } caption)
            {
                writer.WriteString("caption", caption);
            }
            writer.WriteNumber("parked_at_ms", entry.ParkedAt.ToUnixTimeMilliseconds());
            writer.WriteEndObject();
        }
        return buffer.ToArray();
    }

    private static bool IsId(string id) =>
        id.Length == 32 && id.All(c => char.IsAsciiDigit(c) || c is >= 'a' and <= 'f');

    private static void TryDelete(string entry)
    {
        try
        {
            if (Directory.Exists(entry))
            {
                Directory.Delete(entry, recursive: true);
            }
            else if (File.Exists(entry))
            {
                File.Delete(entry);
            }
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            // Held open for a moment: no longer an entry, and the next sweep comes back to it.
        }
    }
}
