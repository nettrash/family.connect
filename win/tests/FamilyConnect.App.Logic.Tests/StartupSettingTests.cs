using FamilyConnect.App.Logic;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The "start when I sign in" row, in every state Windows can answer with.
/// </summary>
/// <remarks>
/// The interesting half is the three states the app cannot change. A toggle
/// that looks live and does nothing is what this is written to prevent:
/// <c>RequestEnableAsync</c> on a task the person disabled in Task Manager
/// answers DisabledByUser and changes nothing.
/// </remarks>
public sealed class StartupSettingTests
{
    [Fact]
    public void TheRowIsDrawnWhereverThereIsAStartupTask()
    {
        Assert.False(StartupSetting.IsOffered(StartupState.Unavailable));
        foreach (var state in new[]
        {
            StartupState.Disabled, StartupState.Enabled, StartupState.DisabledByUser,
            StartupState.DisabledByPolicy, StartupState.EnabledByPolicy,
        })
        {
            Assert.True(StartupSetting.IsOffered(state), state.ToString());
        }
    }

    [Fact]
    public void TheToggleSitsWhereWindowsSaysItDoes()
    {
        Assert.True(StartupSetting.IsOn(StartupState.Enabled));
        Assert.True(StartupSetting.IsOn(StartupState.EnabledByPolicy));
        Assert.False(StartupSetting.IsOn(StartupState.Disabled));
        Assert.False(StartupSetting.IsOn(StartupState.DisabledByUser));
        Assert.False(StartupSetting.IsOn(StartupState.DisabledByPolicy));
    }

    /// <summary>Only two of the five are ours to change.</summary>
    [Fact]
    public void OnlyTheAppsOwnTwoStatesAreChangeable()
    {
        Assert.True(StartupSetting.IsChangeable(StartupState.Disabled));
        Assert.True(StartupSetting.IsChangeable(StartupState.Enabled));
        Assert.False(StartupSetting.IsChangeable(StartupState.DisabledByUser));
        Assert.False(StartupSetting.IsChangeable(StartupState.DisabledByPolicy));
        Assert.False(StartupSetting.IsChangeable(StartupState.EnabledByPolicy));
        Assert.False(StartupSetting.IsChangeable(StartupState.Unavailable));
    }

    [Fact]
    public void EachStateHasItsOwnSentence()
    {
        Assert.Equal(StartupSetting.Note.WhatItDoes, StartupSetting.NoteFor(StartupState.Disabled));
        Assert.Equal(StartupSetting.Note.WhatItDoes, StartupSetting.NoteFor(StartupState.Enabled));
        Assert.Equal(
            StartupSetting.Note.BlockedByTaskManager,
            StartupSetting.NoteFor(StartupState.DisabledByUser));
        Assert.Equal(
            StartupSetting.Note.DecidedByPolicy,
            StartupSetting.NoteFor(StartupState.DisabledByPolicy));
        Assert.Equal(
            StartupSetting.Note.DecidedByPolicy,
            StartupSetting.NoteFor(StartupState.EnabledByPolicy));
    }

    /// <summary>
    /// The one that matters at the moment somebody flips it: a refused
    /// request must put the row back, not leave it looking on.
    /// </summary>
    [Fact]
    public void ARefusedRequestIsNotASuccess()
    {
        Assert.True(StartupSetting.RequestSucceeded(StartupState.Enabled));
        Assert.True(StartupSetting.RequestSucceeded(StartupState.EnabledByPolicy));
        Assert.False(StartupSetting.RequestSucceeded(StartupState.DisabledByUser));
        Assert.False(StartupSetting.RequestSucceeded(StartupState.Disabled));
        Assert.False(StartupSetting.RequestSucceeded(StartupState.DisabledByPolicy));
    }

    [Fact]
    public void A_launch_windows_made_at_sign_in_draws_no_window()
    {
        Assert.True(StartupSetting.StartsHidden(fromStartupTask: true, hasNotificationAreaIcon: true));
    }

    [Fact]
    public void A_launch_a_person_asked_for_always_draws_its_window()
    {
        Assert.False(StartupSetting.StartsHidden(fromStartupTask: false, hasNotificationAreaIcon: true));
        Assert.False(StartupSetting.StartsHidden(fromStartupTask: false, hasNotificationAreaIcon: false));
    }

    [Fact]
    public void With_no_icon_to_come_back_from_even_a_sign_in_launch_shows_itself()
    {
        // Otherwise the app is running with no window and nothing to click.
        Assert.False(StartupSetting.StartsHidden(fromStartupTask: true, hasNotificationAreaIcon: false));
    }
}
