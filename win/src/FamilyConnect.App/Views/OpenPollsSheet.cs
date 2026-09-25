using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace FamilyConnect.App.Views;

/// <summary>
/// The family chat's open polls, oldest first (<see cref="OpenPollsModel"/>). Read when it opens and
/// again after every vote or close; a poll frame that lands while it is up draws the newer state.
/// </summary>
internal static class OpenPollsSheet
{
    public static async Task ShowAsync(XamlRoot root, OpenPollsModel model, Func<PollCard.Seen> seen, Connection connection)
    {
        var say = seen().Say;
        var problem = Dialogs.Problem();
        var ring = new ProgressRing { IsActive = true, HorizontalAlignment = HorizontalAlignment.Center };
        var list = new StackPanel { Spacing = 12 };
        var dialog = Dialogs.Create(root, say.Get("Open polls"), new ScrollViewer
        {
            Content = Dialogs.Column(problem, ring, list),
            MaxHeight = 560,
            MinWidth = 380,
        });
        dialog.CloseButtonText = say.Get("Done");
        dialog.DefaultButton = ContentDialogButton.Close;
        string? refused = null;
        var busy = false;

        void Draw()
        {
            ring.Visibility = model.Loaded || model.Failure is not null ? Visibility.Collapsed : Visibility.Visible;
            var sentence = refused
                ?? (model.Failure is null ? null : $"{say.Get("Couldn't load the polls")} {say.Get("Try again in a moment.")}");
            if (sentence is null)
            {
                problem.Visibility = Visibility.Collapsed;
            }
            else
            {
                Dialogs.ShowProblem(problem, sentence);
            }
            list.Children.Clear();
            if (!model.Loaded)
            {
                return;
            }
            var rows = model.Messages();
            if (rows.Count == 0)
            {
                list.Children.Add(new TextBlock { Text = say.Get("Nothing to decide"), FontWeight = FontWeights.SemiBold });
                list.Children.Add(Dialogs.Secondary(say.Get("Open polls stay here until they're closed. There are none right now.")));
                return;
            }
            var current = seen();
            foreach (var message in rows)
            {
                var id = message.Id;
                if (model.IsHidden(message))
                {
                    // The placeholder and nothing else — no question, no name, no options — until revealed.
                    var reveal = new HyperlinkButton { Content = say.Get("Hidden — blocked member") };
                    reveal.Click += (_, _) =>
                    {
                        model.Reveal(id);
                        Draw();
                    };
                    list.Children.Add(reveal);
                    continue;
                }
                var card = new StackPanel { Spacing = 4 };
                card.Children.Add(new TextBlock { Text = message.Body, FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap });
                card.Children.Add(Dialogs.Secondary(PollText.Name(message.SenderId, current.Reader, current.Member, say)));
                card.Children.Add(PollCard.Build(
                    message, mine: false, current,
                    option => ActAsync(() => model.VoteAsync(id, option)),
                    () => ActAsync(() => model.CloseAsync(id))));
                list.Children.Add(new Border
                {
                    Child = card,
                    Padding = new Thickness(12),
                    CornerRadius = new CornerRadius(8),
                    Background = (Brush)Application.Current.Resources["CardBackgroundFillColorDefaultBrush"],
                });
            }
        }

        async Task ActAsync(Func<Task<ApiError?>> act)
        {
            if (busy)
            {
                return;
            }
            busy = true;
            refused = null;
            try
            {
                if (await act() is not null && model.Failure is null)
                {
                    refused = say.Get("Try again in a moment.");
                }
            }
            catch (Exception e)
            {
                Diagnostics.Write($"a poll on the open list: {e.GetType().Name}");
                refused = say.Get("Try again in a moment.");
            }
            finally
            {
                busy = false;
                Draw();
            }
        }

        async Task LoadAsync()
        {
            try
            {
                await model.LoadAsync();
            }
            catch (Exception e)
            {
                Diagnostics.Write($"reading the open polls: {e.GetType().Name}");
            }
            Draw();
        }

        void OnChat(long chatId)
        {
            if (chatId == model.ChatId)
            {
                dialog.DispatcherQueue.TryEnqueue(Draw);
            }
        }

        connection.Router.ChatChanged += OnChat;
        try
        {
            Draw();
            _ = LoadAsync();
            await dialog.ShowAsync();
        }
        finally
        {
            connection.Router.ChatChanged -= OnChat;
        }
    }
}
