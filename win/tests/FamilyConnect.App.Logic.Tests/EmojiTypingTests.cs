using System.Text;
using System.Text.Json;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// EMOJI AS YOU TYPE, held to <c>fc_text::emoji_typing</c> case for case (issue #101; docs/emoji-as-you-type-2026-10-18.md).
/// <c>Fixtures/emoji-typing-vectors.json</c> was printed by the Rust the web client runs (<c>win/tools/board-oracle</c>,
/// <c>cargo run -- emoji-typing</c>): the emoticon table, and every key sequence typed into an empty field with what the field
/// then holds. The same bytes are held by the Apple and Android ports, and CI compares all three copies with a fresh print.
/// </summary>
/// <remarks>
/// And then what is Windows' own and so not in the vectors: the rule driven through a <c>TextBox</c>'s events
/// (<see cref="CapitalField"/>, by <see cref="TypingBox"/>) — capitals first on a multi-line field, emoji only on a single-line
/// one, the take-back, everything that drops the live swap, and the box's <c>MaxLength</c>.
/// </remarks>
public sealed class EmojiTypingTests
{
    private static readonly Vectors File = Load();

    private sealed record Vectors((string Text, string Emoji)[] Emoticons, (string Keys, string Text)[] Typing);

    private static Vectors Load()
    {
        var path = Path.Combine(AppContext.BaseDirectory, "Fixtures", "emoji-typing-vectors.json");
        using var document = JsonDocument.Parse(System.IO.File.ReadAllText(path));
        var root = document.RootElement;
        return new Vectors(
            [.. root.GetProperty("emoticons").EnumerateArray().Select(row => (Get(row, "text"), Get(row, "emoji")))],
            [.. root.GetProperty("typing").EnumerateArray().Select(row => (Get(row, "keys"), Get(row, "text")))]);
    }

    private static string Get(JsonElement row, string name) =>
        row.GetProperty(name).GetString() ?? throw new InvalidDataException($"a vector without {name}");

    /// <summary>
    /// Rust's <c>typing</c>, written out again here rather than called, so the vectors check <see cref="EmojiTyping.Replace"/> and
    /// <see cref="EmojiSwap"/> through a second reading of the original: the caret always at the end; <c>⌫</c> takes back the
    /// live swap or else removes the last code point; <c>→</c> drops the live swap; anything else is typed — replaced, or
    /// appended.
    /// </summary>
    private static string Simulate(string keys)
    {
        var text = new StringBuilder();
        EmojiSwap? live = null;
        foreach (var key in keys.EnumerateRunes())
        {
            if (key.Value == '⌫')
            {
                if (live?.Backspace(text.ToString(), text.Length) is { } undone)
                {
                    text.Clear().Append(undone.Text);
                }
                else if (text.Length > 0)
                {
                    var last = Rune.GetRuneAt(text.ToString(), text.Length - 1 - (char.IsLowSurrogate(text[^1]) ? 1 : 0));
                    text.Length -= last.Utf16SequenceLength;
                }
                live = null;
            }
            else if (key.Value == '→')
            {
                live = null;
            }
            else if (EmojiTyping.Replace(text.ToString(), key.ToString(), live) is { } swap)
            {
                var (applied, _) = swap.Apply(text.ToString(), string.Empty);
                text.Clear().Append(applied);
                live = swap;
            }
            else
            {
                text.Append(key.ToString());
                live = null;
            }
        }
        return text.ToString();
    }

    private static void AllHold(Func<string, string> typing, Func<string, string>? spell = null)
    {
        spell ??= text => text;
        var failures = File.Typing
            .Where(row => !string.Equals(typing(spell(row.Keys)), spell(row.Text), StringComparison.Ordinal))
            .Select(row => $"{row.Keys} -> {typing(spell(row.Keys))} (expected {row.Text})")
            .ToList();
        Assert.True(failures.Count == 0, $"{failures.Count} of {File.Typing.Length} vectors fail:\n{string.Join('\n', failures.Take(20))}");
    }

    [Fact]
    public void TheVectorsAreTheWholeFile()
    {
        Assert.Equal(41, File.Emoticons.Length);
        Assert.True(File.Typing.Length >= 1600, $"{File.Typing.Length} typing vectors");
    }

    /// <summary>(a) The table, entry for entry and in the original's order: the order is part of the port.</summary>
    [Fact]
    public void TheEmoticonsAreTheOriginals()
    {
        Assert.Equal(File.Emoticons, EmojiTyping.Emoticons);
    }

    /// <summary>(b) Every key sequence, through a simulator that is Rust's <c>typing</c> line for line.</summary>
    [Fact]
    public void EveryVectorHolds() => AllHold(Simulate);

    /// <summary>The port's own <see cref="EmojiTyping.Typing"/>, the original's <c>typing</c>, agrees with every vector too.</summary>
    [Fact]
    public void ThePortsTypingAgrees() => AllHold(EmojiTyping.Typing);

    /// <summary>
    /// Every vector again, typed into a single-line field through the window's whole sequence — the key, the change before it
    /// lands, the selection it leaves, the high-priority turn, the Backspace in PreviewKeyDown: the integration says what the
    /// rule says.
    /// </summary>
    [Fact]
    public void EveryVectorHoldsThroughTheWindowsEvents() => AllHold(keys =>
    {
        var box = new TypingBox(capitals: false);
        box.TypeAll(keys);
        Assert.Equal(box.Text.Length, box.Start);
        return box.Text;
    });

    /// <summary>A <c>TextBox</c> keeps a line break as a lone carriage return: the fences and code spans are found by it too.</summary>
    [Fact]
    public void EveryVectorHoldsWithTheTextBoxsLineBreaks() =>
        AllHold(keys =>
        {
            var box = new TypingBox(capitals: false);
            box.TypeAll(keys);
            return box.Text;
        }, text => text.Replace('\n', '\r'));

    [Fact]
    public void TheNamesLoad()
    {
        Assert.True(EmojiTyping.NameCount > 6000, $"{EmojiTyping.NameCount} names");
        Assert.Equal("🔥", EmojiTyping.EmojiNamed("fire"));
        Assert.Equal("🪅", EmojiTyping.EmojiNamed("piñata"));
        Assert.Equal("👍", EmojiTyping.EmojiNamed("like"));
        Assert.Null(EmojiTyping.EmojiNamed(""));
        Assert.Null(EmojiTyping.EmojiNamed(":fire:"));
        Assert.Null(EmojiTyping.EmojiNamed("Fire"));
        // No comment line became a name.
        Assert.Null(EmojiTyping.EmojiNamed("#"));
    }

    /// <summary>(c) A swap in the middle of the text: <c>Apply</c> keeps what is after the caret, and the Backspace puts back what was typed.</summary>
    [Fact]
    public void ASwapLandsBetweenBeforeAndAfter()
    {
        var swap = EmojiTyping.Replace("ok :", ")", null);
        Assert.Equal(new EmojiSwap(3, "😊", ":)"), swap);
        Assert.NotNull(swap);
        var (text, caret) = swap.Apply("ok :", " bye");
        Assert.Equal("ok 😊 bye", text);
        // UTF-16 units: the emoji is a surrogate pair.
        Assert.Equal(5, caret);
        Assert.Equal(("ok :) bye", 5), swap.Backspace(text, caret));
        // Not right behind it: an ordinary Backspace.
        Assert.Null(swap.Backspace(text, caret + 1));
        Assert.Null(swap.Backspace(text, caret - 1));
        Assert.Null(swap.Backspace("ok x bye", caret));
        Assert.Null(swap.Backspace("ok", caret));
        // A space-ended emoticon keeps its space, and gives back all three characters.
        var spaced = EmojiTyping.Replace("a :o", " ", null);
        Assert.Equal(new EmojiSwap(2, "😨 ", ":o "), spaced);
        Assert.NotNull(spaced);
        var (withSpace, after) = spaced.Apply("a :o", "b");
        Assert.Equal("a 😨 b", withSpace);
        Assert.Equal(("a :o b", 5), spaced.Backspace(withSpace, after));
    }

    [Fact]
    public void ASwapGrowsOnlyFromWhereItStarted()
    {
        // A shortcode's closing colon and a `(` are not a `:(`.
        var fire = EmojiTyping.Replace(":fire", ":", null);
        Assert.NotNull(fire);
        var text = fire.Apply(":fire", string.Empty).Text;
        Assert.Null(EmojiTyping.Replace(text, "(", fire));
        // `:(` then `(` is one swap from the same place.
        var sad = EmojiTyping.Replace("hi :", "(", null);
        Assert.Equal(new EmojiSwap(3, "😞", ":("), sad);
        Assert.NotNull(sad);
        Assert.Equal(new EmojiSwap(3, "😩", ":(("), EmojiTyping.Replace(sad.Apply("hi :", "").Text, "(", sad));
        // Not right behind it any more: no growing (and a lone `(` is nothing).
        Assert.Null(EmojiTyping.Replace(sad.Apply("hi :", "").Text + "x", "(", sad));
    }

    /// <summary>Offsets are UTF-16: an emoji or a Cyrillic word before the face moves it by its units, never splits a pair.</summary>
    [Fact]
    public void OffsetsAreUtf16Units()
    {
        Assert.Equal(new EmojiSwap(2, "😊", ":)"), EmojiTyping.Replace("😀:", ")", null));
        Assert.Equal(new EmojiSwap(6, "🔥", ":fire:"), EmojiTyping.Replace("Привет:fire", ":", null));
        // A typed surrogate pair is one character, and makes nothing.
        Assert.Null(EmojiTyping.Replace(":", "😀", null));
        Assert.Null(EmojiTyping.Replace(":(", "😀", new EmojiSwap(0, "😞", ":(")));
        // The guards are ASCII: a Cyrillic letter is no letter to them.
        Assert.Equal(new EmojiSwap(1, "❤️", "<3"), EmojiTyping.Replace("я<", "3", null));
        Assert.Equal(new EmojiSwap(1, "😔", "3("), EmojiTyping.Replace("я3", "(", null));
        Assert.Null(EmojiTyping.Replace("z3", "(", null));
        // A full-width digit is no ASCII digit either.
        Assert.Equal(new EmojiSwap(1, "😊", ":)"), EmojiTyping.Replace("１:", ")", null));
    }

    // ---- through a TextBox's events (CapitalField) ----------------------------------------------------------------------------

    private static TypingBox Typed(string keys, bool capitals = true)
    {
        var box = new TypingBox(capitals);
        box.TypeAll(keys);
        return box;
    }

    /// <summary>Capitals decide the typed character first (#97), then the emoji rule sees it: <c>xD</c> at a sentence's start is <c>XD</c>.</summary>
    [Fact]
    public void CapitalsFirstThenEmoji()
    {
        var box = Typed("xD ");
        Assert.Equal("😆 ", box.Text);
        Assert.Equal(3, box.Start);
        // The Backspace puts back what the field held: the capital the rule made, and the rest as typed.
        box.Backspace();
        Assert.Equal("XD ", box.Text);
        Assert.Equal(3, box.Start);

        Assert.Equal("Hi 😊", Typed("hi :)").Text);
        Assert.Equal("Ok. 🔥 yes", Typed("ok. :fire: yes").Text);
        // An emoji ends no sentence.
        Assert.Equal("Привет😊 как дела", Typed("привет:) как дела").Text);
    }

    /// <summary>A single-line field gets emoji and no capitals.</summary>
    [Fact]
    public void ASingleLineFieldGetsEmojiOnly()
    {
        Assert.Equal("hi 😊", Typed("hi :)", capitals: false).Text);
        Assert.Equal("😆 ", Typed("xD ", capitals: false).Text);
        Assert.Equal("xD ", Typed("xD ⌫", capitals: false).Text);
    }

    /// <summary>The characters go in as typed and the emoji replaces them: its own undo step, so Ctrl+Z gives the face back.</summary>
    [Fact]
    public void TheEmojiIsItsOwnUndoStep()
    {
        var box = Typed(":)");
        Assert.Equal("😊", box.Text);
        Assert.Equal(3, box.UndoUnits);
        Assert.Equal(2, box.Start);
    }

    [Fact]
    public void GrowingAndTakingBack()
    {
        var box = Typed(":((");
        Assert.Equal("😩", box.Text);
        box.Backspace();
        Assert.Equal(":((", box.Text);
        Assert.Equal(3, box.Start);
        // Once: the next Backspace is an ordinary one.
        box.Backspace();
        Assert.Equal(":(", box.Text);
        Assert.Equal(":fire:", Typed(":fire:⌫").Text);
        Assert.Equal("🔥😊", Typed(":fire::)").Text);
    }

    /// <summary>In the middle of the text: the face is found behind the caret, what is after it stays, and the caret lands right behind the emoji.</summary>
    [Fact]
    public void InTheMiddleOfTheText()
    {
        var box = new TypingBox { Text = "ab", Start = 1 };
        box.TypeAll(":)");
        Assert.Equal("a😊b", box.Text);
        Assert.Equal(3, box.Start);
        box.Backspace();
        Assert.Equal("a:)b", box.Text);
        Assert.Equal(3, box.Start);
    }

    [Fact]
    public void AnythingElseDropsTheSwap()
    {
        // A click elsewhere, even back again.
        var box = Typed(":)");
        box.Click(0);
        box.Click(2);
        box.Backspace();
        Assert.Equal(string.Empty, box.Text);

        // A selection.
        box = Typed(":)");
        box.Click(0, 2);
        Assert.Null(box.Field.Swap);

        // An arrow: no growing after it, and an ordinary Backspace.
        Assert.Equal("😞(", Typed(":(→(").Text);
        Assert.Equal(string.Empty, Typed(":)→⌫").Text);

        // A Ctrl chord.
        box = Typed(":)");
        box.Field.Key(CapitalField.Classify(0x5A, control: true, alt: false));
        Assert.Null(box.Field.Swap);

        // A paste.
        box = Typed(":)");
        box.Paste("x");
        box.Backspace();
        Assert.Equal("😊", box.Text);

        // The focus leaving.
        box = Typed(":)");
        box.Field.FocusLost();
        Assert.Null(box.Field.Swap);

        // An IME composition starting.
        box = Typed(":)");
        box.Field.CompositionStarted();
        Assert.Null(box.Field.Swap);
    }

    /// <summary>
    /// A typing key that typed nothing — Enter in a single-line box, a dead key, a character the box refused — is something else
    /// happening: the Backspace after it is ordinary, and the character after it grows nothing.
    /// </summary>
    [Fact]
    public void AKeyThatTypedNothingDropsTheSwap()
    {
        var box = Typed(":)", capitals: false);
        box.TypeNothing();
        box.Backspace();
        Assert.Equal(string.Empty, box.Text);

        box = Typed(":(", capitals: false);
        box.TypeNothing(0xDE);
        box.Type("(");
        Assert.Equal("😞(", box.Text);
    }

    /// <summary>A Shift on its own is not "another key": Shift+Backspace still takes the emoji back, and Shift+9 still grows it.</summary>
    [Fact]
    public void AModifierDropsNothing()
    {
        var box = Typed(":(");
        box.Field.Key(CapitalField.Classify(0x10, false, false));
        box.Type("(");
        Assert.Equal("😩", box.Text);
        box.Field.Key(CapitalField.Classify(0x10, false, false));
        box.Backspace();
        Assert.Equal(":((", box.Text);
    }

    /// <summary>Only typing is judged: a paste, the window's own <c>Text =</c> and an IME's text are left as they are.</summary>
    [Fact]
    public void OnlyTypingIsJudged()
    {
        var box = new TypingBox();
        box.Paste(":)");
        Assert.Equal(":)", box.Text);

        box = new TypingBox { Text = ":", Start = 1 };
        box.Change(":)", 2);
        box.Turn();
        Assert.Equal(":)", box.Text);

        box = new TypingBox { Text = ":", Start = 1 };
        box.Field.CompositionStarted();
        box.Type(")");
        Assert.Equal(":)", box.Text);
    }

    [Fact]
    public void CodeIsTypedAsItIs()
    {
        Assert.Equal("`:)", Typed("`:)").Text);
        Assert.Equal("```\r:fire:", Typed("```\r:fire:").Text);
        Assert.Equal("```\rx\r```\r😊", Typed("```\rx\r```\r:)", capitals: false).Text);
    }

    /// <summary>
    /// A box's <c>MaxLength</c>: the few shortcodes shorter than their emoji (<c>:wales:</c> is 7 units, its flag 14) stay as typed
    /// when the emoji would not fit, rather than being cut off by the box.
    /// </summary>
    [Fact]
    public void TheEmojiMustFitTheBox()
    {
        var wales = EmojiTyping.EmojiNamed("wales");
        Assert.NotNull(wales);
        Assert.Equal(wales, Typed(":wales:").Text);
        var box = new TypingBox(capitals: false) { MaxLength = 10 };
        box.TypeAll(":wales:");
        Assert.Equal(":wales:", box.Text);
        box = new TypingBox(capitals: false) { MaxLength = 14 };
        box.TypeAll(":wales:");
        Assert.Equal(wales, box.Text);
        // A face is never longer than what was typed.
        box = new TypingBox(capitals: false) { MaxLength = 2 };
        box.TypeAll(":)");
        Assert.Equal("😊", box.Text);
    }

    /// <summary>A capital's own take-back still works beside the emoji, and a change that went elsewhere before the turn is left as typed.</summary>
    [Fact]
    public void TheCapitalStillWorks()
    {
        var box = Typed("h");
        Assert.Equal("H", box.Text);
        box.Backspace();
        Assert.Equal("h", box.Text);

        box = new TypingBox();
        box.Field.Key(CapitalKey.Typing);
        var plan = box.Field.BeforeChange(":", 1, 0, ":)");
        Assert.NotNull(plan);
        Assert.NotNull(plan.Swap);
        Assert.Null(plan.TakeBack);
        Assert.False(box.Field.StillApplies(plan, ":)x", 3, 0));
        Assert.False(box.Field.StillApplies(plan, ":)", 1, 0));
        Assert.True(box.Field.StillApplies(plan, ":)", 2, 0));
    }
}
