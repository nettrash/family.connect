namespace FamilyConnect.App.Services;

/// <summary>
/// What the video-message recorder remembers on THIS device (docs/audio-video-messages-2026-10-04.md, S3.4, S3.5): the camera
/// chosen from "Choose camera", and whether PREVIEW has said "Only you can see this until you start recording." here yet.
/// Kept beside the app's other per-device choices; an unreadable file is the default, never an error.
/// </summary>
internal static class RoundVideoSetting
{
    private static string CameraPath => Path.Combine(AppFolders.Root, "round-camera.txt");

    private static string SeenPath => Path.Combine(AppFolders.Root, "round-preview-seen.txt");

    /// <summary>The camera chosen before, or null for the front-panel one (S3.5: "The choice is remembered on the device").</summary>
    public static string? Camera
    {
        get => Read(CameraPath) is { Length: > 0 } id ? id : null;
        set => Write(CameraPath, value ?? string.Empty);
    }

    /// <summary>Whether PREVIEW's first-time line has been shown on this device (S7.5).</summary>
    public static bool PreviewSeen
    {
        get => Read(SeenPath) == "seen";
        set => Write(SeenPath, value ? "seen" : string.Empty);
    }

    private static string? Read(string path)
    {
        try
        {
            return File.Exists(path) ? File.ReadAllText(path).Trim() : null;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"a video message setting could not be read: {e.GetType().Name}");
            return null;
        }
    }

    private static void Write(string path, string value)
    {
        try
        {
            Directory.CreateDirectory(AppFolders.Root);
            File.WriteAllText(path, value);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"a video message setting could not be written: {e.GetType().Name}");
        }
    }
}
