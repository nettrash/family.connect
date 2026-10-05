// Whether a screen reader is running (docs/audio-video-messages-2026-10-04.md, S6): Narrator, NVDA and JAWS all say so through
// the system's screen-reader flag, and a recording started while one runs waits for "Recording" to be spoken first.
using System.Runtime.InteropServices;

namespace FamilyConnect.App.Services;

/// <summary>Windows' screen-reader flag (<c>SPI_GETSCREENREADER</c>), asked at the moment it matters rather than watched.</summary>
/// <remarks>
/// Asked when a recording starts, never cached: Narrator is switched on and off with a key press (Ctrl+Win+Enter), and an
/// answer kept from before would let the app's own voice into a note. Not run here: trial T5 has Narrator read every state.
/// </remarks>
internal static class ScreenReader
{
    private const uint SpiGetScreenReader = 0x0046;

    /// <summary>Whether a screen reader says it is running now. False when Windows cannot be asked.</summary>
    public static bool Running()
    {
        try
        {
            return SystemParametersInfoW(SpiGetScreenReader, 0, out var running, 0) && running != 0;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"asking about a screen reader: {e.GetType().Name}");
            return false;
        }
    }

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SystemParametersInfoW(uint action, uint parameter, out int value, uint update);
}
