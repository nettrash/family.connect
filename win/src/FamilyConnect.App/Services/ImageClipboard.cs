using FamilyConnect.App.Logic;
using Windows.ApplicationModel.DataTransfer;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;

namespace FamilyConnect.App.Services;

internal static class ImageClipboard
{
    /// <summary>Make a self-contained still image at its oriented dimensions for a later paste.</summary>
    public static async Task<IRandomAccessStream?> PrepareAsync(byte[] bytes)
    {
        if (AnimationPrep.Animated(bytes) is not null)
        {
            return null;
        }

        using var input = new InMemoryRandomAccessStream();
        using (var writer = new DataWriter(input))
        {
            writer.WriteBytes(bytes);
            await writer.StoreAsync();
            await writer.FlushAsync();
            writer.DetachStream();
        }
        input.Seek(0);

        var decoder = await BitmapDecoder.CreateAsync(input);
        if (decoder.FrameCount != 1)
        {
            return null;
        }
        using var bitmap = await decoder.GetSoftwareBitmapAsync(
            BitmapPixelFormat.Bgra8, BitmapAlphaMode.Straight, new BitmapTransform(),
            ExifOrientationMode.RespectExifOrientation, ColorManagementMode.ColorManageToSRgb);

        var output = new InMemoryRandomAccessStream();
        try
        {
            var encoder = await BitmapEncoder.CreateAsync(BitmapEncoder.PngEncoderId, output);
            encoder.SetSoftwareBitmap(bitmap);
            await encoder.FlushAsync();
            output.Seek(0);
            return output;
        }
        catch
        {
            output.Dispose();
            throw;
        }
    }

    public static bool Publish(IRandomAccessStream image)
    {
        var package = new DataPackage { RequestedOperation = DataPackageOperation.Copy };
        package.SetBitmap(RandomAccessStreamReference.CreateFromStream(image));
        if (!Clipboard.SetContentWithOptions(package, new ClipboardContentOptions { IsRoamable = false }))
        {
            return false;
        }

        // Keep paste independent of the viewer and the app before disposing of the stream.
        Clipboard.Flush();
        return true;
    }
}
