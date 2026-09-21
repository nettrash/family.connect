using System.Globalization;
using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Storage.Pickers;
using Windows.Storage.Streams;
using static FamilyConnect.App.Views.Dialogs;

namespace FamilyConnect.App.Views;

/// <summary>
/// Settings: who is signed in, their picture, birthday and password, the family they are in and the
/// way out of it, the family's numbers, notifications, and the account itself — the web client's
/// settings pane, which is the Mac's.
/// </summary>
/// <remarks>
/// <para>
/// <b>LEAVING IS BUILT FROM A FRESH READ, NEVER THE ROSTER HELD.</b> Who inherits, and whether anybody
/// is left to, may have changed since; and a read that FAILED is not a family with nobody left in it —
/// "leaving deletes the family" is said only when the server said so (<see cref="LeaveContext.For"/>).
/// A failed leave does not show the dialog again: what it said was read before the attempt.
/// </para>
/// <para>
/// <b>THE SESSION IS THE STATE.</b> A change that lands (a picture, a birthday) is followed by a
/// <c>GET /me</c>, and the screen redraws from what the session then says rather than from what this
/// screen believes it did.
/// </para>
/// </remarks>
public sealed partial class SettingsView : UserControl
{
    private const string PrivacyUrl = "https://nettrash.me/appstore/familyconnect/privacy.html";
    private const string SupportUrl = "https://nettrash.me/appstore/familyconnect/support.html";

    private static readonly string[] PictureTypes =
        [".jpg", ".jpeg", ".png", ".heic", ".heif", ".bmp", ".gif", ".tif", ".tiff", ".webp"];

    private readonly AppServices services;
    private readonly Connection connection;
    private readonly FamilyModel family;
    private readonly Action<SessionState> onSession;
    private bool pictureBusy;
    private bool leaving;
    private bool drawingSwitch;
    private bool consentBusy;
    private string pictureShown = string.Empty;

    internal SettingsView(AppServices services, Connection connection, Action close)
    {
        this.services = services;
        this.connection = connection;
        family = new FamilyModel(connection.Api, connection.Chats);
        InitializeComponent();
        var say = services.Say;

        Heading.Text = say.Get("Settings");
        DoneButton.Content = say.Get("Done");
        OwnerText.Text = say.Get("Owner");
        ProfileHeading.Text = say.Get("Profile");
        PhotoLabel.Text = say.Get("Photo");
        RemovePhotoButton.Content = say.Get("Remove Photo");
        BirthdayLabel.Text = say.Get("Birthday");
        FamilyHeading.Text = say.Get("Family");
        FamilyNameLabel.Text = say.Get("Name");
        StatisticsHeading.Text = say.Get("Statistics");
        NotificationsHeading.Text = say.Get("Notifications");
        NotifyTitle.Text = say.Get("Tell me when a message arrives");
        KeepRunningFootnote.Text = say.Get("Family Connect stays in the notification area, so messages and calls still reach you. Quit it from its icon there.");
        PrivacyHeading.Text = say.Get("Privacy");
        ServerHeading.Text = say.Get("Server");
        ServerAddressLabel.Text = say.Get("Address");
        ServerAddressValue.Text = connection.Server.AbsoluteUri;
        LinkPreviewTitle.Text = say.Get("Link Previews");
        LinkPreviewFootnote.Text = say.Get("Shows a preview under links in messages. Building one asks the linked website for its title and image, so that site sees a request from this device.");
        MapPreviewFootnote.Text = say.Get("Shows a map on a shared location. Drawing one asks OpenStreetMap for the map around that place, so OpenStreetMap sees a request from this device. With maps off, a shared location still shows its pin and opens in a map when you click it.");
        PrivacyLink.NavigateUri = new Uri(PrivacyUrl);
        SupportLink.NavigateUri = new Uri(SupportUrl);
        // Rows drawn as a glyph, words and a chevron: the words go on the row's text, and are still the control's name to a
        // screen reader, which would otherwise read a button made of shapes as nothing.
        foreach (var (control, text, words) in new (Control, TextBlock, string)[]
        {
            (PasswordButton, PasswordText, say.Get("Change Password…")),
            (LeaveButton, LeaveText, say.Get("Leave Family")),
            (StatisticsButton, StatisticsText, say.Get("Statistics…")),
            (PrivacyLink, PrivacyText, say.Get("Privacy Policy")),
            (SupportLink, SupportText, say.Get("Support")),
            (LogOutButton, LogOutText, say.Get("Log Out")),
            (DeleteButton, DeleteText, say.Get("Delete Account…")),
            (NotifySwitch, NotifyTitle, NotifyTitle.Text),
            (KeepRunningSwitch, KeepRunningTitle, say.Get("Keep running when the window is closed")),
            (LinkPreviewSwitch, LinkPreviewTitle, LinkPreviewTitle.Text),
            (MapPreviewSwitch, MapPreviewTitle, say.Get("Map Previews")),
        })
        {
            text.Text = words;
            AutomationProperties.SetName(control, words);
        }
        // The product's name is the same in every language; the sentence around it is not.
        VersionText.Text = say.Format("Family Connect for Windows %@", AppVersion());

        DoneButton.Click += (_, _) => close();
        PhotoButton.Click += (_, _) => _ = PickPictureAsync();
        RemovePhotoButton.Click += (_, _) => _ = RemovePictureAsync();
        BirthdayButton.Click += (_, _) => _ = BirthdayAsync();
        PasswordButton.Click += (_, _) => _ = ChangePasswordAsync();
        LeaveButton.Click += (_, _) => _ = LeaveAsync();
        StatisticsButton.Click += (_, _) => _ = StatisticsAsync();
        AssistantConsentButton.Click += (_, _) => _ = AssistantConsentAsync();
        NotifySwitch.Toggled += (_, _) =>
        {
            if (!drawingSwitch && Toasts.Available)
            {
                NotifySetting.Wanted = NotifySwitch.IsOn;
            }
        };
        LinkPreviewSwitch.Toggled += (_, _) =>
        {
            if (!drawingSwitch)
            {
                LinkPreviewSetting.Enabled = LinkPreviewSwitch.IsOn;
            }
        };
        KeepRunningSwitch.Toggled += (_, _) =>
        {
            if (!drawingSwitch)
            {
                KeepRunningSetting.Enabled = KeepRunningSwitch.IsOn;
            }
        };
        MapPreviewSwitch.Toggled += (_, _) =>
        {
            if (!drawingSwitch)
            {
                MapPreviewSetting.Enabled = MapPreviewSwitch.IsOn;
            }
        };
        LogOutButton.Click += (_, _) => _ = LogOutAsync();
        DeleteButton.Click += (_, _) => _ = DeleteAccountAsync();

        onSession = _ => DispatcherQueue.TryEnqueue(Draw);
        connection.Session.Changed += onSession;
        Unloaded += (_, _) => connection.Session.Changed -= onSession;
        Draw();
    }

    private void Draw()
    {
        var say = services.Say;
        var state = connection.Session.State;
        if (state.Me is not { } me)
        {
            return;
        }
        NameText.Text = me.DisplayName;
        UsernameText.Text = $"@{me.Username}";
        OwnerCapsule.Visibility = state.IsOwner ? Visibility.Visible : Visibility.Collapsed;

        var hasPicture = me.AvatarVersion > 0;
        PhotoButton.Content = hasPicture ? say.Get("Change Photo") : say.Get("Add Photo");
        PhotoButton.IsEnabled = !pictureBusy;
        RemovePhotoButton.Visibility = hasPicture ? Visibility.Visible : Visibility.Collapsed;
        RemovePhotoButton.IsEnabled = !pictureBusy;
        BirthdayValue.Text = BirthdayRules.Text(me.Birthday, say, services.Culture);
        BirthdayButton.Content = me.Birthday is null ? say.Get("Add Birthday…") : say.Get("Change Birthday…");

        FamilySection.Visibility = state.Family is null ? Visibility.Collapsed : Visibility.Visible;
        FamilyNameValue.Text = state.Family?.Name ?? string.Empty;
        LeaveButton.IsEnabled = !leaving;

        drawingSwitch = true;
        NotifySwitch.IsEnabled = Toasts.Available;
        NotifySwitch.IsOn = Toasts.Available && NotifySetting.Wanted;
        LinkPreviewSwitch.IsOn = LinkPreviewSetting.Enabled;
        KeepRunningSwitch.IsOn = KeepRunningSetting.Enabled;
        MapPreviewSwitch.IsOn = MapPreviewSetting.Enabled;
        drawingSwitch = false;
        NotifyFootnote.Text = Toasts.Available
            ? say.Get("While this window is not in front, a notification says who wrote — never what they wrote.")
            : say.Get("Windows is not showing notifications for Family Connect. Allow them in Windows Settings, under Notifications.");

        DrawAssistantConsent(state);

        Picture.DisplayName = me.DisplayName;
        _ = ShowPictureAsync(me);
    }

    /// <summary>
    /// The assistant question, as this screen shows it (docs/protocol.md, "Consenting to the
    /// assistant"): when they agreed and the way to stop, or what has not been agreed to yet and
    /// the screen that asks.
    /// </summary>
    /// <remarks>
    /// The whole group is COLLAPSED where this server named no processor — there is nothing to
    /// have agreed to, and a row about it would be a setting for a feature that does not exist
    /// here. Stopping is one press and no dialog: agreeing has a screen to read first, and
    /// stopping has nothing to say beyond what it cannot undo, which the footnote already says.
    /// </remarks>
    private void DrawAssistantConsent(SessionState state)
    {
        var say = services.Say;
        var processor = state.Assistant?.Processor;
        if (!AssistantConsent.IsAvailable(processor) || processor is null)
        {
            AssistantHeading.Visibility = Visibility.Collapsed;
            AssistantPanel.Visibility = Visibility.Collapsed;
            return;
        }
        AssistantHeading.Text = say.Get("Assistant");
        AssistantHeading.Visibility = Visibility.Visible;
        AssistantPanel.Visibility = Visibility.Visible;
        var agreed = !string.IsNullOrWhiteSpace(state.AssistantConsentAt);
        if (agreed)
        {
            AssistantConsentTitle.Text = say.Get("Agreed");
            AssistantConsentFootnote.Text = say.Format(
                "What you write to the assistant is sent to %@. Stopping takes effect at once; what has already been sent cannot be taken back.",
                processor);
            AssistantConsentAction.Text = say.Get("Stop Sending My Messages");
        }
        else
        {
            AssistantConsentTitle.Text = say.Get("The Assistant");
            AssistantConsentFootnote.Text = say.Format(
                "Until you agree, nothing you write is sent to %@ and the assistant does not answer you.",
                processor);
            AssistantConsentAction.Text = say.Get("Review and Agree…");
        }
        AutomationProperties.SetName(AssistantConsentButton, AssistantConsentAction.Text);
        AssistantConsentButton.IsEnabled = !consentBusy;
    }

    /// <summary>
    /// Answer the question, or take the answer back. The session is the state: the write is
    /// followed by a <c>GET /me</c>, and the screen redraws from what the server then says.
    /// </summary>
    private async Task AssistantConsentAsync()
    {
        var state = connection.Session.State;
        if (state.Assistant?.Processor is not { } processor || string.IsNullOrWhiteSpace(processor))
        {
            return;
        }
        var agreed = !string.IsNullOrWhiteSpace(state.AssistantConsentAt);
        if (!agreed && !await Dialogs.AssistantConsentAsync(
            XamlRoot, services.Say, processor,
            state.Family?.AiHistory == true, state.Family?.AiVision == true))
        {
            return;
        }
        consentBusy = true;
        Draw();
        var answer = await connection.Api.SetAssistantConsent(!agreed);
        consentBusy = false;
        if (answer.Ok)
        {
            await connection.Session.RefreshAsync();
        }
        Draw();
        if (!answer.Ok)
        {
            await Dialogs.ConfirmAsync(
                XamlRoot, services.Say, services.Say.Get("Couldn't save your answer. Try again."),
                null, services.Say.Get("OK"));
        }
    }

    // ---- the picture ---------------------------------------------------------------------------

    private async Task ShowPictureAsync(UserDto me)
    {
        var key = string.Create(CultureInfo.InvariantCulture, $"{me.Id}-{me.AvatarVersion}");
        if (key == pictureShown)
        {
            return;
        }
        pictureShown = key;
        if (me.AvatarVersion <= 0)
        {
            Picture.ProfilePicture = null;
            return;
        }
        try
        {
            var (bytes, _) = await connection.Avatars.BytesAsync(me.Id, me.AvatarVersion);
            if (bytes is null)
            {
                // Nothing to draw yet: the next redraw asks again.
                pictureShown = string.Empty;
                return;
            }
            if (pictureShown == key)
            {
                Picture.ProfilePicture = await BitmapAsync(bytes);
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"drawing the profile picture: {e.GetType().Name}");
            pictureShown = string.Empty;
        }
    }

    private async Task PickPictureAsync()
    {
        if (pictureBusy)
        {
            return;
        }
        var say = services.Say;
        Windows.Storage.StorageFile? file;
        try
        {
            var picker = new FileOpenPicker
            {
                SuggestedStartLocation = PickerLocationId.PicturesLibrary,
                ViewMode = PickerViewMode.Thumbnail,
            };
            foreach (var type in PictureTypes)
            {
                picker.FileTypeFilter.Add(type);
            }
            WinRT.Interop.InitializeWithWindow.Initialize(picker, services.WindowHandle);
            file = await picker.PickSingleFileAsync();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"picking a profile picture: {e.GetType().Name}");
            ShowProblem(PhotoError, say.Get("Something went wrong. Try again."));
            return;
        }
        if (file is null)
        {
            return;
        }
        await PictureWorkAsync(uploading: true, async () =>
        {
            if (await MediaPreparing.AvatarAsync(file) is not { } jpeg)
            {
                return say.Get("Couldn't read that photo.");
            }
            var answer = await connection.Api.PutAvatar(jpeg, AvatarRules.SendAs);
            return answer.Ok ? null : SettingsText.PictureFailure(answer.Error ?? ApiError.Transport("no answer"), true, say);
        });
    }

    private Task RemovePictureAsync() =>
        PictureWorkAsync(uploading: false, async () =>
        {
            var answer = await connection.Api.DeleteAvatar();
            return answer.Ok
                ? null
                : SettingsText.PictureFailure(answer.Error ?? ApiError.Transport("no answer"), false, services.Say);
        });

    /// <summary>One change to the picture at a time; the session is read again after one that landed.</summary>
    private async Task PictureWorkAsync(bool uploading, Func<Task<string?>> work)
    {
        if (pictureBusy)
        {
            return;
        }
        pictureBusy = true;
        PhotoError.Visibility = Visibility.Collapsed;
        Draw();
        try
        {
            if (await work() is { } problem)
            {
                ShowProblem(PhotoError, problem);
                return;
            }
            await connection.Session.RefreshAsync();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"changing the profile picture: {e.GetType().Name}");
            ShowProblem(PhotoError, SettingsText.PictureFailure(ApiError.Transport(e.GetType().Name), uploading, services.Say));
        }
        finally
        {
            pictureBusy = false;
            Draw();
        }
    }

    // ---- birthday and password -----------------------------------------------------------------

    private async Task BirthdayAsync()
    {
        var changed = await Dialogs.BirthdayAsync(
            XamlRoot, services, connection.Session.State.Me?.Birthday, name: null,
            async (month, day) => (await connection.Api.SetMyBirthday(month, day)).Error,
            async () => (await connection.Api.ClearMyBirthday()).Error);
        if (changed)
        {
            await connection.Session.RefreshAsync();
        }
    }

    private async Task ChangePasswordAsync()
    {
        var say = services.Say;
        var current = new PasswordBox { Header = say.Get("Current Password") };
        var fresh = new PasswordBox { Header = say.Get("New Password") };
        var again = new PasswordBox { Header = say.Get("Confirm New Password") };
        var problem = Problem();
        var dialog = Dialogs.Create(XamlRoot, 
            say.Get("Change Password"),
            Column(
                current,
                Footnote(say.Get("Your other devices will be signed out. This one stays signed in.")),
                fresh,
                again,
                problem));
        dialog.PrimaryButtonText = say.Get("Save");
        dialog.CloseButtonText = say.Get("Cancel");
        dialog.IsPrimaryButtonEnabled = false;
        void Enable() => dialog.IsPrimaryButtonEnabled = current.Password.Length > 0 && fresh.Password.Length > 0;
        current.PasswordChanged += (_, _) => Enable();
        fresh.PasswordChanged += (_, _) => Enable();
        dialog.PrimaryButtonClick += async (_, args) =>
        {
            if (SettingsText.PasswordProblem(fresh.Password, again.Password, say) is { } wrong)
            {
                args.Cancel = true;
                ShowProblem(problem, wrong);
                return;
            }
            await SettleAsync(
                args, problem,
                () => connection.Session.ChangePasswordAsync(current.Password, fresh.Password),
                error => SettingsText.PasswordChangeFailure(error, say));
        };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary)
        {
            return;
        }
        var done = Dialogs.Create(XamlRoot, say.Get("Password changed"), Text(say.Get("Your other devices have been signed out.")));
        done.CloseButtonText = say.Get("OK");
        done.DefaultButton = ContentDialogButton.Close;
        await done.ShowAsync();
    }

    // ---- the family ----------------------------------------------------------------------------

    private async Task LeaveAsync()
    {
        if (leaving)
        {
            return;
        }
        var say = services.Say;
        leaving = true;
        LeaveButton.IsEnabled = false;
        LeaveError.Visibility = Visibility.Collapsed;
        try
        {
            var (deletes, successor, error) = await family.LeaveWouldAsync();
            var context = error is null
                ? LeaveContext.For(connection.Session.State.IsOwner, deletes, successor)
                : null;
            if (context is null)
            {
                ShowProblem(LeaveError, SettingsText.LeaveFailed(say));
                return;
            }
            var dialog = Dialogs.Create(XamlRoot, SettingsText.LeaveTitle(context, say), Text(SettingsText.LeaveMessage(context, say)));
            dialog.PrimaryButtonText = SettingsText.LeaveButton(context, say);
            dialog.CloseButtonText = say.Get("Cancel");
            dialog.DefaultButton = ContentDialogButton.Close;
            if (await dialog.ShowAsync() != ContentDialogResult.Primary)
            {
                return;
            }
            var (chosen, failed) = await family.LeaveAsync();
            if (failed is not null)
            {
                ShowProblem(LeaveError, SettingsText.LeaveFailed(say));
                return;
            }
            // Who inherits is named from the roster as it stood BEFORE the gate moves — afterwards there is none to read.
            if (chosen is { } heirId && connection.Chats.Member(heirId)?.DisplayName is { Length: > 0 } heir)
            {
                var (title, message) = SettingsText.OwnershipPassed(heir, say);
                var notice = Dialogs.Create(XamlRoot, title, Text(message));
                notice.CloseButtonText = say.Get("OK");
                notice.DefaultButton = ContentDialogButton.Close;
                await notice.ShowAsync();
            }
            // The gate moves on what `GET /me` now says: no family, and the window follows it.
            await connection.Session.RefreshAsync();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"leaving the family: {e.GetType().Name}");
            ShowProblem(LeaveError, SettingsText.LeaveFailed(say));
        }
        finally
        {
            leaving = false;
            LeaveButton.IsEnabled = true;
        }
    }

    /// <summary>The family's numbers, read afresh on every opening — and the rows never added up into the totals.</summary>
    private async Task StatisticsAsync()
    {
        var say = services.Say;
        var body = new StackPanel { Spacing = 16, MinWidth = 380 };
        var dialog = Dialogs.Create(XamlRoot, 
            say.Get("Statistics"),
            new ScrollViewer { Content = body, MaxHeight = 560, Padding = new Thickness(0, 0, 16, 0) });
        dialog.CloseButtonText = say.Get("Done");
        dialog.DefaultButton = ContentDialogButton.Close;
        _ = LoadStatisticsAsync(body);
        await dialog.ShowAsync();
    }

    private async Task LoadStatisticsAsync(StackPanel body)
    {
        var say = services.Say;
        var culture = services.Culture;
        body.Children.Clear();
        body.Children.Add(new ProgressRing { IsActive = true, HorizontalAlignment = HorizontalAlignment.Center });
        StatsResponse? stats;
        try
        {
            (stats, _) = await family.StatsAsync();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading statistics: {e.GetType().Name}");
            stats = null;
        }
        body.Children.Clear();
        if (stats is null)
        {
            body.Children.Add(new TextBlock { Text = say.Get("Couldn't load statistics"), FontWeight = FontWeights.SemiBold });
            body.Children.Add(Footnote(say.Get("Check your connection and try again.")));
            var retry = new Button { Content = say.Get("Retry") };
            retry.Click += (_, _) => _ = LoadStatisticsAsync(body);
            body.Children.Add(retry);
            return;
        }
        string Number(long value) => value.ToString("N0", culture);
        var totals = stats.Totals;
        body.Children.Add(Group(
            say.Get("The family"),
            Row(say.Get("Members"), Number(totals.Members)),
            Row(say.Get("Messages"), Number(totals.Messages)),
            Row(say.Get("Board notes"), Number(totals.BoardNotes))));

        var files = totals.Attachments;
        var sent = Math.Max(0, files?.Bytes ?? 0);
        var rows = new List<UIElement>
        {
            Row(say.Get("Photos"), Number(files?.Photo ?? 0)),
            Row(say.Get("Videos"), Number(files?.Video ?? 0)),
            Row(say.Get("Audio"), Number(files?.Audio ?? 0)),
            Row(say.Get("Files"), Number(files?.File ?? 0)),
            Row(say.Get("Locations"), Number(files?.Location ?? 0)),
            Row(say.Get("Sent"), MediaText.DisplaySize(sent, say, culture)),
        };
        if (files?.StoredBytes is { } stored)
        {
            rows.Add(Row(say.Get("On disk"), MediaText.DisplaySize(Math.Max(0, stored), say, culture)));
        }
        if (SettingsText.Saved(files) is { } saved)
        {
            rows.Add(Footnote(say.Format("%@ saved by storing one copy of identical files.", MediaText.DisplaySize(saved, say, culture))));
        }
        body.Children.Add(Group(say.Get("Attachments"), [.. rows]));

        if (totals.Ai is { } ai && (ai.Questions > 0 || ai.Images > 0))
        {
            var assistant = new List<UIElement>
            {
                Row(say.Get("Questions"), Number(ai.Questions)),
                Row(say.Get("Tokens"), Number(ai.PromptTokens + ai.CompletionTokens)),
            };
            if (ai.Images > 0)
            {
                assistant.Add(Row(say.Get("Pictures"), Number(ai.Images)));
            }
            body.Children.Add(Group(say.Get("Assistant"), [.. assistant]));
        }

        var who = new List<UIElement>();
        foreach (var member in stats.Members ?? [])
        {
            var words = new StackPanel();
            words.Children.Add(new TextBlock { Text = member.DisplayName, FontWeight = FontWeights.SemiBold });
            words.Children.Add(Footnote(SettingsText.MemberLine(member, say, culture)));
            who.Add(Row(words, Number(member.Messages)));
        }
        body.Children.Add(Group(say.Get("Who sends what"), [.. who]));
    }

    // ---- the account ---------------------------------------------------------------------------

    private async Task LogOutAsync()
    {
        var say = services.Say;
        var message = say.Get("Messages stay on the family server; this device forgets its session.");
        if (connection.Outbox.All().Any())
        {
            // Two sentences, each its own key: the second is only ever added to the first.
            message = $"{message} {say.Get("What hasn't been sent yet is lost.")}";
        }
        var dialog = Dialogs.Create(XamlRoot, say.Get("Log out?"), Text(message));
        dialog.PrimaryButtonText = say.Get("Log Out");
        dialog.CloseButtonText = say.Get("Cancel");
        dialog.DefaultButton = ContentDialogButton.Close;
        if (await dialog.ShowAsync() == ContentDialogResult.Primary)
        {
            await connection.Session.SignOutAsync();
        }
    }

    /// <summary>
    /// Deleting the account: what happens, said before it does; the password, because being signed
    /// in is not proof; and one last question. Cancelling that question goes back to the form.
    /// </summary>
    private async Task DeleteAccountAsync()
    {
        var say = services.Say;
        var password = new PasswordBox { Header = say.Get("Password") };
        var problem = Problem();
        var content = new StackPanel { Spacing = 8, MaxWidth = 480 };
        content.Children.Add(new TextBlock { Text = say.Get("What happens"), FontWeight = FontWeights.SemiBold });
        var consequences = new List<string>
        {
            say.Get("Your account, password, profile picture and birthday are deleted, and every device you are signed in on is signed out."),
            say.Get("Your direct chats are deleted — for the other person too. So is your private chat with the assistant."),
            say.Get("Your messages in the family chat, your board notes and your reactions stay. They are shown from then on as “Deleted account”."),
        };
        if (connection.Session.State.IsOwner)
        {
            consequences.Add(say.Get("You own this family: ownership passes to the longest-standing remaining member. If you are its last member, the family is deleted with you — its chat, its board and its invite code."));
        }
        foreach (var consequence in consequences)
        {
            content.Children.Add(Text($"• {consequence}"));
        }
        content.Children.Add(Footnote(say.Get("There is no grace period and no way to cancel afterwards.")));
        content.Children.Add(password);
        content.Children.Add(Footnote(say.Get("Type your password to confirm it is you. Being signed in is not proof.")));
        content.Children.Add(problem);

        var dialog = Dialogs.Create(XamlRoot, say.Get("Delete Account"), new ScrollViewer { Content = content, MaxHeight = 560 });
        dialog.PrimaryButtonText = say.Get("Delete");
        dialog.CloseButtonText = say.Get("Cancel");
        dialog.DefaultButton = ContentDialogButton.Close;
        dialog.IsPrimaryButtonEnabled = false;
        password.PasswordChanged += (_, _) => dialog.IsPrimaryButtonEnabled = password.Password.Length > 0;

        while (await dialog.ShowAsync() == ContentDialogResult.Primary)
        {
            var sure = Dialogs.Create(XamlRoot, say.Get("Delete your account?"), Text(say.Get("This happens immediately and cannot be undone.")));
            sure.PrimaryButtonText = say.Get("Delete Account");
            sure.CloseButtonText = say.Get("Cancel");
            sure.DefaultButton = ContentDialogButton.Close;
            if (await sure.ShowAsync() != ContentDialogResult.Primary)
            {
                continue;
            }
            ApiError? error;
            try
            {
                error = await connection.Session.DeleteAccountAsync(password.Password);
            }
            catch (Exception e)
            {
                Diagnostics.Write($"deleting the account: {e.GetType().Name}");
                error = ApiError.Transport(e.GetType().Name);
            }
            if (error is null)
            {
                // The session has ended, and the window has already moved to the sign-in screen.
                return;
            }
            ShowProblem(problem, SettingsText.DeleteFailure(error, say));
        }
    }

    /// <summary>The package's version, or the assembly's when the app runs unpackaged.</summary>
    /// <summary>
    /// "1.1 (437)", or "1.1" for a build nobody stamped — the shape and the rule live in
    /// <see cref="AppVersionText"/>, with the reasons and the tests.
    /// </summary>
    /// <remarks>
    /// The package's own identity first, because that is what Windows installed and what Partner
    /// Center shows; the assembly version is the fallback for an unpackaged run, where
    /// <c>Package.Current</c> throws.
    /// </remarks>
    private static string AppVersion()
    {
        try
        {
            var version = Windows.ApplicationModel.Package.Current.Id.Version;
            return AppVersionText.For(version.Major, version.Minor, version.Build);
        }
        catch (Exception)
        {
            var assembly = typeof(SettingsView).Assembly.GetName().Version;
            return assembly is null
                ? "1.1"
                : AppVersionText.For(assembly.Major, assembly.Minor, assembly.Build);
        }
    }
}
