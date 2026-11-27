using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Data;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.System;

namespace FamilyConnect.App.Views;

/// <summary>The conversation's More reactions flyout.</summary>
public sealed partial class EmojiPicker : UserControl
{
    // The same emoji can occur in recents and the catalogue. Each location needs its own cell.
    private sealed class Cell(ReactionSearchResult result)
    {
        public string Emoji => result.Emoji;
        public string Name => result.Name;
    }

    private sealed class Section(string name, string symbol, IEnumerable<Cell> cells) : List<Cell>(cells)
    {
        public string Name { get; } = name;
        public string Symbol { get; } = symbol;
    }

    private readonly ReactionSearch search;
    private readonly Cell? firstBrowseItem;
    private Cell? browseStart;
    private Cell? requestedFocus;

    private EmojiPicker(IStringCatalog say, string language, IReadOnlyList<string> recent, Action<string> pick)
    {
        InitializeComponent();
        search = new ReactionSearch(language);

        AutomationProperties.SetName(this, say.Get("More reactions"));
        SearchInput.PlaceholderText = say.Get("Search emoji");
        AutomationProperties.SetName(SearchInput, SearchInput.PlaceholderText);
        var clearName = say.Get("Clear search");
        AutomationProperties.SetName(ClearSearch, clearName);
        ToolTipService.SetToolTip(ClearSearch, clearName);
        ResultsHeading.Text = say.Get("Search results");
        AutomationProperties.SetName(Results, ResultsHeading.Text);
        EmptyNotice.Text = say.Get("No emoji found");
        AutomationProperties.SetName(Shortcuts, say.Get("Emoji categories"));

        var sections = new List<Section>();
        if (recent.Count > 0)
        {
            sections.Add(new(say.Get("Recent emoji"), "◷", MakeCells(recent)));
        }
        sections.AddRange(EmojiCatalog.Categories
            .Where(category => category.Emoji.Count > 0)
            .Select(category => new Section(
                SectionName(category.Name, say), category.Emoji[0], MakeCells(category.Emoji))));
        firstBrowseItem = sections.FirstOrDefault()?.FirstOrDefault();
        browseStart = firstBrowseItem;

        WireCells(Catalogue, pick);
        WireCells(Results, pick);
        Catalogue.ContainerContentChanging += (_, e) =>
        {
            if (!e.InRecycleQueue && ReferenceEquals(e.Item, requestedFocus))
            {
                // The container can be prepared before it enters the visual tree.
                DispatcherQueue.TryEnqueue(FocusRequestedCell);
            }
        };
        var groupView = new CollectionViewSource
        {
            IsSourceGrouped = true,
            Source = sections,
        };
        Catalogue.ItemsSource = groupView.View;

        foreach (var section in sections)
        {
            var button = new Button
            {
                Content = section.Symbol,
                FontSize = 18,
                Width = 36,
                Height = 34,
                MinWidth = 0,
                Padding = new Thickness(0),
            };
            AutomationProperties.SetName(button, section.Name);
            ToolTipService.SetToolTip(button, section.Name);
            button.Click += (_, _) => Jump(section[0]);
            Shortcuts.Children.Add(button);
        }

        SearchInput.TextChanged += (_, _) => Render();
        ClearSearch.Click += (_, _) =>
        {
            browseStart = firstBrowseItem;
            requestedFocus = null;
            SearchInput.Focus(FocusState.Programmatic);
            SearchInput.Text = string.Empty;
        };
        SearchInput.PreviewKeyDown += (_, e) =>
        {
            if (e.Key != VirtualKey.Down)
            {
                return;
            }
            if (SearchPane.Visibility == Visibility.Visible)
            {
                e.Handled = Results.ContainerFromIndex(0) is GridViewItem item && item.Focus(FocusState.Keyboard);
            }
            else if (browseStart is { } start)
            {
                // The active category can be far below index zero, outside the realised containers.
                requestedFocus = start;
                ShowCatalogue();
                e.Handled = true;
            }
        };
    }

    internal static Flyout Create(IStringCatalog say, string language, IReadOnlyList<string> recent,
        Action<string> pick, Action returnFocus)
    {
        var flyout = new Flyout();
        string? picked = null;
        var restoreFocus = false;
        var view = new EmojiPicker(say, language, recent, emoji =>
        {
            if (picked is null)
            {
                picked = emoji;
                restoreFocus = true;
                flyout.Hide();
            }
        });
        flyout.Content = view;
        view.PreviewKeyDown += (_, e) =>
        {
            if (e.Key == VirtualKey.Escape)
            {
                e.Handled = true;
                restoreFocus = true;
                flyout.Hide();
            }
        };
        flyout.Opened += (_, _) => view.SearchInput.Focus(FocusState.Programmatic);
        flyout.Closed += (_, _) =>
        {
            view.requestedFocus = null;
            if (restoreFocus)
            {
                returnFocus();
            }
            if (picked is not null)
            {
                pick(picked);
            }
        };
        return flyout;
    }

    private IEnumerable<Cell> MakeCells(IEnumerable<string> emoji) => emoji.Distinct(StringComparer.Ordinal)
        .Select(value => new Cell(search.Describe(value)));

    private void Jump(Cell start)
    {
        browseStart = start;
        requestedFocus = start;
        if (SearchInput.Text.Length != 0)
        {
            SearchInput.Text = string.Empty; // TextChanged renders directly at browseStart.
        }
        else
        {
            ShowCatalogue();
        }
    }

    private void Render()
    {
        ClearSearch.Visibility = SearchInput.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        if (string.IsNullOrWhiteSpace(SearchInput.Text))
        {
            ShowCatalogue();
            return;
        }

        browseStart = firstBrowseItem;
        requestedFocus = null;
        var wasShowingEmpty = SearchPane.Visibility == Visibility.Visible && EmptyNotice.Visibility == Visibility.Visible;
        Catalogue.Visibility = Visibility.Collapsed;
        SearchPane.Visibility = Visibility.Visible;
        var items = search.Search(SearchInput.Text).Select(result => new Cell(result)).ToArray();
        Results.ItemsSource = items;
        Results.Visibility = items.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        EmptyNotice.Visibility = items.Length == 0 ? Visibility.Visible : Visibility.Collapsed;
        if (items.Length > 0)
        {
            Results.UpdateLayout();
            Results.ScrollIntoView(items[0], ScrollIntoViewAlignment.Leading);
        }
        else if (!wasShowingEmpty)
        {
            (FrameworkElementAutomationPeer.FromElement(EmptyNotice)
                ?? FrameworkElementAutomationPeer.CreatePeerForElement(EmptyNotice))
                ?.RaiseAutomationEvent(AutomationEvents.LiveRegionChanged);
        }
    }

    private void ShowCatalogue()
    {
        SearchPane.Visibility = Visibility.Collapsed;
        Catalogue.Visibility = Visibility.Visible;
        Catalogue.UpdateLayout();
        if (ReferenceEquals(browseStart, firstBrowseItem) && FindScrollViewer(Catalogue) is { } scroll)
        {
            scroll.ChangeView(null, 0, null, disableAnimation: true);
        }
        else if (browseStart is { } start)
        {
            Catalogue.ScrollIntoView(start, ScrollIntoViewAlignment.Leading);
        }
        Catalogue.UpdateLayout();
        FocusRequestedCell();
    }

    private void FocusRequestedCell()
    {
        if (requestedFocus is { } cell && Catalogue.ContainerFromItem(cell) is GridViewItem item
            && item.Focus(FocusState.Keyboard))
        {
            requestedFocus = null;
        }
    }

    private static ScrollViewer? FindScrollViewer(DependencyObject root)
    {
        for (var i = 0; i < VisualTreeHelper.GetChildrenCount(root); i++)
        {
            var child = VisualTreeHelper.GetChild(root, i);
            if (child is ScrollViewer scroll)
            {
                return scroll;
            }
            if (FindScrollViewer(child) is { } found)
            {
                return found;
            }
        }
        return null;
    }

    private void ShowSectionName(FrameworkElement sender, DataContextChangedEventArgs args) =>
        ((TextBlock)sender).Text = args.NewValue is Section section ? section.Name : string.Empty;

    private void ShowCellEmoji(FrameworkElement sender, DataContextChangedEventArgs args) =>
        ((TextBlock)sender).Text = args.NewValue is Cell cell ? cell.Emoji : string.Empty;

    private static void WireCells(GridView grid, Action<string> pick)
    {
        // GridView moves focus through all realised items. Invoke on Enter and Space
        // before its selection behavior sees either key.
        void InvokeCell(object sender, KeyRoutedEventArgs e)
        {
            if (e.Key is VirtualKey.Enter or VirtualKey.Space && sender is GridViewItem { Content: Cell cell })
            {
                e.Handled = true;
                pick(cell.Emoji);
            }
        }

        grid.ContainerContentChanging += (_, e) =>
        {
            var container = e.ItemContainer;
            container.PreviewKeyDown -= InvokeCell;
            if (!e.InRecycleQueue && e.Item is Cell cell)
            {
                AutomationProperties.SetName(container, cell.Name);
                ToolTipService.SetToolTip(container, cell.Name);
                container.PreviewKeyDown += InvokeCell;
            }
        };
        grid.ItemClick += (_, e) =>
        {
            if (e.ClickedItem is Cell cell)
            {
                pick(cell.Emoji);
            }
        };
    }

    private static string SectionName(string canonical, IStringCatalog say) => canonical switch
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
