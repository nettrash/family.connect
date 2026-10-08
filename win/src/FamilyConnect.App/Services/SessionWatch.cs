// What a desktop has instead of "the app went to the background" (docs/audio-video-messages-2026-10-04.md, S4): the
// session locking, the screen saver, the computer going to sleep. A hands-free recording must never run on behind any
// of them — it stops, and waits as a voice message that was not sent.
using System.Runtime.InteropServices;

namespace FamilyConnect.App.Services;

/// <summary>
/// Hears the desktop session lock or go away, and the computer about to sleep, and says so through <see cref="Away"/>;
/// <see cref="ScreenSaverRunning"/> is asked while something records.
/// </summary>
/// <remarks>
/// <para>
/// <b>WHICH API A PACKAGED WINUI 3 APP GETS FOR THIS IS UNCONFIRMED</b> (the plan's Blocked 1). This is the most likely
/// one: a full-trust desktop app — which a packaged WinUI 3 app is (<c>runFullTrust</c>) — registers a window with
/// <c>WTSRegisterSessionNotification</c> and hears <c>WM_WTSSESSION_CHANGE</c>; every top-level window hears
/// <c>WM_POWERBROADCAST</c>; and <c>SPI_GETSCREENSAVERRUNNING</c> says whether a screen saver runs. None of it has run:
/// trial T7 settles it, and what fails to start is written down rather than thrown.
/// </para>
/// <para>
/// <b>ITS OWN HIDDEN WINDOW, NOT THE APP'S</b>, for <see cref="TrayIcon"/>'s reason: WinUI subclasses the window it draws
/// in, and two owners of one window procedure break each other. Top-level, never shown, created on the window's thread
/// so its messages arrive through the same loop as WinUI's — which is the thread <see cref="Away"/> is raised on. Its
/// class name is its own: a class registered twice keeps the FIRST registration's procedure, and that delegate may be
/// long collected.
/// </para>
/// </remarks>
internal sealed class SessionWatch : IDisposable
{
    private const uint WmWtsSessionChange = 0x02B1;
    private const uint WmPowerBroadcast = 0x0218;
    private const int WtsConsoleDisconnect = 0x2;
    private const int WtsRemoteDisconnect = 0x4;
    private const int WtsSessionLock = 0x7;
    private const int PbtApmSuspend = 0x4;
    private const int NotifyForThisSession = 0;
    private const uint SpiGetScreenSaverRunning = 0x0072;

    private readonly WindowProc proc;
    private readonly string className = "FamilyConnect.SessionWatch." + Guid.NewGuid().ToString("N");
    private readonly IntPtr instance;
    private readonly IntPtr window;
    private readonly bool registered;

    public SessionWatch()
    {
        // Held in a field: the native side calls it for as long as the window lives, and a collected delegate is a crash.
        proc = OnMessage;
        instance = GetModuleHandleW(null);
        var type = new WindowClass
        {
            Size = Marshal.SizeOf<WindowClass>(),
            Procedure = Marshal.GetFunctionPointerForDelegate(proc),
            Instance = instance,
            Name = className,
        };
        if (RegisterClassExW(ref type) == 0)
        {
            throw new InvalidOperationException($"the session window's class could not be registered ({Marshal.GetLastWin32Error()})");
        }
        window = CreateWindowExW(0, className, "Family Connect", 0, 0, 0, 0, 0, IntPtr.Zero, IntPtr.Zero, instance, IntPtr.Zero);
        if (window == IntPtr.Zero)
        {
            var error = Marshal.GetLastWin32Error();
            UnregisterClassW(className, instance);
            throw new InvalidOperationException($"the session window could not be created ({error})");
        }
        registered = WTSRegisterSessionNotification(window, NotifyForThisSession);
        if (!registered)
        {
            // Sleep is still heard, and the screen saver still asked about; only the lock is not.
            Diagnostics.Write($"session notifications could not be registered ({Marshal.GetLastWin32Error()})");
        }
    }

    /// <summary>
    /// The session locked, or was disconnected — a switch to another user, a remote session dropped — or the computer is
    /// about to sleep. Raised on the window's thread.
    /// </summary>
    public event Action? Away;

    /// <summary>Whether a screen saver is running now. False when Windows cannot be asked.</summary>
    public static bool ScreenSaverRunning()
    {
        try
        {
            return SystemParametersInfoW(SpiGetScreenSaverRunning, 0, out var running, 0) && running != 0;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"asking about the screen saver: {e.GetType().Name}");
            return false;
        }
    }

    private IntPtr OnMessage(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam)
    {
        try
        {
            var reason = (int)(long)wParam;
            var away = message switch
            {
                WmWtsSessionChange => reason is WtsSessionLock or WtsConsoleDisconnect or WtsRemoteDisconnect,
                WmPowerBroadcast => reason == PbtApmSuspend,
                _ => false,
            };
            if (away)
            {
                Away?.Invoke();
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"the session window: {e.GetType().Name}");
        }
        // WM_POWERBROADCAST is answered TRUE by DefWindowProc: nothing here stands in the way of sleep.
        return DefWindowProcW(hwnd, message, wParam, lParam);
    }

    public void Dispose()
    {
        Away = null;
        if (registered)
        {
            WTSUnRegisterSessionNotification(window);
        }
        DestroyWindow(window);
        UnregisterClassW(className, instance);
    }

    private delegate IntPtr WindowProc(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct WindowClass
    {
        public int Size;
        public uint Style;
        public IntPtr Procedure;
        public int ClassExtra;
        public int WindowExtra;
        public IntPtr Instance;
        public IntPtr Icon;
        public IntPtr Cursor;
        public IntPtr Background;
        public string? MenuName;
        public string Name;
        public IntPtr SmallIcon;
    }

    [DllImport("wtsapi32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool WTSRegisterSessionNotification(IntPtr window, int flags);

    [DllImport("wtsapi32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool WTSUnRegisterSessionNotification(IntPtr window);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SystemParametersInfoW(uint action, uint parameter, out int value, uint update);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern ushort RegisterClassExW(ref WindowClass type);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool UnregisterClassW(string className, IntPtr instance);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateWindowExW(
        uint exStyle, string className, string windowName, uint style, int x, int y, int width, int height,
        IntPtr parent, IntPtr menu, IntPtr instance, IntPtr param);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DestroyWindow(IntPtr window);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr DefWindowProcW(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr GetModuleHandleW(string? name);
}
