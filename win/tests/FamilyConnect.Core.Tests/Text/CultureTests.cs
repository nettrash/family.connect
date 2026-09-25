using System.Globalization;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;
using BoardMarks = FamilyConnect.Core.Board.BoardMarks;

namespace FamilyConnect.Core.Tests.Text;

/// <summary>
/// What this client does in a reader's own language and number format — which on Windows is
/// whatever the machine is set to, and is emphatically not the developer's.
/// </summary>
/// <remarks>
/// <para>
/// Two families of bug live here, and neither shows up on a machine set to English. A MACHINE'S
/// NUMBER written in the reader's format ("1.284" in German) is a number the same machine cannot
/// read back; and a STRING COMPARED in the reader's alphabet is the Turkish I — where
/// <c>"IMAGE/JPEG".ToLower()</c> is <c>"ımage/jpeg"</c> and an image type stops being allowed
/// because somebody's computer is set to Turkish.
/// </para>
/// <para>
/// The culture is set HERE rather than by whoever launched the suite, so the check cannot be lost
/// by running the tests a different way — and the class has a collection of its own because the
/// ambient culture is process-wide state and xUnit runs collections in parallel.
/// </para>
/// </remarks>
[Collection("culture")]
public class CultureTests : IDisposable
{
    private readonly CultureInfo was = CultureInfo.CurrentCulture;
    private readonly Database database = Database.OpenInMemory();

    public void Dispose()
    {
        CultureInfo.CurrentCulture = was;
        database.Dispose();
    }

    private static void Speaking(string tag) =>
        CultureInfo.CurrentCulture = CultureInfo.GetCultureInfo(tag);

    private static NoteDto Note(long id, long seq) =>
        new(id, 7, "text", "Milk", "yellow", "medium", "plain", 0.25, 0.5,
            BoardSeq: seq, ContentSeq: seq);

    /// <summary>
    /// The board's cursors and marks are written for this cache and read back by it. In German a
    /// number is grouped with dots and decimals are commas; a cursor written that way is a cursor
    /// the next launch cannot read, and a board that cannot read its cursor re-reads the whole
    /// wall for ever — or worse, reads zero and asks for changes it has already applied.
    /// </summary>
    [Theory]
    [InlineData("de-DE")]
    [InlineData("tr-TR")]
    [InlineData("ru-RU")]
    [InlineData("sv-SE")]
    public void TheBoardsOwnNumbersSurviveAnyReadersFormat(string tag)
    {
        Speaking(tag);
        var board = new BoardStore(database);

        board.Replace([Note(12, 1_234_567)], 1_234_567);
        board.Apply(Note(13, 2_000_111), SeqRoute.LiveFrame);
        board.Mark(new BoardMarks(13, 2_000_111));

        Assert.Equal(2_000_111, board.Cursor);
        Assert.Equal(new BoardMarks(13, 2_000_111), board.Marks);
        // And a second reader of the same file, in another language, reads the same numbers.
        Speaking("en-US");
        Assert.Equal(2_000_111, new BoardStore(database).Cursor);
    }

    /// <summary>
    /// A language TAG is matched the same way whatever the reader's alphabet: <c>SR-LATN</c> is
    /// Latin Serbian in Turkey too.
    /// </summary>
    [Fact]
    public void ALanguageTagIsMatchedTheSameWayEverywhere()
    {
        Speaking("tr-TR");

        Assert.Equal("sr-Latn", Languages.Nearest("SR-LATN-RS"));
        Assert.Equal("de", Languages.Nearest("DE-AT"));
        Assert.Equal("zh-Hans", Languages.Nearest("ZH-HANT-TW"));
    }

    /// <summary>
    /// The wire's instants are read and written in one format, and it is not the reader's: a
    /// German client sending "24.12.2026" would be sending nothing any server could parse.
    /// </summary>
    /// <remarks>
    /// <b>fi-FI IS THE ONE THAT CATCHES IT</b>, and it is why this is a Theory rather than a
    /// sentence about German. In a custom format string <c>:</c> is not a colon — it is the
    /// culture's TIME SEPARATOR — and Finnish spells that <c>.</c>, so
    /// <c>ToString("yyyy-MM-ddTHH:mm:ss.fffZ")</c> on a Finnish machine writes
    /// <c>2026-12-24T16.00.00.000Z</c>: an instant no server can read, from a client that looked
    /// correct in every other language. German, Turkish, Russian and Swedish all spell it
    /// <c>:</c> and would have let it through.
    /// </remarks>
    [Theory]
    [InlineData("de-DE")]
    [InlineData("ja-JP")]
    [InlineData("fi-FI")]
    public void TheWiresInstantsAreReadAndWrittenInOneFormat(string tag)
    {
        Speaking(tag);

        var instant = Times.Instant("2026-12-24T16:00:00Z");

        Assert.NotNull(instant);
        Assert.Equal("2026-12-24T16:00:00.000Z", Times.Rfc3339(instant));
        // And the calendar file's own stamp, which a calendar programme parses.
        Assert.Equal(
            "20261224T160000Z",
            Calendar.Stamp(DateTimeOffset.FromUnixTimeMilliseconds(instant!.Value)));
    }

    /// <summary>
    /// A number INSIDE a sentence is the reader's to format — "1.284" is right in German — but
    /// the sentence is the catalogue's and the placeholders are Apple's, so the two have to work
    /// together rather than one of them quietly winning.
    /// </summary>
    [Fact]
    public void ASentencesNumbersAreDrawnInTheReadersOwnFormat()
    {
        Speaking("de-DE");

        // The catalogue's German value is positional, and the count goes where German wants it.
        Assert.Equal(
            "3 von 7 erledigt", JsonCatalog.For("de").Format("%lld of %lld done", 3, 7));
        // The English source sentence, filled in order, in the same breath.
        Assert.Equal("2 Photos", EnglishCatalog.Instance.Format("%lld Photos", 2));
    }

    /// <summary>
    /// And the `.ics` file is the same bytes on every machine: a calendar file is read by a
    /// programme, and its 75-octet lines are counted in bytes rather than in anybody's letters.
    /// </summary>
    [Theory]
    [InlineData("tr-TR")]
    [InlineData("ru-RU")]
    public void TheCalendarFileIsTheSameBytesInEveryLanguage(string tag)
    {
        var english = Calendar.OneEvent(
            "fc-note-12@nettrash", "Christmas dinner, at Gran's", "20261224T160000Z",
            "20261224T200000Z", "Gran's house", "20260912T120000Z");

        Speaking(tag);

        Assert.Equal(
            english,
            Calendar.OneEvent(
                "fc-note-12@nettrash", "Christmas dinner, at Gran's", "20261224T160000Z",
                "20261224T200000Z", "Gran's house", "20260912T120000Z"));
    }
}
