// The window's own icon — the title bar and Alt+Tab.
//
// <ApplicationIcon> puts AppIcon.ico in the executable, and the shell shows it for the file, Start and
// the taskbar. It does NOT reach the title bar: a WinUI 3 Window creates its HWND from a class with no
// icon and never consults the executable's (md.win, 2026-09-07). AppWindow.SetIcon is the only fix.
using Microsoft.UI.Windowing;

namespace FamilyConnect.App;

internal static class WindowIcon
{
    /// <summary>Beside the executable, packaged and unpackaged alike (the csproj copies it there).</summary>
    private static readonly string IconPath =
        Path.Combine(AppContext.BaseDirectory, "Assets", "AppIcon.ico");

    public static void Apply(AppWindow window)
    {
        try
        {
            if (!File.Exists(IconPath))
            {
                // Cosmetic, never fatal — and exactly the packaging slip a green build hides.
                Diagnostics.Write($"window icon: {IconPath} is not there");
                return;
            }
            window.SetIcon(IconPath);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"window icon could not be set: {e.Message}");
        }
    }
}
