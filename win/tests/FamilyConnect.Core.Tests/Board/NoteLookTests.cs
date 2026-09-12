using FamilyConnect.Core;
using FamilyConnect.Core.Board;

namespace FamilyConnect.Core.Tests.Board;

/// <summary>
/// The names a note's look travels under, and the fallbacks. EVERY name falls back rather than
/// failing — and the fallback is for DRAWING only, which is the half a port keeps getting wrong.
/// </summary>
public class NoteLookTests
{
    [Fact]
    public void EveryNameRoundTripsThroughTheWireSpelling()
    {
        foreach (var size in Notes.Sizes)
        {
            Assert.Equal(size, Notes.SizeFrom(Notes.NameOf(size)));
        }
        foreach (var font in Notes.Fonts)
        {
            Assert.Equal(font, Notes.FontFrom(Notes.NameOf(font)));
        }
        foreach (var answer in Notes.Answers)
        {
            Assert.Equal(answer, Notes.AnswerFrom(Notes.NameOf(answer)));
        }
        foreach (var kind in new[] { NoteKind.Text, NoteKind.Photo, NoteKind.Event, NoteKind.Tasks })
        {
            Assert.Equal(kind, Notes.KindFrom(Notes.NameOf(kind)));
        }
    }

    [Fact]
    public void AnUnknownNameDrawsRatherThanFailing()
    {
        Assert.Equal(NoteSize.Medium, Notes.SizeFrom("huge"));
        Assert.Equal(NoteSize.Medium, Notes.SizeFrom(null));
        Assert.Equal(NoteSize.Medium, Notes.SizeFrom("Large"));
        Assert.Equal(NoteKind.Text, Notes.KindFrom("video"));
        Assert.Equal(NoteFont.Plain, Notes.FontFrom("comic"));
        Assert.Equal("#fff2b3", Notes.ColorHex("chartreuse"));
        Assert.Equal("#fff2b3", Notes.ColorHex(null));
        // An RSVP is the exception: better NO button lit than a claim that somebody said
        // something else.
        Assert.Null(Notes.AnswerFrom("perhaps"));
        Assert.Null(Notes.AnswerFrom(null));
    }

    [Fact]
    public void AnEditSendsTheSizeAndFaceOnlyWhenTheAuthorChangedThem()
    {
        Assert.Null(Notes.PatchSize(NoteSize.Medium, "medium"));
        Assert.Equal("large", Notes.PatchSize(NoteSize.Large, "medium"));
        Assert.Equal("small", Notes.PatchSize(NoteSize.Small, "large"));
        // A FOURTH size from a newer server draws as medium and must not be written back as
        // medium because somebody fixed a typo in the text.
        Assert.Null(Notes.PatchSize(NoteSize.Medium, "huge"));
        Assert.Equal("large", Notes.PatchSize(NoteSize.Large, "huge"));
        Assert.Null(Notes.PatchSize(NoteSize.Medium, null));
        Assert.Null(Notes.PatchFont(NoteFont.Plain, "plain"));
        Assert.Equal("mono", Notes.PatchFont(NoteFont.Mono, "plain"));
        Assert.Null(Notes.PatchFont(NoteFont.Plain, "handwriting"));
    }

    [Fact]
    public void TheSixColoursAreTheProtocolsSixInThePickersOrder()
    {
        Assert.Equal(["yellow", "pink", "blue", "green", "orange", "purple"], Notes.Colors);
        // The apps' own pastels, and all six distinct.
        Assert.Equal(Notes.Colors.Length, Notes.Colors.Select(Notes.ColorHex).Distinct().Count());
        Assert.Equal("#c2e0fc", Notes.ColorHex("blue"));
    }

    [Fact]
    public void EveryHandResolvesToAFaceThatShipsWithWindows()
    {
        // The hand is an INTENT; this is the Windows answer to it. In-box faces only — nothing
        // bundled, nothing synthesised — and each hand distinct, or the picker would be a lie.
        var families = Notes.Fonts.Select(Notes.FontFamily).ToArray();
        Assert.Equal(families.Length, families.Distinct().Count());
        Assert.All(families, family => Assert.False(string.IsNullOrWhiteSpace(family)));
        Assert.Contains("Segoe UI", Notes.FontFamily(NoteFont.Plain));
    }

    [Fact]
    public void TheStickerSaysHowManyAreComingAndNothingWhileNobodyIs()
    {
        var strings = EnglishCatalog.Instance;
        Assert.Null(Notes.GoingLine(0, 0, strings));
        Assert.Equal("2 going", Notes.GoingLine(2, 0, strings));
        Assert.Equal("1 maybe", Notes.GoingLine(0, 1, strings));
        Assert.Equal("2 going, 1 maybe", Notes.GoingLine(2, 1, strings));
        // "Can't" is a word on a button, not a count.
        Assert.Equal("Can't", Notes.Title(RsvpAnswer.No, strings));
        Assert.Equal("Going", Notes.Title(RsvpAnswer.Going, strings));
    }
}
