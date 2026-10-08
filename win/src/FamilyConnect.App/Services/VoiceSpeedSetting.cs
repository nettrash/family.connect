using FamilyConnect.App.Logic;

namespace FamilyConnect.App.Services;

/// <summary>
/// The voice-message speed chosen on THIS device (1×, 1.5× or 2×; <see cref="VoiceSpeed"/>), kept beside the app's other
/// per-device choices and applied to every voice message played here until it is changed. An unreadable file is 1×, never
/// an error.
/// </summary>
internal static class VoiceSpeedSetting
{
    private static string FilePath => Path.Combine(AppFolders.Root, "voice-speed.txt");

    public static double Rate
    {
        get
        {
            try
            {
                return VoiceSpeed.Parse(File.Exists(FilePath) ? File.ReadAllText(FilePath) : null);
            }
            catch (Exception e)
            {
                Diagnostics.Write($"the voice message speed could not be read: {e.GetType().Name}");
                return 1.0;
            }
        }
        set
        {
            try
            {
                Directory.CreateDirectory(AppFolders.Root);
                File.WriteAllText(FilePath, VoiceSpeed.Store(value));
            }
            catch (Exception e)
            {
                Diagnostics.Write($"the voice message speed could not be written: {e.GetType().Name}");
            }
        }
    }
}
