using System.Text.Json;

namespace FamilyConnect.App.Logic;

/// <summary>
/// A send's bytes on disk while the send is under way: one folder per handle, holding the bytes, the
/// preview and what the server has to be told.
/// </summary>
/// <remarks>
/// <para>
/// <b>A STAGE LANDS WHOLE OR NOT AT ALL.</b> It is written under a part name and moved into place, so a
/// crash mid-write can never leave a handle that reads as a shorter photo.
/// </para>
/// <para>
/// <b>A HANDLE IS PINNED FROM THE MOMENT IT IS MADE UNTIL ITS ROW IS QUEUED.</b> The sweep keeps what
/// the outbox names, and a flush on another thread can sweep between "written" and "queued" — the one
/// moment the outbox names nothing. Pinned handles are never swept.
/// </para>
/// <para>
/// <b>A HANDLE IS NOT A PATH.</b> It comes back out of the cache, and anything but the 32 hex digits
/// this store makes is not read, and not deleted.
/// </para>
/// </remarks>
public sealed class FolderMediaStore(string folder) : IMediaStore
{
    private const string Part = ".part";

    private readonly object gate = new();
    private readonly HashSet<string> pinned = new(StringComparer.Ordinal);

    /// <summary>Write one send's media, pinned. Release the handle once its row is queued.</summary>
    public string Stage(StagedMedia media)
    {
        var handle = Guid.NewGuid().ToString("N");
        lock (gate)
        {
            pinned.Add(handle);
        }
        var final = Path.Combine(folder, handle);
        var part = final + Part;
        try
        {
            Directory.CreateDirectory(part);
            File.WriteAllBytes(Path.Combine(part, "bytes"), media.Bytes.ToArray());
            if (media.Preview is { IsEmpty: false } preview)
            {
                File.WriteAllBytes(Path.Combine(part, "preview"), preview.ToArray());
            }
            File.WriteAllBytes(Path.Combine(part, "meta.json"), Meta(media));
            Directory.Move(part, final);
            return handle;
        }
        catch
        {
            Release([handle]);
            TryDelete(part);
            throw;
        }
    }

    /// <summary>The rows naming these handles are in the outbox now: the sweep may judge them.</summary>
    public void Release(IEnumerable<string> handles)
    {
        lock (gate)
        {
            foreach (var handle in handles)
            {
                pinned.Remove(handle);
            }
        }
    }

    public StagedMedia? Read(string handle)
    {
        if (!IsHandle(handle))
        {
            return null;
        }
        var path = Path.Combine(folder, handle);
        try
        {
            if (!Directory.Exists(path))
            {
                return null;
            }
            var bytes = File.ReadAllBytes(Path.Combine(path, "bytes"));
            var previewPath = Path.Combine(path, "preview");
            // Spelled out, never `cond ? bytes : null`: the null LITERAL converts to ReadOnlyMemory
            // itself (through byte[]), so that conditional is an EMPTY memory that is not null — and
            // a preview-less file would read back as having one.
            ReadOnlyMemory<byte>? preview = null;
            if (File.Exists(previewPath))
            {
                preview = File.ReadAllBytes(previewPath);
            }
            using var meta = JsonDocument.Parse(File.ReadAllBytes(Path.Combine(path, "meta.json")));
            var root = meta.RootElement;
            return new StagedMedia(
                Text(root, "kind") ?? "file",
                Text(root, "mime") ?? "application/octet-stream",
                bytes,
                Number(root, "width"),
                Number(root, "height"),
                Number(root, "duration_ms"),
                Text(root, "name"),
                preview,
                Real(root, "latitude"),
                Real(root, "longitude"),
                Real(root, "accuracy_m"));
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or JsonException)
        {
            // Half a folder is no folder: the row is told its media is missing, which is the truth.
            return null;
        }
    }

    public void Sweep(IReadOnlySet<string> keep)
    {
        if (!Directory.Exists(folder))
        {
            return;
        }
        lock (gate)
        {
            foreach (var entry in Directory.EnumerateFileSystemEntries(folder))
            {
                var name = Path.GetFileName(entry);
                var handle = name.EndsWith(Part, StringComparison.Ordinal) ? name[..^Part.Length] : name;
                if (keep.Contains(handle) || pinned.Contains(handle))
                {
                    continue;
                }
                TryDelete(entry);
            }
        }
    }

    private static bool IsHandle(string handle) =>
        handle.Length == 32 && handle.All(c => char.IsAsciiHexDigitLower(c) || char.IsAsciiDigit(c));

    private static byte[] Meta(StagedMedia media)
    {
        using var buffer = new MemoryStream();
        using (var writer = new Utf8JsonWriter(buffer))
        {
            writer.WriteStartObject();
            writer.WriteString("kind", media.Kind);
            writer.WriteString("mime", media.Mime);
            if (media.Width is { } width)
            {
                writer.WriteNumber("width", width);
            }
            if (media.Height is { } height)
            {
                writer.WriteNumber("height", height);
            }
            if (media.DurationMs is { } duration)
            {
                writer.WriteNumber("duration_ms", duration);
            }
            if (media.Name is { } name)
            {
                writer.WriteString("name", name);
            }
            // Written as the doubles they are: a place read back a centimetre off is a different place.
            if (media.Latitude is { } latitude)
            {
                writer.WriteNumber("latitude", latitude);
            }
            if (media.Longitude is { } longitude)
            {
                writer.WriteNumber("longitude", longitude);
            }
            if (media.AccuracyM is { } accuracy && double.IsFinite(accuracy))
            {
                writer.WriteNumber("accuracy_m", accuracy);
            }
            writer.WriteEndObject();
        }
        return buffer.ToArray();
    }

    private static string? Text(JsonElement root, string name) =>
        root.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.String ? value.GetString() : null;

    private static int? Number(JsonElement root, string name) =>
        root.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.Number && value.TryGetInt32(out var number)
            ? number
            : null;

    private static double? Real(JsonElement root, string name) =>
        root.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.Number && value.TryGetDouble(out var number)
            ? number
            : null;

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
            // Open elsewhere for a moment: the next sweep comes back to it.
        }
    }
}
