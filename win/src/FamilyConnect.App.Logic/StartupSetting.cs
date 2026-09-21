namespace FamilyConnect.App.Logic;

/// <summary>
/// Windows' own answer about whether an app may start when its person signs in
/// — <c>Windows.ApplicationModel.StartupTaskState</c>, mirrored here so the
/// rule below is a value this assembly can test off Windows.
/// </summary>
public enum StartupState
{
    /// <summary>Declared in the manifest, not on. The app may ask.</summary>
    Disabled,

    /// <summary>On, and the app may turn it off.</summary>
    Enabled,

    /// <summary>
    /// The PERSON turned it off, in Task Manager or Settings. An app cannot
    /// undo that — a request answers with this state again — so the only
    /// honest thing a settings screen can do is say where the switch is.
    /// </summary>
    DisabledByUser,

    /// <summary>An administrator forbade it.</summary>
    DisabledByPolicy,

    /// <summary>An administrator requires it.</summary>
    EnabledByPolicy,

    /// <summary>
    /// No startup task at all: an unpackaged run, or a build whose manifest
    /// predates the extension. The row is not drawn.
    /// </summary>
    Unavailable,
}

/// <summary>
/// What the "start when I sign in" row shows, and whether it may be touched.
/// </summary>
/// <remarks>
/// <para>
/// Three of the five states are ones the app cannot change, and they are the
/// reason this is a rule rather than a bool. A toggle that looks live and
/// does nothing is the failure mode: <c>RequestEnableAsync</c> on a task the
/// person disabled in Task Manager returns <see cref="StartupState.DisabledByUser"/>
/// and changes nothing, so the row has to say where the real switch is
/// instead of springing back on its own.
/// </para>
/// <para>
/// Kept in App.Logic, free of any Windows type, so every branch is pinned by
/// a plain unit test on the machine this is developed on.
/// </para>
/// </remarks>
public static class StartupSetting
{
    /// <summary>Whether the row is drawn at all.</summary>
    public static bool IsOffered(StartupState state) => state != StartupState.Unavailable;

    /// <summary>Where the toggle sits.</summary>
    public static bool IsOn(StartupState state) =>
        state is StartupState.Enabled or StartupState.EnabledByPolicy;

    /// <summary>Whether this app can change it.</summary>
    public static bool IsChangeable(StartupState state) =>
        state is StartupState.Disabled or StartupState.Enabled;

    /// <summary>
    /// Which sentence belongs under the row. The caller turns these into
    /// localised text; they are a closed set so no branch can go unwritten.
    /// </summary>
    public enum Note
    {
        /// <summary>Ordinary: what turning it on does.</summary>
        WhatItDoes,

        /// <summary>The person switched it off outside this app.</summary>
        BlockedByTaskManager,

        /// <summary>An administrator decided, either way.</summary>
        DecidedByPolicy,
    }

    public static Note NoteFor(StartupState state) => state switch
    {
        StartupState.DisabledByUser => Note.BlockedByTaskManager,
        StartupState.DisabledByPolicy or StartupState.EnabledByPolicy => Note.DecidedByPolicy,
        _ => Note.WhatItDoes,
    };

    /// <summary>
    /// Whether a launch should draw a window. A launch Windows made at sign-in
    /// is not one anybody asked to look at, so it stays in the notification
    /// area — but only when there IS an icon to come back from: a hidden
    /// window with no icon is an app nobody can reach.
    /// </summary>
    public static bool StartsHidden(bool fromStartupTask, bool hasNotificationAreaIcon) =>
        fromStartupTask && hasNotificationAreaIcon;

    /// <summary>
    /// What a request to turn it ON actually achieved, given the state
    /// Windows answered with. False means the row must go back to off —
    /// the request was refused, and pretending otherwise is the bug this
    /// exists to prevent.
    /// </summary>
    public static bool RequestSucceeded(StartupState after) => IsOn(after);
}
