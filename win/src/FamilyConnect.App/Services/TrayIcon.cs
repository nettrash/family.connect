// The app's icon in the notification area: what a closed window leaves behind while the app goes on listening, as a Mac
// app stays in the Dock with its window closed.
using System.Runtime.InteropServices;

namespace FamilyConnect.App.Services;

/// <summary>
/// The notification area icon: a click brings the window back, and its menu opens it or quits the app. Its tooltip is the
/// window's title, so the unread count is there too.
/// </summary>
/// <remarks>
/// <para>
/// <b>ITS OWN HIDDEN WINDOW, NOT THE APP'S.</b> WinUI subclasses the window it draws in, and chaining a second window
/// procedure onto that one is how two owners of one HWND break each other. The icon talks to a plain top-level window of
/// its own that is never shown — top-level rather than message-only, because only a top-level window hears
/// <c>TaskbarCreated</c>, which is how the icon comes back after Explorer restarts.
/// </para>
/// <para>
/// <b>ON THE WINDOW'S THREAD.</b> Created there, so its messages arrive through the same loop that runs WinUI's, and
/// <c>open</c> and <c>quit</c> are called where they may touch the window.
/// </para>
/// </remarks>
internal sealed class TrayIcon : IDisposable
{
    private const string ClassName = "FamilyConnect.NotificationAreaIcon";
    private const uint CallbackMessage = 0x8000 + 0x46;
    private const uint IconId = 1;

    private const uint NimAdd = 0;
    private const uint NimModify = 1;
    private const uint NimDelete = 2;
    private const uint NimSetVersion = 4;
    private const uint NifMessage = 0x1;
    private const uint NifIcon = 0x2;
    private const uint NifTip = 0x4;
    private const uint NifShowTip = 0x80;
    private const uint NotifyIconVersion4 = 4;
    private const int NinSelect = 0x400;
    private const int NinKeySelect = 0x401;
    private const int WmContextMenu = 0x7B;
    private const uint MfString = 0x0;
    private const uint MfSeparator = 0x800;
    private const uint TpmReturnCommand = 0x100;
    private const uint TpmRightButton = 0x2;
    private const uint TpmBottomAlign = 0x20;
    private const int OpenCommand = 1;
    private const int QuitCommand = 2;

    private static readonly string IconPath = Path.Combine(AppContext.BaseDirectory, "Assets", "AppIcon.ico");

    private readonly WindowProc proc;
    private readonly IntPtr window;
    private readonly IntPtr icon;
    private readonly uint taskbarCreated;
    private readonly string openWords;
    private readonly string quitWords;
    private readonly Action open;
    private readonly Action quit;
    private string tip;
    private bool added;

    public TrayIcon(string tip, string openWords, string quitWords, Action open, Action quit)
    {
        this.tip = Clip(tip);
        this.openWords = openWords;
        this.quitWords = quitWords;
        this.open = open;
        this.quit = quit;
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
        // A class left registered by an earlier icon in this process is the same class: reuse it.
        RegisterClassExW(ref type);
        window = CreateWindowExW(0, ClassName, "Family Connect", 0, 0, 0, 0, 0, IntPtr.Zero, IntPtr.Zero, instance, IntPtr.Zero);
        if (window == IntPtr.Zero)
        {
            throw new InvalidOperationException($"the notification area window could not be created ({Marshal.GetLastWin32Error()})");
        }
        taskbarCreated = RegisterWindowMessageW("TaskbarCreated");
        icon = LoadImageW(IntPtr.Zero, IconPath, 1, GetSystemMetrics(49), GetSystemMetrics(50), 0x10);
        Add();
    }

    /// <summary>The words shown on hovering the icon: the window's title, unread count and all.</summary>
    public string Tip
    {
        set
        {
            var clipped = Clip(value);
            if (clipped == tip)
            {
                return;
            }
            tip = clipped;
            if (added)
            {
                var data = Data(NifTip | NifShowTip);
                Shell_NotifyIconW(NimModify, ref data);
            }
        }
    }

    private void Add()
    {
        var data = Data(NifMessage | NifIcon | NifTip | NifShowTip);
        added = Shell_NotifyIconW(NimAdd, ref data);
        if (!added)
        {
            // Cosmetic, never fatal: without the icon a closed window is simply the app closing (MainWindow asks first).
            Diagnostics.Write("the notification area icon could not be added");
            return;
        }
        data.TimeoutOrVersion = NotifyIconVersion4;
        Shell_NotifyIconW(NimSetVersion, ref data);
    }

    /// <summary>Whether the icon is actually in the notification area, which is what makes hiding the window safe.</summary>
    public bool Shown => added;

    private IntPtr OnMessage(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam)
    {
        try
        {
            if (message == CallbackMessage)
            {
                // Version 4: the event is the low word of lParam, and the menu's anchor is in wParam.
                switch ((int)((long)lParam & 0xFFFF))
                {
                    case NinSelect:
                    case NinKeySelect:
                        open();
                        break;
                    case WmContextMenu:
                        ShowMenu((short)((long)wParam & 0xFFFF), (short)(((long)wParam >> 16) & 0xFFFF));
                        break;
                }
                return IntPtr.Zero;
            }
            if (message == taskbarCreated && taskbarCreated != 0)
            {
                // Explorer restarted and forgot every icon: this one goes back.
                Add();
                return IntPtr.Zero;
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"the notification area icon: {e.GetType().Name}");
        }
        return DefWindowProcW(hwnd, message, wParam, lParam);
    }

    private void ShowMenu(int x, int y)
    {
        var menu = CreatePopupMenu();
        try
        {
            AppendMenuW(menu, MfString, OpenCommand, openWords);
            AppendMenuW(menu, MfSeparator, 0, null);
            AppendMenuW(menu, MfString, QuitCommand, quitWords);
            // Without the foreground a notification area menu never closes when the reader clicks elsewhere.
            SetForegroundWindow(window);
            var chosen = TrackPopupMenuEx(menu, TpmReturnCommand | TpmRightButton | TpmBottomAlign, x, y, window, IntPtr.Zero);
            PostMessageW(window, 0, IntPtr.Zero, IntPtr.Zero);
            switch (chosen)
            {
                case OpenCommand:
                    open();
                    break;
                case QuitCommand:
                    quit();
                    break;
            }
        }
        finally
        {
            DestroyMenu(menu);
        }
    }

    private NotifyIconData Data(uint flags) => new()
    {
        Size = Marshal.SizeOf<NotifyIconData>(),
        Window = window,
        Id = IconId,
        Flags = flags,
        CallbackMessage = CallbackMessage,
        Icon = icon,
        Tip = tip,
        Info = string.Empty,
        InfoTitle = string.Empty,
    };

    private static string Clip(string words) => words.Length > 127 ? words[..127] : words;

    public void Dispose()
    {
        if (added)
        {
            var data = Data(0);
            Shell_NotifyIconW(NimDelete, ref data);
            added = false;
        }
        if (window != IntPtr.Zero)
        {
            DestroyWindow(window);
        }
        if (icon != IntPtr.Zero)
        {
            DestroyIcon(icon);
        }
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

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct NotifyIconData
    {
        public int Size;
        public IntPtr Window;
        public uint Id;
        public uint Flags;
        public uint CallbackMessage;
        public IntPtr Icon;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)]
        public string Tip;
        public uint State;
        public uint StateMask;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)]
        public string Info;
        public uint TimeoutOrVersion;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 64)]
        public string InfoTitle;
        public uint InfoFlags;
        public Guid Item;
        public IntPtr BalloonIcon;
    }

    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    private static extern bool Shell_NotifyIconW(uint message, ref NotifyIconData data);

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

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern uint RegisterWindowMessageW(string name);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr GetModuleHandleW(string? name);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr LoadImageW(IntPtr instance, string name, uint type, int width, int height, uint load);

    [DllImport("user32.dll")]
    private static extern bool DestroyIcon(IntPtr icon);

    [DllImport("user32.dll")]
    private static extern int GetSystemMetrics(int index);

    [DllImport("user32.dll")]
    private static extern IntPtr CreatePopupMenu();

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern bool AppendMenuW(IntPtr menu, uint flags, nuint id, string? words);

    [DllImport("user32.dll")]
    private static extern int TrackPopupMenuEx(IntPtr menu, uint flags, int x, int y, IntPtr window, IntPtr parameters);

    [DllImport("user32.dll")]
    private static extern bool DestroyMenu(IntPtr menu);

    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(IntPtr window);

    [DllImport("user32.dll")]
    private static extern bool PostMessageW(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
}
