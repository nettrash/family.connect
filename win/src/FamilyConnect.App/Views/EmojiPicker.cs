using FamilyConnect.Core;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace FamilyConnect.App.Views;

/// <summary>
/// "More reactions…": the apps' whole catalogue, a section at a time — a row of tabs, each its section's first emoji,
/// over a grid of the section's emoji (the web client's <c>EmojiPicker</c>). A flyout, so it opens over the conversation
/// without taking it away.
/// </summary>
internal static class EmojiPicker
{
    public static void Show(FrameworkElement anchor, IStringCatalog say, Action<string> pick)
    {
        var tabs = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2 };
        var heading = new TextBlock { FontWeight = FontWeights.SemiBold, Margin = new Thickness(6, 4, 0, 2) };
        var grid = new GridView
        {
            SelectionMode = ListViewSelectionMode.None,
            IsItemClickEnabled = true,
            Width = 344,
            MaxHeight = 280,
        };
        var panel = new StackPanel { Spacing = 4 };
        AutomationProperties.SetName(panel, say.Get("More reactions"));
        panel.Children.Add(tabs);
        panel.Children.Add(heading);
        panel.Children.Add(grid);
        var flyout = new Flyout { Content = panel };

        var buttons = new List<Button>();
        void ShowSection(int index)
        {
            var section = EmojiCatalog.Categories[index];
            heading.Text = Name(section.Name, say);
            AutomationProperties.SetName(grid, heading.Text);
            grid.Items.Clear();
            foreach (var emoji in section.Emoji)
            {
                grid.Items.Add(new TextBlock { Text = emoji, FontSize = 22, HorizontalAlignment = HorizontalAlignment.Center });
            }
            for (var at = 0; at < buttons.Count; at++)
            {
                buttons[at].Background = at == index
                    ? (Brush)Application.Current.Resources["SubtleFillColorSecondaryBrush"]
                    : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            }
        }
        for (var index = 0; index < EmojiCatalog.Categories.Count; index++)
        {
            var section = EmojiCatalog.Categories[index];
            var at = index;
            var tab = new Button
            {
                Content = new TextBlock { Text = section.Emoji.Count > 0 ? section.Emoji[0] : "·", FontSize = 18 },
                BorderThickness = new Thickness(0),
                Padding = new Thickness(6, 2, 6, 2),
            };
            ToolTipService.SetToolTip(tab, Name(section.Name, say));
            AutomationProperties.SetName(tab, Name(section.Name, say));
            tab.Click += (_, _) => ShowSection(at);
            buttons.Add(tab);
            tabs.Children.Add(tab);
        }
        grid.ItemClick += (_, e) =>
        {
            if (e.ClickedItem is TextBlock { Text: { Length: > 0 } emoji })
            {
                flyout.Hide();
                pick(emoji);
            }
        };
        ShowSection(0);
        flyout.ShowAt(anchor);
    }

    /// <summary>A section's header, in the reader's language — each spelled out, so the catalogue scan finds its key.</summary>
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
