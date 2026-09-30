using System.Diagnostics;
using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml.Controls;

namespace FamilyConnect.App.Views;

/// <summary>
/// What makes an animated chat sticker move: one timer for every sticker a view is showing, which
/// puts the frame that is due into each <c>Image</c>.
/// </summary>
/// <remarks>
/// <para>
/// <b>ONE TIMER, NOT ONE PER STICKER.</b> A conversation can hold a dozen, and a dozen timers each
/// waking the window's thread is a dozen wake-ups for one redraw. Which frame is due is
/// <see cref="StickerAnimation.FrameAt"/>, from how long each sticker has been up — arithmetic,
/// and tested where no window exists.
/// </para>
/// <para>
/// <b>AN IMAGE THAT HAS LEFT THE WINDOW IS LET GO.</b> The conversation is rebuilt on every change,
/// so its elements come and go constantly; an entry whose image was drawn once and no longer is,
/// is dropped on the next tick, and the timer stops when nothing is left to move.
/// </para>
/// <para>
/// A still picture never reaches the timer at all: it is set and forgotten.
/// </para>
/// <para>
/// <b>AND IT IS THE ONE PLACE THAT KNOWS WHAT IS ON SCREEN</b>, which is what decides how a picture
/// may give its frames back: one no image here is drawing is released whole, one that is being
/// drawn is put back on frame zero first and then made still (<see cref="StickerShelf{TPicture}"/>
/// asks; <see cref="IsShowing"/>, <see cref="MakeStill"/> and <see cref="Release"/> answer). A frame
/// disposed under an image that is showing it is a hole in the conversation, so after a tick has
/// FAILED — when this no longer knows which image shows what — nothing is disposed at all, and the
/// frames go when the collector says.
/// </para>
/// </remarks>
internal sealed class StickerAnimator
{
    /// <summary>How long an image may wait to be drawn before it is taken for one that never will be.</summary>
    private const long NeverShownMs = 10_000;

    private sealed class Playing(Image image, StickerPicture picture, long started, long made)
    {
        public Image Image { get; } = image;

        public StickerPicture Picture { get; } = picture;

        /// <summary>When the STICKER began moving — earlier than this image, where a rebuild made the image.</summary>
        public long Started { get; } = started;

        /// <summary>When this image was handed over, which is what "never drawn" is counted from.</summary>
        public long Made { get; } = made;

        public int Shown { get; set; }

        public bool WasLoaded { get; set; }
    }

    private readonly DispatcherQueueTimer timer;
    private readonly List<Playing> playing = [];
    private readonly Stopwatch clock = Stopwatch.StartNew();

    /// <summary>A tick threw, and the list of what is showing was dropped with it: nothing here may be disposed any more.</summary>
    private bool lostTrack;

    public StickerAnimator(DispatcherQueue queue)
    {
        timer = queue.CreateTimer();
        // Thirty a second: stickers are authored at 15 to 30, and the floor for a frame is a tenth of a second.
        timer.Interval = TimeSpan.FromMilliseconds(33);
        timer.Tick += (_, _) => Tick();
    }

    /// <summary>This animator's clock, in milliseconds — what <see cref="Show"/>'s <c>started</c> is measured on.</summary>
    public long Now => clock.ElapsedMilliseconds;

    /// <summary>Draw this picture in that image — and keep it moving, when it moves.</summary>
    /// <param name="started">
    /// When this STICKER began moving, where somebody remembers: a conversation is rebuilt on every change, and an image
    /// made by a rebuild joins the animation where it was instead of starting it again. Null starts it now.
    /// </param>
    public void Show(Image image, StickerPicture picture, long? started = null)
    {
        Forget(image);
        try
        {
            image.Source = picture.First;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"drawing a sticker: {e.GetType().Name}");
            return;
        }
        if (!picture.Moves)
        {
            return;
        }
        var now = clock.ElapsedMilliseconds;
        playing.Add(new Playing(image, picture, Math.Min(started ?? now, now), now));
        timer.Start();
    }

    /// <summary>Whether an image here is drawing that picture's frames — or might be, once track has been lost.</summary>
    public bool IsShowing(StickerPicture picture) =>
        lostTrack || playing.Exists(entry => ReferenceEquals(entry.Picture, picture));

    /// <summary>
    /// That picture stops moving, to give its frames back: every image drawing it is put on frame zero FIRST, and only
    /// then do the other frames go.
    /// </summary>
    public void MakeStill(StickerPicture picture)
    {
        if (lostTrack)
        {
            // Some image may be showing any of its frames: they are left for the collector.
            picture.MakeStill(dispose: false);
            return;
        }
        try
        {
            foreach (var entry in playing)
            {
                if (ReferenceEquals(entry.Picture, picture))
                {
                    entry.Image.Source = picture.First;
                }
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"stilling a sticker: {e.GetType().Name}");
            playing.RemoveAll(entry => ReferenceEquals(entry.Picture, picture));
            picture.MakeStill(dispose: false);
            return;
        }
        playing.RemoveAll(entry => ReferenceEquals(entry.Picture, picture));
        picture.MakeStill(dispose: true);
    }

    /// <summary>That picture is kept nowhere any more: its frames are given back, unless an image here still draws them.</summary>
    public void Release(StickerPicture picture)
    {
        if (!IsShowing(picture))
        {
            picture.Release();
        }
    }

    /// <summary>That image is about to show something else.</summary>
    public void Forget(Image image) => playing.RemoveAll(entry => ReferenceEquals(entry.Image, image));

    /// <summary>The view is going: nothing moves any more.</summary>
    public void Stop()
    {
        timer.Stop();
        playing.Clear();
    }

    private void Tick()
    {
        try
        {
            var now = clock.ElapsedMilliseconds;
            for (var at = playing.Count - 1; at >= 0; at--)
            {
                var entry = playing[at];
                if (entry.Image.IsLoaded)
                {
                    entry.WasLoaded = true;
                }
                else if (entry.WasLoaded || now - entry.Made > NeverShownMs)
                {
                    playing.RemoveAt(at);
                    continue;
                }
                else
                {
                    // Built and not yet in the window: its turn has not started.
                    continue;
                }
                if (entry.Picture is not { Frames: { } frames, Clock: { } durations })
                {
                    playing.RemoveAt(at);
                    continue;
                }
                var due = Math.Min(StickerAnimation.FrameAt(durations, now - entry.Started), frames.Count - 1);
                if (due != entry.Shown)
                {
                    entry.Shown = due;
                    entry.Image.Source = frames[due];
                }
            }
            if (playing.Count == 0)
            {
                timer.Stop();
            }
        }
        catch (Exception e)
        {
            // Whatever was on screen stays on screen — a frame of the sticker, which is a sticker. WHICH frame is no
            // longer known, so from here on no frame is disposed under it.
            Diagnostics.Write($"animating stickers: {e.GetType().Name}");
            lostTrack = true;
            Stop();
        }
    }
}
