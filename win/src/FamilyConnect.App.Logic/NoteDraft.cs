using System.Globalization;
using FamilyConnect.Core;
using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>One line of a list as the author is writing it. The id says "the line you already have", which is what carries its TICK through a rewrite.</summary>
public sealed record DraftLine(long? Id, string Text);

/// <summary>
/// What the author is writing: a note's words and look, an event's when and where, a list's lines — kept
/// apart from the note so a save sends ONLY WHAT CHANGED (the web client's <c>Draft</c>).
/// </summary>
/// <remarks>
/// <para>
/// <b>A PATCH IS A CHANGE FROM THE NOTE AS IT STOOD WHEN THE SHEET OPENED</b> — the caller keeps that copy.
/// Diffing against the note as it stands now would send the author's stale copy of anything changed
/// meanwhile on another device.
/// </para>
/// <para>
/// <b>THE NAMES ARE RESOLVED FROM THE TEXT AT SAVE</b>, against the live roster, and sent WITH the text:
/// a text patch that carries no names clears the note's old ones, which is right when the words no longer
/// say any and is exactly why an editor that forgot them would erase every mention on a typo fix.
/// </para>
/// </remarks>
public sealed class NoteDraft
{
    public string Text { get; set; } = string.Empty;

    public string Color { get; set; } = "yellow";

    public NoteSize Size { get; set; } = NoteSize.Medium;

    public NoteFont Font { get; set; } = NoteFont.Plain;

    public DateTimeOffset? Starts { get; set; }

    public bool HasEnd { get; set; }

    public DateTimeOffset? Ends { get; set; }

    public string Place { get; set; } = string.Empty;

    public List<DraftLine> Lines { get; } = [];

    /// <summary>
    /// A blank of <paramref name="kind"/>: an event starts on the next round hour and is blue, as on the
    /// phone; any other note takes a colour at random, so a run of them is not one yellow pile; a list starts
    /// with one empty line, so the first thing to do is one tap away.
    /// </summary>
    public static NoteDraft Blank(NoteKind kind, DateTimeOffset now, TimeZoneInfo zone, Func<int, int> pick)
    {
        var starts = NextRoundHour(now, zone);
        var draft = new NoteDraft
        {
            Color = kind == NoteKind.Event ? "blue" : Notes.Colors[pick(Notes.Colors.Length)],
            Starts = starts,
            Ends = starts.AddHours(1),
        };
        if (kind == NoteKind.Tasks)
        {
            draft.Lines.Add(new DraftLine(null, string.Empty));
        }
        return draft;
    }

    /// <summary>The note as it stands, to be edited. An event with no end offers an hour after its start to begin from.</summary>
    public static NoteDraft Of(NoteDto note)
    {
        var starts = EventText.Instant(note.StartsAt);
        var draft = new NoteDraft
        {
            Text = note.Text ?? string.Empty,
            Color = note.Color ?? "yellow",
            Size = Notes.SizeFrom(note.Size),
            Font = Notes.FontFrom(note.Font),
            Starts = starts,
            HasEnd = note.EndsAt is not null,
            Ends = EventText.Instant(note.EndsAt) ?? starts?.AddHours(1),
            Place = note.Place ?? string.Empty,
        };
        foreach (var item in note.TaskList)
        {
            draft.Lines.Add(new DraftLine(item.Id, item.Text));
        }
        return draft;
    }

    /// <summary>A start moved past the end takes the end with it, an hour on, rather than refusing the save.</summary>
    public void KeepEndAfterStart()
    {
        if (Starts is { } starts && Ends is { } ends && ends < starts)
        {
            Ends = starts.AddHours(1);
        }
    }

    /// <summary>
    /// Why this cannot be saved as it stands, or null. EMPTY means "not yet, and nothing to say": the Save
    /// button is simply off while a note has no words.
    /// </summary>
    public string? Problem(NoteKind kind, IStringCatalog say)
    {
        if (kind != NoteKind.Photo && Text.Trim().Length == 0)
        {
            return string.Empty;
        }
        if (kind == NoteKind.Tasks && Lines.Count > NoteText.MaxTaskItems)
        {
            return say.Get("That's more things than one list holds.");
        }
        if (kind == NoteKind.Event)
        {
            if (Starts is not { } starts)
            {
                return say.Get("Pick when it starts.");
            }
            if (HasEnd)
            {
                if (Ends is not { } ends)
                {
                    return say.Get("Pick when it ends, or turn the end off.");
                }
                if (ends < starts)
                {
                    return say.Get("The end can't be before the start.");
                }
            }
        }
        return null;
    }

    /// <summary>The lines that say something, trimmed — what a save sends. An empty row is somebody who started typing and stopped.</summary>
    public IReadOnlyList<DraftLine> WrittenLines() =>
        [.. Lines.Where(line => line.Text.Trim().Length > 0).Select(line => line with { Text = line.Text.Trim() })];

    /// <summary>A new note of <paramref name="kind"/>, dropped at <paramref name="at"/>, a fraction of the wall.</summary>
    public NoteRequest NewNote(NoteKind kind, (double X, double Y) at, IReadOnlyList<MemberDto> roster)
    {
        var isEvent = kind == NoteKind.Event;
        var place = Place.Trim();
        var named = Named(Text, roster);
        return new NoteRequest(
            Text, Color, at.X, at.Y, Notes.NameOf(Size), Notes.NameOf(Font),
            // Named unless it is a plain note, which is what an absent kind means.
            Kind: kind == NoteKind.Text ? null : Notes.NameOf(kind),
            StartsAt: isEvent ? WireTime(Starts) : null,
            EndsAt: isEvent && HasEnd ? WireTime(Ends) : null,
            Place: isEvent && place.Length > 0 ? place : null,
            Mentions: named.Length > 0 ? named : null,
            Items: kind == NoteKind.Tasks ? [.. WrittenLines().Select(line => new TaskLineRequest(line.Text, line.Id))] : null);
    }

    /// <summary>
    /// What changed against <paramref name="note"/> — and nothing else, so a size or face this client does
    /// not know is not written back as its default, and a save that changed nothing sends nothing.
    /// </summary>
    public NotePatch Patch(NoteDto note, IReadOnlyList<MemberDto> roster)
    {
        string? text = null;
        MentionDto[]? mentions = null;
        // Trailing space is not a change: the server trims.
        if (Text.Trim() != (note.Text ?? string.Empty).Trim())
        {
            text = Text;
            var named = Named(Text, roster);
            mentions = named.Length > 0 ? named : null;
        }
        var kind = Notes.KindFrom(note.Kind);
        string? startsAt = null;
        string? endsAt = null;
        string? place = null;
        var clearsEnd = false;
        if (kind == NoteKind.Event)
        {
            if (Starts is { } starts && starts != EventText.Instant(note.StartsAt))
            {
                startsAt = WireTime(starts);
            }
            var ends = HasEnd ? Ends : null;
            if (ends != EventText.Instant(note.EndsAt))
            {
                // An end taken off is a NULL on the wire; a field left out leaves it alone.
                if (ends is { } end)
                {
                    endsAt = WireTime(end);
                }
                else
                {
                    clearsEnd = true;
                }
            }
            if (Place.Trim() != (note.Place ?? string.Empty).Trim())
            {
                place = Place.Trim();
            }
        }
        TaskLineRequest[]? items = null;
        if (kind == NoteKind.Tasks)
        {
            var written = WrittenLines();
            var held = note.TaskList.Select(item => new DraftLine(item.Id, item.Text)).ToList();
            // The list is the AUTHOR's, and a patch that carried it unchanged would make opening a note to read it an edit.
            if (!written.SequenceEqual(held))
            {
                items = [.. written.Select(line => new TaskLineRequest(line.Text, line.Id))];
            }
        }
        return new NotePatch(
            text,
            Color != note.Color ? Color : null,
            Notes.PatchSize(Size, note.Size),
            Notes.PatchFont(Font, note.Font),
            StartsAt: startsAt,
            EndsAt: endsAt,
            Place: place,
            Mentions: mentions,
            Items: items)
        { ClearsEnd = clearsEnd };
    }

    /// <summary>Whether a patch changes nothing at all.</summary>
    public static bool IsEmpty(NotePatch patch) => patch == new NotePatch();

    /// <summary>The next round hour after <paramref name="now"/>, on the reader's clock.</summary>
    public static DateTimeOffset NextRoundHour(DateTimeOffset now, TimeZoneInfo zone)
    {
        var local = TimeZoneInfo.ConvertTime(now.AddHours(1), zone);
        return new DateTimeOffset(local.Year, local.Month, local.Day, local.Hour, 0, 0, local.Offset);
    }

    /// <summary>An instant as the wire writes one: RFC 3339, UTC, whole seconds.</summary>
    public static string? WireTime(DateTimeOffset? at) =>
        at?.UtcDateTime.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", CultureInfo.InvariantCulture);

    private static MentionDto[] Named(string text, IReadOnlyList<MemberDto> roster) =>
    [
        .. Mentions.Resolve(
                text,
                [.. roster.Where(member => !member.Deleted && !member.IsFormer).Select(member => new Named(member.Id, member.DisplayName))])
            .Take(Mentions.MaxPerMessage)
            .Select(member => new MentionDto(member.UserId, member.Name)),
    ];
}

/// <summary>The words the board's sheet says about itself and its refusals (the web client's board pane and <c>board_failure</c>).</summary>
public static class NoteSheetText
{
    public static string Title(bool isNew, NoteKind kind, IStringCatalog say) => (isNew, kind) switch
    {
        (true, NoteKind.Event) => say.Get("New Event"),
        (true, NoteKind.Tasks) => say.Get("New List"),
        (true, _) => say.Get("New Note"),
        (false, NoteKind.Event) => say.Get("Event"),
        (false, NoteKind.Tasks) => say.Get("List"),
        (false, NoteKind.Photo) => say.Get("Photo"),
        _ => say.Get("Note"),
    };

    public static string FieldLabel(NoteKind kind, IStringCatalog say) => kind switch
    {
        NoteKind.Event or NoteKind.Tasks => say.Get("Title"),
        NoteKind.Photo => say.Get("Caption"),
        _ => say.Get("Note"),
    };

    /// <summary>What to tell somebody whose change to the board did not go in — said, never swallowed.</summary>
    public static string Failure(ApiError error, IStringCatalog say) => error.Code switch
    {
        ErrorCodes.BoardFull => say.Get("The board is full. Take a note down to make room for this one."),
        ErrorCodes.NotNoteAuthor => say.Get("Only the person who wrote a note can change it."),
        ErrorCodes.NoteNotFound => say.Get("That note has been taken down."),
        ErrorCodes.AttachmentExpired => say.Get("The photo took too long to pin. Try again."),
        ErrorCodes.AttachmentTooLarge => say.Get("That photo is too large to pin."),
        ErrorCodes.InvalidAttachment => say.Get("The board pins photos only."),
        ErrorCodes.NotInFamily => say.Get("You're not in a family, so there is no board."),
        _ => FamilyText.GenericFailure(error, say),
    };
}
