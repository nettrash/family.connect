using System.Globalization;
using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using FamilyConnect.Core.Protocol;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.ApplicationModel.DataTransfer;
using static FamilyConnect.App.Views.Dialogs;

namespace FamilyConnect.App.Views;

/// <summary>
/// The family: who is in it, with a way to message, report or block each — and, for its owner, the
/// invite code, the join requests and the report inbox, and a birthday, a password reset and a
/// removal on each member (the web client's family pane; the house rules are the next screen's).
/// </summary>
/// <remarks>
/// <para>
/// <b>A REMOVAL IS ASKED ABOUT FIRST</b> — the iPhone asks and the Mac does not, and the web and this
/// client side with the iPhone. A member the reader has blocked is not offered "Message", which would
/// only be refused.
/// </para>
/// <para>
/// <b>A FULL FAMILY LEAVES A REQUEST WAITING.</b> <c>family_full</c> on an approval is a condition and
/// not a decision: the row stays, and the owner is told what would let it in.
/// </para>
/// <para>
/// <b>A JOIN REQUEST IS DRAWN WITH INITIALS ONLY.</b> The server shows a stranger's picture to nobody,
/// and a refusal cached now would outlive the approval.
/// </para>
/// </remarks>
public sealed partial class FamilyView : UserControl
{
    private readonly AppServices services;
    private readonly Connection connection;
    private readonly FamilyModel family;
    private readonly Action<long> openChat;
    private readonly Action onRoster;
    private readonly Action<long, bool> onBlock;
    private readonly Action<SessionState> onSession;
    private readonly Action<Resync.Report> onResync;
    private readonly Dictionary<string, BitmapImage> faces = [];
    private IReadOnlyList<JoinRequestDto> requests = [];
    private IReadOnlyList<ReportDto> reports = [];
    private bool busy;
    private long? deciding;
    private long? resolving;
    private int redrawQueued;

    internal FamilyView(AppServices services, Connection connection, Action close, Action<long> openChat)
    {
        this.services = services;
        this.connection = connection;
        this.openChat = openChat;
        family = new FamilyModel(connection.Api, connection.Chats);
        InitializeComponent();
        var say = services.Say;

        Heading.Text = say.Get("Family");
        DoneButton.Content = say.Get("Done");
        InviteHeading.Text = say.Get("Invite code");
        CopyButton.Content = say.Get("Copy");
        RotateButton.Content = say.Get("Rotate");
        InviteFootnote.Text = say.Get("Rotating invalidates the current code immediately.");
        RequestsHeading.Text = say.Get("Join requests");
        ReportsHeading.Text = say.Get("Reports");
        ReportsEmpty.Text = say.Get("Members can report a message or a person to you.");
        MembersHeading.Text = say.Get("Members");

        DoneButton.Click += (_, _) => close();
        CopyButton.Click += (_, _) =>
        {
            if (connection.Session.State.Family?.InviteCode is { } code)
            {
                var package = new DataPackage();
                package.SetText(code);
                Clipboard.SetContent(package);
            }
        };
        RotateButton.Click += (_, _) => _ = RotateAsync();

        onRoster = QueueRedraw;
        onBlock = (_, _) => QueueRedraw();
        onSession = _ => QueueRedraw();
        onResync = report => DispatcherQueue.TryEnqueue(() => _ = LoadOwnerListsAsync());
        connection.Router.RosterChanged += onRoster;
        connection.Router.BlockChanged += onBlock;
        connection.Session.Changed += onSession;
        connection.Live.Resynced += onResync;
        Unloaded += (_, _) =>
        {
            connection.Router.RosterChanged -= onRoster;
            connection.Router.BlockChanged -= onBlock;
            connection.Session.Changed -= onSession;
            connection.Live.Resynced -= onResync;
        };
        Draw();
        _ = LoadOwnerListsAsync();
    }

    /// <summary>A frame lands on the socket's thread: one redraw, on this one, however many arrive.</summary>
    private void QueueRedraw()
    {
        if (Interlocked.Exchange(ref redrawQueued, 1) == 1)
        {
            return;
        }
        DispatcherQueue.TryEnqueue(() =>
        {
            Interlocked.Exchange(ref redrawQueued, 0);
            Draw();
        });
    }

    private void Draw()
    {
        var say = services.Say;
        var state = connection.Session.State;
        if (state.Family is not { } house || state.Me is not { } me)
        {
            return;
        }
        FamilyNameText.Text = house.Name;
        FamilyPicture.DisplayName = house.Name;
        var roster = FamilyText.Roster(family.Present());
        MembersText.Text = FamilyText.MembersLine(roster.Count, say);

        var owner = state.IsOwner;
        OwnerSections.Visibility = owner ? Visibility.Visible : Visibility.Collapsed;
        if (owner)
        {
            InviteCodeText.Text = house.InviteCode ?? "…";
            CopyButton.IsEnabled = house.InviteCode is not null;
            RotateButton.IsEnabled = !busy;
            DrawRequests();
            DrawReports();
        }

        MembersList.Children.Clear();
        foreach (var member in roster)
        {
            MembersList.Children.Add(MemberRow(member, me.Id, owner));
        }
    }

    // ---- members -------------------------------------------------------------------------------

    private Grid MemberRow(MemberDto member, long me, bool owner)
    {
        var say = services.Say;
        var blocked = connection.Chats.IsBlocked(member.Id);
        var tools = FamilyText.ToolsFor(member, me, owner, blocked);

        var row = new Grid { ColumnSpacing = 12 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var face = new PersonPicture { Width = 32, Height = 32, DisplayName = member.DisplayName, VerticalAlignment = VerticalAlignment.Top };
        _ = ShowFaceAsync(face, member.Id, member.AvatarVersion);
        row.Children.Add(face);

        var words = new StackPanel();
        var name = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        name.Children.Add(new TextBlock { Text = member.DisplayName, FontWeight = FontWeights.SemiBold });
        if (member.Owner)
        {
            name.Children.Add(Capsule(say.Get("Owner")));
        }
        words.Children.Add(name);
        words.Children.Add(Secondary($"@{member.Username}"));
        if (member.Birthday is { } birthday)
        {
            words.Children.Add(Secondary($"🎂 {BirthdayRules.Text(birthday, say, services.Culture)}"));
        }
        Grid.SetColumn(words, 1);
        row.Children.Add(words);

        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
        if (tools.Message)
        {
            var message = new HyperlinkButton { Content = say.Get("Message") };
            AutomationProperties.SetName(message, say.Format("Message %@", member.DisplayName));
            message.Click += (_, _) => _ = MessageAsync(member.Id);
            actions.Children.Add(message);
        }
        if (tools.Safety)
        {
            var menu = new MenuFlyout();
            var report = new MenuFlyoutItem { Text = say.Get("Report…") };
            report.Click += (_, _) => _ = ReportAsync(member);
            menu.Items.Add(report);
            var block = new MenuFlyoutItem { Text = blocked ? say.Get("Unblock") : say.Get("Block") };
            block.Click += (_, _) => _ = BlockAsync(member.Id, !blocked);
            menu.Items.Add(block);
            var safety = new DropDownButton { Content = say.Get("Safety"), Flyout = menu };
            AutomationProperties.SetName(safety, say.Format("Safety for %@", member.DisplayName));
            actions.Children.Add(safety);
        }
        if (tools.OwnerMenu)
        {
            var menu = new MenuFlyout();
            var birthdayItem = new MenuFlyoutItem { Text = say.Get("Birthday…") };
            birthdayItem.Click += (_, _) => _ = MemberBirthdayAsync(member, me);
            menu.Items.Add(birthdayItem);
            if (tools.Removable)
            {
                var reset = new MenuFlyoutItem { Text = say.Get("Reset Password…") };
                reset.Click += (_, _) => _ = ResetPasswordAsync(
                    XamlRoot, say, member.DisplayName,
                    async password => (await connection.Api.ResetMemberPassword(member.Id, password)).Error);
                menu.Items.Add(reset);
                var remove = new MenuFlyoutItem { Text = say.Get("Remove from Family") };
                remove.Click += (_, _) => _ = RemoveAsync(member);
                menu.Items.Add(remove);
            }
            var more = new DropDownButton { Content = new SymbolIcon(Symbol.More), Flyout = menu };
            AutomationProperties.SetName(more, say.Format("More for %@", member.DisplayName));
            actions.Children.Add(more);
        }
        Grid.SetColumn(actions, 2);
        row.Children.Add(actions);
        return row;
    }

    private async Task ShowFaceAsync(PersonPicture face, long userId, int version)
    {
        if (version <= 0)
        {
            return;
        }
        var key = string.Create(CultureInfo.InvariantCulture, $"{userId}-{version}");
        try
        {
            if (!faces.TryGetValue(key, out var bitmap))
            {
                var (bytes, _) = await connection.Avatars.BytesAsync(userId, version);
                if (bytes is null)
                {
                    return;
                }
                bitmap = await BitmapAsync(bytes);
                faces[key] = bitmap;
            }
            face.ProfilePicture = bitmap;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"drawing a member's picture: {e.GetType().Name}");
        }
    }

    /// <summary>Get-or-create the direct chat, put it in the list, and open it behind this screen.</summary>
    private async Task MessageAsync(long userId)
    {
        if (busy)
        {
            return;
        }
        busy = true;
        Quiet();
        try
        {
            var answer = await connection.Api.DirectChat(userId);
            if (answer is not { Ok: true, Value: { } opened })
            {
                ShowProblem(ProblemText, FamilyText.GenericFailure(answer.Error ?? ApiError.Transport("no answer"), services.Say));
                return;
            }
            // Into the list first, so the conversation opens with its title: a chat made just now is in no list yet.
            var list = await connection.Api.Chats();
            if (list is { Ok: true, Value.Chats: { } rows })
            {
                connection.Chats.Replace(rows);
            }
            openChat(opened.Chat.Id);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"opening a direct chat: {e.GetType().Name}");
            ShowProblem(ProblemText, FamilyText.GenericFailure(ApiError.Transport(e.GetType().Name), services.Say));
        }
        finally
        {
            busy = false;
        }
    }

    private async Task ReportAsync(MemberDto member)
    {
        var say = services.Say;
        Quiet();
        var reason = await Dialogs.ReportAsync(
            XamlRoot, say, member.DisplayName, aboutMessage: false, connection.Session.State.SupportContact);
        if (reason is null)
        {
            return;
        }
        try
        {
            var answer = await connection.Api.Report(member.Id, reason);
            if (answer.Ok)
            {
                NoticeText.Text = say.Get("Report sent.");
                NoticeText.Visibility = Visibility.Visible;
                return;
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reporting a member: {e.GetType().Name}");
        }
        ShowProblem(ProblemText, say.Get("Couldn't send the report. Try again."));
    }

    private async Task BlockAsync(long userId, bool blocked)
    {
        Quiet();
        ApiError? error;
        try
        {
            error = await family.BlockAsync(userId, blocked);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"blocking a member: {e.GetType().Name}");
            error = ApiError.Transport(e.GetType().Name);
        }
        if (error is not null)
        {
            ShowProblem(ProblemText, services.Say.Get("Couldn't change that right now. Try again."));
            return;
        }
        // A blocked member's direct chat leaves the list: a pass brings the list as the server now has it.
        connection.Live.Refresh();
        Draw();
    }

    private async Task MemberBirthdayAsync(MemberDto member, long me)
    {
        var changed = await Dialogs.BirthdayAsync(
            XamlRoot, services, member.Birthday, member.DisplayName,
            async (month, day) => (await connection.Api.SetMemberBirthday(member.Id, month, day)).Error,
            async () => (await connection.Api.ClearMemberBirthday(member.Id)).Error);
        if (!changed)
        {
            return;
        }
        await family.ReadAsync();
        if (member.Id == me)
        {
            await connection.Session.RefreshAsync();
        }
        Draw();
    }

    private async Task RemoveAsync(MemberDto member)
    {
        var say = services.Say;
        Quiet();
        if (!await ConfirmAsync(XamlRoot, say, say.Format("Remove %@ from the family?", member.DisplayName), null, say.Get("Remove")))
        {
            return;
        }
        await OwnerActionAsync(async () => (await connection.Api.RemoveMember(member.Id)).Error);
    }

    private async Task RotateAsync()
    {
        var say = services.Say;
        Quiet();
        if (!await ConfirmAsync(
                XamlRoot, say, say.Get("Rotate the invite code?"),
                say.Get("Rotating invalidates the current code immediately."), say.Get("Rotate Code")))
        {
            return;
        }
        await OwnerActionAsync(async () => (await connection.Api.RotateInviteCode()).Error);
    }

    /// <summary>
    /// One change to the family at a time, said on the screen's one line when it fails — and the
    /// family read again when it lands, so what is drawn is what the server now holds.
    /// </summary>
    private async Task OwnerActionAsync(Func<Task<ApiError?>> act)
    {
        if (busy)
        {
            return;
        }
        busy = true;
        RotateButton.IsEnabled = false;
        try
        {
            if (await act() is { } error)
            {
                ShowProblem(ProblemText, FamilyText.GenericFailure(error, services.Say));
                return;
            }
            await family.ReadAsync();
            await connection.Session.RefreshFamilyAsync();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"an owner's change: {e.GetType().Name}");
            ShowProblem(ProblemText, FamilyText.GenericFailure(ApiError.Transport(e.GetType().Name), services.Say));
        }
        finally
        {
            busy = false;
            Draw();
        }
    }

    private void Quiet()
    {
        ProblemText.Visibility = Visibility.Collapsed;
        NoticeText.Visibility = Visibility.Collapsed;
    }

    // ---- the owner's lists ---------------------------------------------------------------------

    private async Task LoadOwnerListsAsync()
    {
        if (!connection.Session.State.IsOwner)
        {
            return;
        }
        try
        {
            var (waiting, error) = await family.RequestsAsync();
            if (error is null)
            {
                requests = waiting;
            }
            var inbox = await connection.Api.Reports();
            if (inbox is { Ok: true, Value: { } open })
            {
                reports = open.Reports ?? [];
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading the owner's lists: {e.GetType().Name}");
        }
        Draw();
    }

    private void DrawRequests()
    {
        var say = services.Say;
        RequestsSection.Visibility = requests.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        RequestsList.Children.Clear();
        foreach (var request in requests)
        {
            var row = new Grid { ColumnSpacing = 12 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            // Initials only: a stranger's picture is shown to nobody.
            row.Children.Add(new PersonPicture { Width = 32, Height = 32, DisplayName = request.User.DisplayName });
            var words = new StackPanel();
            words.Children.Add(new TextBlock { Text = request.User.DisplayName, FontWeight = FontWeights.SemiBold });
            words.Children.Add(Secondary($"@{request.User.Username}"));
            Grid.SetColumn(words, 1);
            row.Children.Add(words);
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
            var approve = new HyperlinkButton { Content = say.Get("Approve"), IsEnabled = deciding is null };
            approve.Click += (_, _) => _ = DecideAsync(request, approve: true);
            var decline = new HyperlinkButton { Content = say.Get("Decline"), IsEnabled = deciding is null };
            decline.Click += (_, _) => _ = DecideAsync(request, approve: false);
            actions.Children.Add(approve);
            actions.Children.Add(decline);
            Grid.SetColumn(actions, 2);
            row.Children.Add(actions);
            RequestsList.Children.Add(row);
        }
    }

    private async Task DecideAsync(JoinRequestDto request, bool approve)
    {
        if (deciding is not null)
        {
            return;
        }
        deciding = request.Id;
        RequestsError.Visibility = Visibility.Collapsed;
        DrawRequests();
        try
        {
            ApiError? error;
            if (approve)
            {
                (_, _, error) = await family.ApproveAsync(request.Id);
            }
            else
            {
                error = await family.RejectAsync(request.Id);
            }
            if (error is not null)
            {
                ShowProblem(RequestsError, FamilyText.RequestFailure(error, services.Say));
            }
            else if (approve)
            {
                await family.ReadAsync();
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"deciding a join request: {e.GetType().Name}");
            ShowProblem(RequestsError, FamilyText.GenericFailure(ApiError.Transport(e.GetType().Name), services.Say));
        }
        finally
        {
            deciding = null;
        }
        // The list as the server now has it: a full family's request is still there, a decided one is not.
        await LoadOwnerListsAsync();
    }

    private void DrawReports()
    {
        var say = services.Say;
        ReportsEmpty.Visibility = reports.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        ReportsList.Children.Clear();
        foreach (var report in reports)
        {
            var card = new StackPanel { Spacing = 4 };
            card.Children.Add(new TextBlock { Text = FamilyText.ReasonLabel(report.Reason, say), FontWeight = FontWeights.SemiBold });
            card.Children.Add(Secondary(FamilyText.Reported(report, say)));
            if (report.MessageExcerpt is { Length: > 0 } excerpt)
            {
                card.Children.Add(new Border
                {
                    Child = new TextBlock { Text = excerpt, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true },
                    Padding = new Thickness(10, 6, 10, 6),
                    CornerRadius = new CornerRadius(6),
                    Background = (Brush)Application.Current.Resources["CardBackgroundFillColorDefaultBrush"],
                });
            }
            if (FamilyText.Carried(report.MessageAttachments, say) is { } carried)
            {
                card.Children.Add(Secondary(carried));
            }
            var handled = new HyperlinkButton { Content = say.Get("Mark as handled"), IsEnabled = resolving is null, Padding = new Thickness(0, 4, 0, 4) };
            handled.Click += (_, _) => _ = ResolveAsync(report.Id);
            card.Children.Add(handled);
            ReportsList.Children.Add(card);
        }
    }

    private async Task ResolveAsync(long reportId)
    {
        if (resolving is not null)
        {
            return;
        }
        resolving = reportId;
        ReportsError.Visibility = Visibility.Collapsed;
        DrawReports();
        try
        {
            var answer = await connection.Api.ResolveReport(reportId);
            if (!answer.Ok)
            {
                ShowProblem(ReportsError, services.Say.Get("Couldn't mark that as handled. Try again."));
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"resolving a report: {e.GetType().Name}");
            ShowProblem(ReportsError, services.Say.Get("Couldn't mark that as handled. Try again."));
        }
        finally
        {
            resolving = null;
        }
        await LoadOwnerListsAsync();
    }
}
