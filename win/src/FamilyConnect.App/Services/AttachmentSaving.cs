using FamilyConnect.App.Logic;
using Windows.Storage;
using Windows.Storage.Pickers;
using Windows.System;

namespace FamilyConnect.App.Services;

/// <summary>Handing an attachment's bytes to the reader: saved where they choose, or opened by Windows.</summary>
internal static class AttachmentSaving
{
    /// <summary>
    /// Ask where, and write it there. The WinRT save picker, as md.win uses it: a desktop app must name
    /// the window that owns the picker or it throws. Answers whether anything was saved.
    /// </summary>
    public static async Task<bool> SaveAsync(nint window, string suggestedName, byte[] bytes)
    {
        var safe = AttachmentFiles.SafeFileName(suggestedName);
        var extension = Path.GetExtension(safe);
        if (string.IsNullOrEmpty(extension))
        {
            extension = ".bin";
        }
        var picker = new FileSavePicker
        {
            SuggestedStartLocation = PickerLocationId.Downloads,
            SuggestedFileName = Path.GetFileNameWithoutExtension(safe),
            DefaultFileExtension = extension,
        };
        // The choice's label is the extension itself, which needs no translating.
        picker.FileTypeChoices.Add(extension.TrimStart('.').ToUpperInvariant(), [extension]);
        WinRT.Interop.InitializeWithWindow.Initialize(picker, window);
        var file = await picker.PickSaveFileAsync();
        if (file is null)
        {
            return false;
        }
        await FileIO.WriteBytesAsync(file, bytes);
        return true;
    }

    /// <summary>Open it with whatever Windows opens such a file with — a video, a recording.</summary>
    public static async Task OpenAsync(string name, byte[] bytes)
    {
        var folder = Path.Combine(Path.GetTempPath(), "FamilyConnect");
        Directory.CreateDirectory(folder);
        var path = Path.Combine(folder, AttachmentFiles.SafeFileName(name));
        await File.WriteAllBytesAsync(path, bytes);
        var file = await StorageFile.GetFileFromPathAsync(path);
        await Launcher.LaunchFileAsync(file);
    }

}
