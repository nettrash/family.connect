// Ctrl+Alt+Shift+F from any app: the Mac's ⌃⌥⌘F (#80), so the window a closed app left in the notification area is one
// chord away as well as one click.
using System.Runtime.InteropServices;
using FamilyConnect.App.Logic;

namespace FamilyConnect.App.Services;

/// <summary>
/// The system-wide shortcut (<see cref="GlobalHotKeyRules"/>), registered with <c>RegisterHotKey</c> on a hidden
/// message-only window of its own — for the reason <see cref="TrayIcon"/> keeps one: WinUI owns the procedure of the window
/// it draws in. Created on the window's thread, so <c>WM_HOTKEY</c> arrives through WinUI's own message loop and
/// <c>pressed</c> runs where it may touch the window.
/// </summary>
/// <remarks>
/// <c>RegisterHotKey</c> needs no permission and sees nothing but its own chord. It fails, honestly, when another app holds
/// the combination (<see cref="GlobalHotKeyRules.TakenError"/>), and Settings says so.
/// </remarks>
internal sealed class GlobalHotKey : IDisposable
{
    private const string ClassName = "FamilyConnect.GlobalHotKey";
    private const uint WmHotKey = 0x0312;
    private const int HotKeyId = 0x4643;
    private static readonly IntPtr MessageOnly = new(-3);

    private readonly WindowProc proc;
    private readonly IntPtr window;
    private readonly Action pressed;
    private bool registered;

    /// <summary>The one shortcut of this process, for Settings to switch and to ask about.</summary>
    public static GlobalHotKey? Current { get; private set; }

    public GlobalHotKey(Action pressed)
    {
        this.pressed = pressed;
        // Held in a field: the native side calls it for as long as the window lives, and a collected delegate is a crash.
        proc = OnMessage;
        var instance = GetModuleHandleW(null);
        var type = new WindowClass
        {
            Size = Marshal.SizeOf<WindowClass>(),
            Procedure = Marshal.GetFunctionPointerForDelegate(proc),
            Instance = instance,
            Name = ClassName,
        };
        RegisterClassExW(ref type);
        window = CreateWindowExW(0, ClassName, "Family Connect", 0, 0, 0, 0, 0, MessageOnly, IntPtr.Zero, instance, IntPtr.Zero);
        if (window == IntPtr.Zero)
        {
            throw new InvalidOperationException($"the shortcut's window could not be created ({Marshal.GetLastWin32Error()})");
        }
        Current = this;
    }

    /// <summary>Whether the last attempt to register it failed because another app holds the combination.</summary>
    public bool IsTaken { get; private set; }

    /// <summary>Registered or not, as the setting says.</summary>
    public void Apply(bool on)
    {
        if (on && !registered)
        {
            registered = RegisterHotKey(window, HotKeyId, GlobalHotKeyRules.Modifiers, GlobalHotKeyRules.Key);
            var error = registered ? 0 : Marshal.GetLastWin32Error();
            IsTaken = GlobalHotKeyRules.IsTaken(error);
            if (!registered)
            {
                Diagnostics.Write($"the global shortcut could not be registered ({error})");
            }
        }
        else if (!on && registered)
        {
            UnregisterHotKey(window, HotKeyId);
            registered = false;
            IsTaken = false;
        }
        else if (!on)
        {
            IsTaken = false;
        }
    }

    private IntPtr OnMessage(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam)
    {
        try
        {
            if (message == WmHotKey && (int)wParam == HotKeyId)
            {
                pressed();
                return IntPtr.Zero;
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"the global shortcut: {e.GetType().Name}");
        }
        return DefWindowProcW(hwnd, message, wParam, lParam);
    }

    public void Dispose()
    {
        if (registered)
        {
            UnregisterHotKey(window, HotKeyId);
            registered = false;
        }
        if (window != IntPtr.Zero)
        {
            DestroyWindow(window);
        }
        if (ReferenceEquals(Current, this))
        {
            Current = null;
        }
    }

    /// <summary>Whether <paramref name="window"/> is the one in front of every app — what a second press hides.</summary>
    public static bool IsForeground(IntPtr window) => window != IntPtr.Zero && GetForegroundWindow() == window;

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

    [DllImport("user32.dll")]
    private static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool RegisterHotKey(IntPtr window, int id, uint modifiers, uint key);

    [DllImport("user32.dll")]
    private static extern bool UnregisterHotKey(IntPtr window, int id);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern ushort RegisterClassExW(ref WindowClass type);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateWindowExW(
        uint exStyle, string className, string windowName, uint style, int x, int y, int width, int height,
        IntPtr parent, IntPtr menu, IntPtr instance, IntPtr param);

    [DllImport("user32.dll")]
    private static extern bool DestroyWindow(IntPtr window);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr DefWindowProcW(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr GetModuleHandleW(string? name);
}
