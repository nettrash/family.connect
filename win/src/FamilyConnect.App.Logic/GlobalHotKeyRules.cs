namespace FamilyConnect.App.Logic;

/// <summary>What the global shortcut does when pressed (#80).</summary>
public enum HotKeyAction
{
    /// <summary>Bring the window forward — from the notification area, from behind other windows, from minimised.</summary>
    Show,

    /// <summary>Put it back in the notification area.</summary>
    Hide,
}

/// <summary>
/// The shortcut that brings Family Connect forward from any app (issue #80, docs/mac-menu-bar-2026-10-07.md) — the Mac's
/// ⌃⌥⌘F, here as CTRL+ALT+SHIFT+F. Three modifiers, as on the Mac, and never Ctrl+Alt alone: Windows reads Ctrl+Alt as
/// AltGr, and on a Hungarian or Czech keyboard AltGr+F TYPES "[", which a system-wide shortcut would take from every app.
/// </summary>
public static class GlobalHotKeyRules
{
    /// <summary>MOD_ALT | MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT: a held key is one press, not a stream of them.</summary>
    public const uint Modifiers = 0x0001 | 0x0002 | 0x0004 | 0x4000;

    /// <summary>The virtual key: F.</summary>
    public const uint Key = 0x46;

    /// <summary>The shortcut as Windows writes it in English; translated, because German says "Strg+Alt+Umschalt+F".</summary>
    public const string Label = "Ctrl+Alt+Shift+F";

    /// <summary>ERROR_HOTKEY_ALREADY_REGISTERED: another app holds the combination.</summary>
    public const int TakenError = 1409;

    /// <summary>
    /// From anywhere it brings the window forward; pressed while the window is the one in front, it puts it back in the
    /// notification area — only when there is an icon there to come back from and closing is allowed to keep the app
    /// running (otherwise there is nowhere to hide it, and it simply stays in front).
    /// </summary>
    public static HotKeyAction Action(bool windowInFront, bool canHideToTray) =>
        windowInFront && canHideToTray ? HotKeyAction.Hide : HotKeyAction.Show;

    /// <summary>Whether a failed registration was the combination being taken (said in Settings) rather than anything else.</summary>
    public static bool IsTaken(int win32Error) => win32Error == TakenError;
}
