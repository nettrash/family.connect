using FamilyConnect.App.Logic;
using Windows.ApplicationModel;

namespace FamilyConnect.App.Services;

/// <summary>
/// The manifest's startup task, as the settings screen needs it: what state
/// it is in, and the two changes this app is allowed to make.
/// </summary>
/// <remarks>
/// A thin edge over <c>Windows.ApplicationModel.StartupTask</c> — every
/// decision lives in <see cref="StartupSetting"/>, which is testable off
/// Windows; this file only fetches and maps. It answers
/// <see cref="StartupState.Unavailable"/> rather than throwing when there is
/// no task at all: an unpackaged run has no package identity, and a settings
/// row is not worth a crash.
/// </remarks>
internal static class StartupLaunch
{
    /// <summary>The id declared in Package.appxmanifest. Changing one changes both.</summary>
    private const string TaskId = "FamilyConnectStartup";

    public static async Task<StartupState> StateAsync()
    {
        try
        {
            var task = await StartupTask.GetAsync(TaskId);
            return Map(task.State);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"the startup task could not be read: {e.GetType().Name} 0x{e.HResult:X8}");
            return StartupState.Unavailable;
        }
    }

    /// <summary>
    /// Ask for it, or give it up. Answers the state AFTERWARDS, which is the
    /// only thing worth trusting: a request on a task the person disabled in
    /// Task Manager comes back DisabledByUser and changes nothing.
    /// </summary>
    public static async Task<StartupState> SetAsync(bool wanted)
    {
        try
        {
            var task = await StartupTask.GetAsync(TaskId);
            if (wanted)
            {
                return Map(await task.RequestEnableAsync());
            }

            // Disable() is synchronous and has no answer; the state is read
            // back rather than assumed.
            task.Disable();
            var after = await StartupTask.GetAsync(TaskId);
            return Map(after.State);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"the startup task could not be set: {e.GetType().Name} 0x{e.HResult:X8}");
            return StartupState.Unavailable;
        }
    }

    private static StartupState Map(StartupTaskState state) => state switch
    {
        StartupTaskState.Disabled => StartupState.Disabled,
        StartupTaskState.DisabledByUser => StartupState.DisabledByUser,
        StartupTaskState.DisabledByPolicy => StartupState.DisabledByPolicy,
        StartupTaskState.Enabled => StartupState.Enabled,
        StartupTaskState.EnabledByPolicy => StartupState.EnabledByPolicy,
        _ => StartupState.Unavailable,
    };
}
