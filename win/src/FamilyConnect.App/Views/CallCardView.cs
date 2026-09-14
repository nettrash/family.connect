using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace FamilyConnect.App.Views;

/// <summary>
/// The call on screen (web <c>views/call.rs</c>, ios CallView): who it is, where it has got to, and the three or four things
/// a person can do about it — a card over the corner of the window, not a screen of its own, because the chat behind a call
/// is worth keeping in view. On a video call it grows to hold the picture.
/// </summary>
/// <remarks>
/// <b>THE CARD IS HIDDEN, NEVER COLLAPSED.</b> The page carrying the media lives inside it, and a browser engine taken out of
/// the tree is one that stops — mid-ring, or mid-call.
/// </remarks>
internal sealed class CallCardView
{
    private readonly AppServices services;
    private readonly Border card;
    private readonly ContentControl face;
    private readonly TextBlock name;
    private readonly TextBlock status;
    private readonly Border picture;
    private readonly StackPanel actions;
    private readonly DispatcherQueueTimer clock;
    private Connection? connection;
    private CallEngine? engine;
    private AvatarFaces? faces;
    private string faceDrawn = string.Empty;
    private string actionsDrawn = string.Empty;

    public CallCardView(
        AppServices services, Border card, ContentControl face, TextBlock name, TextBlock status, Border picture, StackPanel actions, DispatcherQueue queue)
    {
        this.services = services;
        this.card = card;
        this.face = face;
        this.name = name;
        this.status = status;
        this.picture = picture;
        this.actions = actions;
        // The duration is a clock: a second's worth of redraw of its own, while there is a conversation to time.
        clock = queue.CreateTimer();
        clock.Interval = TimeSpan.FromSeconds(1);
        clock.Tick += (_, _) => ShowStatus();
        Draw();
    }

    /// <summary>The connection whose calls this card shows — or none, while there is no server.</summary>
    public void Attach(Connection? connection, CallEngine? engine)
    {
        this.connection = connection;
        this.engine = engine;
        faces = connection is null ? null : new AvatarFaces(connection);
        faceDrawn = string.Empty;
        actionsDrawn = string.Empty;
        Draw();
    }

    public void Draw()
    {
        if (engine is not { Call: { } call } calls || connection is null)
        {
            card.Opacity = 0;
            card.IsHitTestVisible = false;
            AutomationProperties.SetAccessibilityView(card, AccessibilityView.Raw);
            card.Width = 360;
            picture.Height = 1;
            actions.Children.Clear();
            actionsDrawn = string.Empty;
            face.Content = null;
            faceDrawn = string.Empty;
            name.Text = string.Empty;
            status.Text = string.Empty;
            clock.Stop();
            return;
        }
        var say = services.Say;
        card.Opacity = 1;
        card.IsHitTestVisible = true;
        AutomationProperties.SetAccessibilityView(card, AccessibilityView.Content);
        var member = connection.Chats.Member(call.PeerUserId);
        var who = member?.DisplayName is { Length: > 0 } display ? display : say.Get("Someone");
        name.Text = who;
        AutomationProperties.SetName(card, say.Format("Call with %@", who));
        var faceKey = $"{call.PeerUserId}/{member?.AvatarVersion}/{who}";
        if (faceKey != faceDrawn && faces is not null)
        {
            faceDrawn = faceKey;
            face.Content = faces.Face(who, false, call.PeerUserId, member?.AvatarVersion ?? 0, 44);
        }
        ShowStatus();

        // The picture only once there is one: a voice call has nothing to look at, and its audio plays all the same.
        var picturing = call.Video && call.Stage is CallStage.Connecting or CallStage.Talking;
        picture.Height = picturing ? 270 : 1;
        card.Width = picturing ? 480 : 360;

        var actionsKey = $"{call.CallId}/{call.Stage}/{call.Muted}/{call.Camera}/{call.Taken}";
        if (actionsKey != actionsDrawn)
        {
            actionsDrawn = actionsKey;
            actions.Children.Clear();
            if (call.Stage == CallStage.Incoming)
            {
                actions.Children.Add(Act(say.Get("Decline"), calls.Decline, danger: true));
                var accept = Act(say.Get("Accept"), () => _ = calls.AnswerAsync(), accent: true);
                // A ringing call takes the focus, on Accept: Enter answers it, and a screen reader reads the call.
                accept.Loaded += (_, _) => accept.Focus(FocusState.Programmatic);
                actions.Children.Add(accept);
            }
            else if (call.Stage != CallStage.Ended)
            {
                // Each button is named for what it DOES, so it needs no pressed state to be read correctly.
                actions.Children.Add(Act(call.Muted ? say.Get("Unmute") : say.Get("Mute"), calls.ToggleMute));
                if (call.Video)
                {
                    actions.Children.Add(Act(call.Camera ? say.Get("Turn camera off") : say.Get("Turn camera on"), calls.ToggleCamera));
                }
                actions.Children.Add(Act(call.Taken ? say.Get("Hang Up") : say.Get("Cancel"), calls.End, danger: true));
            }
        }
        if (call.Stage == CallStage.Talking)
        {
            clock.Start();
        }
        else
        {
            clock.Stop();
        }
    }

    private void ShowStatus()
    {
        if (engine?.Call is { } call)
        {
            status.Text = CallText.StatusLine(call, DateTimeOffset.UtcNow, services.Say);
        }
    }

    private static Button Act(string text, Action act, bool accent = false, bool danger = false)
    {
        var resources = Application.Current.Resources;
        var button = new Button { Content = text, MinWidth = 88 };
        if (accent)
        {
            button.Style = (Style)resources["AccentButtonStyle"];
        }
        if (danger)
        {
            // Red in every state, as ending a call is on every platform — the default hover grey would read as a different button.
            var red = (Brush)resources["SystemFillColorCriticalBrush"];
            var ink = (Brush)resources["TextOnAccentFillColorPrimaryBrush"];
            foreach (var key in new[] { "ButtonBackground", "ButtonBackgroundPointerOver", "ButtonBackgroundPressed" })
            {
                button.Resources[key] = red;
            }
            foreach (var key in new[] { "ButtonForeground", "ButtonForegroundPointerOver", "ButtonForegroundPressed" })
            {
                button.Resources[key] = ink;
            }
        }
        button.Click += (_, _) => act();
        return button;
    }
}
