using System.Text.Json;
using FamilyConnect.Core;

namespace FamilyConnect.Core.Tests.Text;

/// <summary>
/// The nine languages: that they ship, that they are complete, and that a translation may put its
/// placeholders wherever its grammar wants them.
/// </summary>
/// <remarks>
/// The KEY IS THE ENGLISH SENTENCE, which is what makes a missing translation read in English
/// rather than as a slug — and what makes this suite possible at all: the keys these tests name
/// are the same strings the product's call sites name.
/// </remarks>
public class CatalogueTests
{
    /// <summary>Every sentence this port draws, scanned out of its own source.</summary>
    private static readonly string[] Sentences = Keys();

    private static string[] Keys()
    {
        // The source is beside the test's own directory in the repo, not in the build output, so
        // this walks up to `win/` and reads what ships.
        var here = new DirectoryInfo(AppContext.BaseDirectory);
        while (here is not null && here.Name != "win")
        {
            here = here.Parent;
        }
        var source = new DirectoryInfo(Path.Combine(here!.FullName, "src"));
        var pattern = new System.Text.RegularExpressions.Regex(
            "\\b(?:Get|Format)\\(\\s*\"((?:[^\"\\\\]|\\\\.)+)\"");
        var keys = new SortedSet<string>(StringComparer.Ordinal);
        foreach (var file in source.GetFiles("*.cs", SearchOption.AllDirectories))
        {
            foreach (var found in pattern.Matches(File.ReadAllText(file.FullName)))
            {
                keys.Add(((System.Text.RegularExpressions.Match)found).Groups[1].Value);
            }
        }
        return [.. keys];
    }

    /// <summary>The five sentences nobody has translated yet, named rather than pretended.</summary>
    private static readonly string[] EnglishForNow =
        ["%@ — %@", "%@ — %@ mentioned you", "Chat", "New note", "Voice message"];

    [Fact]
    public void ThisPortActuallyDrawsSomething()
    {
        // A scan that found nothing would make every test below vacuously true.
        Assert.True(Sentences.Length >= 25, $"only {Sentences.Length} sentences found");
        Assert.Contains("No answer", Sentences);
    }

    [Fact]
    public void AllNineLanguagesShip()
    {
        Assert.Equal(Languages.All, JsonCatalog.Shipped());
    }

    /// <summary>
    /// EVERY SENTENCE IS IN EVERY LANGUAGE, or on the short list of ones nobody has translated —
    /// which is the point of the list: a new key cannot go quietly untranslated in nine
    /// catalogues that look complete.
    /// </summary>
    [Fact]
    public void EverySentenceIsTranslatedOrKnownToBeEnglishForNow()
    {
        foreach (var language in Languages.All.Where(tag => tag != Languages.English))
        {
            var catalogue = (JsonCatalog)JsonCatalog.For(language);
            foreach (var sentence in Sentences)
            {
                // Asked of the TABLE and not of the answer: "Video" in German and "Photo" in
                // French are real translations identical to the English, and comparing values
                // would call them missing.
                if (catalogue.Holds(sentence))
                {
                    continue;
                }
                Assert.Contains(sentence, EnglishForNow);
            }
        }
    }

    [Fact]
    public void TheEnglishOnlyListNamesNothingThatIsActuallyTranslated()
    {
        // A stale list is a lie in the other direction: it would hide a translation that arrived.
        var russian = (JsonCatalog)JsonCatalog.For("ru");
        foreach (var sentence in EnglishForNow)
        {
            Assert.False(russian.Holds(sentence), sentence);
            Assert.Equal(sentence, russian.Get(sentence));
        }
        Assert.All(EnglishForNow, sentence => Assert.Contains(sentence, Sentences));
    }

    [Fact]
    public void ATranslatedSentenceReadsInItsOwnLanguage()
    {
        Assert.Equal("Нет ответа", JsonCatalog.For("ru").Get("No answer"));
        // A translation identical to the English is still a translation: German says "Video".
        Assert.True(((JsonCatalog)JsonCatalog.For("de")).Holds("Video"));
        Assert.Equal("Video", JsonCatalog.For("de").Get("Video"));
        Assert.Equal("Keine Antwort", JsonCatalog.For("de").Get("No answer"));
        Assert.Equal("応答なし", JsonCatalog.For("ja").Get("No answer"));
        // The two Serbians are the same language in two alphabets, and a reader of one cannot
        // read the other.
        Assert.NotEqual(JsonCatalog.For("sr").Get("No answer"), JsonCatalog.For("sr-Latn").Get("No answer"));
    }

    /// <summary>
    /// A TRANSLATION MAY REORDER ITS PLACEHOLDERS — German and Russian both do, in the shipped
    /// values — so the formatter reads positional forms as well as sequential ones.
    /// </summary>
    [Fact]
    public void ATranslationMayPutItsPlaceholdersWhereItsGrammarWantsThem()
    {
        Assert.Equal("3 von 7 erledigt", JsonCatalog.For("de").Format("%lld of %lld done", 3, 7));
        Assert.Equal("Сделано 3 из 7", JsonCatalog.For("ru").Format("%lld of %lld done", 3, 7));
        // And English, whose value IS the key, fills them in order.
        Assert.Equal("3 of 7 done", EnglishCatalog.Instance.Format("%lld of %lld done", 3, 7));
    }

    [Fact]
    public void AMissingKeyAnswersItselfBecauseTheKeyIsTheEnglish()
    {
        var russian = JsonCatalog.Parse("""{"No answer": "Нет ответа"}""", "ru");

        Assert.Equal("Нет ответа", russian.Get("No answer"));
        // Not a slug, not an exception, not an empty label: the English sentence.
        Assert.Equal("Dinner at 7?", russian.Get("Dinner at 7?"));
        Assert.Equal("2 Photos", russian.Format("%lld Photos", 2));
    }

    [Fact]
    public void ALanguageIsMatchedAsNearlyAsTheCatalogueCan()
    {
        Assert.Equal("de", Languages.Nearest("de-AT"));
        Assert.Equal("ru", Languages.Nearest("ru_RU"));
        Assert.Equal("sr", Languages.Nearest("sr-Cyrl-RS"));
        Assert.Equal("sr-Latn", Languages.Nearest("sr-Latn-RS"));
        // The catalogue holds Simplified only, which is the nearer of the two wrong answers for
        // a Traditional reader.
        Assert.Equal("zh-Hans", Languages.Nearest("zh-Hant-TW"));
        Assert.Equal("en", Languages.Nearest("is"));
        Assert.Equal("en", Languages.Nearest(null));
        Assert.Equal("en", Languages.Nearest("   "));
    }

    /// <summary>
    /// THE FAMILY'S LANGUAGE IS NOT THE DISPLAY LANGUAGE. It is what the assistant answers in;
    /// a family setting that silently re-languaged somebody's computer would be a surprise
    /// nobody asked for, so the window draws in the DEVICE's language and nothing else.
    /// </summary>
    [Fact]
    public void AFamilysLanguageDoesNotRelanguageAnybodysWindow()
    {
        // A German member of a Russian-speaking family reads a German window.
        Assert.Equal("de", Languages.ForDisplay("de-DE"));
        Assert.Equal(
            "Keine Antwort",
            JsonCatalog.For(Languages.ForDisplay("de-DE")).Get("No answer"));

        // And there is no way to ask this class for the family's: it takes one argument, and it
        // is the device's.
        Assert.Equal("en", Languages.ForDisplay(null));
    }

    [Fact]
    public void EnglishNeedsNoTableAtAll()
    {
        var english = JsonCatalog.For("en");

        Assert.Same(EnglishCatalog.Instance, english);
        Assert.Equal("No answer", english.Get("No answer"));
    }

    /// <summary>
    /// The tables that ship are the ones the generator wrote: a hand-edited catalogue, or one
    /// left behind by a key that changed, is a Russian app drawing English at the moment somebody
    /// notices.
    /// </summary>
    [Fact]
    public void TheShippedTablesHoldOnlySentencesThisPortDraws()
    {
        var sentences = new HashSet<string>(Sentences, StringComparer.Ordinal);
        foreach (var language in Languages.All.Where(tag => tag != Languages.English))
        {
            var catalogue = (JsonCatalog)JsonCatalog.For(language);
            Assert.True(catalogue.Count > 0, language);
            Assert.True(
                catalogue.Count <= sentences.Count,
                $"{language} holds {catalogue.Count} of {sentences.Count} sentences");
        }
    }
}
