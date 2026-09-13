using FamilyConnect.App.Logic;
using Microsoft.Windows.AppNotifications;
using Microsoft.Windows.AppNotifications.Builder;
using Microsoft.Windows.BadgeNotifications;

namespace FamilyConnect.App.Services;

/// <summary>
/// Windows' notifications and the taskbar badge. What to say and when is <see cref="NotificationRules"/>;
/// this is only the saying. Every call here is cosmetic and must never take the app down, so each
/// failure is written down and swallowed.
/// </summary>
/// <remarks>
/// <b>SOCKET-ONLY, AND THAT IS THE PROTOCOL'S ANSWER.</b> A Windows client registers no push device
/// (<c>POST /devices</c> takes ios, macos or android), so these are raised from the live socket while
/// the app runs — the browser's position (win/README.md).
/// </remarks>
internal static class Toasts
{
    private const string Group = "family-connect";

    /// <summary>Whether a notification can be shown at all: supported, and not turned off by the user.</summary>
    public static bool Available
    {
        get
        {
            try
            {
                return AppNotificationManager.IsSupported()
                    && AppNotificationManager.Default.Setting == AppNotificationSetting.Enabled;
            }
            catch (Exception)
            {
                return false;
            }
        }
    }

    public static void Show(Toast toast)
    {
        try
        {
            var builder = new AppNotificationBuilder();
            foreach (var (key, value) in ToastArguments.For(toast))
            {
                builder.AddArgument(key, value);
            }
            var notification = builder.AddText(toast.Title).AddText(toast.Body).BuildNotification();
            // A second notification about the same chat REPLACES the first rather than stacking on it.
            notification.Tag = toast.Tag;
            notification.Group = Group;
            AppNotificationManager.Default.Show(notification);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"notification: {e.GetType().Name}");
        }
    }

    /// <summary>The chat was opened: what was said about it has been seen.</summary>
    public static void Clear(string tag)
    {
        try
        {
            _ = AppNotificationManager.Default.RemoveByTagAndGroupAsync(tag, Group);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"clearing notifications: {e.GetType().Name}");
        }
    }

    public static void Badge(int unread)
    {
        try
        {
            if (unread > 0)
            {
                BadgeNotificationManager.Current.SetBadgeAsCount((uint)unread);
            }
            else
            {
                BadgeNotificationManager.Current.ClearBadge();
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"taskbar badge: {e.GetType().Name}");
        }
    }

    public static void Unregister()
    {
        try
        {
            AppNotificationManager.Default.Unregister();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"unregistering notifications: {e.GetType().Name}");
        }
    }
}
