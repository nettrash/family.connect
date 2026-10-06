using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// A video message played IN PLACE on Windows (docs/audio-video-messages-2026-10-04.md, S5.3, S5.4, S6): the circle's machine
/// — rest, loading, playing, paused, ended, failed — what a tap does in each, when its dot goes, the one ring, what shows,
/// when it is out of view, and how often a frame is copied into the circle.
/// </summary>
public sealed class RoundPlaybackTests
{
    private const long A = 91;
    private const long B = 92;

    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    private static readonly AttachmentDto Circle =
        new(A, "video", "video/mp4", 1649700, 480, 480, 23400, HasPreview: true, Round: true);

    /// <summary>A machine with circle A playing: tapped, opened, playing, its first frame drawn.</summary>
    private static RoundPlayback PlayingA()
    {
        var round = new RoundPlayback();
        round.Tap(A, recording: false);
        var load = round.Load;
        Assert.True(round.Opened(load, 23.4));
        round.Playing(load);
        round.FirstFrame(load);
        return round;
    }

    // ---- a tap ---------------------------------------------------------------------------------------

    /// <summary>A tap at rest loads it — and nothing plays by itself: there is no circle until one is tapped (no autoplay).</summary>
    [Fact]
    public void ATapAtRestLoadsIt()
    {
        var round = new RoundPlayback();
        Assert.Null(round.Id);
        Assert.Equal(RoundPhase.Rest, round.PhaseOf(A));

        var step = round.Tap(A, recording: false);

        Assert.Equal(new RoundStep(RoundTap.Started, RoundAct.Load), step);
        Assert.Equal(A, round.Id);
        Assert.Equal(RoundPhase.Loading, round.Phase);
        Assert.False(round.HasFrames);
        Assert.False(round.SeenPlaying);
    }

    /// <summary>Opened, it is played only while still waited for; the player saying "playing" is what makes it play.</summary>
    [Fact]
    public void LoadingThenPlaying()
    {
        var round = new RoundPlayback();
        round.Tap(A, recording: false);
        var load = round.Load;

        Assert.True(round.Opened(load, 23.4));
        Assert.Equal(RoundPhase.Loading, round.Phase);
        Assert.Equal(23.4, round.Total);
        round.Playing(load);
        Assert.Equal(RoundPhase.Playing, round.Phase);
        Assert.True(round.SeenPlaying);
        Assert.False(round.HasFrames);
        round.FirstFrame(load);
        Assert.True(round.HasFrames);
    }

    /// <summary>A second tap while it plays pauses it, keeping its frame; a third plays on.</summary>
    [Fact]
    public void ASecondTapPausesAndAThirdPlaysOn()
    {
        var round = PlayingA();

        Assert.Equal(new RoundStep(RoundTap.Paused, RoundAct.Pause), round.Tap(A, recording: false));
        Assert.Equal(RoundPhase.Paused, round.Phase);
        Assert.True(round.HasFrames);

        Assert.Equal(new RoundStep(RoundTap.Resumed, RoundAct.Play), round.Tap(A, recording: false));
        Assert.Equal(RoundPhase.Playing, round.Phase);
    }

    /// <summary>"A second tap while it loads gives up" (S5.3): back to the poster, and the download landing later plays nothing.</summary>
    [Fact]
    public void ASecondTapWhileItLoadsGivesUp()
    {
        var round = new RoundPlayback();
        round.Tap(A, recording: false);
        var load = round.Load;

        Assert.Equal(new RoundStep(RoundTap.GaveUp, RoundAct.Stop), round.Tap(A, recording: false));
        Assert.Equal(RoundPhase.Rest, round.Phase);

        // The bytes land after all: stale, and nothing starts.
        Assert.False(round.Opened(load, 23.4));
        round.Playing(load);
        round.FirstFrame(load);
        Assert.Equal(RoundPhase.Rest, round.Phase);
        Assert.False(round.HasFrames);
        Assert.Equal(RoundAct.None, round.Failed(load));
        Assert.Equal(RoundPhase.Rest, round.Phase);
    }

    /// <summary>A stall is loading too, and a tap in it gives up as well.</summary>
    [Fact]
    public void AStallShowsTheLoadingRingAndATapGivesUp()
    {
        var round = PlayingA();
        round.Waiting(round.Load);
        Assert.Equal(RoundPhase.Loading, round.Phase);
        Assert.True(RoundInline.ShowsSpinner(round.Phase));
        Assert.True(RoundInline.ShowsFrames(round.Phase, round.HasFrames));

        Assert.Equal(RoundTap.GaveUp, round.Tap(A, recording: false).Tap);
        Assert.Equal(RoundPhase.Rest, round.Phase);
    }

    /// <summary>Paused before the player opened, it stays paused once it opens.</summary>
    [Fact]
    public void APauseBeforeTheOpeningIsKept()
    {
        var round = new RoundPlayback();
        round.Tap(A, recording: false);
        var load = round.Load;
        round.FirstFrame(load);
        Assert.Equal(RoundAct.Pause, round.Yield());

        Assert.False(round.Opened(load, 23.4));
        Assert.Equal(RoundPhase.Paused, round.Phase);
    }

    /// <summary>Nothing STARTS while a recording runs (S1.7); a pause is always allowed.</summary>
    [Fact]
    public void NothingStartsUnderARecordingButAPauseIsAllowed()
    {
        var round = new RoundPlayback();
        Assert.Equal(new RoundStep(RoundTap.Refused, RoundAct.None), round.Tap(A, recording: true));
        Assert.Null(round.Id);

        var playing = PlayingA();
        Assert.Equal(RoundTap.Paused, playing.Tap(A, recording: true).Tap);
        Assert.Equal(new RoundStep(RoundTap.Refused, RoundAct.None), playing.Tap(A, recording: true));
        Assert.Equal(RoundPhase.Paused, playing.Phase);
        Assert.Equal(RoundTap.Refused, playing.Tap(B, recording: true).Tap);
        Assert.Equal(A, playing.Id);
    }

    /// <summary>One circle at a time: tapping another takes the player from the first, which goes back to its poster.</summary>
    [Fact]
    public void TappingAnotherCircleReplacesTheFirst()
    {
        var round = PlayingA();
        var first = round.Load;

        var step = round.Tap(B, recording: false);

        Assert.Equal(new RoundStep(RoundTap.Started, RoundAct.Load, Replaced: A), step);
        Assert.Equal(B, round.Id);
        Assert.Equal(RoundPhase.Rest, round.PhaseOf(A));
        Assert.Equal(RoundPhase.Loading, round.PhaseOf(B));
        // The first one's late news reaches nothing.
        round.Playing(first);
        Assert.Equal(RoundPhase.Loading, round.Phase);
        Assert.Equal((RoundAct.None, false), round.Ended(first));
    }

    // ---- the end, failures --------------------------------------------------------------------------------

    /// <summary>At the end it returns to the poster and, played through here, loses its dot (S5.3).</summary>
    [Fact]
    public void TheEndReturnsToThePosterAndTheDotGoes()
    {
        var round = PlayingA();
        round.Progress(round.Load, 23.4, 23.4);

        Assert.Equal((RoundAct.Stop, true), round.Ended(round.Load));
        Assert.Equal(RoundPhase.Rest, round.Phase);
        Assert.False(round.HasFrames);
        Assert.Equal(0, round.Position);
        Assert.Equal(0, RoundInline.Ring(round.Phase, round.SeenPlaying, round.Position, round.Total, reducedMotion: false));
        Assert.True(RoundInline.ShowsDisc(round.Phase));

        // And a tap plays it again from the start.
        Assert.Equal(RoundTap.Started, round.Tap(A, recording: false).Tap);
    }

    /// <summary>An end the player reports before it was ever seen playing is no play-through: the dot stays.</summary>
    [Fact]
    public void AnEndNeverSeenPlayingKeepsTheDot()
    {
        var round = new RoundPlayback();
        round.Tap(A, recording: false);
        Assert.Equal((RoundAct.Stop, false), round.Ended(round.Load));
    }

    /// <summary>A failure waits with its sentence, and a tap tries again.</summary>
    [Fact]
    public void AFailureWaitsForATapToTryAgain()
    {
        var round = new RoundPlayback();
        round.Tap(A, recording: false);
        var load = round.Load;

        Assert.Equal(RoundAct.Stop, round.Failed(load));
        Assert.Equal(RoundPhase.Failed, round.Phase);
        Assert.True(RoundInline.ShowsFailure(round.Phase));
        Assert.True(RoundInline.ShowsDisc(round.Phase));

        var again = round.Tap(A, recording: false);
        Assert.Equal(new RoundStep(RoundTap.Started, RoundAct.Load), again);
        Assert.NotEqual(load, round.Load);
        Assert.False(RoundInline.ShowsFailure(round.Phase));
    }

    // ---- one thing at a time, interruptions, stopping -------------------------------------------------------

    /// <summary>Something else starts — a voice note, the viewer, a recording: a playing circle pauses; one only loading gives up.</summary>
    [Fact]
    public void ItMakesWayForWhateverElseStarts()
    {
        var playing = PlayingA();
        Assert.Equal(RoundAct.Pause, playing.Yield());
        Assert.Equal(RoundPhase.Paused, playing.Phase);
        Assert.Equal(RoundAct.None, playing.Yield());

        var loading = new RoundPlayback();
        loading.Tap(A, recording: false);
        Assert.Equal(RoundAct.Stop, loading.Yield());
        Assert.Equal(RoundPhase.Rest, loading.Phase);

        Assert.Equal(RoundAct.None, new RoundPlayback().Yield());
    }

    /// <summary>A call, a lock, a new output device and a hidden window pause it (S4); losing focus does not.</summary>
    [Theory]
    [InlineData(PlaybackEvent.Call, RoundAct.Pause)]
    [InlineData(PlaybackEvent.SessionLocked, RoundAct.Pause)]
    [InlineData(PlaybackEvent.OutputChanged, RoundAct.Pause)]
    [InlineData(PlaybackEvent.WindowHidden, RoundAct.Pause)]
    [InlineData(PlaybackEvent.FocusLost, RoundAct.None)]
    public void WhatPausesItInPlace(PlaybackEvent happened, RoundAct act)
    {
        var round = PlayingA();
        Assert.Equal(act, round.Interrupt(happened));
        Assert.Equal(act == RoundAct.Pause ? RoundPhase.Paused : RoundPhase.Playing, round.Phase);
    }

    /// <summary>Its row scrolled away or its chat closed: it stops, back to the poster, and says which circle that was.</summary>
    [Fact]
    public void StoppingReturnsItToThePoster()
    {
        var round = PlayingA();
        var load = round.Load;

        Assert.Equal(A, round.Stop());
        Assert.Null(round.Id);
        Assert.Equal(RoundPhase.Rest, round.PhaseOf(A));
        round.Playing(load);
        Assert.Equal(RoundPhase.Rest, round.Phase);
        Assert.Null(round.Stop());
    }

    /// <summary>The system pausing the player is a pause, not a stop.</summary>
    [Fact]
    public void ThePlayerPausingByItselfIsAPause()
    {
        var round = PlayingA();
        round.PausedByPlayer(round.Load);
        Assert.Equal(RoundPhase.Paused, round.Phase);
        Assert.True(RoundInline.ShowsDisc(round.Phase));
    }

    // ---- how it is drawn ---------------------------------------------------------------------------------

    /// <summary>The ring follows the time while it plays, steps once a second under Reduce Motion, and is never drawn at rest.</summary>
    [Theory]
    [InlineData(RoundPhase.Playing, true, 5.5, 22.0, false, 0.25)]
    [InlineData(RoundPhase.Playing, true, 5.5, 22.0, true, 5.0 / 22.0)]
    [InlineData(RoundPhase.Paused, true, 11.0, 22.0, false, 0.5)]
    [InlineData(RoundPhase.Loading, true, 11.0, 22.0, false, 0.5)]
    [InlineData(RoundPhase.Loading, false, 0.0, 22.0, false, 0.0)]
    [InlineData(RoundPhase.Rest, true, 11.0, 22.0, false, 0.0)]
    [InlineData(RoundPhase.Failed, true, 11.0, 22.0, false, 0.0)]
    [InlineData(RoundPhase.Playing, true, 30.0, 22.0, false, 1.0)]
    [InlineData(RoundPhase.Playing, true, 3.0, 0.0, false, 0.0)]
    [InlineData(RoundPhase.Playing, true, double.NaN, 22.0, false, 0.0)]
    public void TheOneRing(RoundPhase phase, bool seen, double position, double total, bool reduced, double ring) =>
        Assert.Equal(ring, RoundInline.Ring(phase, seen, position, total, reduced), 6);

    /// <summary>What shows in each phase: the disc, the loading ring, the frames over the poster, the expand control.</summary>
    [Theory]
    [InlineData(RoundPhase.Rest, false, true, false, false)]
    [InlineData(RoundPhase.Loading, false, false, true, false)]
    [InlineData(RoundPhase.Loading, true, false, true, true)]
    [InlineData(RoundPhase.Playing, false, false, false, false)]
    [InlineData(RoundPhase.Playing, true, false, false, true)]
    [InlineData(RoundPhase.Paused, true, true, false, true)]
    [InlineData(RoundPhase.Failed, false, true, false, false)]
    public void WhatShows(RoundPhase phase, bool hasFrames, bool disc, bool spinner, bool frames)
    {
        Assert.Equal(disc, RoundInline.ShowsDisc(phase));
        Assert.Equal(spinner, RoundInline.ShowsSpinner(phase));
        Assert.Equal(frames, RoundInline.ShowsFrames(phase, hasFrames));
        Assert.Equal(frames, RoundInline.ShowsExpand(phase, hasFrames));
    }

    /// <summary>The capsule counts up once it has started, and says the length otherwise; Narrator hears what a press does.</summary>
    [Fact]
    public void TheCapsuleAndTheAction()
    {
        Assert.Equal("0:23", RoundInline.Capsule(Circle, RoundPhase.Rest, seenPlaying: false, 0));
        Assert.Equal("0:23", RoundInline.Capsule(Circle, RoundPhase.Loading, seenPlaying: false, 0));
        Assert.Equal("0:07", RoundInline.Capsule(Circle, RoundPhase.Playing, seenPlaying: true, 7.2));
        Assert.Equal("0:07", RoundInline.Capsule(Circle, RoundPhase.Paused, seenPlaying: true, 7.2));

        Assert.Equal("Pause", RoundInline.Action(RoundPhase.Playing, Say));
        Assert.Equal("Pause", RoundInline.Action(RoundPhase.Loading, Say));
        Assert.Equal("Play", RoundInline.Action(RoundPhase.Paused, Say));
        Assert.Equal("Play", RoundInline.Action(RoundPhase.Rest, Say));
        Assert.Equal("Play", RoundInline.Action(RoundPhase.Failed, Say));
    }

    /// <summary>In place is this build's choice; the viewer stays Open Full Screen's.</summary>
    [Fact]
    public void ThisBuildPlaysCirclesInPlace() => Assert.True(RoundInline.PlaysInPlace);

    /// <summary>The ring runs outside the edge, past the gap — never over the picture.</summary>
    [Fact]
    public void TheRingIsOutsideTheEdge() => Assert.Equal(RoundInline.RingGap + RoundLook.Ring, RoundInline.RingReach);

    /// <summary>Wholly out of its scroller's view, it is out of view; any part seen, or no scroller at all, it is in.</summary>
    [Theory]
    [InlineData(0, 0, 400, 600, true)]
    [InlineData(-100, -200, 400, 300, true)]
    [InlineData(0, -600, 400, 600, false)]
    [InlineData(0, -360, 400, 600, true)]
    [InlineData(0, 240, 400, 600, false)]
    [InlineData(0, 239, 400, 600, true)]
    [InlineData(500, 0, 400, 600, false)]
    [InlineData(0, 0, 0, 0, false)]
    [InlineData(0, 0, -1, -1, false)]
    public void InView(double x, double y, double w, double h, bool seen) =>
        Assert.Equal(seen, RoundInline.InView(x, y, w, h, 240, 240));

    /// <summary>WinUI's empty rectangle (infinite corner, negative infinite size) is out of view; an unbounded one is in.</summary>
    [Fact]
    public void EmptyAndUnboundedViewports()
    {
        Assert.False(RoundInline.InView(double.PositiveInfinity, double.PositiveInfinity, double.NegativeInfinity, double.NegativeInfinity, 240, 240));
        Assert.True(RoundInline.InView(double.NegativeInfinity, double.NegativeInfinity, double.PositiveInfinity, double.PositiveInfinity, 240, 240));
        Assert.False(RoundInline.InView(double.NaN, 0, 400, 600, 240, 240));
    }

    // ---- frames ------------------------------------------------------------------------------------------

    /// <summary>The clip's own rate, never over 30, and 30 when it says none.</summary>
    [Theory]
    [InlineData(30.0, 1000.0 / 30)]
    [InlineData(25.0, 40.0)]
    [InlineData(60.0, 1000.0 / 30)]
    [InlineData(15.0, 1000.0 / 15)]
    [InlineData(null, 1000.0 / 30)]
    [InlineData(0.0, 1000.0 / 30)]
    [InlineData(double.NaN, 1000.0 / 30)]
    public void FramesAreCopiedAtTheClipsRateAtMostThirty(double? rate, double interval) =>
        Assert.Equal(interval, RoundFrames.IntervalMs(rate), 6);

    /// <summary>A 30 fps clip copies every frame despite jitter; a 60 fps clip every other one; never two copies at once.</summary>
    [Fact]
    public void ThrottlingKeepsTheRateAndOneCopyAtATime()
    {
        var thirty = RoundFrames.IntervalMs(30);
        Assert.True(RoundFrames.Due(1000, null, thirty, copying: false));
        Assert.False(RoundFrames.Due(1000, null, thirty, copying: true));
        // 30 fps arriving 2 ms early still copies.
        Assert.True(RoundFrames.Due(1031, 1000, thirty, copying: false));
        Assert.False(RoundFrames.Due(1031, 1000, thirty, copying: true));

        // A 60 fps clip, offered every 16.7 ms: copies at 0, 33, 67, … — 30 a second, not 15 and not 60.
        long? last = null;
        var copies = 0;
        for (var frame = 0; frame < 60; frame++)
        {
            var now = (long)Math.Round(frame * 1000.0 / 60);
            if (RoundFrames.Due(now, last, thirty, copying: false))
            {
                copies++;
                last = now;
            }
        }
        Assert.Equal(30, copies);

        // A clock that went backwards copies rather than freezing.
        Assert.True(RoundFrames.Due(10, 1000, thirty, copying: false));
    }

    /// <summary>The surface is the circle at the screen's scale, even, within bounds.</summary>
    [Theory]
    [InlineData(240, 1.0, 240)]
    [InlineData(240, 1.5, 360)]
    [InlineData(240, 1.25, 300)]
    [InlineData(240, 1.75, 420)]
    [InlineData(240, 2.0, 480)]
    [InlineData(240, 5.0, 960)]
    [InlineData(240, 0.0, 240)]
    [InlineData(241, 1.0, 240)]
    [InlineData(4, 1.0, 16)]
    public void TheFrameSurfaceFitsTheCircle(double diameter, double scale, int side) =>
        Assert.Equal(side, RoundFrames.Side(diameter, scale));

    /// <summary>Playing two seconds with no frame drawn is playing blind: the circle gives up on its frames and opens the viewer.</summary>
    [Theory]
    [InlineData(RoundPhase.Playing, false, 2.0, true)]
    [InlineData(RoundPhase.Playing, false, 1.9, false)]
    [InlineData(RoundPhase.Playing, true, 5.0, false)]
    [InlineData(RoundPhase.Paused, false, 5.0, false)]
    [InlineData(RoundPhase.Loading, false, 5.0, false)]
    [InlineData(RoundPhase.Playing, false, double.NaN, false)]
    public void PlayingBlindGivesUpOnTheFrames(RoundPhase phase, bool hasFrames, double position, bool starved) =>
        Assert.Equal(starved, RoundFrames.Starved(phase, hasFrames, position));

    [Theory]
    [InlineData(30u, 1u, 30.0)]
    [InlineData(30000u, 1001u, 30000.0 / 1001)]
    [InlineData(0u, 1u, null)]
    [InlineData(30u, 0u, null)]
    public void AClipsRateFromItsRatio(uint numerator, uint denominator, double? rate) =>
        Assert.Equal(rate, RoundFrames.Rate(numerator, denominator));

    /// <summary>A mark the same build left behind means it died playing in place; another build's, or none, does not.</summary>
    [Theory]
    [InlineData("playing in place: 1111", "1111", true)]
    [InlineData("playing in place: 1111\r\n", "1111", true)]
    [InlineData("playing in place: 1111", "2222", false)]
    [InlineData(null, "1111", false)]
    [InlineData("", "1111", false)]
    [InlineData("playing in place: ", "", false)]
    public void OnlyTheBuildThatDiedPlayingInPlaceOpensTheViewer(string? held, string build, bool tripped) =>
        Assert.Equal(tripped, RoundCrashGuard.Tripped(held, build));

    [Fact]
    public void TheMarkNamesTheBuild() =>
        Assert.True(RoundCrashGuard.Tripped(RoundCrashGuard.Mark("abc"), "abc"));

    [Fact]
    public void ACopiedFrameIsOpaqueAndKeepsItsColour()
    {
        byte[] frame = [1, 2, 3, 0, 4, 5, 6, 128, 7, 8, 9, 255];
        RoundFrames.Opaque(frame);
        Assert.Equal(new byte[] { 1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255 }, frame);
    }
}
