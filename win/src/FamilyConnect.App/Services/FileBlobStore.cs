using FamilyConnect.App.Logic;

namespace FamilyConnect.App.Services;

/// <summary>
/// Downloaded attachment bytes on disk, one file per key — the <see cref="IBlobStore"/> the attachment
/// cache keeps what it fetched in.
/// </summary>
/// <remarks>
/// <para>
/// <b>A WRITE LANDS WHOLE OR NOT AT ALL.</b> Bytes go to a uniquely named part file first and are moved
/// into place, so a crash or two downloads of the same picture at once can never leave a truncated
/// file that reads as a picture.
/// </para>
/// <para>
/// <b>ONE SERVER'S FILES.</b> Attachment ids are a server's own, so the folder is wiped when the app is
/// pointed at another server (<see cref="AppServices.UseAsync"/>), exactly as the SQLite cache is.
/// </para>
/// </remarks>
internal sealed class FileBlobStore(string folder) : IBlobStore
{
    public byte[]? Read(string key)
    {
        var path = PathFor(key);
        try
        {
            return File.Exists(path) ? File.ReadAllBytes(path) : null;
        }
        catch (IOException)
        {
            return null;
        }
    }

    public void Write(string key, ReadOnlyMemory<byte> bytes)
    {
        Directory.CreateDirectory(folder);
        var path = PathFor(key);
        var part = $"{path}.{Guid.NewGuid():N}.part";
        File.WriteAllBytes(part, bytes.ToArray());
        File.Move(part, path, overwrite: true);
    }

    public static void Wipe(string folder)
    {
        try
        {
            if (Directory.Exists(folder))
            {
                Directory.Delete(folder, recursive: true);
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"wiping attachment files: {e.GetType().Name}");
        }
    }

    // A key is "34" or "34.preview"; anything else is flattened rather than trusted as a path.
    private string PathFor(string key) =>
        Path.Combine(folder, string.Concat(key.Select(c => char.IsAsciiLetterOrDigit(c) || c is '.' or '-' ? c : '_')));
}
