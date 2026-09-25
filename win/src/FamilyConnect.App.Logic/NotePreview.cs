using FamilyConnect.Core;
using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>
/// The sticker the note sheet shows under its fields: the draft as the wall will draw it, type already
/// fitted, so a size or a face is a choice with its result in front of the author (the web client's sheet
/// preview, docs/protocol.md "Board").
/// </summary>
/// <remarks>
/// <para>
/// <b>IT IS A <see cref="Sticker"/></b>, drawn by the wall's own code: a second, simpler renderer is a second
/// place for the rules to drift.
/// </para>
/// <para>
/// Words not written yet read as the kind's placeholder. A photo has none, so an uncaptioned photo is the
/// bare picture with its card fitted to it — the shape the wall draws, or the author would be shown one it
/// never does. An event's picture is its backdrop; the preview is where its author sees what arrived.
/// </para>
/// </remarks>
public static class NotePreview
{
    public static Sticker Of(NoteDraft draft, NoteKind kind, NoteDto? current, long reader, bool compact, IStringCatalog say)
    {
        var text = draft.Text.Trim().Length > 0 ? draft.Text : Placeholder(kind, say);
        var picture = kind is NoteKind.Photo or NoteKind.Event ? current?.Attachment : null;
        var card = BoardWall.Card(draft.Size, compact);
        if (kind == NoteKind.Photo && text.Length == 0 && picture is not null)
        {
            card = BoardPicture.Fitted(card.Width, card.Height, picture.Width, picture.Height);
        }
        var isEvent = kind == NoteKind.Event;
        var note = new NoteDto(
            current?.Id ?? 0,
            current?.AuthorId ?? reader,
            Kind: Notes.NameOf(kind),
            Text: text,
            Color: draft.Color,
            Size: Notes.NameOf(draft.Size),
            Font: Notes.NameOf(draft.Font),
            Attachment: picture,
            StartsAt: isEvent ? NoteDraft.WireTime(draft.Starts) : null,
            EndsAt: isEvent && draft.HasEnd ? NoteDraft.WireTime(draft.Ends) : null,
            Place: isEvent && draft.Place.Trim() is { Length: > 0 } place ? place : null,
            Rsvps: isEvent ? current?.Rsvps : null);
        // Upright: a preview is looked at, not pinned.
        return new Sticker(
            note, kind, draft.Size, draft.Font, draft.Color, 0, 0, card.Width, card.Height,
            TiltDegrees: 0, Unread: false, Mine: true);
    }

    /// <summary>What an unwritten note says in its preview — nothing, for a photo.</summary>
    public static string Placeholder(NoteKind kind, IStringCatalog say) => kind switch
    {
        NoteKind.Photo => string.Empty,
        NoteKind.Event => say.Get("Your event"),
        NoteKind.Tasks => say.Get("Your list"),
        _ => say.Get("Your note"),
    };
}
