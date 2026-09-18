// Hands a second process's activation to the running instance. Pre-window, pre-pump: no XAML exists
// yet in either process.
using Microsoft.Windows.AppLifecycle;

namespace FamilyConnect.App;

internal static class Redirection
{
    /// <summary>
    /// RedirectActivationToAsync must not be awaited on the STA thread; the Windows App SDK's
    /// instancing sample redirects on a worker and blocks on a semaphore, which is this. The wait
    /// is bounded, so an instance that never answers cannot leave a second process hanging.
    /// </summary>
    public static void RedirectAndWait(AppInstance target, AppActivationArguments args)
    {
        var done = new SemaphoreSlim(0, 1);
        Task.Run(() =>
        {
            try
            {
                target.RedirectActivationToAsync(args).AsTask().Wait();
            }
            finally
            {
                done.Release();
            }
        });
        done.Wait(TimeSpan.FromSeconds(10));
    }
}
