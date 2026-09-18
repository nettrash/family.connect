// Files shared INTO the app from anywhere in Windows: copied into the app's own folder by the process Windows launched for
// the share, and taken out by the window that stages them.
using Microsoft.Windows.AppLifecycle;
using Windows.ApplicationModel.Activation;
using Windows.ApplicationModel.DataTransfer;
using Windows.ApplicationModel.DataTransfer.ShareTarget;
using Windows.Storage;

namespace FamilyConnect.App.Services;

/// <summary>What has been shared into the app and not yet put into a chat.</summary>
/// <remarks>
/// <para>
/// <b>COPIED BEFORE ANYTHING ELSE HAPPENS.</b> A share usually launches a SECOND process, which hands its activation to the
/// running window and exits; what Windows shared belongs to that process and to a share operation that ends with it. So
/// the bytes are copied into <see cref="Root"/> first — and only a folder whose copy FINISHED is ever taken, which is what
/// the marker is for — and the window works from those files, never from the share.
/// </para>
/// <para>
/// <b>NOTHING IS SENT.</b> The files land staged in the chat the reader chooses, and they press Send there: sharing into a
/// family chat is choosing to say something, not having said it (ios ShareImport).
/// </para>
/// </remarks>
internal static class ShareInbox
{
    /// <summary>The most one share brings in — the Mac's cap, and what one message's composer holds.</summary>
    private const int Cap = 10;

    private const string Ready = ".ready";

    private static string Root => Path.Combine(AppFolders.Root, "incoming");

    /// <summary>
    /// If this process was launched for a share, copy what was shared in. Runs before any window exists, on the thread
    /// Main runs on — so the work goes to the thread pool and this waits for it, bounded.
    /// </summary>
    public static void Receive(AppActivationArguments activation)
    {
        if (activation.Kind != ExtendedActivationKind.ShareTarget || activation.Data is not IShareTargetActivatedEventArgs shared)
        {
            return;
        }
        var operation = shared.ShareOperation;
        using var done = new ManualResetEventSlim();
        Task.Run(async () =>
        {
            try
            {
                await CopyAsync(operation).ConfigureAwait(false);
            }
            catch (Exception e)
            {
                Diagnostics.Write($"receiving a share: {e.GetType().Name} 0x{e.HResult:X8}");
                try
                {
                    operation.ReportError("Family Connect could not take these files.");
                }
                catch (Exception)
                {
                    // The share is over either way.
                }
            }
            finally
            {
                done.Set();
            }
        });
        done.Wait(TimeSpan.FromSeconds(60));
    }

    private static async Task CopyAsync(ShareOperation operation)
    {
        operation.ReportStarted();
        var folder = Path.Combine(Root, Guid.NewGuid().ToString("N"));
        var written = 0;
        if (operation.Data.Contains(StandardDataFormats.StorageItems))
        {
            foreach (var item in await operation.Data.GetStorageItemsAsync())
            {
                if (written >= Cap)
                {
                    break;
                }
                if (item is not StorageFile file)
                {
                    // A folder is not an attachment.
                    continue;
                }
                Directory.CreateDirectory(folder);
                await using var source = await file.OpenStreamForReadAsync().ConfigureAwait(false);
                await using var target = File.Create(Unique(folder, file.Name));
                await source.CopyToAsync(target).ConfigureAwait(false);
                written++;
            }
        }
        else if (operation.Data.Contains(StandardDataFormats.Bitmap))
        {
            // A picture shared as a picture (a screenshot tool, a browser's image) rather than as a file.
            var reference = await operation.Data.GetBitmapAsync();
            using var stream = await reference.OpenReadAsync();
            var extension = stream.ContentType switch
            {
                "image/jpeg" => ".jpg",
                "image/gif" => ".gif",
                "image/bmp" => ".bmp",
                _ => ".png",
            };
            Directory.CreateDirectory(folder);
            await using var source = stream.AsStreamForRead();
            await using var target = File.Create(Path.Combine(folder, "picture" + extension));
            await source.CopyToAsync(target).ConfigureAwait(false);
            written++;
        }
        if (written > 0)
        {
            // Written LAST: a folder without it is a copy that never finished, and is never taken.
            File.WriteAllText(Path.Combine(folder, Ready), string.Empty);
        }
        operation.ReportCompleted();
    }

    /// <summary>The oldest finished share, if any: its folder, and the files in it.</summary>
    public static (string Folder, IReadOnlyList<string> Files)? Peek()
    {
        try
        {
            if (!Directory.Exists(Root))
            {
                return null;
            }
            foreach (var folder in Directory.GetDirectories(Root).OrderBy(Directory.GetCreationTimeUtc))
            {
                if (!File.Exists(Path.Combine(folder, Ready)))
                {
                    continue;
                }
                var files = Directory.GetFiles(folder)
                    .Where(path => Path.GetFileName(path) != Ready)
                    .OrderBy(path => path, StringComparer.Ordinal)
                    .ToList();
                if (files.Count > 0)
                {
                    return (folder, files);
                }
                Discard(folder);
            }
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            Diagnostics.Write($"reading the share inbox: {e.GetType().Name}");
        }
        return null;
    }

    /// <summary>A share that was staged, or that the reader sent nowhere: its copies go.</summary>
    public static void Discard(string folder)
    {
        try
        {
            Directory.Delete(folder, recursive: true);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            Diagnostics.Write($"clearing a share: {e.GetType().Name}");
        }
    }

    /// <summary>The file's own name, made unique in the folder — two shared files may both be called image.png.</summary>
    private static string Unique(string folder, string name)
    {
        var safe = string.Concat(Path.GetFileName(name).Select(c => Path.GetInvalidFileNameChars().Contains(c) ? '_' : c));
        if (safe.Length == 0)
        {
            safe = "file";
        }
        var path = Path.Combine(folder, safe);
        for (var n = 2; File.Exists(path); n++)
        {
            path = Path.Combine(folder, $"{Path.GetFileNameWithoutExtension(safe)} ({n}){Path.GetExtension(safe)}");
        }
        return path;
    }
}
