using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using FamilyConnect.Core.Protocol;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace FamilyConnect.App.Views;

// ---- groups (issue #107) ----------------------------------------------------------------------------------------------------
//
// Some of the family in a chat of their own, with a name and an admin (docs/protocol.md, "Groups"; docs/groups-2026-10-21.md).
// Who may do what, what a name may be and whom the lists offer are GroupRules'; what is said is GroupText's; the requests and
// what they leave in the cache are GroupModel's. This is the dialogs.
//
// One ContentDialog may be up at a time, so the info dialog's actions CLOSE it and run once it has gone; a change that leaves
// the reader in the group opens the info again, so the admin sees what they did.

public sealed partial class ChatsView
{
    /// <summary>One thing picked in the group's info, run once the info has closed.</summary>
    private sealed record GroupStep(GroupAction Action, long UserId = 0);

    private GroupModel Groups => new(connection.Api, connection.Chats);

    /// <summary>The open chat as the cache holds it, when it is a group.</summary>
    private ChatDto? OpenGroup =>
        open is { } chat && connection.Chats.Chat(chat.ChatId)?.Chat is { } held && ChatKinds.IsGroup(held.Kind) ? held : null;

    /// <summary>The header's info button: in a group, for everyone in it — leaving is anybody's.</summary>
    private void ShowGroupInfoButton() =>
        GroupInfoButton.Visibility = OpenGroup is not null ? Visibility.Visible : Visibility.Collapsed;

    /// <summary>
    /// The open chat's header, and whether there is still a chat to show: a group the reader has left, been removed from or
    /// seen deleted — dropped by its frame, or no longer in <c>GET /chats</c> — closes, and its name follows a rename.
    /// </summary>
    private void DrawHeader()
    {
        if (open is not { } chat)
        {
            return;
        }
        var held = connection.Chats.Chat(chat.ChatId);
        if ((held is null && openWasGroup)
            || (held is not null && ChatKinds.IsGroup(held.Chat.Kind) && !connection.Chats.IsListed(chat.ChatId)))
        {
            CloseConversation();
            return;
        }
        if (held is not null && list.Title(held.Chat) is var title && title != ConversationTitle.Text)
        {
            ConversationTitle.Text = title;
            ConversationFace.Content = Face(held.Chat, title, 32);
        }
        ShowGroupInfoButton();
    }

    /// <summary>
    /// No chat open any more — the one that was is a group the reader is out of. Everything that belonged to it goes as it
    /// does when another chat opens, and the pane is left as it was before the first one did.
    /// </summary>
    private void CloseConversation()
    {
        if (open is not { } closing)
        {
            return;
        }
        _ = Interrupt(RecordingEnd.LeftChat);
        // Its notifications go: one clicked would open a chat that is not there.
        Toasts.Clear(NotificationRules.ChatTag(closing.ChatId));
        // And what was being drafted there goes with it — "a client drops the chat, its messages and anything it was
        // drafting there" (docs/protocol.md, "Groups"): the files staged in its composer, and the voice messages waiting
        // there as not sent. Its words go with the list's next draw (drafts.Retain).
        if (strips.Remove(closing.ChatId, out var strip))
        {
            strip.CancelPreparing();
        }
        try
        {
            foreach (var entry in parked.Of(closing.ChatId))
            {
                parked.Remove(entry);
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"dropping a gone group's voice messages: {e.GetType().Name}");
        }
        StopAudio();
        locationHunt?.Cancel();
        CloseViewer();
        CloseThread();
        EndComposerMode(clear: true);
        open = null;
        openWasGroup = false;
        openPolls = null;
        OpenPollsButton.Visibility = Visibility.Collapsed;
        GroupInfoButton.Visibility = Visibility.Collapsed;
        ConversationTitle.Text = string.Empty;
        ConversationFace.Content = null;
        ConversationHeader.Visibility = Visibility.Collapsed;
        ComposerPanel.Visibility = Visibility.Collapsed;
        ComposerError.Visibility = Visibility.Collapsed;
        conversationDrawn = string.Empty;
        listDrawn = string.Empty;
        DrawStaging();
        DrawNotSent();
        ShowCallButtons();
        DrawConversation(keepFromBottom: null);
        DrawList();
    }

    /// <summary>
    /// "New Group": a name and the members to start it with — anybody in the family may make one, and becomes its admin. It
    /// opens once the server has it.
    /// </summary>
    private async Task NewGroupAsync()
    {
        var say = services.Say;
        var name = new TextBox { Header = say.Get("Group Name") };
        var (picker, chosen) = MemberChecks(GroupRules.Choosable(connection.Chats.Members(), Reader, connection.Session.State.Assistant?.UserId));
        var problem = Dialogs.Problem();
        var dialog = Dialogs.Create(XamlRoot, say.Get("New Group"), Dialogs.Column(name, picker, problem));
        dialog.PrimaryButtonText = say.Get("Create Group");
        dialog.CloseButtonText = say.Get("Cancel");
        long? created = null;
        dialog.PrimaryButtonClick += async (_, args) =>
        {
            var ids = chosen();
            if (GroupRules.CreateProblem(name.Text, ids, Reader, say) is { } wrong)
            {
                args.Cancel = true;
                Dialogs.ShowProblem(problem, wrong);
                return;
            }
            await Dialogs.SettleAsync(
                args,
                problem,
                async () =>
                {
                    var (chatId, error) = await Groups.CreateAsync(name.Text, ids);
                    created = chatId;
                    return error;
                },
                error => GroupText.Failure(error, creating: true, say));
        };
        try
        {
            await dialog.ShowAsync();
        }
        catch (Exception e)
        {
            // Only one dialog may be up at a time.
            Diagnostics.Write($"making a group: {e.GetType().Name}");
            return;
        }
        if (created is { } id && !gone)
        {
            listDrawn = string.Empty;
            OpenChat(id);
        }
    }

    /// <summary>
    /// The group's info: its name, how many are in it, each member with the admin marked — and what the reader may do: the
    /// admin renames, adds, removes and deletes; anybody leaves. Each action runs once the info has closed, and the info
    /// opens again after one that leaves the reader in the group.
    /// </summary>
    private async Task ShowGroupInfoAsync()
    {
        while (!gone && OpenGroup is { } group)
        {
            GroupStep? step;
            try
            {
                step = await GroupInfoAsync(group);
            }
            catch (Exception e)
            {
                Diagnostics.Write($"showing a group's info: {e.GetType().Name}");
                return;
            }
            if (step is null || gone || open?.ChatId != group.Id)
            {
                return;
            }
            if (!await RunGroupStepAsync(group, step))
            {
                return;
            }
        }
    }

    /// <summary>The info dialog itself; answers what was picked in it, or null when it was closed.</summary>
    private async Task<GroupStep?> GroupInfoAsync(ChatDto group)
    {
        var say = services.Say;
        GroupStep? picked = null;
        var title = list.Title(group);
        var content = Dialogs.Column(
            new TextBlock { Text = title, FontWeight = FontWeights.SemiBold, FontSize = 18, TextWrapping = TextWrapping.Wrap },
            Dialogs.Secondary(GroupText.Count(group, say)));
        var dialog = Dialogs.Create(XamlRoot, say.Get("Group Info"), content);
        dialog.CloseButtonText = say.Get("Close");
        dialog.DefaultButton = ContentDialogButton.Close;
        void Pick(GroupStep step)
        {
            picked = step;
            dialog.Hide();
        }

        var members = new StackPanel { Spacing = 6 };
        AutomationProperties.SetName(members, say.Get("Members"));
        foreach (var member in GroupRules.Members(group, connection.Chats.Members(), Reader, say))
        {
            var row = new Grid { ColumnSpacing = 8 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            var version = connection.Chats.Member(member.UserId)?.AvatarVersion ?? 0;
            row.Children.Add(faces.Face(member.Name, false, member.UserId, version, 28));
            var who = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, VerticalAlignment = VerticalAlignment.Center };
            who.Children.Add(new TextBlock
            {
                Text = member.Name,
                VerticalAlignment = VerticalAlignment.Center,
                TextTrimming = TextTrimming.CharacterEllipsis,
            });
            if (member.Admin)
            {
                who.Children.Add(Dialogs.Capsule(say.Get("Admin")));
            }
            Grid.SetColumn(who, 1);
            row.Children.Add(who);
            if (member.Removable)
            {
                var remove = new HyperlinkButton { Content = say.Get("Remove from Group"), VerticalAlignment = VerticalAlignment.Center };
                // Which member it takes out, for a screen reader that hears the link away from its row.
                AutomationProperties.SetHelpText(remove, member.Name);
                var userId = member.UserId;
                remove.Click += (_, _) => Pick(new GroupStep(GroupAction.RemoveMember, userId));
                Grid.SetColumn(remove, 2);
                row.Children.Add(remove);
            }
            members.Children.Add(row);
        }
        content.Children.Add(new ScrollViewer { Content = members, MaxHeight = 280 });

        var actions = new StackPanel { Spacing = 4 };
        void Offer(string words, GroupAction action)
        {
            if (!GroupRules.May(action, group, Reader))
            {
                return;
            }
            var button = new HyperlinkButton { Content = words };
            button.Click += (_, _) => Pick(new GroupStep(action));
            actions.Children.Add(button);
        }
        Offer(say.Get("Rename Group"), GroupAction.Rename);
        Offer(say.Get("Add Members"), GroupAction.AddMembers);
        Offer(say.Get("Leave Group"), GroupAction.Leave);
        Offer(say.Get("Delete Group"), GroupAction.Delete);
        content.Children.Add(actions);

        await dialog.ShowAsync();
        return picked;
    }

    /// <summary>One action from the info. Answers whether the info should open again: the reader is still in the group.</summary>
    private async Task<bool> RunGroupStepAsync(ChatDto group, GroupStep step)
    {
        var say = services.Say;
        var title = list.Title(group);
        ApiError? error;
        switch (step.Action)
        {
            case GroupAction.Rename:
                await RenameGroupAsync(group);
                return true;
            case GroupAction.AddMembers:
                await AddGroupMembersAsync(group);
                return true;
            case GroupAction.RemoveMember:
                error = await Groups.RemoveAsync(group.Id, step.UserId);
                break;
            case GroupAction.Leave:
                if (!await Dialogs.ConfirmAsync(XamlRoot, say, say.Get("Leave Group"), GroupText.LeaveQuestion(title, say), say.Get("Leave Group")))
                {
                    return true;
                }
                error = await Groups.LeaveAsync(group.Id);
                break;
            case GroupAction.Delete:
                if (!await Dialogs.ConfirmAsync(XamlRoot, say, say.Get("Delete Group"), GroupText.DeleteQuestion(title, say), say.Get("Delete Group")))
                {
                    return true;
                }
                error = await Groups.DeleteAsync(group.Id);
                break;
            default:
                return false;
        }
        if (gone)
        {
            return false;
        }
        Redraw();
        if (error is not null)
        {
            ShowProblem(GroupText.Failure(error, creating: false, say));
            return false;
        }
        return step.Action == GroupAction.RemoveMember;
    }

    private async Task RenameGroupAsync(ChatDto group)
    {
        var say = services.Say;
        var name = new TextBox { Header = say.Get("Group Name"), Text = group.Title };
        name.SelectAll();
        var problem = Dialogs.Problem();
        var dialog = Dialogs.Create(XamlRoot, say.Get("Rename Group"), Dialogs.Column(name, problem));
        dialog.PrimaryButtonText = say.Get("Save");
        dialog.CloseButtonText = say.Get("Cancel");
        dialog.PrimaryButtonClick += async (_, args) =>
        {
            if (GroupRules.Name(name.Text) is null)
            {
                args.Cancel = true;
                Dialogs.ShowProblem(problem, GroupText.NameProblem(say));
                return;
            }
            await Dialogs.SettleAsync(
                args, problem, () => Groups.RenameAsync(group.Id, name.Text), error => GroupText.Failure(error, creating: false, say));
        };
        await dialog.ShowAsync();
        if (!gone)
        {
            Redraw();
        }
    }

    private async Task AddGroupMembersAsync(ChatDto group)
    {
        var say = services.Say;
        var addable = GroupRules.Addable(group, connection.Chats.Members(), Reader, connection.Session.State.Assistant?.UserId);
        if (addable.Count == 0)
        {
            // Nobody left to add: said, rather than a list with nothing in it and a button that cannot be pressed.
            var nobody = Dialogs.Create(XamlRoot, say.Get("Add Members"), Dialogs.Text(GroupText.EveryoneIsIn(say)));
            nobody.CloseButtonText = say.Get("OK");
            nobody.DefaultButton = ContentDialogButton.Close;
            await nobody.ShowAsync();
            return;
        }
        var (picker, chosen) = MemberChecks(addable);
        var problem = Dialogs.Problem();
        var dialog = Dialogs.Create(XamlRoot, say.Get("Add Members"), Dialogs.Column(picker, problem));
        dialog.PrimaryButtonText = say.Get("Add");
        dialog.CloseButtonText = say.Get("Cancel");
        dialog.PrimaryButtonClick += async (_, args) =>
        {
            var ids = chosen();
            if (ids.Count == 0)
            {
                args.Cancel = true;
                Dialogs.ShowProblem(problem, GroupText.NobodyChosen(say));
                return;
            }
            await Dialogs.SettleAsync(
                args, problem, () => Groups.AddAsync(group.Id, ids), error => GroupText.Failure(error, creating: false, say));
        };
        await dialog.ShowAsync();
        if (!gone)
        {
            Redraw();
        }
    }

    /// <summary>A check box per member, under "Members", and how to read which are ticked.</summary>
    private (FrameworkElement Picker, Func<IReadOnlyList<long>> Chosen) MemberChecks(IReadOnlyList<MemberDto> members)
    {
        var say = services.Say;
        var boxes = new StackPanel { Spacing = 2 };
        AutomationProperties.SetName(boxes, say.Get("Members"));
        foreach (var member in members)
        {
            boxes.Children.Add(new CheckBox { Content = member.DisplayName, Tag = member.Id });
        }
        var picker = new StackPanel { Spacing = 6 };
        picker.Children.Add(new TextBlock { Text = say.Get("Members"), FontWeight = FontWeights.SemiBold });
        picker.Children.Add(new ScrollViewer { Content = boxes, MaxHeight = 280 });
        return (picker, () => [.. boxes.Children.OfType<CheckBox>().Where(box => box.IsChecked == true && box.Tag is long).Select(box => (long)box.Tag)]);
    }
}
