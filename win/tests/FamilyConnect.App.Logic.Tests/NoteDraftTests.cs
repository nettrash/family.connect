using FamilyConnect.Core;
using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>The note editor's draft — the web client's Draft tests, ported.</summary>
public sealed class NoteDraftTests
{
    private const long Me = 7;
    private const long Anna = 11;
    private static readonly IStringCatalog Say = EnglishCatalog.Instance;
    private static readonly TimeZoneInfo Utc = TimeZoneInfo.Utc;
    private static readonly DateTimeOffset Now = new(2026, 9, 10, 14, 37, 0, TimeSpan.Zero);

    private static NoteDto Note(long id, string text) => new(id, Me, Text: text, X: 0.1, Y: 0.1);

    private static NoteDto EventNote(long id, string? endsAt = null) => new(
        id, Me, Kind: "event", Text: "Picnic", Color: "blue", StartsAt: "2026-09-12T11:00:00Z", EndsAt: endsAt, Place: "The park");

    [Fact]
    public void ADraftPatchesOnlyWhatItChanged()
    {
        var stored = Note(1, "Milk") with { Size = "huge", Font = "gothic", Color = "pink" };
        // A size and a face from a newer server draw as the defaults and are NOT written back as them.
        var draft = NoteDraft.Of(stored);
        Assert.Equal(NoteSize.Medium, draft.Size);
        Assert.True(NoteDraft.IsEmpty(draft.Patch(stored, [])), "nothing changed, nothing sent");

        draft.Text = "Milk and eggs";
        Assert.Equal(new NotePatch(Text: "Milk and eggs"), draft.Patch(stored, []));

        var padded = NoteDraft.Of(stored);
        padded.Text = "Milk  ";
        Assert.True(NoteDraft.IsEmpty(padded.Patch(stored, [])), "the server trims");

        var bigger = NoteDraft.Of(stored);
        bigger.Size = NoteSize.Large;
        Assert.Equal("large", bigger.Patch(stored, []).Size);

        // An event: its end taken off is a null, its place cleared an empty string, a start that did not move is not sent.
        var ended = EventNote(2, "2026-09-12T15:00:00Z");
        var eventDraft = NoteDraft.Of(ended);
        Assert.True(eventDraft.HasEnd);
        Assert.True(NoteDraft.IsEmpty(eventDraft.Patch(ended, [])));
        eventDraft.HasEnd = false;
        eventDraft.Place = "  ";
        var patch = eventDraft.Patch(ended, []);
        Assert.True(patch.ClearsEnd);
        Assert.Null(patch.EndsAt);
        Assert.Equal(string.Empty, patch.Place);
        Assert.Null(patch.StartsAt);

        // An event with no end offers an hour after its start to begin from, with the end still off.
        var open = NoteDraft.Of(EventNote(3));
        Assert.False(open.HasEnd);
        Assert.Equal(open.Starts!.Value.AddHours(1), open.Ends);

        var moved = NoteDraft.Of(ended);
        moved.Starts = moved.Starts!.Value.AddHours(1);
        Assert.Equal("2026-09-12T12:00:00Z", moved.Patch(ended, []).StartsAt);

        // A text note never sends an event's fields.
        var plain = NoteDraft.Of(stored);
        plain.Place = "Somewhere";
        plain.HasEnd = true;
        Assert.True(NoteDraft.IsEmpty(plain.Patch(stored, [])));
    }

    [Fact]
    public void WhatMayBeSavedDependsOnTheKind()
    {
        var draft = NoteDraft.Blank(NoteKind.Text, Now, Utc, _ => 0);
        Assert.Equal(string.Empty, draft.Problem(NoteKind.Text, Say));
        Assert.Null(draft.Problem(NoteKind.Photo, Say));
        draft.Text = "Picnic";
        Assert.Null(draft.Problem(NoteKind.Text, Say));
        Assert.Null(draft.Problem(NoteKind.Event, Say));
        draft.Starts = null;
        Assert.Equal("Pick when it starts.", draft.Problem(NoteKind.Event, Say));

        var @event = NoteDraft.Blank(NoteKind.Event, Now, Utc, _ => 3);
        Assert.Equal("blue", @event.Color);
        Assert.False(@event.HasEnd);
        Assert.Equal(new DateTimeOffset(2026, 9, 10, 15, 0, 0, TimeSpan.Zero), @event.Starts);
        @event.Text = "Picnic";
        @event.HasEnd = true;
        @event.Ends = @event.Starts!.Value.AddMinutes(-5);
        Assert.Equal("The end can't be before the start.", @event.Problem(NoteKind.Event, Say));
        @event.Ends = null;
        Assert.Equal("Pick when it ends, or turn the end off.", @event.Problem(NoteKind.Event, Say));
        // An end picked and then switched off is not sent: the switch is the answer, not the picker.
        @event.Ends = @event.Starts!.Value.AddHours(2);
        @event.HasEnd = false;
        @event.Place = " The park ";
        var created = @event.NewNote(NoteKind.Event, (0.3, 0.4), []);
        Assert.Equal("event", created.Kind);
        Assert.Equal("2026-09-10T15:00:00Z", created.StartsAt);
        Assert.Null(created.EndsAt);
        Assert.Equal("The park", created.Place);

        Assert.Equal("green", NoteDraft.Blank(NoteKind.Text, Now, Utc, count => count - 3).Color);
        Assert.Null(NoteDraft.Blank(NoteKind.Text, Now, Utc, _ => 0).NewNote(NoteKind.Text, (0.1, 0.1), []).Kind);
    }

    [Fact]
    public void AStartMovedPastTheEndTakesTheEndWithIt()
    {
        var draft = NoteDraft.Blank(NoteKind.Event, Now, Utc, _ => 0);
        draft.Starts = draft.Ends!.Value.AddHours(2);
        draft.KeepEndAfterStart();
        Assert.Equal(draft.Starts.Value.AddHours(1), draft.Ends);
    }

    /// <summary>The next round hour is on the READER'S clock: half past two in a zone of +5:30 is three there.</summary>
    [Fact]
    public void ANewEventStartsOnTheNextRoundHour()
    {
        Assert.Equal(new DateTimeOffset(2026, 9, 10, 15, 0, 0, TimeSpan.Zero), NoteDraft.NextRoundHour(Now, Utc));
        var india = TimeZoneInfo.CreateCustomTimeZone("fc+5:30", new TimeSpan(5, 30, 0), "fc+5:30", "fc+5:30");
        Assert.Equal(new DateTimeOffset(2026, 9, 10, 21, 0, 0, new TimeSpan(5, 30, 0)), NoteDraft.NextRoundHour(Now, india));
        Assert.Equal(new DateTimeOffset(2026, 9, 11, 0, 0, 0, TimeSpan.Zero),
            NoteDraft.NextRoundHour(new DateTimeOffset(2026, 9, 10, 23, 5, 0, TimeSpan.Zero), Utc));
    }

    [Fact]
    public void AListIsWrittenAndItsLinesKeepTheirIds()
    {
        var blank = NoteDraft.Blank(NoteKind.Tasks, Now, Utc, _ => 0);
        Assert.Single(blank.Lines);
        blank.Text = "Shopping";
        blank.Lines[0] = new DraftLine(null, " milk ");
        blank.Lines.Add(new DraftLine(null, "   "));
        var created = blank.NewNote(NoteKind.Tasks, (0.2, 0.2), []);
        Assert.Equal([new TaskLineRequest("milk")], created.Items!);

        var stored = new NoteDto(5, Me, Kind: "tasks", Text: "Shopping",
            Items: [new TaskItemDto(1, "milk", true), new TaskItemDto(2, "eggs", false)]);
        var draft = NoteDraft.Of(stored);
        Assert.Null(draft.Patch(stored, []).Items);
        draft.Lines.RemoveAt(1);
        draft.Lines.Add(new DraftLine(null, "bread"));
        Assert.Equal([new TaskLineRequest("milk", 1), new TaskLineRequest("bread")], draft.Patch(stored, []).Items!);

        for (var line = 0; line < NoteText.MaxTaskItems; line++)
        {
            draft.Lines.Add(new DraftLine(null, "x"));
        }
        Assert.Equal("That's more things than one list holds.", draft.Problem(NoteKind.Tasks, Say));
    }

    [Fact]
    public void TheNamesAreResolvedFromTheTextAtSave()
    {
        MemberDto[] roster =
        [
            new(Anna, "anna", "Anna"),
            new(99, "gran", "Gran"),
            new(50, "ghost", "Ghost", HasLeft: true),
        ];
        var draft = NoteDraft.Blank(NoteKind.Text, Now, Utc, _ => 0);
        draft.Text = "@Anna the kit is in the hall";
        var created = draft.NewNote(NoteKind.Text, (0.1, 0.2), roster);
        Assert.Equal([new MentionDto(Anna, "Anna")], created.Mentions!);

        draft.Text = "@Nobody hello @Ghost";
        Assert.Null(draft.NewNote(NoteKind.Text, (0.1, 0.2), roster).Mentions);

        // An EDIT sends the list with the text, re-decided.
        var stored = Note(6, "@Anna the kit");
        var movedOn = NoteDraft.Of(stored);
        movedOn.Text = "@Gran the kit";
        var patch = movedOn.Patch(stored, roster);
        Assert.Equal("@Gran the kit", patch.Text);
        Assert.Equal([new MentionDto(99, "Gran")], patch.Mentions!);

        var nobody = NoteDraft.Of(stored);
        nobody.Text = "the kit is in the hall";
        Assert.Null(nobody.Patch(stored, roster).Mentions);
    }

    [Fact]
    public void TheSheetSaysWhatItIsAndWhyAChangeDidNotGoIn()
    {
        Assert.Equal("New Event", NoteSheetText.Title(true, NoteKind.Event, Say));
        Assert.Equal("New List", NoteSheetText.Title(true, NoteKind.Tasks, Say));
        Assert.Equal("New Note", NoteSheetText.Title(true, NoteKind.Photo, Say));
        Assert.Equal("Photo", NoteSheetText.Title(false, NoteKind.Photo, Say));
        Assert.Equal("Note", NoteSheetText.Title(false, NoteKind.Text, Say));
        Assert.Equal("Title", NoteSheetText.FieldLabel(NoteKind.Tasks, Say));
        Assert.Equal("Caption", NoteSheetText.FieldLabel(NoteKind.Photo, Say));
        Assert.Equal("The board is full. Take a note down to make room for this one.",
            NoteSheetText.Failure(new ApiError(ErrorCodes.BoardFull, "x", 409), Say));
        Assert.Equal("That note has been taken down.",
            NoteSheetText.Failure(new ApiError(ErrorCodes.NoteNotFound, "x", 404), Say));
        Assert.Equal("Can't reach the server. Check your connection.",
            NoteSheetText.Failure(ApiError.Transport("x"), Say));
    }
}
