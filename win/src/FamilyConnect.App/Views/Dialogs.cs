using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Storage.Streams;

namespace FamilyConnect.App.Views;

/// <summary>
/// The pieces the settings and family screens are built from, and the dialogs both of them open —
/// a birthday, a password reset, a report, a yes-or-no.
/// </summary>
internal static class Dialogs
{
    public static ContentDialog Create(XamlRoot root, string title, object content)
    {
        var dialog = new ContentDialog
        {
            XamlRoot = root,
            Title = title,
            Content = content,
            DefaultButton = ContentDialogButton.Primary,
        };
        if (Application.Current.Resources.TryGetValue("DefaultContentDialogStyle", out var style) && style is Style styled)
        {
            dialog.Style = styled;
        }
        return dialog;
    }

    /// <summary>
    /// A dialog button that does something: the dialog waits for it, and stays open to say why when
    /// it failed.
    /// </summary>
    public static async Task SettleAsync(
        ContentDialogButtonClickEventArgs args, TextBlock problem, Func<Task<ApiError?>> act, Func<ApiError, string> sentence)
    {
        var deferral = args.GetDeferral();
        try
        {
            ApiError? error;
            try
            {
                error = await act();
            }
            catch (Exception e)
            {
                Diagnostics.Write($"a dialog's action: {e.GetType().Name}");
                error = ApiError.Transport(e.GetType().Name);
            }
            if (error is not null)
            {
                args.Cancel = true;
                ShowProblem(problem, sentence(error));
            }
        }
        finally
        {
            deferral.Complete();
        }
    }

    /// <summary>Asked first: a title, perhaps a sentence, and the one button that does it. Cancel is the default.</summary>
    public static async Task<bool> ConfirmAsync(XamlRoot root, IStringCatalog say, string title, string? message, string confirm)
    {
        var dialog = Create(root, title, message is null ? string.Empty : Text(message));
        dialog.PrimaryButtonText = confirm;
        dialog.CloseButtonText = say.Get("Cancel");
        dialog.DefaultButton = ContentDialogButton.Close;
        return await dialog.ShowAsync() == ContentDialogResult.Primary;
    }

    /// <summary>
    /// A birthday: a month and a day, no year. <paramref name="name"/> is null for the reader's own and
    /// names the member for an owner setting somebody else's. Answers whether anything changed.
    /// </summary>
    public static async Task<bool> BirthdayAsync(
        XamlRoot root,
        AppServices services,
        BirthdayDto? held,
        string? name,
        Func<int, int, Task<ApiError?>> save,
        Func<Task<ApiError?>> clear)
    {
        var say = services.Say;
        var (month, day) = BirthdayRules.Start(held);
        var months = new ComboBox { Header = say.Get("Month"), MinWidth = 180 };
        for (var value = 1; value <= 12; value++)
        {
            months.Items.Add(BirthdayRules.MonthName(value, services.Culture));
        }
        months.SelectedIndex = month - 1;
        var days = new ComboBox { Header = say.Get("Day"), MinWidth = 100 };
        void FillDays(int wanted)
        {
            days.Items.Clear();
            for (var value = 1; value <= BirthdayRules.DaysIn(months.SelectedIndex + 1); value++)
            {
                days.Items.Add(value.ToString(services.Culture));
            }
            // The 31st of a month with 30 is the 30th.
            days.SelectedIndex = Math.Clamp(wanted, 1, days.Items.Count) - 1;
        }
        FillDays(day);
        months.SelectionChanged += (_, _) => FillDays(days.SelectedIndex + 1);

        var fields = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        fields.Children.Add(months);
        fields.Children.Add(days);
        var problem = Problem();
        var dialog = Create(
            root,
            name is null ? say.Get("Birthday") : say.Format("Birthday for %@", name),
            Column(
                fields,
                Footnote(name is null
                    ? say.Get("A day and a month, with no year — so being wished a happy birthday never means publishing your age.")
                    : say.Get("A day and a month, with no year. Everyone in the family sees it.")),
                problem));
        dialog.PrimaryButtonText = say.Get("Save");
        dialog.CloseButtonText = say.Get("Cancel");
        if (held is not null)
        {
            dialog.SecondaryButtonText = say.Get("Remove Birthday");
        }
        dialog.PrimaryButtonClick += async (_, args) => await SettleAsync(
            args, problem,
            () => save(months.SelectedIndex + 1, days.SelectedIndex + 1),
            error => SettingsText.BirthdayFailure(error, say));
        dialog.SecondaryButtonClick += async (_, args) => await SettleAsync(
            args, problem, clear, error => SettingsText.BirthdayFailure(error, say));
        return await dialog.ShowAsync() is ContentDialogResult.Primary or ContentDialogResult.Secondary;
    }

    /// <summary>
    /// The owner's reset. The member is signed out everywhere and needs this password to come back,
    /// and the server has no way to send it to them — so the owner is told to, somewhere safe.
    /// </summary>
    public static async Task ResetPasswordAsync(XamlRoot root, IStringCatalog say, string name, Func<string, Task<ApiError?>> reset)
    {
        var fresh = new PasswordBox { Header = say.Get("New Password") };
        var again = new PasswordBox { Header = say.Get("Confirm New Password") };
        var problem = Problem();
        var dialog = Create(
            root,
            say.Get("Reset Password"),
            Column(
                Text(say.Format("New password for %@", name)),
                fresh,
                again,
                Footnote(say.Format(
                    "%@ will be signed out on every device and will need this password to sign back in. Tell it to them somewhere safe — the server has no way to email it.",
                    name)),
                problem));
        dialog.PrimaryButtonText = say.Get("Reset");
        dialog.CloseButtonText = say.Get("Cancel");
        dialog.DefaultButton = ContentDialogButton.Close;
        dialog.IsPrimaryButtonEnabled = false;
        fresh.PasswordChanged += (_, _) => dialog.IsPrimaryButtonEnabled = fresh.Password.Length > 0;
        dialog.PrimaryButtonClick += async (_, args) =>
        {
            if (SettingsText.PasswordProblem(fresh.Password, again.Password, say) is { } wrong)
            {
                args.Cancel = true;
                ShowProblem(problem, wrong);
                return;
            }
            await SettleAsync(
                args, problem, () => reset(fresh.Password), _ => say.Get("Couldn't reset that password. Try again."));
        };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary)
        {
            return;
        }
        var done = Create(root, say.Get("Password reset"), Text(say.Format("%@ has been signed out everywhere.", name)));
        done.CloseButtonText = say.Get("OK");
        done.DefaultButton = ContentDialogButton.Close;
        await done.ShowAsync();
    }

    /// <summary>
    /// Reporting a member, or one of their messages: the four reasons, the disclosure, and — where
    /// the operator published one — the contact for when the problem is the owner, shown verbatim.
    /// Answers the reason chosen, or null.
    /// </summary>
    public static async Task<string?> ReportAsync(XamlRoot root, IStringCatalog say, string name, bool aboutMessage, string? supportContact)
    {
        var reasons = new RadioButtons { Header = say.Format("Why are you reporting %@?", name) };
        foreach (var (code, key) in FamilyText.Reasons)
        {
            reasons.Items.Add(new RadioButton { Content = say.Get(key), Tag = code });
            if (code == FamilyText.FirstReason)
            {
                reasons.SelectedIndex = reasons.Items.Count - 1;
            }
        }
        var content = Column(reasons, Footnote(FamilyText.ReportDisclosure(aboutMessage, say)));
        if (!string.IsNullOrEmpty(supportContact))
        {
            content.Children.Add(new TextBlock { Text = say.Get("If the problem is the owner"), FontWeight = FontWeights.SemiBold });
            // Verbatim, and never made a link: an address, a URL or a whole sentence all read as sent.
            content.Children.Add(new TextBlock { Text = supportContact, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true });
            content.Children.Add(Footnote(say.Get("This server's operator published this contact.")));
        }
        var dialog = Create(root, say.Get("Report"), content);
        dialog.PrimaryButtonText = say.Get("Report");
        dialog.CloseButtonText = say.Get("Cancel");
        return await dialog.ShowAsync() == ContentDialogResult.Primary && reasons.SelectedItem is RadioButton { Tag: string chosen }
            ? chosen
            : null;
    }

    /// <summary>
    /// Reporting an ASSISTANT reply: the same four reasons, an optional note, and a different
    /// disclosure — the people who run the server read this one, never the family owner
    /// (docs/protocol.md, "Reporting the assistant"). Answers the reason and the note, or null if
    /// the reader changed their mind.
    /// </summary>
    public static async Task<(string Reason, string? Note)?> ReportAssistantAsync(
        XamlRoot root, IStringCatalog say, string? supportContact)
    {
        var reasons = new RadioButtons { Header = say.Get("What was wrong with this reply?") };
        foreach (var (code, key) in FamilyText.Reasons)
        {
            reasons.Items.Add(new RadioButton { Content = say.Get(key), Tag = code });
            if (code == FamilyText.FirstReason)
            {
                reasons.SelectedIndex = reasons.Items.Count - 1;
            }
        }
        // Free text, which a member report does not have: the reader here is one operator rather
        // than a nine-language owner, and "it invented a person" is not any of four words. The cap
        // is the server's (1,000 characters), enforced here so nobody types past it and is refused.
        var note = new TextBox
        {
            PlaceholderText = say.Get("Say something about it (optional)"),
            AcceptsReturn = true,
            TextWrapping = TextWrapping.Wrap,
            MaxLength = 1000,
            Height = 92,
        };
        var content = Column(reasons, note, Footnote(FamilyText.AssistantReportDisclosure(say)));
        if (!string.IsNullOrEmpty(supportContact))
        {
            content.Children.Add(new TextBlock { Text = supportContact, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true });
            content.Children.Add(Footnote(say.Get("This server's operator published this contact.")));
        }
        var dialog = Create(root, say.Get("Report this reply"), content);
        dialog.PrimaryButtonText = say.Get("Report");
        dialog.CloseButtonText = say.Get("Cancel");
        return await dialog.ShowAsync() == ContentDialogResult.Primary
            && reasons.SelectedItem is RadioButton { Tag: string chosen }
            ? (chosen, string.IsNullOrWhiteSpace(note.Text) ? null : note.Text.Trim())
            : null;
    }

    public static StackPanel Column(params UIElement[] children)
    {
        var column = new StackPanel { Spacing = 12, MinWidth = 320 };
        foreach (var child in children)
        {
            column.Children.Add(child);
        }
        return column;
    }

    public static StackPanel Group(string title, params UIElement[] rows)
    {
        var group = new StackPanel { Spacing = 6 };
        group.Children.Add(new TextBlock
        {
            Text = title,
            Style = (Style)Application.Current.Resources["BodyStrongTextBlockStyle"],
        });
        foreach (var row in rows)
        {
            group.Children.Add(row);
        }
        return group;
    }

    public static Grid Row(string label, string value) => Row(new TextBlock { Text = label, TextWrapping = TextWrapping.Wrap }, value);

    public static Grid Row(FrameworkElement label, string value)
    {
        var row = new Grid { ColumnSpacing = 12 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.Children.Add(label);
        var number = new TextBlock { Text = value, VerticalAlignment = VerticalAlignment.Center };
        Grid.SetColumn(number, 1);
        row.Children.Add(number);
        return row;
    }

    public static TextBlock Text(string text) => new() { Text = text, TextWrapping = TextWrapping.Wrap };

    public static TextBlock Secondary(string text) => new()
    {
        Text = text,
        TextWrapping = TextWrapping.Wrap,
        Foreground = (Brush)Application.Current.Resources["TextFillColorSecondaryBrush"],
    };

    public static TextBlock Footnote(string text) => new()
    {
        Text = text,
        TextWrapping = TextWrapping.Wrap,
        Style = (Style)Application.Current.Resources["CaptionTextBlockStyle"],
        Foreground = (Brush)Application.Current.Resources["TextFillColorSecondaryBrush"],
    };

    public static Border Capsule(string text) => new()
    {
        Child = new TextBlock
        {
            Text = text,
            FontSize = 12,
            Foreground = (Brush)Application.Current.Resources["TextOnAccentFillColorPrimaryBrush"],
        },
        CornerRadius = new CornerRadius(8),
        Padding = new Thickness(8, 1, 8, 1),
        VerticalAlignment = VerticalAlignment.Center,
        Background = (Brush)Application.Current.Resources["AccentFillColorDefaultBrush"],
    };

    public static TextBlock Problem() => new()
    {
        TextWrapping = TextWrapping.Wrap,
        Visibility = Visibility.Collapsed,
        Foreground = (Brush)Application.Current.Resources["SystemFillColorCriticalBrush"],
    };

    public static void ShowProblem(TextBlock problem, string sentence)
    {
        problem.Text = sentence;
        problem.Visibility = Visibility.Visible;
    }

    public static async Task<BitmapImage> BitmapAsync(byte[] bytes)
    {
        using var stream = new InMemoryRandomAccessStream();
        using (var writer = new DataWriter(stream))
        {
            writer.WriteBytes(bytes);
            await writer.StoreAsync();
            writer.DetachStream();
        }
        stream.Seek(0);
        var bitmap = new BitmapImage();
        await bitmap.SetSourceAsync(stream);
        return bitmap;
    }
}
