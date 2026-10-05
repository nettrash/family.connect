using System.Text.Json;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// THE SEND SLOT, THE VIDEO BUTTON AND THE ROUND VIDEO'S ARITHMETIC, held to <c>fc_text::record</c> case for case
/// (docs/audio-video-messages-2026-10-04.md, S1.3, S1.4, S5; issue #79). Every case in <c>Fixtures/record-vectors.json</c>
/// was printed by the Rust the web client runs (<c>win/tools/board-oracle</c>, <c>cargo run -- record</c>); the same bytes
/// are held by the Apple and Android ports and CI compares all three copies with a fresh print. The Windows port reads every
/// function in it but the hold's (S8.6: every input clicks here) — and a function added to the original and not read here
/// fails <see cref="TheFileIsTheOneTheOriginalPrinted"/>, so it cannot be skipped in silence.
/// </summary>
/// <remarks>
/// And then what is Windows' own and so not in the vectors: how each row is drawn and named (<see cref="ComposerButton.Face"/>),
/// which press opens the microphone's menu (<see cref="ComposerButton.Respond"/>), the 600 ms guard and the press reaching
/// the slot (<see cref="SlotGuard"/>), and Ctrl+Shift+R held down (<see cref="ShortcutPresses"/>).
/// </remarks>
public sealed class ComposerButtonTests
{
    private static readonly IStringCatalog Say = EnglishCatalog.Instance;
    private static readonly JsonElement[] Cases = Load();

    private static JsonElement[] Load()
    {
        var path = Path.Combine(AppContext.BaseDirectory, "Fixtures", "record-vectors.json");
        using var document = JsonDocument.Parse(File.ReadAllText(path));
        return [.. document.RootElement.EnumerateArray().Select(element => element.Clone())];
    }

    private static IEnumerable<JsonElement> Of(string function) =>
        Cases.Where(row => row.GetProperty("function").GetString() == function);

    private static string? OptionalString(JsonElement row, string name) =>
        row.GetProperty(name).ValueKind == JsonValueKind.Null ? null : row.GetProperty(name).GetString();

    private static Recording RecordingOf(string? spelled) => spelled switch
    {
        "none" => Recording.None,
        "held" => Recording.Held,
        "hands_free" => Recording.HandsFree,
        "hands_free_beside_draft" => Recording.HandsFreeBesideDraft,
        _ => throw new InvalidDataException($"a recording spelled {spelled}"),
    };

    private static SlotInputs SlotOf(JsonElement input) => new(
        RecorderOpen: input.GetProperty("recorder_open").GetBoolean(),
        Recording: RecordingOf(input.GetProperty("recording").GetString()),
        Editing: input.GetProperty("editing").GetBoolean(),
        DraftBlank: input.GetProperty("draft_blank").GetBoolean(),
        Staged: input.GetProperty("staged").GetBoolean(),
        AssistantChat: input.GetProperty("assistant_chat").GetBoolean(),
        CanRecord: input.GetProperty("can_record").GetBoolean(),
        Call: input.GetProperty("call").GetBoolean(),
        Busy: input.GetProperty("busy").GetBoolean(),
        NotSent: input.GetProperty("not_sent").GetBoolean());

    private static DoorInputs DoorOf(JsonElement input) => new(
        SlotOf(input.GetProperty("slot")),
        FamilyOrDirectChat: input.GetProperty("family_or_direct_chat").GetBoolean(),
        UndoWindow: input.GetProperty("undo_window").GetBoolean(),
        ServerOffersRound: input.GetProperty("server_offers_round").GetBoolean(),
        HasCamera: input.GetProperty("has_camera").GetBoolean(),
        EncoderProbePasses: input.GetProperty("encoder_probe_passes").GetBoolean(),
        RecordsRoundVideo: input.GetProperty("records_round_video").GetBoolean());

    /// <summary>The slot as the vectors spell it: its kind in snake case.</summary>
    private static string Spelled(SlotKind kind) => kind switch
    {
        SlotKind.Recorder => "recorder",
        SlotKind.HeldMicrophone => "held_microphone",
        SlotKind.SendVoice => "send_voice",
        SlotKind.StopRecording => "stop_recording",
        SlotKind.Save => "save",
        SlotKind.Send => "send",
        SlotKind.SendDisabled => "send_disabled",
        SlotKind.Dimmed => "dimmed",
        _ => "microphone",
    };

    private static string? Spelled(Dimmed? reason) => reason switch
    {
        Dimmed.Call => "call",
        Dimmed.Busy => "busy",
        Dimmed.NotSent => "not_sent",
        _ => null,
    };

    private static string Spelled(DoorKind kind) => kind switch
    {
        DoorKind.Hidden => "hidden",
        DoorKind.Dimmed => "dimmed",
        _ => "shown",
    };

    [Fact]
    public void TheFileIsTheOneTheOriginalPrinted()
    {
        // 383 when this was written; fewer would be a truncated copy.
        Assert.True(Cases.Length >= 383, $"only {Cases.Length} cases");
        string[] read = ["constants", "composer_slot", "video_door", "round_cap_ms", "round_warning_ms", "round_diameter", "is_round"];
        // The hold's reducer and its threshold are the phones' and tablets' — Windows has no hold (S8.6) — and nothing else is left out.
        string[] notWindows = ["hold_step", "hold_threshold_ms"];
        var named = Cases.Select(row => row.GetProperty("function").GetString()!).ToHashSet();
        Assert.Equal(read.Concat(notWindows).Order(), named.Order());
        Assert.All(read, function => Assert.NotEmpty(Of(function)));
    }

    /// <summary>S1.1's numbers by value, read from the original rather than copied: a changed one is a changed rule.</summary>
    [Fact]
    public void TheConstantsAreTheOriginals()
    {
        var constants = Of("constants").Single().GetProperty("expected");
        Assert.Equal(ComposerButton.ActivationGuardMs, constants.GetProperty("activation_guard_ms").GetInt64());
        Assert.Equal(ComposerButton.ShortestRecordingMs, constants.GetProperty("shortest_recording_ms").GetInt64());
        Assert.Equal(ComposerButton.VoiceCapMs, constants.GetProperty("voice_cap_ms").GetInt64());
        Assert.Equal(ComposerButton.VoiceWarningMs, constants.GetProperty("voice_warning_ms").GetInt64());
        Assert.Equal(ComposerButton.DeleteAsksFromMs, constants.GetProperty("delete_asks_from_ms").GetInt64());
        Assert.Equal(ComposerButton.SlotCrossfadeMs, constants.GetProperty("slot_crossfade_ms").GetInt64());
        Assert.Equal(ComposerButton.MinTargetWindowsEpx, constants.GetProperty("min_target_windows_epx").GetInt32());
        Assert.Equal(ComposerButton.DefaultMaxRoundVideoMs, constants.GetProperty("default_max_round_video_ms").GetInt64());
        Assert.Equal(ComposerButton.RoundCapMarginMs, constants.GetProperty("round_cap_margin_ms").GetInt64());
        Assert.Equal(ComposerButton.RoundWarningLeadMs, constants.GetProperty("round_warning_lead_ms").GetInt64());
        Assert.Equal(ComposerButton.RoundDiameterCompact, constants.GetProperty("round_diameter_compact").GetInt32());
        Assert.Equal(ComposerButton.RoundDiameterRegular, constants.GetProperty("round_diameter_regular").GetInt32());
        Assert.Equal(ComposerButton.VideoDoorLabel, constants.GetProperty("video_door_label").GetString());
        Assert.Equal(ComposerButton.VideoDoorTooltip, constants.GetProperty("video_door_tooltip").GetString());
        // And the voice note's own numbers, which the recorder and the review chip read, are the same ones.
        Assert.Equal(ComposerButton.VoiceCapMs, (long)VoiceNotes.Longest.TotalMilliseconds);
        Assert.Equal(ComposerButton.VoiceWarningMs, (long)VoiceNotes.WarnFrom.TotalMilliseconds);
        Assert.Equal(ComposerButton.ShortestRecordingMs, (long)VoiceNotes.Shortest.TotalMilliseconds);
        Assert.Equal(ComposerButton.DeleteAsksFromMs, (long)VoiceNotes.DeleteAsksFrom.TotalMilliseconds);
        // The visual is 40 in a target that meets the 44 (S8.6).
        Assert.True(ComposerButton.SlotVisualEpx < ComposerButton.MinTargetWindowsEpx);
    }

    /// <summary>
    /// Every slot case: the row, the kind, Save's switch, a dimmed microphone's reason, the label and what activation says.
    /// One test for all of them, so the assertion names the case that disagreed.
    /// </summary>
    [Fact]
    public void EverySlotIsTheRowTheOriginalChooses()
    {
        var checkedCases = 0;
        foreach (var row in Of("composer_slot"))
        {
            var name = row.GetProperty("name").GetString();
            var expected = row.GetProperty("expected");
            var slot = ComposerButton.ComposerSlot(SlotOf(row.GetProperty("input")));
            Assert.True(expected.GetProperty("row").GetInt32() == slot.Row, $"{name}: row {slot.Row}");
            Assert.True(expected.GetProperty("slot").GetString() == Spelled(slot.Kind), $"{name}: {slot.Kind}");
            var enabled = expected.GetProperty("enabled");
            Assert.True(
                (enabled.ValueKind == JsonValueKind.Null ? null : enabled.GetBoolean()) == slot.Enabled,
                $"{name}: enabled {slot.Enabled}");
            Assert.True(OptionalString(expected, "reason") == Spelled(slot.Reason), $"{name}: reason {slot.Reason}");
            Assert.True(OptionalString(expected, "label") == ComposerButton.Label(slot, Say), $"{name}: label {ComposerButton.Label(slot, Say)}");
            Assert.True(OptionalString(expected, "notice") == ComposerButton.Notice(slot.Reason, Say), $"{name}: notice");
            Assert.True(slot.IsMicrophone == slot.Row is >= 7 and <= 10, $"{name}: microphone");
            checkedCases++;
        }
        Assert.Equal(58, checkedCases);
    }

    [Fact]
    public void EveryVideoButtonIsTheOneTheOriginalChooses()
    {
        var checkedCases = 0;
        foreach (var row in Of("video_door"))
        {
            var name = row.GetProperty("name").GetString();
            var expected = row.GetProperty("expected");
            var door = ComposerButton.VideoDoor(DoorOf(row.GetProperty("input")));
            Assert.True(expected.GetProperty("door").GetString() == Spelled(door.Kind), $"{name}: {door.Kind}");
            Assert.True(OptionalString(expected, "reason") == Spelled(door.Reason), $"{name}: reason {door.Reason}");
            Assert.True(OptionalString(expected, "label") == ComposerButton.DoorLabel(door), $"{name}: label");
            Assert.True(OptionalString(expected, "notice") == ComposerButton.Notice(door.Reason, Say), $"{name}: notice");
            checkedCases++;
        }
        Assert.Equal(35, checkedCases);
    }

    /// <summary>
    /// Decision 40: no Windows build before Phase 3d records round video, so wherever the slot is — every slot case in the
    /// file — the door stays shut, even on a server that offers it and a device with a camera.
    /// </summary>
    [Fact]
    public void NoWindowsBuildBeforePhase3dShowsAVideoButton()
    {
        foreach (var row in Of("composer_slot"))
        {
            var door = ComposerButton.VideoDoor(new DoorInputs(
                SlotOf(row.GetProperty("input")), FamilyOrDirectChat: true, UndoWindow: false,
                ServerOffersRound: true, HasCamera: true, EncoderProbePasses: true, RecordsRoundVideo: false));
            Assert.Equal(Door.Hidden, door);
        }
    }

    [Fact]
    public void TheRoundArithmeticIsTheOriginals()
    {
        foreach (var row in Of("round_cap_ms"))
        {
            Assert.True(
                row.GetProperty("expected").GetProperty("cap_ms").GetInt64()
                    == ComposerButton.RoundCapMs(row.GetProperty("input").GetProperty("max_round_video_ms").GetInt64()),
                row.GetProperty("name").GetString());
        }
        foreach (var row in Of("round_warning_ms"))
        {
            Assert.True(
                row.GetProperty("expected").GetProperty("warning_ms").GetInt64()
                    == ComposerButton.RoundWarningMs(row.GetProperty("input").GetProperty("max_round_video_ms").GetInt64()),
                row.GetProperty("name").GetString());
        }
        foreach (var row in Of("round_diameter"))
        {
            var width = row.GetProperty("input").GetProperty("width_class").GetString() switch
            {
                "compact" => WidthClass.Compact,
                "regular" => WidthClass.Regular,
                var other => throw new InvalidDataException($"a width class spelled {other}"),
            };
            Assert.Equal(row.GetProperty("expected").GetProperty("diameter").GetInt32(), ComposerButton.RoundDiameter(width));
        }
        Assert.Equal(22, Of("round_cap_ms").Count() + Of("round_warning_ms").Count());
    }

    [Fact]
    public void ARoundVideoIsExactlyOneFlaggedVideoWithNoBody()
    {
        var checkedCases = 0;
        foreach (var row in Of("is_round"))
        {
            var input = row.GetProperty("input");
            AttachmentFlags[] attachments =
            [
                .. input.GetProperty("attachments").EnumerateArray()
                    .Select(item => new AttachmentFlags(item.GetProperty("kind").GetString()!, item.GetProperty("round").GetBoolean())),
            ];
            var body = input.GetProperty("body").GetString()!;
            var expected = row.GetProperty("expected").GetProperty("round").GetBoolean();
            Assert.True(expected == ComposerButton.IsRound(body, attachments), row.GetProperty("name").GetString());
            // And the test the conversation actually DRAWS by — the message's own — says the same, body and all.
            AttachmentDto[] media =
            [
                .. attachments.Select((item, at) => new AttachmentDto(90 + at, item.Kind, "video/mp4", 1, Round: item.Round)),
            ];
            var message = new MessageDto(1, 42, 9, null, body, "2026-10-05T10:00:00Z", Attachments: media);
            Assert.True(expected == message.RoundVideo is not null, $"MessageDto.RoundVideo: {row.GetProperty("name").GetString()}");
            checkedCases++;
        }
        Assert.Equal(12, checkedCases);
    }

    // ---- what Windows draws for each row (S8.6) ----------------------------------------------------------------

    private static SlotFace FaceOf(Slot slot, bool sendHeld = false) => ComposerButton.Face(slot, sendHeld, Say);

    /// <summary>Send E724, the microphone E720, Stop E71A, Save E73E — each named, each with a tooltip.</summary>
    [Fact]
    public void EachRowHasItsGlyphNameAndTooltip()
    {
        const string Record = "Record a voice message (Ctrl+Shift+R)";
        Assert.Equal(new SlotFace(0xE724, "Send voice message", "Send voice message", "", true, false), FaceOf(new(SlotKind.SendVoice)));
        Assert.Equal(new SlotFace(0xE71A, "Stop recording", "Stop recording", "", true, false), FaceOf(new(SlotKind.StopRecording)));
        Assert.Equal(new SlotFace(0xE73E, "Save", "Save", "", true, false), FaceOf(new(SlotKind.Save, Enabled: true)));
        Assert.Equal(new SlotFace(0xE724, "Send", "Send", "", true, false), FaceOf(new(SlotKind.Send)));
        Assert.Equal(new SlotFace(0xE724, "Send", "Send", "", false, false), FaceOf(new(SlotKind.SendDisabled)));
        Assert.Equal(new SlotFace(0xE720, "Record voice message", Record, "", true, false), FaceOf(new(SlotKind.Microphone)));
        // The phones' held microphone is the Send arrow's row too; Windows never draws it, and would draw it the same.
        Assert.Equal(FaceOf(new(SlotKind.SendVoice)), FaceOf(new(SlotKind.HeldMicrophone)));
        // The recorder (Phase 3d) draws its own controls: until then the slot behind it is a Send that does nothing.
        Assert.False(FaceOf(new(SlotKind.Recorder)).Enabled);
    }

    /// <summary>
    /// DIMMED IS NOT DISABLED (S1.3): a dimmed microphone looks disabled but stays enabled — focusable, clickable — and gives
    /// its reason as Narrator's HelpText (S6). Only row 6, Save with nothing to save, and Send while a send is being written
    /// down are disabled.
    /// </summary>
    [Fact]
    public void ADimmedMicrophoneStaysLiveAndSaysWhy()
    {
        foreach (var (reason, sentence) in new (Dimmed, string)[]
        {
            (Dimmed.Call, "You can record a message after the call."),
            (Dimmed.Busy, "Wait until the current attachment is done."),
            (Dimmed.NotSent, "Send or delete the voice message that wasn't sent first."),
        })
        {
            var face = FaceOf(new(SlotKind.Dimmed, Reason: reason));
            Assert.True(face.Enabled);
            Assert.True(face.LooksDimmed);
            Assert.Equal(0xE720, face.Glyph);
            Assert.Equal("Record voice message", face.Name);
            Assert.Equal(sentence, face.HelpText);
        }
        Assert.False(FaceOf(new(SlotKind.Save, Enabled: false)).Enabled);
        Assert.False(FaceOf(new(SlotKind.Send), sendHeld: true).Enabled);
        // Waiting on a send dims nothing else: a running recording's Send and Stop are always live.
        Assert.True(FaceOf(new(SlotKind.SendVoice), sendHeld: true).Enabled);
        Assert.True(FaceOf(new(SlotKind.StopRecording), sendHeld: true).Enabled);
        Assert.All(
            new[] { SlotKind.SendVoice, SlotKind.StopRecording, SlotKind.Save, SlotKind.Send, SlotKind.SendDisabled, SlotKind.Microphone },
            kind => Assert.False(FaceOf(new(kind, Enabled: true)).LooksDimmed));
    }

    [Fact]
    public void TheSlotSpeaksTheReadersLanguage()
    {
        var russian = JsonCatalog.For("ru");
        Assert.NotEqual("Record voice message", ComposerButton.Face(new(SlotKind.Microphone), false, russian).Name);
        Assert.NotEqual("Send voice message", ComposerButton.Face(new(SlotKind.SendVoice), false, russian).Name);
        Assert.NotEqual("Stop recording", ComposerButton.Face(new(SlotKind.StopRecording), false, russian).Name);
        Assert.NotEqual(
            "Send or delete the voice message that wasn't sent first.",
            ComposerButton.Face(new(SlotKind.Dimmed, Reason: Dimmed.NotSent), false, russian).HelpText);
        // The shortcut is the same keys in every language.
        Assert.EndsWith("(Ctrl+Shift+R)", ComposerButton.Face(new(SlotKind.Microphone), false, russian).Tooltip);
    }

    // ---- which press does what (S8.6) ---------------------------------------------------------------------------

    /// <summary>EVERY INPUT CLICKS: a press of any length with a mouse, a finger or a pen, the keyboard, a screen reader.</summary>
    [Fact]
    public void EveryInputClicks()
    {
        foreach (var device in new[] { SlotDevice.Mouse, SlotDevice.Touch, SlotDevice.Pen, SlotDevice.None })
        {
            Assert.Equal(SlotResponse.Activate, ComposerButton.Respond(SlotPress.Click, device, barrelAtPress: false, microphone: true));
            Assert.Equal(SlotResponse.Activate, ComposerButton.Respond(SlotPress.Click, device, barrelAtPress: false, microphone: false));
        }
        // A barrel button "down" on anything but a pen is no barrel.
        Assert.Equal(SlotResponse.Activate, ComposerButton.Respond(SlotPress.Click, SlotDevice.Mouse, barrelAtPress: true, microphone: true));
    }

    /// <summary>The microphone's menu: a mouse right-click, a pen tap with the barrel button down, Shift+F10 or the Menu key.</summary>
    [Fact]
    public void TheMenuOpensForTheMouseThePensBarrelAndTheKeyboard()
    {
        Assert.Equal(SlotResponse.Menu, ComposerButton.Respond(SlotPress.RightTapped, SlotDevice.Mouse, false, microphone: true));
        Assert.Equal(SlotResponse.Menu, ComposerButton.Respond(SlotPress.RightTapped, SlotDevice.Pen, barrelAtPress: true, microphone: true));
        // The barrel tap may arrive as the button's click: it is the pen's right-click, never a recording.
        Assert.Equal(SlotResponse.Menu, ComposerButton.Respond(SlotPress.Click, SlotDevice.Pen, barrelAtPress: true, microphone: true));
        Assert.Equal(SlotResponse.Menu, ComposerButton.Respond(SlotPress.ContextKey, SlotDevice.None, false, microphone: true));
    }

    /// <summary>
    /// NEVER A HOLD: a touch or pen press-and-hold that Windows turns into a RightTapped anyway is ignored, and so is a context
    /// request at a pointer — RightTapped has already answered the mouse. Trial T4 (Blocked 1) checks it on a real screen.
    /// </summary>
    [Fact]
    public void AHoldNeverOpensTheMenu()
    {
        Assert.Equal(SlotResponse.Ignore, ComposerButton.Respond(SlotPress.RightTapped, SlotDevice.Touch, false, microphone: true));
        Assert.Equal(SlotResponse.Ignore, ComposerButton.Respond(SlotPress.RightTapped, SlotDevice.Pen, barrelAtPress: false, microphone: true));
        Assert.Equal(SlotResponse.Ignore, ComposerButton.Respond(SlotPress.RightTapped, SlotDevice.Touch, barrelAtPress: true, microphone: true));
        foreach (var device in new[] { SlotDevice.Mouse, SlotDevice.Touch, SlotDevice.Pen, SlotDevice.None })
        {
            Assert.Equal(SlotResponse.Ignore, ComposerButton.Respond(SlotPress.ContextAtPointer, device, true, microphone: true));
        }
    }

    /// <summary>The menu is the MICROPHONE's (S1.6): on Send, Save or a running recording a right-click does nothing — and a barrel tap never sends.</summary>
    [Fact]
    public void OnlyTheMicrophoneHasAMenu()
    {
        Assert.Equal(SlotResponse.Ignore, ComposerButton.Respond(SlotPress.RightTapped, SlotDevice.Mouse, false, microphone: false));
        Assert.Equal(SlotResponse.Ignore, ComposerButton.Respond(SlotPress.ContextKey, SlotDevice.None, false, microphone: false));
        Assert.Equal(SlotResponse.Ignore, ComposerButton.Respond(SlotPress.Click, SlotDevice.Pen, barrelAtPress: true, microphone: false));
    }

    // ---- the activation guard (S1.1) and the press reaching the slot (S8.6) ----------------------------------------

    private static readonly Slot Microphone = new(SlotKind.Microphone);
    private static readonly Slot SendText = new(SlotKind.Send);
    private static readonly Slot SendVoice = new(SlotKind.SendVoice);
    private static readonly Slot StopBesideDraft = new(SlotKind.StopRecording);

    [Fact]
    public void TheGuardRunsSixHundredMillisecondsAfterTheSlotsOwnActivation()
    {
        var guard = new SlotGuard();
        Assert.Equal(SlotResponse.Activate, guard.Click(0, Microphone, ""));
        guard.Arm(10_000, "");
        Assert.Equal(SlotResponse.Ignore, guard.Click(10_000, Microphone, ""));
        Assert.Equal(SlotResponse.Ignore, guard.Click(10_599, Microphone, ""));
        Assert.Equal(SlotResponse.Activate, guard.Click(10_600, Microphone, ""));
        Assert.True(guard.Running(10_599));
        Assert.False(guard.Running(10_600));
    }

    /// <summary>
    /// A press that went down while the guard ran is ignored WHOLE, however late it lifts — a slow double click on Send cannot
    /// start a recording — while one that went down before the guard was armed, or after it ended, is a press like any other.
    /// </summary>
    [Fact]
    public void APressThatBeganWhileGuardedIsIgnoredWhole()
    {
        // Down under the guard, lifted long after it: ignored — a pointer's press and a key's alike.
        var pointer = new SlotGuard();
        pointer.Arm(10_000, "");
        pointer.PointerDown(10_100, SlotDevice.Mouse, barrel: false);
        Assert.Equal(SlotResponse.Ignore, pointer.Click(11_500, Microphone, ""));
        var key = new SlotGuard();
        key.Arm(10_000, "");
        key.KeyDown(10_000);
        Assert.Equal(SlotResponse.Ignore, key.Click(12_000, Microphone, ""));
        // Down once it has run out: a press like any other.
        var after = new SlotGuard();
        after.Arm(10_000, "");
        after.PointerDown(10_600, SlotDevice.Touch, barrel: false);
        Assert.Equal(SlotResponse.Activate, after.Click(10_700, Microphone, ""));
        // Down before it was armed: only the clock decides.
        var before = new SlotGuard();
        before.PointerDown(9_990, SlotDevice.Mouse, barrel: false);
        before.Arm(10_000, "");
        Assert.Equal(SlotResponse.Activate, before.Click(10_700, Microphone, ""));
        // A screen reader's Invoke began with no press at all: only the clock decides.
        Assert.Equal(SlotResponse.Activate, pointer.Click(11_600, Microphone, ""));
    }

    /// <summary>
    /// ARMED AGAIN, IT FORGETS NOTHING (S1.1): a click on the microphone arms the guard and the recording starting arms it
    /// again — and the second press of a double click, gone down between the two and held past the later one, is still
    /// ignored whole when it lifts on the Send arrow. It cannot send the recording it double-clicked into.
    /// </summary>
    [Fact]
    public void ArmingAgainKeepsAPressThatBeganUnderTheFirstGuardIgnored()
    {
        foreach (var lift in new long[] { 10_200 + 900, 10_200 + 1_300 })
        {
            var guard = new SlotGuard();
            guard.Arm(10_000, "");
            guard.PointerDown(10_200, SlotDevice.Mouse, barrel: false);
            guard.Arm(10_400, "");
            Assert.Equal(SlotResponse.Ignore, guard.Click(lift, SendVoice, ""));
        }
        // Armed again while it runs, it runs on to 600 ms after the later activation — never shorter.
        var longer = new SlotGuard();
        longer.Arm(10_000, "");
        longer.Arm(10_400, "");
        Assert.True(longer.Running(10_999));
        Assert.False(longer.Running(11_000));
        var shorter = new SlotGuard();
        shorter.Arm(20_000, "");
        shorter.Arm(19_500, "");
        Assert.True(shorter.Running(20_599));
        Assert.False(shorter.Running(20_600));
        // A press that goes down after every guard has run out is a press like any other.
        longer.PointerDown(11_000, SlotDevice.Mouse, barrel: false);
        Assert.Equal(SlotResponse.Activate, longer.Click(11_100, SendVoice, ""));
    }

    /// <summary>
    /// A PRESS IS USED ONCE, AND ENDS. A press that went down while the guard ran and ended without a click — dragged off the
    /// button — leaves nothing behind for the next activation that comes with no press of its own: Narrator's Invoke, Voice
    /// Access.
    /// </summary>
    [Fact]
    public void APressThatEndedWithoutAClickLeavesNothingBehind()
    {
        var guard = new SlotGuard();
        guard.Arm(10_000, "");
        var dragged = guard.PointerDown(10_100, SlotDevice.Mouse, barrel: false);
        guard.PressEnded(dragged);
        Assert.Equal(SlotResponse.Activate, guard.Click(20_000, Microphone, ""));
        // A key that went down under the guard and came up with no click is gone the same way.
        guard.Arm(30_000, "");
        var key = guard.KeyDown(30_100);
        guard.PressEnded(key);
        Assert.Equal(SlotResponse.Activate, guard.Click(31_000, Microphone, ""));
    }

    /// <summary>
    /// A press's end is queued behind the button's own handling (the window posts it), so it can arrive after the NEXT press
    /// went down: an old press ending forgets only itself, and the press under way — gone down while the guard ran — is still
    /// ignored whole when it lifts after the guard has run out.
    /// </summary>
    [Fact]
    public void AnOldPressEndingLeavesTheNewOneAlone()
    {
        var guard = new SlotGuard();
        guard.Arm(10_000, "");
        var first = guard.PointerDown(10_100, SlotDevice.Mouse, barrel: false);
        guard.PointerDown(10_200, SlotDevice.Mouse, barrel: false);
        guard.PressEnded(first);
        Assert.Equal(SlotResponse.Ignore, guard.Click(10_900, SendVoice, ""));
        // A key's press the same way.
        var keys = new SlotGuard();
        keys.Arm(20_000, "");
        var down = keys.KeyDown(20_050);
        keys.KeyDown(20_100);
        keys.PressEnded(down);
        Assert.Equal(SlotResponse.Ignore, keys.Click(20_800, SendVoice, ""));
    }

    /// <summary>
    /// A pen's barrel tap that reaches the slot only as RightTapped opens the menu — and the next activation with no press, a
    /// screen reader's Invoke, is an activation: never the menu's twin, never swallowed as the same barrel tap, on Send too.
    /// </summary>
    [Fact]
    public void ABarrelTapLeavesNothingForTheNextInvoke()
    {
        var guard = new SlotGuard();
        var tap = guard.PointerDown(1_000, SlotDevice.Pen, barrel: true);
        guard.PressEnded(tap);
        Assert.Equal(SlotResponse.Menu, guard.RightTapped(SlotDevice.Pen, microphone: true));
        Assert.Equal(SlotResponse.Activate, guard.Click(5_000, Microphone, ""));
        Assert.Equal(SlotResponse.Activate, guard.Click(5_100, SendText, "hello"));
    }

    /// <summary>
    /// The barrel tap that arrives twice — as the button's click and as RightTapped — opens ONE menu; the next barrel tap opens
    /// its own; and a barrel button "down" on anything but a pen is no barrel.
    /// </summary>
    [Fact]
    public void ABarrelTapOpensOneMenu()
    {
        var guard = new SlotGuard();
        var tap = guard.PointerDown(1_000, SlotDevice.Pen, barrel: true);
        Assert.Equal(SlotResponse.Menu, guard.Click(1_050, Microphone, ""));
        Assert.Equal(SlotResponse.Ignore, guard.RightTapped(SlotDevice.Pen, microphone: true));
        guard.PressEnded(tap);
        guard.PointerDown(2_000, SlotDevice.Pen, barrel: true);
        Assert.Equal(SlotResponse.Menu, guard.RightTapped(SlotDevice.Pen, microphone: true));
        Assert.Equal(SlotResponse.Ignore, guard.Click(2_050, Microphone, ""));
        // A barrel tap on Send is nothing at all — never a send.
        guard.PointerDown(3_000, SlotDevice.Pen, barrel: true);
        Assert.Equal(SlotResponse.Ignore, guard.Click(3_050, SendText, "hello"));
        // A mouse's right-click is always the menu; a touch hold never is.
        guard.PointerDown(4_000, SlotDevice.Mouse, barrel: true);
        Assert.Equal(SlotResponse.Menu, guard.RightTapped(SlotDevice.Mouse, microphone: true));
        Assert.Equal(SlotResponse.Menu, guard.RightTapped(SlotDevice.Mouse, microphone: true));
        guard.PointerDown(5_000, SlotDevice.Touch, barrel: false);
        Assert.Equal(SlotResponse.Ignore, guard.RightTapped(SlotDevice.Touch, microphone: true));
        Assert.Equal(SlotResponse.Activate, guard.Click(5_050, Microphone, ""));
    }

    /// <summary>
    /// ENTER IN THE FIELD IS THE SLOT'S ACTIVATION (S1.3): after the slot's Stop in row 3 — which arms the guard and hands
    /// focus to the field, the words still there — a second Enter sends nothing until the guard has run out, exactly as a
    /// second click on the slot would not (fc_text::record: "a double tap on the Stop square must not send what it staged").
    /// </summary>
    [Fact]
    public void ASecondEnterAfterTheSlotsStopSendsNothing()
    {
        var guard = new SlotGuard();
        guard.Arm(10_000, "the words beside the note");
        Assert.True(guard.IgnoresFieldKey(10_150, repeat: false, sends: true, "the words beside the note"));
        Assert.True(guard.IgnoresFieldKey(10_599, repeat: false, sends: true, "the words beside the note"));
        Assert.False(guard.IgnoresFieldKey(10_600, repeat: false, sends: true, "the words beside the note"));
        // The click on what is now Send is guarded the same way.
        Assert.Equal(SlotResponse.Ignore, guard.Click(10_300, SendText, "the words beside the note"));
        Assert.Equal(SlotResponse.Activate, guard.Click(10_600, SendText, "the words beside the note"));
        // Nothing guards a composer nobody's slot changed.
        Assert.False(new SlotGuard().IgnoresFieldKey(0, repeat: false, sends: true, "hello"));
    }

    /// <summary>
    /// A CHANGE MADE BY TYPING IS NEVER GUARDED (S1.1): after a send emptied the box, "ok" typed at once and sent at once
    /// goes — by Enter or by a click, even before the field has said the words changed. A microphone nobody's words have
    /// touched since still waits.
    /// </summary>
    [Fact]
    public void WordsTypedSinceTheGuardWasArmedAreNeverGuarded()
    {
        var guard = new SlotGuard();
        guard.Arm(10_000, "");
        Assert.False(guard.IgnoresFieldKey(10_300, repeat: false, sends: true, "ok"));
        Assert.Equal(SlotResponse.Activate, guard.Click(10_300, SendText, "ok"));
        // Nothing typed: the microphone the send left behind waits.
        Assert.Equal(SlotResponse.Ignore, guard.Click(10_300, Microphone, ""));
        // A Save after the words changed is a Save.
        guard.Arm(20_000, "");
        Assert.Equal(SlotResponse.Activate, guard.Click(20_100, new Slot(SlotKind.Save, Enabled: true), "fixed"));
        // A Stop is not a send: words changed meanwhile change nothing for it.
        guard.Arm(30_000, "before");
        Assert.Equal(SlotResponse.Ignore, guard.Click(30_100, StopBesideDraft, "after"));
    }

    /// <summary>
    /// STAGING IS NEVER GUARDED (S1.1): after a send emptied the box, a picture pasted or dropped at once and sent at once
    /// goes — by Enter or by a click — though the words did not change. A held key's repeats still never act, and the slot's
    /// own activation arming the guard again — a Stop in row 3, whose note the slot stages itself — forgets what the person
    /// staged before it.
    /// </summary>
    [Fact]
    public void SomethingStagedSinceTheGuardWasArmedIsNeverGuarded()
    {
        var guard = new SlotGuard();
        guard.Arm(10_000, "");
        guard.Changed(recording: false);
        Assert.False(guard.IgnoresFieldKey(10_300, repeat: false, sends: true, ""));
        Assert.Equal(SlotResponse.Activate, guard.Click(10_300, SendText, ""));
        Assert.True(guard.IgnoresFieldKey(10_300, repeat: true, sends: true, ""));
        // A press that went down under the guard, then the picture landed: its Send is the person's, and goes.
        var pressed = new SlotGuard();
        pressed.Arm(20_000, "");
        pressed.PointerDown(20_100, SlotDevice.Mouse, barrel: false);
        pressed.Changed(recording: false);
        Assert.Equal(SlotResponse.Activate, pressed.Click(20_300, SendText, ""));
        // Armed again by the slot's own activation: what was staged before it is forgotten, and Enter waits.
        guard.Arm(30_000, "the words beside the note");
        Assert.True(guard.IgnoresFieldKey(30_100, repeat: false, sends: true, "the words beside the note"));
        Assert.Equal(SlotResponse.Ignore, guard.Click(30_100, SendText, "the words beside the note"));
        // Nothing staged, nothing typed: a send's guard holds, as it always did.
        var untouched = new SlotGuard();
        untouched.Arm(40_000, "");
        Assert.True(untouched.IgnoresFieldKey(40_300, repeat: false, sends: true, ""));
    }

    /// <summary>
    /// THE PERSON'S OWN CHANGE LIFTS THE GUARD (S1.1; fc_text::record's OtherAction — "the vector …typed and deleted…"):
    /// "ok" sent, then "x" typed and deleted at once, leaves a microphone that records; words typed and taken back to the
    /// very words the guard was armed over leave a Send that sends; and the field's own clear after a send, which leaves
    /// what the guard was armed over, lifts nothing. While a recording runs nothing lifts the guard on its Send, and a
    /// press on the microphone that went down under the guard stays ignored whole.
    /// </summary>
    [Fact]
    public void ThePersonsOwnChangeLiftsTheGuardForTheMicrophoneToo()
    {
        var typed = new SlotGuard();
        typed.Arm(10_000, "");
        typed.TextChanged("", recording: false);
        Assert.Equal(SlotResponse.Ignore, typed.Click(10_100, Microphone, ""));
        typed.TextChanged("x", recording: false);
        typed.TextChanged("", recording: false);
        Assert.False(typed.Running(10_200));
        Assert.Equal(SlotResponse.Activate, typed.Click(10_200, Microphone, ""));

        // Back to the words the Stop in row 3 left beside its note: still the person's change.
        var back = new SlotGuard();
        back.Arm(20_000, "dinner?");
        back.TextChanged("dinner", recording: false);
        back.TextChanged("dinner?", recording: false);
        Assert.Equal(SlotResponse.Activate, back.Click(20_200, SendText, "dinner?"));
        Assert.False(back.IgnoresFieldKey(20_200, repeat: false, sends: true, "dinner?"));

        // Recording: the box is behind the row, and the guard on its Send stays.
        var recording = new SlotGuard();
        recording.Arm(30_000, "");
        recording.TextChanged("x", recording: true);
        recording.Changed(recording: true);
        Assert.True(recording.Running(30_100));
        Assert.Equal(SlotResponse.Ignore, recording.Click(30_100, SendVoice, ""));

        // A press on the microphone that went down under the guard is ignored whole, whatever changes before it lifts.
        var pressed = new SlotGuard();
        pressed.Arm(40_000, "");
        pressed.PointerDown(40_100, SlotDevice.Mouse, barrel: false);
        pressed.TextChanged("x", recording: false);
        pressed.TextChanged("", recording: false);
        Assert.Equal(SlotResponse.Ignore, pressed.Click(40_200, Microphone, ""));
    }

    /// <summary>
    /// AN ITEM TAKEN OFF THE STRIP IS THE PERSON'S CHANGE (S1.1, S2.7): after a Stop beside words, ✕ on the note and Send at
    /// once sends the words — as on Android and the web.
    /// </summary>
    [Fact]
    public void AnItemTakenOffAfterAStopBesideItLeavesASendThatSends()
    {
        var guard = new SlotGuard();
        guard.Arm(10_000, "the words beside the note");
        Assert.Equal(SlotResponse.Ignore, guard.Click(10_100, SendText, "the words beside the note"));
        guard.Changed(recording: false);
        Assert.Equal(SlotResponse.Activate, guard.Click(10_150, SendText, "the words beside the note"));
        Assert.False(guard.IgnoresFieldKey(10_150, repeat: false, sends: true, "the words beside the note"));
    }

    /// <summary>
    /// A KEY HELD DOWN IS ONE PRESS (S2.4, S8.6): its first went wherever focus was — the slot's Stop or Send, the recording
    /// row's Stop or Delete, Esc stopping a recording — and handed focus to the field; its repeats there never send, guard
    /// or no guard, and an Esc's never drop the reply the note was recorded under. A fresh Esc is not the slot's and waits
    /// for nothing.
    /// </summary>
    [Fact]
    public void AHeldKeysRepeatsNeverActInTheField()
    {
        var guard = new SlotGuard();
        Assert.True(guard.IgnoresFieldKey(50_000, repeat: true, sends: true, "hello"));
        Assert.True(guard.IgnoresFieldKey(50_000, repeat: true, sends: false, "hello"));
        guard.Arm(60_000, "hello");
        Assert.True(guard.IgnoresFieldKey(61_000, repeat: true, sends: true, "hello"));
        Assert.True(guard.IgnoresFieldKey(61_000, repeat: true, sends: true, "hello, edited"));
        Assert.False(guard.IgnoresFieldKey(60_100, repeat: false, sends: false, "hello"));
    }

    /// <summary>
    /// CTRL+SHIFT+R HELD DOWN IS ONE PRESS (S1.6): an accelerator repeats while its key is held, so the window swallows the
    /// chord's repeats as they come in and an invocation that still follows one does nothing — a held chord neither stops
    /// the recording it started nor starts one after its Stop. A fresh press always acts, and so does one whose key-down the
    /// window never saw.
    /// </summary>
    [Fact]
    public void AHeldShortcutActsOnce()
    {
        var shortcut = new ShortcutPresses();
        Assert.True(shortcut.Fresh(0));
        Assert.False(shortcut.KeyDown(1_000, repeat: false));
        Assert.True(shortcut.Fresh(1_000));
        Assert.True(shortcut.KeyDown(1_500, repeat: true));
        Assert.False(shortcut.Fresh(1_500));
        Assert.False(shortcut.Fresh(1_500 + ShortcutPresses.SameKeyMs));
        Assert.True(shortcut.KeyDown(1_533, repeat: true));
        Assert.False(shortcut.Fresh(1_540));
        // Long after the last repeat seen, an invocation is another press — one whose key went down outside this view.
        Assert.True(shortcut.Fresh(1_533 + ShortcutPresses.SameKeyMs + 1));
        Assert.False(shortcut.KeyDown(3_000, repeat: false));
        Assert.True(shortcut.Fresh(3_000));
    }
}
