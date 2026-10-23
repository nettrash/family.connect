using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace FamilyConnect.App.Views;

/// <summary>
/// The dialogs of one account in several families (docs/protocol.md, "Several families"): the two doors to a further family —
/// "New Family…" and "Join Another Family…", the existing create and join with their own words — and the owner's
/// "Invite…". What they ask of the server and what they leave behind are <see cref="FamilyDoor"/>'s and
/// <see cref="InviteModel"/>'s; this is the windows they are asked through, opened from the chat list's switcher and from
/// Settings alike.
/// </summary>
internal static class FamilyDialogs
{
    /// <summary>
    /// "New Family…": the create screen's field and button, under the sentence that says what starting another family means.
    /// Done, the family is added and shown (<see cref="FamilyDoor.CreateAsync"/>).
    /// </summary>
    public static async Task NewFamilyAsync(XamlRoot root, IStringCatalog say, FamilyDoor door)
    {
        var name = new TextBox { Header = say.Get("Family name"), PlaceholderText = "The Smiths", MaxLength = FamilyDoor.NameLimit };
        var problem = Dialogs.Problem();
        var dialog = Dialogs.Create(
            root,
            say.Get("Create a Family"),
            Dialogs.Column(
                Dialogs.Text(AnotherFamily.Explanation(say)),
                name,
                Dialogs.Footnote(say.Get("This names your family chat too. 1–64 characters.")),
                problem));
        dialog.PrimaryButtonText = say.Get("Create Family");
        dialog.CloseButtonText = say.Get("Cancel");
        dialog.IsPrimaryButtonEnabled = false;
        name.TextChanged += (_, _) => dialog.IsPrimaryButtonEnabled = FamilyDoor.MayCreate(name.Text);
        dialog.PrimaryButtonClick += async (_, args) => await SettleAsync(args, problem, () => door.CreateAsync(name.Text));
        await ShowAsync(dialog, "starting another family");
    }

    /// <summary>
    /// "Join Another Family…": the join screen's field and button. Joined, the family is added and shown; waiting for an
    /// owner, the switcher says so (<see cref="FamilyDoor.JoinAsync"/>).
    /// </summary>
    public static async Task JoinAnotherFamilyAsync(XamlRoot root, IStringCatalog say, FamilyDoor door)
    {
        var code = new TextBox { Header = say.Get("Invite code"), PlaceholderText = "ABCD2345", IsSpellCheckEnabled = false };
        var problem = Dialogs.Problem();
        var dialog = Dialogs.Create(
            root,
            say.Get("Join a Family"),
            Dialogs.Column(
                code,
                Dialogs.Footnote(say.Get("Any family member can read the code to you; the owner finds it in Settings.")),
                problem));
        dialog.PrimaryButtonText = say.Get("Join");
        dialog.CloseButtonText = say.Get("Cancel");
        dialog.IsPrimaryButtonEnabled = false;
        code.TextChanged += (_, _) => dialog.IsPrimaryButtonEnabled = FamilyDoor.MayJoin(code.Text);
        dialog.PrimaryButtonClick += async (_, args) => await SettleAsync(args, problem, () => door.JoinAsync(code.Text));
        await ShowAsync(dialog, "joining another family");
    }

    /// <summary>
    /// The owner's "Invite…": the members of their OTHER families who are not in this one, each a row that invites them when
    /// clicked and says "Invited" once they are — or the sentence that says who would appear here.
    /// </summary>
    /// <returns>Whether anybody was invited, so the pending list can be read again.</returns>
    public static async Task<bool> InviteAsync(
        XamlRoot root, IStringCatalog say, InviteModel invites, SessionState state, IReadOnlyList<InvitationDto> pending)
    {
        if (state.CurrentFamilyId is not { } familyId || state.Family is not { } family)
        {
            return false;
        }
        var candidates = invites.Candidates(state, pending);
        var problem = Dialogs.Problem();
        var rows = new StackPanel { Spacing = 2 };
        var invited = false;
        if (candidates.Count == 0)
        {
            rows.Children.Add(Dialogs.Secondary(say.Get("Members of your other families who aren't in this one appear here.")));
        }
        foreach (var candidate in candidates)
        {
            var mark = new TextBlock
            {
                Text = say.Get("Invited"),
                VerticalAlignment = VerticalAlignment.Center,
                Foreground = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TextFillColorSecondaryBrush"],
                Visibility = candidate.Invited ? Visibility.Visible : Visibility.Collapsed,
            };
            var line = new Grid { ColumnSpacing = 12 };
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            line.Children.Add(new TextBlock
            {
                Text = candidate.Member.DisplayName,
                TextTrimming = TextTrimming.CharacterEllipsis,
                VerticalAlignment = VerticalAlignment.Center,
            });
            Grid.SetColumn(mark, 1);
            line.Children.Add(mark);
            var row = new Button
            {
                Content = line,
                HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Stretch,
                Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent),
                BorderThickness = new Thickness(0),
                Padding = new Thickness(10, 8, 10, 8),
                IsEnabled = !candidate.Invited,
            };
            AutomationProperties.SetName(
                row, candidate.Invited ? $"{candidate.Member.DisplayName}, {say.Get("Invited")}" : candidate.Member.DisplayName);
            row.Click += async (_, _) =>
            {
                row.IsEnabled = false;
                problem.Visibility = Visibility.Collapsed;
                ApiError? error;
                try
                {
                    (_, error) = await invites.InviteAsync(familyId, candidate.Member.Id);
                }
                catch (Exception e)
                {
                    Diagnostics.Write($"inviting a member: {e.GetType().Name}");
                    error = ApiError.Transport(e.GetType().Name);
                }
                if (error is null)
                {
                    invited = true;
                    mark.Visibility = Visibility.Visible;
                    AutomationProperties.SetName(row, $"{candidate.Member.DisplayName}, {say.Get("Invited")}");
                    return;
                }
                row.IsEnabled = true;
                Dialogs.ShowProblem(problem, InviteFailure(error, say));
            };
            rows.Children.Add(row);
        }
        var dialog = Dialogs.Create(
            root,
            say.Format("Invite to “%@”", family.Name),
            Dialogs.Column(new ScrollViewer { Content = rows, MaxHeight = 420 }, problem));
        dialog.CloseButtonText = say.Get("Done");
        dialog.DefaultButton = ContentDialogButton.Close;
        await ShowAsync(dialog, "inviting");
        return invited;
    }

    /// <summary>Why an invitation did not go: the server's own word where the protocol gives one, else the general one.</summary>
    public static string InviteFailure(ApiError error, IStringCatalog say) => error switch
    {
        { Code: ErrorCodes.Transport } => say.Get("Can't reach the server. Try again."),
        { Status: 400 or 403 or 409 } when error.Canonical && error.Message is { Length: > 0 } said => said,
        _ => say.Get("Something went wrong. Try again."),
    };

    /// <summary>
    /// An invitation joined or declined from the banner or the switcher; a refusal is said in a dialog of its own —
    /// "This invitation is no longer available." for one that is gone.
    /// </summary>
    public static async Task AnswerInvitationAsync(
        XamlRoot root, IStringCatalog say, AppSession session, long invitationId, bool join)
    {
        ApiError? error;
        try
        {
            error = join
                ? await session.AcceptInvitationAsync(invitationId)
                : await session.DeclineInvitationAsync(invitationId);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"answering an invitation: {e.GetType().Name}");
            error = ApiError.Transport(e.GetType().Name);
        }
        if (error is null)
        {
            return;
        }
        var sentence = FamilySentences.For(error, session.State.MaxFamiliesPerAccount, say)
            ?? (error.Code == ErrorCodes.FamilyFull
                ? say.Get("That family is full right now. Ask them to make room, then try the code again.")
                : say.Get("Something went wrong. Try again."));
        var notice = Dialogs.Create(root, say.Get("Invitations"), Dialogs.Text(sentence));
        notice.CloseButtonText = say.Get("OK");
        notice.DefaultButton = ContentDialogButton.Close;
        await ShowAsync(notice, "an invitation's refusal");
    }

    /// <summary>A door's button, waited for: the dialog stays open to say why when it was refused.</summary>
    private static async Task SettleAsync(ContentDialogButtonClickEventArgs args, TextBlock problem, Func<Task<string?>> act)
    {
        var deferral = args.GetDeferral();
        try
        {
            string? refused;
            try
            {
                refused = await act();
            }
            catch (Exception e)
            {
                Diagnostics.Write($"a family door: {e.GetType().Name}");
                refused = e.GetType().Name;
            }
            if (refused is not null)
            {
                args.Cancel = true;
                Dialogs.ShowProblem(problem, refused);
            }
        }
        finally
        {
            deferral.Complete();
        }
    }

    /// <summary>Shown — or not, when another dialog is already up: only one may be at a time, and the click is written down.</summary>
    private static async Task ShowAsync(ContentDialog dialog, string what)
    {
        try
        {
            await dialog.ShowAsync();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"{what}: {e.GetType().Name}");
        }
    }
}
