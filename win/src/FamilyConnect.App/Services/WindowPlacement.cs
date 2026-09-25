// Where the window opens: where it was left — or, the first time and whenever that place is on no screen any more, a
// comfortable size in the middle of the screen it opens on.
using System.Globalization;
using System.Runtime.InteropServices;
using Microsoft.UI.Windowing;
using Windows.Graphics;

namespace FamilyConnect.App.Services;

/// <summary>The window's size and place, restored at launch and written down when it closes.</summary>
/// <remarks>
/// <para>
/// <b>APPWINDOW COUNTS PHYSICAL PIXELS.</b> <c>Resize(1100, 760)</c> is 1100 by 760 on a 100% display and half that, in
/// everything a reader sees, at 200% — which is how this window used to open the size of a dialog on a high-DPI laptop.
/// The first size is chosen in effective pixels and scaled by the window's own DPI, and never more than the screen holds.
/// </para>
/// <para>
/// <b>THE RESTORED SIZE IS KEPT, NOT THE MAXIMISED ONE.</b> A window closed maximised opens maximised, and un-maximising it
/// then goes back to the size it had before — not to a restored window as big as the screen.
/// </para>
/// </remarks>
internal sealed class WindowPlacement
{
    /// <summary>The first size, in effective pixels: room for the chat list and a conversation beside it.</summary>
    private const int FirstWidth = 1200;
    private const int FirstHeight = 820;

    private static string Path => System.IO.Path.Combine(AppFolders.Root, "window.txt");

    private readonly AppWindow window;
    private RectInt32 restored;

    private WindowPlacement(AppWindow window) => this.window = window;

    public static WindowPlacement Apply(AppWindow window, IntPtr handle)
    {
        var placement = new WindowPlacement(window);
        var maximized = false;
        try
        {
            if (Read() is { } saved
                && DisplayArea.GetFromRect(saved.Bounds, DisplayAreaFallback.None) is { } area
                && Visible(saved.Bounds, area.WorkArea))
            {
                window.MoveAndResize(saved.Bounds);
                maximized = saved.Maximized;
            }
            else
            {
                window.MoveAndResize(First(window, handle));
            }
        }
        catch (Exception e)
        {
            // A size is never worth a launch.
            Diagnostics.Write($"placing the window: {e.GetType().Name} 0x{e.HResult:X8}");
        }
        placement.restored = new RectInt32(window.Position.X, window.Position.Y, window.Size.Width, window.Size.Height);
        window.Changed += placement.OnChanged;
        if (maximized && window.Presenter is OverlappedPresenter presenter)
        {
            presenter.Maximize();
        }
        return placement;
    }

    /// <summary>Written when the window closes: the restored bounds, and whether it was maximised.</summary>
    public void Save()
    {
        try
        {
            var maximized = window.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Maximized };
            Directory.CreateDirectory(AppFolders.Root);
            File.WriteAllText(Path, string.Create(
                CultureInfo.InvariantCulture,
                $"{restored.X},{restored.Y},{restored.Width},{restored.Height},{(maximized ? 1 : 0)}"));
        }
        catch (Exception e)
        {
            Diagnostics.Write($"window placement could not be written: {e.GetType().Name}");
        }
    }

    private void OnChanged(AppWindow sender, AppWindowChangedEventArgs args)
    {
        // Only a restored window's bounds are its size: maximised and minimised are states, not sizes to come back to.
        if ((args.DidPositionChange || args.DidSizeChange)
            && sender.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Restored })
        {
            restored = new RectInt32(sender.Position.X, sender.Position.Y, sender.Size.Width, sender.Size.Height);
        }
    }

    /// <summary>The first size, in the middle of the work area of the screen the window is on.</summary>
    private static RectInt32 First(AppWindow window, IntPtr handle)
    {
        var work = DisplayArea.GetFromWindowId(window.Id, DisplayAreaFallback.Nearest).WorkArea;
        var dpi = GetDpiForWindow(handle);
        var scale = dpi > 0 ? dpi / 96.0 : 1.0;
        var width = Math.Min((int)Math.Round(FirstWidth * scale), (int)(work.Width * 0.9));
        var height = Math.Min((int)Math.Round(FirstHeight * scale), (int)(work.Height * 0.9));
        return new RectInt32(work.X + ((work.Width - width) / 2), work.Y + ((work.Height - height) / 2), width, height);
    }

    /// <summary>
    /// Whether a saved place can be used: its title bar on the screen, and no bigger than the screen — a monitor unplugged
    /// or a resolution lowered since would otherwise open the window somewhere nobody can reach it.
    /// </summary>
    private static bool Visible(RectInt32 bounds, RectInt32 work) =>
        bounds.Width >= 400 && bounds.Height >= 300
        && bounds.Width <= work.Width + 16 && bounds.Height <= work.Height + 16
        && bounds.X + bounds.Width - 120 >= work.X && bounds.X + 120 <= work.X + work.Width
        && bounds.Y >= work.Y - 8 && bounds.Y + 48 <= work.Y + work.Height;

    private static (RectInt32 Bounds, bool Maximized)? Read()
    {
        try
        {
            if (!File.Exists(Path))
            {
                return null;
            }
            var parts = File.ReadAllText(Path).Trim().Split(',');
            var numbers = new int[5];
            if (parts.Length != 5)
            {
                return null;
            }
            for (var at = 0; at < 5; at++)
            {
                if (!int.TryParse(parts[at], NumberStyles.Integer, CultureInfo.InvariantCulture, out numbers[at]))
                {
                    return null;
                }
            }
            return (new RectInt32(numbers[0], numbers[1], numbers[2], numbers[3]), numbers[4] == 1);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"window placement could not be read: {e.GetType().Name}");
            return null;
        }
    }

    [DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(IntPtr hwnd);
}
