using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace FamilyConnect.App.Views;

/// <summary>
/// The form behind "Poll": a question and two to ten options. Create stays off until the draft is a poll
/// the server would take (<see cref="PollDraft"/>); the caller sends.
/// </summary>
internal static class PollComposer
{
    public static async Task<(string Question, string[] Options)?> AskAsync(XamlRoot root, IStringCatalog say)
    {
        var draft = new PollDraft();
        var question = new TextBox
        {
            Header = say.Get("Question"),
            PlaceholderText = say.Get("Ask the family something…"),
            TextWrapping = TextWrapping.Wrap,
            // The body's limit is 4000 characters; UTF-16 units never count fewer, so this never lets one over.
            MaxLength = 4000,
        };
        var rows = new StackPanel { Spacing = 6 };
        var add = new HyperlinkButton { Content = say.Get("Add option") };
        var form = Dialogs.Column(
            question,
            Dialogs.Footnote(say.Get("The question is the message everyone sees.")),
            new TextBlock { Text = say.Get("Options"), FontWeight = FontWeights.SemiBold, Margin = new Thickness(0, 8, 0, 0) },
            rows,
            add,
            Dialogs.Footnote(say.Get("Between 2 and 10 options. They can't be changed once the poll is sent.")));
        var dialog = Dialogs.Create(root, say.Get("New poll"), new ScrollViewer { Content = form, MaxHeight = 520 });
        dialog.PrimaryButtonText = say.Get("Create");
        dialog.CloseButtonText = say.Get("Cancel");

        void Refresh()
        {
            dialog.IsPrimaryButtonEnabled = draft.Checked() is not null;
            add.Visibility = draft.MayAdd ? Visibility.Visible : Visibility.Collapsed;
        }

        void Draw(int? focus = null)
        {
            rows.Children.Clear();
            for (var index = 0; index < draft.Options.Count; index++)
            {
                var at = index;
                var box = new TextBox { Text = draft.Options[at], PlaceholderText = say.Get("Option") };
                AutomationProperties.SetName(box, say.Get("Option"));
                box.TextChanged += (_, _) =>
                {
                    draft.Set(at, box.Text);
                    Refresh();
                };
                var row = new Grid { ColumnSpacing = 6 };
                row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
                row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
                row.Children.Add(box);
                if (draft.MayRemove)
                {
                    var remove = new Button { Content = new SymbolIcon(Symbol.Cancel) };
                    AutomationProperties.SetName(remove, say.Get("Remove option"));
                    ToolTipService.SetToolTip(remove, say.Get("Remove option"));
                    Grid.SetColumn(remove, 1);
                    remove.Click += (_, _) =>
                    {
                        draft.Remove(at);
                        Draw();
                    };
                    row.Children.Add(remove);
                }
                rows.Children.Add(row);
                if (focus == at)
                {
                    box.Loaded += (_, _) => box.Focus(FocusState.Programmatic);
                }
            }
            Refresh();
        }

        question.TextChanged += (_, _) =>
        {
            draft.Question = question.Text;
            Refresh();
        };
        add.Click += (_, _) =>
        {
            if (draft.Add())
            {
                Draw(focus: draft.Options.Count - 1);
            }
        };
        Draw();
        return await dialog.ShowAsync() == ContentDialogResult.Primary ? draft.Checked() : null;
    }
}
