using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>The note sheet's preview: the draft as the wall will draw it (the web client's sheet preview).</summary>
public sealed class NotePreviewTests
{
    private const long Me = 7;
    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    private static (double Width, double Height) Size(Sticker sticker) => (sticker.Width, sticker.Height);

    [Fact]
    public void AnUnwrittenNoteReadsAsItsKindsPlaceholderAndWrittenWordsAsWritten()
    {
        var draft = new NoteDraft { Text = "  \n" };
        Assert.Equal("Your note", NotePreview.Of(draft, NoteKind.Text, null, Me, false, Say).Note.Text);
        Assert.Equal("Your list", NotePreview.Of(draft, NoteKind.Tasks, null, Me, false, Say).Note.Text);
        Assert.Equal("Your event", NotePreview.Of(draft, NoteKind.Event, null, Me, false, Say).Note.Text);
        Assert.Equal(string.Empty, NotePreview.Of(draft, NoteKind.Photo, null, Me, false, Say).Note.Text);

        draft.Text = " Milk ";
        Assert.Equal(" Milk ", NotePreview.Of(draft, NoteKind.Text, null, Me, false, Say).Note.Text);
    }

    [Fact]
    public void ThePreviewWearsTheDraftsLookUprightAndAsTheReaders()
    {
        var draft = new NoteDraft { Text = "Milk", Color = "pink", Size = NoteSize.Large, Font = NoteFont.Serif };
        var sticker = NotePreview.Of(draft, NoteKind.Tasks, null, Me, compact: false, Say);

        Assert.Equal(("pink", NoteSize.Large, NoteFont.Serif, NoteKind.Tasks), (sticker.Color, sticker.Size, sticker.Font, sticker.Kind));
        Assert.Equal(("pink", "large", "serif", "tasks"), (sticker.Note.Color, sticker.Note.Size, sticker.Note.Font, sticker.Note.Kind));
        Assert.Equal(BoardWall.Card(NoteSize.Large, false), Size(sticker));
        Assert.Equal(BoardWall.Card(NoteSize.Small, true), Size(NotePreview.Of(new NoteDraft { Size = NoteSize.Small }, NoteKind.Text, null, Me, compact: true, Say)));
        Assert.Equal(0, sticker.TiltDegrees);
        Assert.True(sticker.Mine);
        Assert.False(sticker.Unread);
        Assert.Equal((0L, Me), (sticker.Note.Id, sticker.Note.AuthorId));

        // A note being edited keeps who it is.
        var held = NotePreview.Of(draft, NoteKind.Text, new NoteDto(12, 11, Text: "old"), Me, false, Say);
        Assert.Equal((12L, 11L), (held.Note.Id, held.Note.AuthorId));
    }

    /// <summary>A bare photo's card IS its picture, fitted, as on the wall; with a caption the card is the size's own.</summary>
    [Fact]
    public void AnUncaptionedPhotoIsItsPictureAndACaptionedOneKeepsTheCard()
    {
        var photo = new AttachmentDto(34, "photo", Width: 400, Height: 1200, HasPreview: true);
        var held = new NoteDto(12, Me, Kind: "photo", Attachment: photo);
        var card = BoardWall.Card(NoteSize.Medium, false);

        var bare = NotePreview.Of(new NoteDraft(), NoteKind.Photo, held, Me, false, Say);
        Assert.Equal(BoardPicture.Fitted(card.Width, card.Height, 400, 1200), Size(bare));
        Assert.NotEqual(card, Size(bare));
        Assert.Same(photo, bare.Picture);

        var captioned = NotePreview.Of(new NoteDraft { Text = "Beach" }, NoteKind.Photo, held, Me, false, Say);
        Assert.Equal(card, Size(captioned));
        Assert.Same(photo, captioned.Picture);

        // An event's picture is its backdrop; a note of any other kind carries none.
        Assert.Same(photo, NotePreview.Of(new NoteDraft(), NoteKind.Event, held, Me, false, Say).Picture);
        Assert.Null(NotePreview.Of(new NoteDraft { Text = "x" }, NoteKind.Text, held, Me, false, Say).Picture);
        Assert.Null(NotePreview.Of(new NoteDraft { Text = "x" }, NoteKind.Tasks, held, Me, false, Say).Picture);
        // A photo not uploaded yet has nothing to fit.
        Assert.Equal(card, Size(NotePreview.Of(new NoteDraft(), NoteKind.Photo, null, Me, false, Say)));
    }

    [Fact]
    public void AnEventCarriesItsWhenWhereAndAnswersAndNothingElseDoes()
    {
        var draft = new NoteDraft
        {
            Starts = new DateTimeOffset(2026, 9, 12, 13, 0, 0, TimeSpan.FromHours(2)),
            Ends = new DateTimeOffset(2026, 9, 12, 15, 0, 0, TimeSpan.FromHours(2)),
            Place = "  The park ",
        };
        var held = new NoteDto(12, Me, Kind: "event", Rsvps: [new RsvpDto(11, "going")]);

        var open = NotePreview.Of(draft, NoteKind.Event, held, Me, false, Say).Note;
        Assert.Equal("2026-09-12T11:00:00Z", open.StartsAt);
        Assert.Null(open.EndsAt);
        Assert.Equal("The park", open.Place);
        Assert.Equal(1, open.Count("going"));

        draft.HasEnd = true;
        Assert.Equal("2026-09-12T13:00:00Z", NotePreview.Of(draft, NoteKind.Event, held, Me, false, Say).Note.EndsAt);
        draft.Place = "   ";
        Assert.Null(NotePreview.Of(draft, NoteKind.Event, held, Me, false, Say).Note.Place);

        draft.Place = "The park";
        var text = NotePreview.Of(draft, NoteKind.Text, held, Me, false, Say).Note;
        Assert.Equal((null, null, null, null), (text.StartsAt, text.EndsAt, text.Place, text.Rsvps));
    }
}
