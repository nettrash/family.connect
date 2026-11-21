using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Windows.System;

namespace FamilyConnect.App.Views;

/// <summary>The conversation's More reactions flyout.</summary>
internal static class EmojiPicker
{
    public static Flyout Create(IStringCatalog say, string language, IReadOnlyList<string> recent,
        Action<string> pick, Action returnFocus)
    {
        var search = new ReactionSearch(language);
        var input = new TextBox { PlaceholderText = say.Get("Search emoji"), IsSpellCheckEnabled = false };
        AutomationProperties.SetName(input, input.PlaceholderText);
        var tabs = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2 };
        var recentPanel = new StackPanel { Spacing = 2 };
        var heading = Heading();
        var empty = new TextBlock
        {
            Text = say.Get("No emoji found"),
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(6, 8, 6, 8),
            Visibility = Visibility.Collapsed,
        };
        AutomationProperties.SetLiveSetting(empty, AutomationLiveSetting.Polite);

        var panel = new StackPanel { Spacing = 4, Width = 344 };
        AutomationProperties.SetName(panel, say.Get("More reactions"));
        var flyout = new Flyout { Content = panel };
        string? picked = null;
        var restoreFocus = false;
        void Pick(string emoji)
        {
            if (picked is not null)
            {
                return;
            }
            picked = emoji;
            restoreFocus = true;
            flyout.Hide();
        }

        var results = CreateGrid(240, Pick);
        if (recent.Count > 0)
        {
            var recentName = say.Get("Recent emoji");
            recentPanel.Children.Add(Heading(recentName));
            var recentGrid = CreateGrid(120, Pick);
            AutomationProperties.SetName(recentGrid, recentName);
            recentGrid.ItemsSource = recent.Select(search.Describe).ToArray();
            recentPanel.Children.Add(recentGrid);
        }

        panel.Children.Add(input);
        panel.Children.Add(recentPanel);
        panel.Children.Add(tabs);
        panel.Children.Add(heading);
        panel.Children.Add(empty);
        panel.Children.Add(results);

        var categoryIndex = 0;
        var buttons = new List<ToggleButton>();
        void Render()
        {
            var searching = !string.IsNullOrWhiteSpace(input.Text);
            var category = EmojiCatalog.Categories[categoryIndex];
            var items = searching
                ? search.Search(input.Text)
                : category.Emoji.Distinct(StringComparer.Ordinal).Select(search.Describe).ToArray();
            recentPanel.Visibility = !searching && recent.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
            tabs.Visibility = searching ? Visibility.Collapsed : Visibility.Visible;
            heading.Text = searching ? say.Get("Search results") : Name(category.Name, say);
            AutomationProperties.SetName(results, heading.Text);
            results.ItemsSource = items;
            results.Visibility = items.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
            empty.Visibility = items.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
            if (items.Count > 0)
            {
                results.ScrollIntoView(items[0], ScrollIntoViewAlignment.Leading);
            }
            else if (FrameworkElementAutomationPeer.FromElement(empty) is { } peer)
            {
                peer.RaiseAutomationEvent(AutomationEvents.LiveRegionChanged);
            }
        }

        for (var index = 0; index < EmojiCatalog.Categories.Count; index++)
        {
            var category = EmojiCatalog.Categories[index];
            var at = index;
            var tab = new ToggleButton
            {
                Content = new TextBlock { Text = category.Emoji[0], FontSize = 18 },
                BorderThickness = new Thickness(0),
                Padding = new Thickness(6, 2, 6, 2),
                IsChecked = at == categoryIndex,
            };
            var name = Name(category.Name, say);
            ToolTipService.SetToolTip(tab, name);
            AutomationProperties.SetName(tab, name);
            tab.Click += (_, _) =>
            {
                categoryIndex = at;
                for (var i = 0; i < buttons.Count; i++)
                {
                    buttons[i].IsChecked = i == categoryIndex;
                }
                Render();
            };
            buttons.Add(tab);
            tabs.Children.Add(tab);
        }

        input.TextChanged += (_, _) => Render();
        input.PreviewKeyDown += (_, e) =>
        {
            if (e.Key == VirtualKey.Down && results.Visibility == Visibility.Visible
                && results.ContainerFromIndex(0) is GridViewItem first && first.Focus(FocusState.Keyboard))
            {
                e.Handled = true;
            }
        };
        panel.PreviewKeyDown += (_, e) =>
        {
            if (e.Key == VirtualKey.Escape)
            {
                e.Handled = true;
                restoreFocus = true;
                flyout.Hide();
            }
        };
        flyout.Opened += (_, _) => input.Focus(FocusState.Programmatic);
        flyout.Closed += (_, _) =>
        {
            if (restoreFocus)
            {
                returnFocus();
            }
            if (picked is not null)
            {
                pick(picked);
            }
        };
        Render();
        return flyout;
    }

    private static GridView CreateGrid(double maxHeight, Action<string> pick)
    {
        var grid = new GridView
        {
            SelectionMode = ListViewSelectionMode.None,
            IsItemClickEnabled = true,
            DisplayMemberPath = nameof(ReactionSearchResult.Emoji),
            FontSize = 22,
            MaxHeight = maxHeight,
        };
        // GridView moves focus and scrolls through its realised items. Intercept Enter and
        // Space at each cell before the grid's selection behavior sees them.
        void InvokeCell(object sender, KeyRoutedEventArgs e)
        {
            if (e.Key is VirtualKey.Enter or VirtualKey.Space
                && sender is GridViewItem { Content: ReactionSearchResult cell })
            {
                e.Handled = true;
                pick(cell.Emoji);
            }
        }

        grid.ContainerContentChanging += (_, e) =>
        {
            var container = e.ItemContainer;
            container.PreviewKeyDown -= InvokeCell;
            if (!e.InRecycleQueue && e.Item is ReactionSearchResult cell)
            {
                AutomationProperties.SetName(container, cell.Name);
                ToolTipService.SetToolTip(container, cell.Name);
                container.PreviewKeyDown += InvokeCell;
            }
        };
        grid.ItemClick += (_, e) =>
        {
            if (e.ClickedItem is ReactionSearchResult cell)
            {
                pick(cell.Emoji);
            }
        };
        return grid;
    }

    private static TextBlock Heading(string text = "") =>
        new() { Text = text, FontWeight = FontWeights.SemiBold, Margin = new Thickness(6, 4, 0, 2) };

    private static string Name(string canonical, IStringCatalog say) => canonical switch
    {
        "Smileys" => say.Get("Smileys"),
        "Gestures" => say.Get("Gestures"),
        "Hearts" => say.Get("Hearts"),
        "Animals & Nature" => say.Get("Animals & Nature"),
        "Food & Drink" => say.Get("Food & Drink"),
        "Activities" => say.Get("Activities"),
        "Travel & Places" => say.Get("Travel & Places"),
        "Objects & Symbols" => say.Get("Objects & Symbols"),
        _ => canonical,
    };
}
