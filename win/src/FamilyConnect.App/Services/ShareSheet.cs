using System.Runtime.InteropServices;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage;
using WinRT;

namespace FamilyConnect.App.Services;

/// <summary>
/// Windows' own share window for one file — the desktop form of it, which has to be told which window it belongs to, as the
/// save picker does (Windows App SDK, "Display WinRT UI objects that depend on CoreWindow").
/// </summary>
internal static class ShareSheet
{
    [ComImport]
    [Guid("3A3DCD6C-3EAB-43DC-BCDE-45671CE800C8")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IDataTransferManagerInterop
    {
        IntPtr GetForWindow([In] IntPtr appWindow, [In] ref Guid riid);

        void ShowShareUIForWindow(IntPtr appWindow);
    }

    /// <summary><c>Windows.ApplicationModel.DataTransfer.IDataTransferManager</c>.</summary>
    private static readonly Guid DataTransferManagerIid = new(0xa5caee9b, 0x8708, 0x49d1, 0x8d, 0x36, 0x67, 0xd2, 0x5a, 0x8d, 0xa0, 0x0c);

    /// <summary>Write the bytes where the share can reach them, and offer that file.</summary>
    public static async Task ShareFileAsync(nint window, string name, byte[] bytes)
    {
        var folder = await ApplicationData.Current.TemporaryFolder.CreateFolderAsync("Share", CreationCollisionOption.OpenIfExists);
        var file = await folder.CreateFileAsync(AttachmentSaving.Safe(name), CreationCollisionOption.ReplaceExisting);
        await FileIO.WriteBytesAsync(file, bytes);

        var interop = DataTransferManager.As<IDataTransferManagerInterop>();
        var iid = DataTransferManagerIid;
        var manager = MarshalInterface<DataTransferManager>.FromAbi(interop.GetForWindow(window, ref iid));
        void OnRequested(DataTransferManager sender, DataRequestedEventArgs args)
        {
            // Once: the manager belongs to the window, and a second share must not offer this file again.
            sender.DataRequested -= OnRequested;
            args.Request.Data.Properties.Title = name;
            args.Request.Data.SetStorageItems(new IStorageItem[] { file });
        }
        manager.DataRequested += OnRequested;
        interop.ShowShareUIForWindow(window);
    }
}
