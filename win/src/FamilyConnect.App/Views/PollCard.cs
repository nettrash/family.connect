using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace FamilyConnect.App.Views;

/// <summary>
/// A poll under its question — the options with their bars and counts, who chose each, how many voted,
/// and the author's way out. Drawn in the conversation's bubble and on the open-polls sheet alike.
/// </summary>
/// <remarks>
/// A poll the server has not numbered, or one that is closed, draws its options as a RESULT and not as
/// buttons: a disabled button greys its words, and a result is for reading.
/// </remarks>
internal static class PollCard
{
    /// <summary>What drawing a poll needs to know about the reader and the family.</summary>
    internal sealed record Seen(
        IStringCatalog Say, long Reader, Func<long, MemberDto?> Member, Func<long, bool> Blocked, int MemberCount);

    public static FrameworkElement Build(
        MessageDto message, bool mine, Seen seen, Func<long, Task> vote, Func<Task> close)
    {
        var poll = message.Poll!;
        var say = seen.Say;
        var resources = Application.Current.Resources;
        var ink = (Brush)resources[mine ? "TextOnAccentFillColorPrimaryBrush" : "TextFillColorPrimaryBrush"];
        var votable = Polls.Votable(message);
        var held = Polls.MyOption(poll, seen.Reader);
        string Name(long user) => PollText.Name(user, seen.Reader, seen.Member, say);

        var panel = new StackPanel { Spacing = 4, MinWidth = 240, MaxWidth = 360, HorizontalAlignment = HorizontalAlignment.Left };
        foreach (var option in poll.Options)
        {
            var chosen = held == option.Id;
            var body = OptionBody(option, chosen, Polls.Fraction(poll, option.Votes.Length), ink);
            FrameworkElement row;
            if (votable)
            {
                var button = new Button
                {
                    Content = body,
                    HorizontalAlignment = HorizontalAlignment.Stretch,
                    HorizontalContentAlignment = HorizontalAlignment.Stretch,
                    Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
                    BorderThickness = new Thickness(0),
                    Padding = new Thickness(6, 4, 6, 4),
                };
                if (ink is SolidColorBrush solid)
                {
                    // The system's hover ground is light grey, and on the reader's own tinted bubble the words are white.
                    var hover = new SolidColorBrush(Microsoft.UI.ColorHelper.FromArgb(0x24, solid.Color.R, solid.Color.G, solid.Color.B));
                    button.Resources["ButtonBackgroundPointerOver"] = hover;
                    button.Resources["ButtonBackgroundPressed"] = hover;
                }
                var optionId = option.Id;
                // The one held clears it, any other casts it — decided by PollVoting, not here.
                button.Click += (_, _) => _ = vote(optionId);
                Contain(button);
                row = button;
            }
            else
            {
                row = new Border { Child = body, Padding = new Thickness(6, 4, 6, 4) };
            }
            AutomationProperties.SetName(row, PollText.OptionLabel(option, chosen, say));
            panel.Children.Add(row);

            var drawable = Polls.DrawableVoters(option.Votes, seen.Blocked);
            if (drawable.Count > 0)
            {
                var voters = new HyperlinkButton
                {
                    Content = new TextBlock
                    {
                        Text = PollText.Voters(drawable, Name, say),
                        FontSize = 12,
                        TextWrapping = TextWrapping.Wrap,
                        Foreground = ink,
                        Opacity = 0.8,
                    },
                    Padding = new Thickness(28, 0, 6, 2),
                };
                ToolTipService.SetToolTip(voters, say.Get("See who voted"));
                AutomationProperties.SetName(voters, PollText.Voters(drawable, Name, say));
                AutomationProperties.SetHelpText(voters, say.Get("See who voted"));
                var text = option.Text;
                // Everyone, named: the half the line cannot show once it says "+N".
                voters.Click += (_, _) => ShowVoters(voters, text, drawable, Name);
                Contain(voters);
                panel.Children.Add(voters);
            }
        }

        var footer = new Grid { ColumnSpacing = 8, Margin = new Thickness(6, 2, 6, 0) };
        footer.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        footer.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        footer.Children.Add(new TextBlock
        {
            Text = PollText.Footer(poll, seen.MemberCount, say),
            FontSize = 12,
            Opacity = 0.75,
            Foreground = ink,
            TextWrapping = TextWrapping.Wrap,
            VerticalAlignment = VerticalAlignment.Center,
        });
        FrameworkElement? end = null;
        if (poll.Closed)
        {
            end = new TextBlock { Text = say.Get("Poll closed"), FontSize = 12, Opacity = 0.75, Foreground = ink, VerticalAlignment = VerticalAlignment.Center };
        }
        else if (PollVoting.MayClose(message, seen.Reader))
        {
            var closing = new HyperlinkButton
            {
                Content = new TextBlock { Text = say.Get("Close poll"), FontSize = 12, Foreground = ink, FontWeight = FontWeights.SemiBold },
                Padding = new Thickness(4, 0, 4, 0),
            };
            ToolTipService.SetToolTip(closing, say.Get("Ends the poll. This cannot be undone."));
            AutomationProperties.SetHelpText(closing, say.Get("Ends the poll. This cannot be undone."));
            closing.Click += (_, _) => _ = close();
            Contain(closing);
            end = closing;
        }
        if (end is not null)
        {
            Grid.SetColumn(end, 1);
            footer.Children.Add(end);
        }
        panel.Children.Add(footer);
        return panel;
    }

    /// <summary>
    /// A double click on a control here is two clicks on it and nothing more: left to rise, it would reach
    /// the bubble's own double tap and put a heart on the message as well.
    /// </summary>
    private static void Contain(UIElement control) => control.DoubleTapped += (_, e) => e.Handled = true;

    /// <summary>The mark, the words and the count on one line, and the bar under them.</summary>
    private static StackPanel OptionBody(PollOptionDto option, bool chosen, double fraction, Brush ink)
    {
        var line = new Grid { ColumnSpacing = 8 };
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        // The mark says "yours" without colour carrying it alone — on a tinted bubble a tint is not enough.
        var mark = new FontIcon
        {
            Glyph = ((char)(chosen ? 0xECCB : 0xECCA)).ToString(),
            FontSize = 14,
            Foreground = ink,
            VerticalAlignment = VerticalAlignment.Center,
        };
        line.Children.Add(mark);
        var words = new TextBlock
        {
            Text = option.Text,
            TextWrapping = TextWrapping.Wrap,
            Foreground = ink,
            FontWeight = chosen ? FontWeights.SemiBold : FontWeights.Normal,
        };
        Grid.SetColumn(words, 1);
        line.Children.Add(words);
        var count = new TextBlock { Text = option.Votes.Length.ToString(System.Globalization.CultureInfo.CurrentCulture), Foreground = ink, Opacity = 0.8 };
        Grid.SetColumn(count, 2);
        line.Children.Add(count);

        // Proportional by star columns, so the bar needs no width of its own to be measured against.
        var bar = new Grid { Height = 6, Margin = new Thickness(22, 0, 0, 0) };
        bar.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(fraction, GridUnitType.Star) });
        bar.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1 - fraction, GridUnitType.Star) });
        var track = new Border { Background = ink, Opacity = 0.15, CornerRadius = new CornerRadius(3) };
        Grid.SetColumnSpan(track, 2);
        bar.Children.Add(track);
        if (fraction > 0)
        {
            bar.Children.Add(new Border { Background = ink, Opacity = chosen ? 1 : 0.45, CornerRadius = new CornerRadius(3) });
        }
        return new StackPanel { Spacing = 4, Children = { line, bar } };
    }

    private static void ShowVoters(FrameworkElement anchor, string optionText, IReadOnlyList<long> voters, Func<long, string> name)
    {
        var list = new StackPanel { Spacing = 6, MaxWidth = 280 };
        list.Children.Add(new TextBlock { Text = optionText, FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap });
        foreach (var voter in voters)
        {
            list.Children.Add(new TextBlock { Text = name(voter), TextWrapping = TextWrapping.Wrap });
        }
        new Flyout { Content = list }.ShowAt(anchor);
    }
}
