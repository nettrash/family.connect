using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.Media.Core;
using Windows.Media.Playback;
using Windows.Storage.Streams;

namespace FamilyConnect.App.Views;

// ---- animations (issue #96) ----------------------------------------------------------------------------------------------------
//
// An ANIMATION is drawn as a picture that moves, not as a video (docs/protocol.md, "Animations", "How it is drawn"): in the box
// a photograph of its shape would take, with no play button, no length, no sound and no scrubber — only a small "GIF" in a
// corner. It plays by itself, muted, looping, while it is on screen, and stops when it scrolls away. Its poster stands instead,
// still, when Windows' animation effects are off or the video is over 8 MiB (AnimationPrep.PlaysInTile); a click opens it
// larger, in the viewer, playing, muted and looping, with no controls.
//
// ONE PLAYER PER ANIMATION, NOT PER TILE. The conversation is rebuilt on every change, so its tiles come and go constantly; a
// player kept under the attachment's id is handed to whichever of its tiles is on screen (MediaPlayerElement.SetMediaPlayer),
// so a redraw neither restarts it nor flashes its poster. It is made only when one of its tiles is in view, and let go —
// stopped and disposed — once none is, on the turn after, so a redraw that puts a new tile up first keeps it.

public sealed partial class ChatsView
{
    /// <summary>One animation tile as drawn: the attachment, its player's surface, and whether it is on screen.</summary>
    private sealed class AnimationCopy(AttachmentDto video, MediaPlayerElement surface)
    {
        public AttachmentDto Video { get; } = video;

        public MediaPlayerElement Surface { get; } = surface;

        public bool InView { get; set; }

        public bool Loaded { get; set; }
    }

    /// <summary>Every animation tile on the screen, by attachment.</summary>
    private readonly DrawnCopies<long, AnimationCopy> animationCopies = new();

    /// <summary>The players of the animations playing, by attachment.</summary>
    private readonly Dictionary<long, MediaPlayer> animationPlayers = [];

    /// <summary>The animations whose video is being fetched for a player.</summary>
    private readonly HashSet<long> animationsLoading = [];

    /// <summary>
    /// An animation's tile: its poster, the "GIF" label, and — while it is on screen and may move — its video, playing muted
    /// and looping over the poster.
    /// </summary>
    private FrameworkElement AnimationElement(AttachmentDto video, double width, double height)
    {
        var say = services.Say;
        var frame = new Grid
        {
            Width = width,
            Height = height,
            CornerRadius = new CornerRadius(8),
            Background = (Brush)Application.Current.Resources["ControlFillColorSecondaryBrush"],
        };
        var poster = new Image { Stretch = Stretch.UniformToFill };
        frame.Children.Add(poster);
        var surface = new MediaPlayerElement
        {
            AreTransportControlsEnabled = false,
            Stretch = Stretch.UniformToFill,
            IsHitTestVisible = false,
            Visibility = Visibility.Collapsed,
        };
        frame.Children.Add(surface);
        // The one word an animation says, in a corner: the same in every language.
        frame.Children.Add(new Border
        {
            Background = new SolidColorBrush(Windows.UI.Color.FromArgb(140, 0, 0, 0)),
            CornerRadius = new CornerRadius(4),
            Padding = new Thickness(5, 1, 5, 2),
            Margin = new Thickness(6),
            HorizontalAlignment = HorizontalAlignment.Left,
            VerticalAlignment = VerticalAlignment.Bottom,
            IsHitTestVisible = false,
            Child = new TextBlock
            {
                Text = say.Get("GIF"),
                FontSize = 11,
                FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
                Foreground = new SolidColorBrush(Microsoft.UI.Colors.White),
            },
        });
        AutomationProperties.SetName(frame, say.Get("GIF"));
        if (video.HasPreview)
        {
            _ = ShowPictureAsync(poster, video, preview: true);
        }
        var copy = new AnimationCopy(video, surface);
        frame.Loaded += (_, _) =>
        {
            if (gone)
            {
                return;
            }
            copy.Loaded = true;
            animationCopies.Add(video.Id, copy);
            DrawAnimation(video.Id);
        };
        frame.Unloaded += (_, _) =>
        {
            copy.Loaded = false;
            copy.InView = false;
            animationCopies.Remove(video.Id, copy);
            DrawAnimation(video.Id);
        };
        // Scrolled wholly out of view, it stops — unless another tile of it is still on the screen.
        frame.EffectiveViewportChanged += (_, args) =>
        {
            var seen = args.EffectiveViewport;
            var inView = RoundInline.InView(seen.X, seen.Y, seen.Width, seen.Height, width, height);
            if (inView != copy.InView)
            {
                copy.InView = inView;
                DrawAnimation(video.Id);
            }
        };
        frame.Tapped += (_, _) => OpenViewer([video], 0);
        return frame;
    }

    /// <summary>
    /// What an animation's tiles show now: its player on the newest one in view — fetched and made first when there is none —
    /// or, with none in view, nothing moving, and the player let go on the next turn if that is still so.
    /// </summary>
    private void DrawAnimation(long id)
    {
        if (gone)
        {
            return;
        }
        var copies = animationCopies.Of(id);
        var showing = copies.LastOrDefault(copy => copy.Loaded && copy.InView);
        foreach (var copy in copies.Where(copy => !ReferenceEquals(copy, showing)))
        {
            copy.Surface.Visibility = Visibility.Collapsed;
        }
        if (showing is null)
        {
            if (animationPlayers.ContainsKey(id))
            {
                // On the next turn: a redraw puts the new tile up before the old one's Unloaded has settled anything.
                DispatcherQueue.TryEnqueue(() =>
                {
                    if (!animationCopies.Of(id).Any(copy => copy.Loaded && copy.InView))
                    {
                        ReleaseAnimation(id);
                    }
                });
            }
            return;
        }
        if (!AnimationPrep.PlaysInTile(StickerImaging.AnimationsWanted(), showing.Video.Size))
        {
            // Still: its poster and its label. A click plays it in the viewer.
            return;
        }
        if (animationPlayers.TryGetValue(id, out var player))
        {
            Attach(showing, player);
            return;
        }
        _ = StartAnimationAsync(showing.Video);
    }

    private static void Attach(AnimationCopy copy, MediaPlayer player)
    {
        try
        {
            copy.Surface.SetMediaPlayer(player);
            copy.Surface.Visibility = Visibility.Visible;
            player.Play();
        }
        catch (Exception e)
        {
            // The poster stays: a still animation is still the message.
            Diagnostics.Write($"showing an animation: {e.GetType().Name}");
        }
    }

    /// <summary>
    /// The video fetched — the one a tile downloads to draw itself, within 8 MiB — and its player made: muted, looping, out of
    /// Windows' media controls, which a picture that moves has nothing to do with.
    /// </summary>
    private async Task StartAnimationAsync(AttachmentDto video)
    {
        if (!animationsLoading.Add(video.Id))
        {
            return;
        }
        try
        {
            var (bytes, error) = await connection.Attachments.BytesAsync(video, preview: false);
            if (gone)
            {
                return;
            }
            if (bytes is null)
            {
                if (error is not null)
                {
                    Diagnostics.Write($"an animation: {error.Code} {error.Status}");
                }
                return;
            }
            if (animationPlayers.ContainsKey(video.Id) || !animationCopies.Of(video.Id).Any(copy => copy.Loaded && copy.InView))
            {
                // Made meanwhile, or scrolled away while it came: nothing to start.
                DrawAnimation(video.Id);
                return;
            }
            var stream = new InMemoryRandomAccessStream();
            using (var writer = new DataWriter(stream))
            {
                writer.WriteBytes(bytes);
                await writer.StoreAsync();
                await writer.FlushAsync();
                writer.DetachStream();
            }
            stream.Seek(0);
            if (gone)
            {
                stream.Dispose();
                return;
            }
            var player = new MediaPlayer
            {
                IsLoopingEnabled = true,
                IsMuted = true,
                AutoPlay = false,
            };
            player.CommandManager.IsEnabled = false;
            player.Source = MediaSource.CreateFromStream(stream, video.Mime ?? "video/mp4");
            animationPlayers[video.Id] = player;
            DrawAnimation(video.Id);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"starting an animation: {e.GetType().Name}");
        }
        finally
        {
            animationsLoading.Remove(video.Id);
        }
    }

    /// <summary>An animation no tile shows any more: its player stopped and given back.</summary>
    private void ReleaseAnimation(long id)
    {
        if (!animationPlayers.Remove(id, out var player))
        {
            return;
        }
        foreach (var copy in animationCopies.Of(id))
        {
            copy.Surface.Visibility = Visibility.Collapsed;
        }
        try
        {
            player.Pause();
            var source = player.Source as MediaSource;
            player.Source = null;
            source?.Dispose();
            player.Dispose();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"letting an animation go: {e.GetType().Name}");
        }
    }

    /// <summary>The view is going: every animation stops.</summary>
    private void ReleaseAnimations()
    {
        foreach (var id in animationPlayers.Keys.ToList())
        {
            ReleaseAnimation(id);
        }
        animationCopies.Clear();
    }

    /// <summary>
    /// What a visible animation tile fetches — its poster, and its video where it would play by itself — fetched for a hidden
    /// row and drawn nowhere (docs/protocol.md, "A hidden row still fetches").
    /// </summary>
    private async Task FetchHiddenAnimationAsync(AttachmentDto video)
    {
        await FetchHiddenPosterAsync(video);
        try
        {
            if (AnimationPrep.PlaysInTile(StickerImaging.AnimationsWanted(), video.Size))
            {
                await connection.Attachments.BytesAsync(video, preview: false);
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"fetching a hidden animation: {e.GetType().Name}");
        }
    }

    /// <summary>
    /// The viewer's player as an animation asks — looping and muted — or back to an ordinary video's, its sound as it was
    /// before the animation muted it. Its controls are <see cref="QuietViewerVideo"/>'s to draw.
    /// </summary>
    private void PlayViewerAs(bool animation)
    {
        try
        {
            if (ViewerVideo.MediaPlayer is not { } player)
            {
                return;
            }
            player.IsLoopingEnabled = animation;
            if (animation)
            {
                viewerMutedBeforeAnimation ??= player.IsMuted;
                player.IsMuted = true;
            }
            else if (viewerMutedBeforeAnimation is { } before)
            {
                viewerMutedBeforeAnimation = null;
                player.IsMuted = before;
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"setting the viewer up for an animation: {e.GetType().Name}");
        }
    }

    /// <summary>Whether the viewer's player was muted before an animation muted it; null while no animation has.</summary>
    private bool? viewerMutedBeforeAnimation;
}
