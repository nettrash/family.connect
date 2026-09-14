using System.Globalization;
using System.Text;
using FamilyConnect.App.Logic;
using FamilyConnect.App.Services;
using FamilyConnect.Core;
using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;
using Microsoft.UI;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Xaml.Shapes;
using Windows.UI.Text;
using static FamilyConnect.App.Views.Dialogs;
using IcsCalendar = FamilyConnect.Core.Calendar;

namespace FamilyConnect.App.Views;

/// <summary>
/// A note, opened: written by its author, read — and ticked, and answered — by everybody (the web
/// client's <c>NoteSheet</c>, which is the phone's NoteEditor).
/// </summary>
/// <remarks>
/// <para>
/// <b>TWO AUTHORSHIP RULES, BOTH LEGIBLE HERE.</b> Only the author changes the words, the look, the when
/// and where, and takes the note down. ANY member ticks a line and answers an event: those are acts of
/// their own, outside every author gate and never part of the save.
/// </para>
/// <para>
/// <b>A SAVE SENDS ONLY WHAT CHANGED</b>, diffed against the note as it stood when the sheet OPENED
/// (<see cref="NoteDraft.Patch"/>), and a refused save keeps the sheet open with the words and the
/// reason beside them.
/// </para>
/// <para>
/// Not here yet, and in the web client: the sticker preview, and the strip of names a half-typed
/// <c>@</c> could mean — a name typed in full is still resolved at save.
/// </para>
/// </remarks>
internal sealed class NoteSheet
{
    private readonly XamlRoot root;
    private readonly AppServices services;
    private readonly Connection connection;
    private readonly BoardModel board;
    private readonly NoteDto? opened;
    private readonly NoteKind kind;
    private readonly bool editable;
    private readonly NoteDraft draft;
    private readonly IStringCatalog say;
    private readonly Dictionary<long, bool> ticking = [];
    private readonly TextBlock problem = Problem();
    private readonly StackPanel lines = new() { Spacing = 6 };
    private readonly StackPanel answers = new() { Spacing = 6 };
    private ContentDialog dialog = null!;
    private RsvpAnswer? answering;
    private bool answerSending;
    private (bool Queued, RsvpAnswer? Choice) answerQueued;
    private bool drawing;

    private NoteSheet(XamlRoot root, AppServices services, Connection connection, BoardModel board, NoteDto? opened, NoteKind kind, bool mine)
    {
        this.root = root;
        this.services = services;
        this.connection = connection;
        this.board = board;
        this.opened = opened;
        this.kind = kind;
        editable = mine;
        say = services.Say;
        draft = opened is null
            ? NoteDraft.Blank(kind, DateTimeOffset.Now, TimeZoneInfo.Local, Random.Shared.Next)
            : NoteDraft.Of(opened);
    }

    /// <summary>Open a note (or a blank of <paramref name="kind"/> when <paramref name="opened"/> is null), and wait until it closes.</summary>
    public static Task ShowAsync(
        XamlRoot root, AppServices services, Connection connection, BoardModel board, NoteDto? opened, NoteKind kind, bool mine) =>
        new NoteSheet(root, services, connection, board, opened, kind, mine).RunAsync();

    private NoteDto? Current => opened is null ? null : connection.Board.Note(opened.Id) ?? opened;

    private async Task RunAsync()
    {
        var body = new StackPanel { Spacing = 14, MinWidth = 420 };
        if (Current is { } note && note.Attachment is { } picture && kind is NoteKind.Event or NoteKind.Photo)
        {
            body.Children.Add(Picture(picture, kind == NoteKind.Photo));
        }
        if (editable)
        {
            BuildEditor(body);
        }
        else if (Current is { } shown)
        {
            BuildReader(body, shown);
        }
        if (kind == NoteKind.Tasks && Current is not null && !editable)
        {
            body.Children.Add(lines);
            DrawReaderLines();
        }
        if (kind == NoteKind.Event && Current is { } @event)
        {
            body.Children.Add(answers);
            DrawAnswers();
            body.Children.Add(EventActions(@event));
        }
        body.Children.Add(problem);

        dialog = Create(root, NoteSheetText.Title(opened is null, kind, say), new ScrollViewer { Content = body, MaxHeight = 620, Padding = new Thickness(0, 0, 16, 0) });
        dialog.Resources["ContentDialogMaxWidth"] = 720.0;
        if (editable)
        {
            dialog.PrimaryButtonText = say.Get("Save");
            dialog.CloseButtonText = say.Get("Cancel");
            dialog.PrimaryButtonClick += async (_, args) => await SaveAsync(args);
            if (opened is not null)
            {
                dialog.SecondaryButtonText = say.Get("Delete Note");
                dialog.SecondaryButtonClick += (_, args) =>
                {
                    // Asked about first, inside the sheet: one dialog at a time.
                    args.Cancel = true;
                    AskDelete(body);
                };
            }
            RefreshSave();
        }
        else
        {
            dialog.CloseButtonText = say.Get("Done");
            dialog.DefaultButton = ContentDialogButton.Close;
        }
        await dialog.ShowAsync();
    }

    // ---- the author's editor -------------------------------------------------------------------

    private void BuildEditor(StackPanel body)
    {
        var words = new TextBox
        {
            Header = NoteSheetText.FieldLabel(kind, say),
            Text = draft.Text,
            AcceptsReturn = true,
            TextWrapping = TextWrapping.Wrap,
            MinHeight = 90,
            PlaceholderText = kind == NoteKind.Photo ? say.Get("Say something about it (optional)") : string.Empty,
        };
        var counter = Footnote(string.Empty);
        void Count()
        {
            counter.Visibility = NoteText.ShowsCounter(draft.Text) ? Visibility.Visible : Visibility.Collapsed;
            counter.Text = say.Format("%lld characters left", NoteText.Remaining(draft.Text));
        }
        Capped(words, NoteText.MaxTextChars, value =>
        {
            draft.Text = value;
            Count();
            RefreshSave();
        });
        Count();
        body.Children.Add(words);
        body.Children.Add(counter);

        if (kind == NoteKind.Event)
        {
            body.Children.Add(WhenFields());
        }
        if (kind == NoteKind.Tasks)
        {
            body.Children.Add(new TextBlock { Text = say.Get("Things to do"), FontWeight = FontWeights.SemiBold });
            body.Children.Add(lines);
            DrawEditorLines();
        }

        body.Children.Add(Swatches());
        body.Children.Add(Choices(say.Get("Size"), Notes.Sizes, size => size == draft.Size, size => say.Get(size switch
        {
            NoteSize.Small => "Small",
            NoteSize.Large => "Large",
            _ => "Medium",
        }), size => draft.Size = size, _ => null));
        body.Children.Add(Choices(say.Get("Font"), Notes.Fonts, font => font == draft.Font, font => say.Get(font switch
        {
            NoteFont.Serif => "Serif",
            NoteFont.Mono => "Mono",
            NoteFont.Casual => "Casual",
            _ => "Plain",
        }), font => draft.Font = font, font => new FontFamily(Notes.FontFamily(font))));
    }

    /// <summary>The cap where the typing is, counted as the server counts — a full field, never a refused save.</summary>
    private static void Capped(TextBox box, int max, Action<string> changed)
    {
        box.TextChanged += (_, _) =>
        {
            var (kept, caret) = NoteText.CapAtCaret(box.Text, box.SelectionStart, max);
            if (kept != box.Text)
            {
                box.Text = kept;
                box.SelectionStart = caret;
            }
            changed(kept);
        };
    }

    private void RefreshSave()
    {
        if (dialog is not null && editable)
        {
            dialog.IsPrimaryButtonEnabled = draft.Problem(kind, say) != string.Empty;
        }
    }

    private StackPanel WhenFields()
    {
        var zone = TimeZoneInfo.Local;
        DateTimeOffset LocalOf(DatePicker date, TimePicker time)
        {
            var local = date.Date.Date + time.Time;
            return new DateTimeOffset(local, zone.GetUtcOffset(local));
        }
        DateTimeOffset Shown(DateTimeOffset? at) => TimeZoneInfo.ConvertTime(at ?? NoteDraft.NextRoundHour(DateTimeOffset.Now, zone), zone);

        var starts = Shown(draft.Starts);
        var startDate = new DatePicker { Header = say.Get("Starts"), Date = starts };
        var startTime = new TimePicker { Time = starts.TimeOfDay, MinuteIncrement = 5 };
        var hasEnd = new CheckBox { Content = say.Get("Has an end"), IsChecked = draft.HasEnd };
        var ends = Shown(draft.Ends);
        var endDate = new DatePicker { Header = say.Get("Ends"), Date = ends };
        var endTime = new TimePicker { Time = ends.TimeOfDay, MinuteIncrement = 5 };
        var endRow = Row2(endDate, endTime);
        endRow.Visibility = draft.HasEnd ? Visibility.Visible : Visibility.Collapsed;
        var place = new TextBox { Header = say.Get("Place"), PlaceholderText = say.Get("Where"), Text = draft.Place };

        var syncing = false;
        void StartChanged()
        {
            if (syncing)
            {
                return;
            }
            draft.Starts = LocalOf(startDate, startTime);
            draft.KeepEndAfterStart();
            syncing = true;
            var end = Shown(draft.Ends);
            endDate.Date = end;
            endTime.Time = end.TimeOfDay;
            syncing = false;
            RefreshSave();
        }
        void EndChanged()
        {
            if (!syncing)
            {
                draft.Ends = LocalOf(endDate, endTime);
                RefreshSave();
            }
        }
        startDate.DateChanged += (_, _) => StartChanged();
        startTime.TimeChanged += (_, _) => StartChanged();
        endDate.DateChanged += (_, _) => EndChanged();
        endTime.TimeChanged += (_, _) => EndChanged();
        hasEnd.Checked += (_, _) =>
        {
            draft.HasEnd = true;
            endRow.Visibility = Visibility.Visible;
            RefreshSave();
        };
        hasEnd.Unchecked += (_, _) =>
        {
            draft.HasEnd = false;
            endRow.Visibility = Visibility.Collapsed;
            RefreshSave();
        };
        Capped(place, NoteText.MaxPlaceChars, value => draft.Place = value);

        var fields = new StackPanel { Spacing = 8 };
        fields.Children.Add(new TextBlock { Text = say.Get("When"), FontWeight = FontWeights.SemiBold });
        fields.Children.Add(Row2(startDate, startTime));
        fields.Children.Add(hasEnd);
        fields.Children.Add(endRow);
        fields.Children.Add(place);
        return fields;
    }

    private static StackPanel Row2(FrameworkElement first, FrameworkElement second)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        second.VerticalAlignment = VerticalAlignment.Bottom;
        row.Children.Add(first);
        row.Children.Add(second);
        return row;
    }

    /// <summary>The list as its author writes it: the same boxes everybody ticks, with the words beside them.</summary>
    private void DrawEditorLines()
    {
        lines.Children.Clear();
        for (var at = 0; at < draft.Lines.Count; at++)
        {
            var index = at;
            var line = draft.Lines[at];
            var row = new Grid { ColumnSpacing = 8 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.Children.Add(TickBox(line.Id, line.Text));
            var text = new TextBox { Text = line.Text, PlaceholderText = say.Get("Thing to do") };
            AutomationProperties.SetName(text, say.Get("Thing to do"));
            Capped(text, NoteText.MaxTaskItemChars, value =>
            {
                if (index < draft.Lines.Count)
                {
                    draft.Lines[index] = draft.Lines[index] with { Text = value };
                    RefreshSave();
                }
            });
            Grid.SetColumn(text, 1);
            row.Children.Add(text);
            var remove = new Button { Content = "×" };
            AutomationProperties.SetName(remove, say.Get("Remove"));
            ToolTipService.SetToolTip(remove, say.Get("Remove"));
            remove.Click += (_, _) =>
            {
                if (index < draft.Lines.Count)
                {
                    draft.Lines.RemoveAt(index);
                    DrawEditorLines();
                    RefreshSave();
                }
            };
            Grid.SetColumn(remove, 2);
            row.Children.Add(remove);
            lines.Children.Add(row);
        }
        // Held to the server's ceiling here, where somebody can see why.
        var add = new Button { Content = say.Get("Add a thing"), IsEnabled = draft.Lines.Count < NoteText.MaxTaskItems };
        add.Click += (_, _) =>
        {
            draft.Lines.Add(new DraftLine(null, string.Empty));
            DrawEditorLines();
            RefreshSave();
        };
        lines.Children.Add(add);
        DoneLine();
    }

    /// <summary>The list as everybody else reads it — and ticks it.</summary>
    private void DrawReaderLines()
    {
        lines.Children.Clear();
        lines.Children.Add(new TextBlock { Text = say.Get("Things to do"), FontWeight = FontWeights.SemiBold });
        var items = Current?.TaskList ?? [];
        if (items.Count == 0)
        {
            lines.Children.Add(Footnote(say.Get("Nothing on this list yet.")));
        }
        foreach (var item in items)
        {
            lines.Children.Add(TickBox(item.Id, item.Text, labelled: true));
        }
        DoneLine();
    }

    private void DoneLine()
    {
        var items = Current?.TaskList ?? [];
        if (items.Count > 0)
        {
            var done = items.Count(item => Ticked(item.Id) ?? false);
            lines.Children.Add(Footnote(NoteText.DoneOf(done, items.Count, say)));
        }
    }

    /// <summary>The tap's own answer first, then the note's: a box that waited for the round trip would feel broken.</summary>
    private bool? Ticked(long? id) =>
        id is not { } known ? null
        : ticking.TryGetValue(known, out var sent) ? sent
        : Current?.TaskList.Any(item => item.Id == known && item.Done) ?? false;

    /// <summary>
    /// One line's box. A line never saved has no id, so there is nothing to tick yet: the box is there —
    /// the row would jump when it appeared — and disabled, which is also what says why.
    /// </summary>
    private CheckBox TickBox(long? id, string text, bool labelled = false)
    {
        var box = new CheckBox
        {
            IsChecked = Ticked(id) ?? false,
            IsEnabled = id is not null && opened is not null,
            MinWidth = 0,
        };
        if (labelled)
        {
            box.Content = text;
        }
        AutomationProperties.SetName(box, text.Trim().Length > 0 ? text.Trim() : say.Get("Done"));
        box.Click += (_, _) =>
        {
            if (id is { } itemId && opened is not null)
            {
                _ = TickAsync(itemId, box.IsChecked == true);
            }
        };
        return box;
    }

    private async Task TickAsync(long itemId, bool done)
    {
        // One request per line at a time: a second tap while the first is out is the tap that would undo it.
        if (opened is null || !ticking.TryAdd(itemId, done))
        {
            return;
        }
        problem.Visibility = Visibility.Collapsed;
        var error = await Safely(() => board.TickAsync(opened.Id, itemId, done));
        ticking.Remove(itemId);
        if (error is not null)
        {
            ShowProblem(problem, say.Get("Couldn't tick that off."));
        }
        if (editable)
        {
            DrawEditorLines();
        }
        else
        {
            DrawReaderLines();
        }
    }

    private StackPanel Swatches()
    {
        var panel = new StackPanel { Spacing = 6 };
        panel.Children.Add(new TextBlock { Text = say.Get("Colour"), FontWeight = FontWeights.SemiBold });
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        var buttons = new List<ToggleButton>();
        foreach (var name in Notes.Colors)
        {
            var swatch = new ToggleButton
            {
                IsChecked = draft.Color == name,
                Padding = new Thickness(4),
                Content = new Ellipse
                {
                    Width = 22,
                    Height = 22,
                    Fill = new SolidColorBrush(Hex(Notes.ColorHex(name))),
                    Stroke = new SolidColorBrush(ColorHelper.FromArgb(0x33, 0, 0, 0)),
                },
            };
            // The colour's own name, as the web client labels its swatches.
            AutomationProperties.SetName(swatch, name);
            swatch.Click += (_, _) =>
            {
                draft.Color = name;
                foreach (var other in buttons)
                {
                    other.IsChecked = ReferenceEquals(other, swatch);
                }
            };
            buttons.Add(swatch);
            row.Children.Add(swatch);
        }
        panel.Children.Add(row);
        return panel;
    }

    private static RadioButtons Choices<T>(
        string header, IEnumerable<T> all, Func<T, bool> chosen, Func<T, string> title, Action<T> choose, Func<T, FontFamily?> face)
    {
        var choices = new RadioButtons { Header = header, MaxColumns = 4 };
        var values = all.ToList();
        foreach (var value in values)
        {
            var button = new RadioButton { Content = title(value) };
            if (face(value) is { } family)
            {
                // Each face's name written IN that face.
                button.FontFamily = family;
            }
            choices.Items.Add(button);
            if (chosen(value))
            {
                choices.SelectedIndex = choices.Items.Count - 1;
            }
        }
        choices.SelectionChanged += (_, _) =>
        {
            if (choices.SelectedIndex >= 0)
            {
                choose(values[choices.SelectedIndex]);
            }
        };
        return choices;
    }

    private async Task SaveAsync(ContentDialogButtonClickEventArgs args)
    {
        var deferral = args.GetDeferral();
        try
        {
            if (draft.Problem(kind, say) is { } reason)
            {
                args.Cancel = true;
                if (reason.Length > 0)
                {
                    ShowProblem(problem, reason);
                }
                return;
            }
            ApiError? error;
            if (opened is null)
            {
                var at = (0.25 + (Random.Shared.NextDouble() * 0.4), 0.25 + (Random.Shared.NextDouble() * 0.4));
                error = await Safely(() => board.CreateAsync(draft.NewNote(kind, at, connection.Chats.Members())));
            }
            else
            {
                var patch = draft.Patch(opened, connection.Chats.Members());
                error = NoteDraft.IsEmpty(patch) ? null : await Safely(() => board.PatchAsync(opened.Id, patch));
            }
            if (error is not null)
            {
                args.Cancel = true;
                ShowProblem(problem, NoteSheetText.Failure(error, say));
            }
        }
        finally
        {
            deferral.Complete();
        }
    }

    private void AskDelete(StackPanel body)
    {
        if (opened is null || body.Children.OfType<StackPanel>().Any(panel => panel.Tag as string == "confirm"))
        {
            return;
        }
        var confirm = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Tag = "confirm" };
        confirm.Children.Add(new TextBlock { Text = say.Get("Delete this note?"), VerticalAlignment = VerticalAlignment.Center, FontWeight = FontWeights.SemiBold });
        var keep = new Button { Content = say.Get("Keep") };
        keep.Click += (_, _) => body.Children.Remove(confirm);
        var delete = new Button
        {
            Content = say.Get("Delete"),
            Foreground = new SolidColorBrush(Colors.White),
            Background = (Brush)Application.Current.Resources["SystemFillColorCriticalBrush"],
        };
        delete.Click += async (_, _) =>
        {
            delete.IsEnabled = false;
            var error = await Safely(() => board.DeleteAsync(opened.Id));
            if (error is null)
            {
                dialog.Hide();
                return;
            }
            delete.IsEnabled = true;
            ShowProblem(problem, NoteSheetText.Failure(error, say));
        };
        confirm.Children.Add(keep);
        confirm.Children.Add(delete);
        body.Children.Insert(0, confirm);
    }

    // ---- everybody's -----------------------------------------------------------------------------

    private void BuildReader(StackPanel body, NoteDto note)
    {
        if (kind == NoteKind.Event && note.StartsAt is { } starts)
        {
            var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
            if (EventText.DateBlock(starts, services.Culture, TimeZoneInfo.Local) is { } date)
            {
                var page = new StackPanel { Padding = new Thickness(8, 4, 8, 6) };
                page.Children.Add(new TextBlock { Text = date.Day, FontSize = 22, FontWeight = FontWeights.Bold, HorizontalAlignment = HorizontalAlignment.Center });
                page.Children.Add(new TextBlock
                {
                    Text = date.Month.ToUpper(services.Culture),
                    FontSize = 11,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    Foreground = new SolidColorBrush(ColorHelper.FromArgb(0xD9, 0xB2, 0x26, 0x1E)),
                });
                row.Children.Add(new Border { Child = page, CornerRadius = new CornerRadius(6), BorderThickness = new Thickness(1), BorderBrush = new SolidColorBrush(ColorHelper.FromArgb(0x22, 0, 0, 0)) });
            }
            var words = new StackPanel { VerticalAlignment = VerticalAlignment.Center };
            words.Children.Add(Text(EventText.When(starts, note.EndsAt, services.Culture, TimeZoneInfo.Local)));
            if (note.Place is { Length: > 0 } place)
            {
                words.Children.Add(Secondary(place));
            }
            row.Children.Add(words);
            body.Children.Add(row);
        }
        if (WallText.CaptionOf(note).Length > 0)
        {
            body.Children.Add(new TextBlock
            {
                Text = note.Text,
                TextWrapping = TextWrapping.Wrap,
                IsTextSelectionEnabled = true,
                FontSize = 16,
                FontFamily = new FontFamily(Notes.FontFamily(Notes.FontFrom(note.Font))),
            });
        }
        var author = WallText.AuthorName(note, connection.Chats.Reader, id => connection.Chats.Member(id)?.DisplayName, say);
        body.Children.Add(Footnote(say.Format("Written by %@", author)));
    }

    /// <summary>Who is coming, BY NAME — the card has room for the counts, the note for the people — and the reader's own answer.</summary>
    private void DrawAnswers()
    {
        answers.Children.Clear();
        if (Current is not { } note)
        {
            return;
        }
        var any = false;
        foreach (var answer in Notes.Answers)
        {
            var names = (note.Rsvps ?? [])
                .Where(rsvp => rsvp.Answer == Notes.NameOf(answer))
                .Select(rsvp => connection.Chats.Member(rsvp.UserId)?.DisplayName is { Length: > 0 } name ? name : say.Get("Someone"))
                .ToList();
            if (names.Count == 0)
            {
                continue;
            }
            any = true;
            var line = new TextBlock { TextWrapping = TextWrapping.Wrap };
            line.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = $"{Notes.Title(answer, say)}  ", FontWeight = FontWeights.SemiBold });
            line.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = string.Join(", ", names) });
            answers.Children.Add(line);
        }
        if (!any)
        {
            // A sentence, not an empty list: a member who has not answered is in no group at all.
            answers.Children.Add(Footnote(say.Get("Nobody has answered yet.")));
        }

        answers.Children.Add(new TextBlock { Text = say.Get("Are you coming?"), FontWeight = FontWeights.SemiBold, Margin = new Thickness(0, 6, 0, 0) });
        var mine = answerSending || answerQueued.Queued ? answering : Notes.AnswerFrom(note.MyAnswer(connection.Chats.Reader));
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        foreach (var choice in new RsvpAnswer?[] { null, RsvpAnswer.Going, RsvpAnswer.Maybe, RsvpAnswer.No })
        {
            var button = new ToggleButton
            {
                Content = choice is { } picked ? Notes.Title(picked, say) : say.Get("No answer"),
                IsChecked = mine == choice,
            };
            button.Click += (_, _) => Answer(choice);
            row.Children.Add(button);
        }
        answers.Children.Add(row);
    }

    /// <summary>One answer at a time, the latest waiting: answers sent side by side land in whatever order the server takes them.</summary>
    private void Answer(RsvpAnswer? choice)
    {
        answering = choice;
        if (answerSending)
        {
            answerQueued = (true, choice);
            DrawAnswers();
            return;
        }
        answerSending = true;
        DrawAnswers();
        _ = SendAnswerAsync(choice);
    }

    private async Task SendAnswerAsync(RsvpAnswer? choice)
    {
        if (opened is null)
        {
            return;
        }
        var error = await Safely(() => choice is { } answer ? board.AnswerAsync(opened.Id, answer) : board.RetractAsync(opened.Id));
        if (error is not null)
        {
            ShowProblem(problem, say.Get("Couldn't send your answer."));
        }
        if (answerQueued.Queued)
        {
            var next = answerQueued.Choice;
            answerQueued = (false, null);
            _ = SendAnswerAsync(next);
            return;
        }
        answerSending = false;
        DrawAnswers();
    }

    private StackPanel EventActions(NoteDto note)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        if (note.StartsAt is { } starts)
        {
            var calendar = new Button { Content = say.Get("Add to Calendar") };
            calendar.Click += async (_, _) => await AddToCalendarAsync(Current ?? note, starts);
            row.Children.Add(calendar);
        }
        // The assistant's picture behind it — the AUTHOR's, and only where this server can draw at all.
        if (editable && connection.Session.State.Assistant is { Images: true })
        {
            var backdrop = new Button { Content = BackdropLabel(note) };
            backdrop.Click += async (_, _) =>
            {
                if (drawing)
                {
                    return;
                }
                drawing = true;
                backdrop.IsEnabled = false;
                backdrop.Content = say.Get("Drawing…");
                (AttachmentDto? Drawn, ApiError? Error) answer;
                try
                {
                    answer = await board.DrawBackdropAsync(note.Id);
                }
                catch (Exception e)
                {
                    Diagnostics.Write($"drawing a backdrop: {e.GetType().Name}");
                    answer = (null, ApiError.Transport(e.GetType().Name));
                }
                drawing = false;
                backdrop.IsEnabled = true;
                backdrop.Content = BackdropLabel(Current ?? note);
                if (answer.Error is not null)
                {
                    ShowProblem(problem, say.Get("Couldn't draw that."));
                }
            };
            row.Children.Add(backdrop);
        }
        return row;
    }

    private string BackdropLabel(NoteDto note) =>
        note.Attachment is null ? say.Get("Draw a backdrop") : say.Get("Draw another backdrop");

    /// <summary>
    /// Add to Calendar: built HERE, out of the title, the times and the place — the server carries no
    /// calendar at all — and handed over as one <c>.ics</c> file, with a uid stable for the event so a
    /// calendar that already has it updates rather than keeping two.
    /// </summary>
    private async Task AddToCalendarAsync(NoteDto note, string starts)
    {
        if (EventText.Instant(starts) is not { } from)
        {
            return;
        }
        var title = note.Text ?? string.Empty;
        var ics = IcsCalendar.OneEvent(
            string.Create(CultureInfo.InvariantCulture, $"fc-note-{note.Id}@family.connect"),
            title,
            IcsCalendar.Stamp(from),
            EventText.Instant(note.EndsAt) is { } to ? IcsCalendar.Stamp(to) : null,
            note.Place is { Length: > 0 } place ? place : null,
            IcsCalendar.Stamp(DateTimeOffset.UtcNow));
        try
        {
            await AttachmentSaving.SaveAsync(services.WindowHandle, $"{FileStem(title)}.ics", Encoding.UTF8.GetBytes(ics));
        }
        catch (Exception e)
        {
            Diagnostics.Write($"saving an event: {e.GetType().Name}");
            ShowProblem(problem, say.Get("Something went wrong. Try again."));
        }
    }

    /// <summary>A title as a file name: the words it has, and nothing a file system would refuse — "event" when nothing is left.</summary>
    private static string FileStem(string title)
    {
        var kept = new string([.. title.Select(ch => char.IsLetterOrDigit(ch) || ch is ' ' or '-' or '_' ? ch : ' ')]);
        var trimmed = string.Join(' ', kept.Split(' ', StringSplitOptions.RemoveEmptyEntries));
        return trimmed.Length == 0 ? "event" : trimmed;
    }

    /// <summary>A photo drawn whole, or an event's backdrop as a banner over the note.</summary>
    private Border Picture(AttachmentDto attachment, bool whole)
    {
        var image = new Image { Stretch = whole ? Stretch.Uniform : Stretch.UniformToFill, MaxHeight = whole ? 360 : 140 };
        _ = LoadAsync(image, attachment);
        return new Border { Child = image, CornerRadius = new CornerRadius(8), MaxHeight = whole ? 360 : 140 };
    }

    private async Task LoadAsync(Image image, AttachmentDto attachment)
    {
        var source = AttachmentFiles.SourceFor(attachment);
        if (source == AttachmentFiles.TileSource.None)
        {
            return;
        }
        try
        {
            var (bytes, _) = await connection.Attachments.BytesAsync(attachment, preview: source == AttachmentFiles.TileSource.Preview);
            if (bytes is not null)
            {
                image.Source = await BitmapAsync(bytes);
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"a note's picture: {e.GetType().Name}");
        }
    }

    private static async Task<ApiError?> Safely(Func<Task<ApiError?>> act)
    {
        try
        {
            return await act();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"a board change: {e.GetType().Name}");
            return ApiError.Transport(e.GetType().Name);
        }
    }

    private static Windows.UI.Color Hex(string hex) => ColorHelper.FromArgb(
        0xFF,
        Convert.ToByte(hex.Substring(1, 2), 16),
        Convert.ToByte(hex.Substring(3, 2), 16),
        Convert.ToByte(hex.Substring(5, 2), 16));
}
