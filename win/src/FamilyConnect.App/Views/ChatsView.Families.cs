using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace FamilyConnect.App.Views;

// ---- several families (issue #88) ------------------------------------------------------------------------------------------
//
// One account in several families, one family SHOWN at a time (docs/protocol.md, "Several families"). The list shows the chats
// whose family_ids name the shown family (ChatListModel); this is the switcher in the list's heading — every family with its
// unread count, the join requests and invitations waiting, "New Family…" and "Join Another Family…" — and the invitation cards
// over the rows. Which rows there are and what they say is FamilySwitcher's; an account in one family with nothing pending sees
// none of it.

public sealed partial class ChatsView
{
    /// <summary>The family the list was last drawn for — a switch closes a conversation the new family does not list.</summary>
    private long? familyDrawn;

    /// <summary>What the switcher's menu and the invitation cards were built from, so an unchanged state builds nothing.</summary>
    private string switcherDrawn = string.Empty;
    private string invitationsDrawn = string.Empty;

    /// <summary>The switcher's words that are not the families: its name to a screen reader, and its tooltip.</summary>
    private void WireFamilies()
    {
        var say = services.Say;
        AutomationProperties.SetName(SwitcherButton, say.Get("Switch Family"));
        ToolTipService.SetToolTip(SwitcherButton, say.Get("Switch Family"));
    }

    /// <summary>
    /// The session moved: another family may be shown, or one may have gone with its rows. A conversation the shown family
    /// does not list — or one the cache no longer holds — closes, and the list is drawn again for the family now shown.
    /// </summary>
    private void FamilyMoved()
    {
        var shown = connection.Session.State.CurrentFamilyId;
        if (open is { } chat
            && (connection.Chats.Chat(chat.ChatId)?.Chat is not { } held || !FamilyRules.Lists(held, shown)))
        {
            CloseConversation();
        }
        if (familyDrawn != shown)
        {
            familyDrawn = shown;
            // Another family's rows, from the top: nothing of the last family's list is kept.
            listDrawn = string.Empty;
        }
        QueueRedraw();
    }

    /// <summary>The switcher and the invitation cards, as the session and the list now stand.</summary>
    private void DrawFamilies()
    {
        var state = connection.Session.State;
        var say = services.Say;
        var shows = FamilyRules.ShowsSwitcher(state);
        SwitcherButton.Visibility = shows ? Visibility.Visible : Visibility.Collapsed;
        if (shows)
        {
            var held = list.Held;
            var rows = FamilySwitcher.Rows(state, held, say);
            SwitcherText.Text = state.Family?.Name ?? string.Empty;
            // What is unread OUTSIDE the family shown — a chat it lists is on screen already, and one listed in several is
            // counted once.
            var elsewhere = held.Where(row => !FamilyRules.Lists(row.Chat, state.CurrentFamilyId)).Sum(row => row.UnreadCount);
            OtherFamiliesBadge.Visibility = elsewhere > 0 ? Visibility.Visible : Visibility.Collapsed;
            if (elsewhere > 0 && OtherFamiliesBadge.Value != elsewhere)
            {
                OtherFamiliesBadge.Value = elsewhere;
            }
            var signature = string.Join(Row, rows.Select(row => string.Join(Field, row.Kind, row.Id, row.Title, row.Detail, row.Unread, row.Current)));
            if (signature != switcherDrawn)
            {
                switcherDrawn = signature;
                SwitcherButton.Flyout = SwitcherMenu(rows);
            }
        }
        else
        {
            switcherDrawn = string.Empty;
        }
        DrawInvitations(state, say);
    }

    /// <summary>
    /// The switcher's menu: the families — the one shown checked, each with its unread count beside it — then what is waiting,
    /// then the doors. Rebuilt only when what it says changes.
    /// </summary>
    private MenuFlyout SwitcherMenu(IReadOnlyList<SwitcherRow> rows)
    {
        var say = services.Say;
        var menu = new MenuFlyout { Placement = Microsoft.UI.Xaml.Controls.Primitives.FlyoutPlacementMode.BottomEdgeAlignedLeft };
        SwitcherRowKind? last = null;
        foreach (var row in rows)
        {
            // A rule between the families, what waits, and the doors.
            if (last is { } before && Section(before) != Section(row.Kind))
            {
                menu.Items.Add(new MenuFlyoutSeparator());
            }
            last = row.Kind;
            switch (row.Kind)
            {
                case SwitcherRowKind.Family:
                {
                    var item = new RadioMenuFlyoutItem
                    {
                        Text = row.Title,
                        IsChecked = row.Current,
                        GroupName = "families",
                        // The count where a shortcut would be: right-aligned and quieter than the name.
                        KeyboardAcceleratorTextOverride = row.Unread > 0 ? row.Unread.ToString(services.Culture) : string.Empty,
                    };
                    AutomationProperties.SetName(item, row.AccessibleName ?? row.Title);
                    var familyId = row.Id!.Value;
                    item.Click += (_, _) => _ = SwitchToAsync(familyId);
                    menu.Items.Add(item);
                    break;
                }
                case SwitcherRowKind.Pending:
                    menu.Items.Add(new MenuFlyoutItem
                    {
                        Text = row.Title,
                        KeyboardAcceleratorTextOverride = row.Detail ?? string.Empty,
                        IsEnabled = false,
                    });
                    break;
                case SwitcherRowKind.Invitation:
                {
                    var invitationId = row.Id!.Value;
                    var invitation = new MenuFlyoutSubItem { Text = row.Detail ?? row.Title, Icon = Glyph(0xE715) };
                    var join = new MenuFlyoutItem { Text = say.Get("Join") };
                    join.Click += (_, _) => _ = AnswerInvitationAsync(invitationId, join: true);
                    var decline = new MenuFlyoutItem { Text = say.Get("Decline") };
                    decline.Click += (_, _) => _ = AnswerInvitationAsync(invitationId, join: false);
                    invitation.Items.Add(join);
                    invitation.Items.Add(decline);
                    menu.Items.Add(invitation);
                    break;
                }
                case SwitcherRowKind.NewFamily:
                {
                    var item = new MenuFlyoutItem { Text = row.Title, Icon = Glyph(0xE710) };
                    item.Click += (_, _) => _ = FamilyDialogs.NewFamilyAsync(XamlRoot, say, FamilyDoorway);
                    menu.Items.Add(item);
                    break;
                }
                case SwitcherRowKind.JoinAnother:
                {
                    var item = new MenuFlyoutItem { Text = row.Title, Icon = Glyph(0xE77B) };
                    item.Click += (_, _) => _ = FamilyDialogs.JoinAnotherFamilyAsync(XamlRoot, say, FamilyDoorway);
                    menu.Items.Add(item);
                    break;
                }
            }
        }
        return menu;
    }

    private static int Section(SwitcherRowKind kind) => kind switch
    {
        SwitcherRowKind.Family => 0,
        SwitcherRowKind.Pending or SwitcherRowKind.Invitation => 1,
        _ => 2,
    };

    private static FontIcon Glyph(int glyph) => new() { Glyph = ((char)glyph).ToString() };

    private FamilyDoor FamilyDoorway => new(connection.Api, connection.Session, services.Say);

    /// <summary>Show another family. Not awaited by the menu: a click handler that throws ends the process.</summary>
    private async Task SwitchToAsync(long familyId)
    {
        try
        {
            await connection.Session.SwitchFamilyAsync(familyId);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"switching family: {e.GetType().Name}");
        }
    }

    private async Task AnswerInvitationAsync(long invitationId, bool join)
    {
        try
        {
            await FamilyDialogs.AnswerInvitationAsync(XamlRoot, services.Say, connection.Session, invitationId, join);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"answering an invitation: {e.GetType().Name}");
        }
    }

    /// <summary>
    /// An invitation to this account is a card at the top of the list until it is answered: who invited them into which family,
    /// with Join and Decline.
    /// </summary>
    private void DrawInvitations(SessionState state, IStringCatalog say)
    {
        var invitations = state.AllInvitations;
        var signature = string.Join(Row, invitations.Select(invitation => string.Join(Field, invitation.Id, InvitationText.Banner(invitation, say))));
        if (signature == invitationsDrawn)
        {
            return;
        }
        invitationsDrawn = signature;
        InvitationsPanel.Children.Clear();
        InvitationsPanel.Visibility = invitations.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        var resources = Application.Current.Resources;
        foreach (var invitation in invitations)
        {
            var words = new TextBlock { Text = InvitationText.Banner(invitation, say), TextWrapping = TextWrapping.Wrap };
            var join = new Button { Content = say.Get("Join"), Style = (Style)resources["AccentButtonStyle"] };
            var decline = new Button { Content = say.Get("Decline") };
            var id = invitation.Id;
            join.Click += (_, _) => _ = AnswerInvitationAsync(id, join: true);
            decline.Click += (_, _) => _ = AnswerInvitationAsync(id, join: false);
            var buttons = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            buttons.Children.Add(join);
            buttons.Children.Add(decline);
            var body = new StackPanel { Spacing = 10 };
            var heading = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
            heading.Children.Add(new FontIcon
            {
                Glyph = "",
                FontSize = 16,
                VerticalAlignment = VerticalAlignment.Top,
                Foreground = (Brush)resources["AccentTextFillColorPrimaryBrush"],
            });
            heading.Children.Add(words);
            body.Children.Add(heading);
            body.Children.Add(buttons);
            InvitationsPanel.Children.Add(new Border
            {
                Child = body,
                Padding = new Thickness(14, 12, 14, 12),
                CornerRadius = new CornerRadius(8),
                BorderThickness = new Thickness(1),
                Background = (Brush)resources["CardBackgroundFillColorDefaultBrush"],
                BorderBrush = (Brush)resources["CardStrokeColorDefaultBrush"],
            });
        }
    }
}
