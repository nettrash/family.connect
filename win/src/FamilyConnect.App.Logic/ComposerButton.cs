using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

// Voice and video messages from the Send button (docs/audio-video-messages-2026-10-04.md — "the plan" below — issue #79):
// the composer's trailing slot, the video button inside the empty field, and the round video's arithmetic, ported from
// fc_text::record and held to it case for case by record-vectors.json (ComposerButtonTests). Windows reads every rule in
// the vectors but the voice reducer's (S8.6: every input clicks here, and the window drives its recording itself), and adds
// the two of its own the plan gives it — which press opens the slot's menu, and the 600 ms guard measured on the slot's own
// presses. Amended 2026-10-06 (#79): the phones' hold-to-talk is gone from every client, so the vectors no longer carry a
// held microphone, a hold threshold or an Undo window — one way into a voice recording, an activation, everywhere.

/// <summary>
/// The voice recording the composer is showing, as the slot sees it (S1.3). Every recording is hands-free: an activation of
/// the slot or the menu starts it, on every client (the hold was withdrawn 2026-10-06).
/// </summary>
public enum Recording
{
    None,

    /// <summary>Hands-free, started with the composer empty: row 2, the Send arrow.</summary>
    HandsFree,

    /// <summary>Hands-free, started from the paperclip or the shortcut beside words or staged items: row 3, the Stop square.</summary>
    HandsFreeBesideDraft,
}

/// <summary>Why the microphone is dimmed — rows 7, 8 and 9, in that order. Dimmed is not disabled: it says why.</summary>
public enum Dimmed
{
    /// <summary>A call in any phase but idle or ended (Windows' <c>callBusy</c>).</summary>
    Call,

    /// <summary>The composer's attachment guard (Windows' <c>strip.Preparing || sendingMedia</c>).</summary>
    Busy,

    /// <summary>The chat holds a "Voice message not sent" row (S2.8).</summary>
    NotSent,
}

/// <summary>What the composer is, for the slot (S1.2). Thread composers do not ask: they keep today's Send.</summary>
/// <param name="RecorderOpen">The video recorder is open (S3) — it owns the row. Never before Windows' Phase 3d.</param>
/// <param name="DraftBlank">The draft is blank after the client's own trim.</param>
/// <param name="Staged">Anything is staged. A primed reply is not: whatever is recorded carries it.</param>
/// <param name="AssistantChat">The assistant's chat (<c>kind = ai</c>).</param>
/// <param name="CanRecord">The platform can record sound at all.</param>
/// <param name="Call">A call in any phase but idle or ended.</param>
/// <param name="Busy">The composer's attachment guard.</param>
/// <param name="NotSent">The chat holds a not-sent voice message.</param>
public readonly record struct SlotInputs(
    bool RecorderOpen,
    Recording Recording,
    bool Editing,
    bool DraftBlank,
    bool Staged,
    bool AssistantChat,
    bool CanRecord,
    bool Call,
    bool Busy,
    bool NotSent)
{
    /// <summary>S1.2's <b>empty</b>: the draft is blank AND nothing is staged.</summary>
    public bool Empty => DraftBlank && !Staged;
}

/// <summary>The rows of S1.3, by what the slot shows.</summary>
public enum SlotKind
{
    /// <summary>Row 1: the video recorder owns the row.</summary>
    Recorder,

    /// <summary>Row 2: the Send arrow — stops the recording and sends it.</summary>
    SendVoice,

    /// <summary>Row 3: the Stop square — stops; the note is staged beside the words.</summary>
    StopRecording,

    /// <summary>Row 4: today's Save, disabled while the field is blank. Never a microphone.</summary>
    Save,

    /// <summary>Row 5: Send, by today's rules.</summary>
    Send,

    /// <summary>Row 6: Send, disabled — the assistant's chat, or nothing can record.</summary>
    SendDisabled,

    /// <summary>Rows 7–9: the microphone, dimmed; activating it says why.</summary>
    Dimmed,

    /// <summary>Row 10: the microphone — a hands-free recording when activated.</summary>
    Microphone,
}

/// <summary>What the slot shows and does: one row of S1.3.</summary>
/// <param name="Enabled">Save's own switch (row 4); null on every other row.</param>
/// <param name="Reason">Why a dimmed microphone is dimmed (rows 7–9); null on every other row.</param>
public readonly record struct Slot(SlotKind Kind, bool? Enabled = null, Dimmed? Reason = null)
{
    /// <summary>The S1.3 row this is.</summary>
    public int Row => Kind switch
    {
        SlotKind.Recorder => 1,
        SlotKind.SendVoice => 2,
        SlotKind.StopRecording => 3,
        SlotKind.Save => 4,
        SlotKind.Send => 5,
        SlotKind.SendDisabled => 6,
        SlotKind.Dimmed => Reason switch
        {
            Dimmed.Call => 7,
            Dimmed.Busy => 8,
            _ => 9,
        },
        _ => 10,
    };

    /// <summary>"The slot is a microphone" — rows 7 to 10, where the video button may show (S1.4) and the menu opens (S1.6).</summary>
    public bool IsMicrophone => Kind is SlotKind.Dimmed or SlotKind.Microphone;
}

/// <summary>The video button inside the empty field (S1.4).</summary>
public enum DoorKind
{
    /// <summary>Not drawn: the field gets its width back.</summary>
    Hidden,

    /// <summary>Drawn dimmed, saying the slot's own sentence — rows 7 and 8 only.</summary>
    Dimmed,

    /// <summary>Drawn; activating it opens the recorder (S3).</summary>
    Shown,
}

/// <summary>Whether the video button is hidden, dimmed — and why — or shown.</summary>
public readonly record struct Door(DoorKind Kind, Dimmed? Reason = null)
{
    public static readonly Door Hidden = new(DoorKind.Hidden);
}

/// <summary>What the video button needs to know besides the slot (S1.4, S1.2).</summary>
/// <param name="FamilyOrDirectChat">The chat's main composer, in a family or a direct chat — never the assistant's, never a thread.</param>
/// <param name="ServerOffersRound">The server sends <c>max_round_video_ms</c> on <c>GET /families/mine</c>.</param>
/// <param name="HasCamera">The device has a camera.</param>
/// <param name="EncoderProbePasses">The web's encoder probe; true everywhere else.</param>
/// <param name="RecordsRoundVideo">THIS build records round video on this platform — on Windows, once Phase 3d ships (Decision 40).</param>
public readonly record struct DoorInputs(
    SlotInputs Slot,
    bool FamilyOrDirectChat,
    bool ServerOffersRound,
    bool HasCamera,
    bool EncoderProbePasses,
    bool RecordsRoundVideo)
{
    /// <summary>S1.2's <b>round available</b>: all four.</summary>
    public bool RoundAvailable => ServerOffersRound && HasCamera && EncoderProbePasses && RecordsRoundVideo;
}

/// <summary>How wide a window draws a received circle (S5.2): Windows is always <see cref="Regular"/>.</summary>
public enum WidthClass
{
    Compact,
    Regular,
}

/// <summary>What <see cref="ComposerButton.IsRound"/> reads of one attachment: its kind as the wire spells it, and its flag.</summary>
public readonly record struct AttachmentFlags(string Kind, bool Round);

/// <summary>Which press reached the slot (S8.6), as WinUI raises it.</summary>
public enum SlotPress
{
    /// <summary><c>Click</c>: a press of any length with any pointer, Enter or Space, a screen reader's Invoke.</summary>
    Click,

    /// <summary><c>RightTapped</c>: a mouse right-click — and, perhaps, a touch or pen press-and-hold (trial T4).</summary>
    RightTapped,

    /// <summary><c>ContextRequested</c> with no position: Shift+F10 or the Menu key.</summary>
    ContextKey,

    /// <summary><c>ContextRequested</c> at a position: the pointer's own context gesture, which RightTapped already answers.</summary>
    ContextAtPointer,
}

/// <summary>The pointer behind a press, as <c>PointerDeviceType</c> says it — <see cref="None"/> for the keyboard and a screen reader.</summary>
public enum SlotDevice
{
    None,
    Mouse,
    Touch,
    Pen,
}

/// <summary>What a press on the slot does.</summary>
public enum SlotResponse
{
    /// <summary>The slot's own activation (S1.3's last column).</summary>
    Activate,

    /// <summary>The microphone's secondary menu (S1.6).</summary>
    Menu,

    /// <summary>Nothing.</summary>
    Ignore,
}

/// <summary>How the slot is drawn and named in one state: a Segoe Fluent glyph, the name and tooltip, and whether it is live.</summary>
/// <param name="Glyph">The Segoe Fluent Icons code point.</param>
/// <param name="Name">Its <c>AutomationProperties.Name</c> — S6's label.</param>
/// <param name="Tooltip">What a pointer resting on it is told.</param>
/// <param name="HelpText">Narrator's HelpText: a dimmed microphone's reason (S6); empty otherwise.</param>
/// <param name="Enabled">Whether the button takes activation at all — false only where S1.3 says "disabled".</param>
/// <param name="LooksDimmed">Drawn in the disabled colours although it is enabled — rows 7–9: dimmed is not disabled.</param>
public sealed record SlotFace(int Glyph, string Name, string Tooltip, string HelpText, bool Enabled, bool LooksDimmed);

/// <summary>
/// The composer's trailing slot, Send and microphone in one place (S1.3), the video button beside it (S1.4), and the round
/// video's arithmetic — <c>fc_text::record</c> in C#, held to it by the vectors — and what Windows adds of its own (S8.6).
/// </summary>
/// <remarks>
/// <para>
/// <b>THE FIRST MATCHING ROW WINS</b>, exactly as the shared rule's: the recorder, a recording running, an edit (never a
/// microphone, even with the field cleared), something to send, the assistant's chat, and only then the microphone —
/// dimmed for a call, an attachment on its way or a voice message not sent, in that order.
/// </para>
/// <para>
/// <b>THE VIDEO BUTTON'S RULE IS HERE BEFORE THE VIDEO BUTTON IS</b>: Windows records no round video until Phase 3d, so
/// every caller passes <see cref="DoorInputs.RecordsRoundVideo"/> false and the door stays hidden — Decision 40, a camera
/// button with nothing behind it is never drawn — and that phase only wires it.
/// </para>
/// <para>
/// <b>EVERY INPUT CLICKS ON WINDOWS</b> (S8.6, Decision 28): a press of any length with a mouse, a finger or a pen records
/// hands-free. The menu opens for a mouse right-click, a pen tap with the barrel button down and Shift+F10 or the Menu
/// key — never for a touch or pen hold, which simply clicks (<see cref="Respond"/>).
/// </para>
/// </remarks>
public static class ComposerButton
{
    // S1.1's numbers, as the shared module has them (ComposerButtonTests reads each one back from the vectors).

    /// <summary>The slot ignores activation this long after its OWN activation changed it (<see cref="SlotGuard"/>).</summary>
    public const long ActivationGuardMs = 600;

    /// <summary>Nothing shorter is ever sent (<see cref="VoiceNotes.Shortest"/>).</summary>
    public const long ShortestRecordingMs = 1_000;

    /// <summary>A voice note's length, and where "30 seconds left" is shown and announced.</summary>
    public const long VoiceCapMs = 300_000;
    public const long VoiceWarningMs = 270_000;

    /// <summary>Deleting a recording this long or longer asks first.</summary>
    public const long DeleteAsksFromMs = 10_000;

    /// <summary>The slot's Send ↔ microphone cross-fade; none when Windows' animations are off.</summary>
    public const long SlotCrossfadeMs = 150;

    /// <summary>The least hit area on Windows, in effective pixels: the slot's visual is 40, its target 44.</summary>
    public const int MinTargetWindowsEpx = 44;

    /// <summary>The slot's visual, inside its <see cref="MinTargetWindowsEpx"/> target (S8.6).</summary>
    public const int SlotVisualEpx = 40;

    /// <summary><c>max_round_video_ms</c> when a server has round video, and the round video's margins.</summary>
    public const long DefaultMaxRoundVideoMs = 60_000;
    public const long RoundCapMarginMs = 500;
    public const long RoundWarningLeadMs = 10_000;

    /// <summary>A received circle's diameter (S5.2).</summary>
    public const int RoundDiameterCompact = 200;
    public const int RoundDiameterRegular = 240;

    /// <summary>
    /// The video button's label and tooltip — English keys, not yet said through the catalogue: no Windows build draws
    /// the button before Phase 3d, whose strings arrive with it.
    /// </summary>
    public const string VideoDoorLabel = "Record video message";
    public const string VideoDoorTooltip = "Record a video message";

    /// <summary>The shortcut that records (S1.6): Ctrl+Shift+R — the app's first keyboard accelerator — as the tooltip and menus write it.</summary>
    public const string RecordShortcut = "Ctrl+Shift+R";

    // Segoe Fluent Icons (S1.3's glyphs, S8.6).
    public const int SendGlyph = 0xE724;
    public const int MicrophoneGlyph = 0xE720;
    public const int StopGlyph = 0xE71A;
    public const int SaveGlyph = 0xE73E;

    /// <summary>The composer's trailing slot (S1.3): the first matching row wins.</summary>
    public static Slot ComposerSlot(SlotInputs inputs)
    {
        if (inputs.RecorderOpen)
        {
            return new(SlotKind.Recorder);
        }
        switch (inputs.Recording)
        {
            case Recording.HandsFree:
                return new(SlotKind.SendVoice);
            case Recording.HandsFreeBesideDraft:
                return new(SlotKind.StopRecording);
        }
        if (inputs.Editing)
        {
            return new(SlotKind.Save, Enabled: !inputs.DraftBlank);
        }
        if (!inputs.Empty)
        {
            return new(SlotKind.Send);
        }
        if (inputs.AssistantChat || !inputs.CanRecord)
        {
            return new(SlotKind.SendDisabled);
        }
        if (inputs.Call)
        {
            return new(SlotKind.Dimmed, Reason: Dimmed.Call);
        }
        if (inputs.Busy)
        {
            return new(SlotKind.Dimmed, Reason: Dimmed.Busy);
        }
        if (inputs.NotSent)
        {
            return new(SlotKind.Dimmed, Reason: Dimmed.NotSent);
        }
        return new(SlotKind.Microphone);
    }

    /// <summary>The slot's accessibility label (S6), or null where the recorder owns the row.</summary>
    public static string? Label(Slot slot, IStringCatalog say) => slot.Kind switch
    {
        SlotKind.Recorder => null,
        SlotKind.SendVoice => say.Get("Send voice message"),
        SlotKind.StopRecording => say.Get("Stop recording"),
        SlotKind.Save => say.Get("Save"),
        SlotKind.Send or SlotKind.SendDisabled => say.Get("Send"),
        _ => say.Get("Record voice message"),
    };

    /// <summary>What activating a dimmed slot — or a dimmed video button — says instead of acting; null where nothing is dimmed.</summary>
    public static string? Notice(Dimmed? reason, IStringCatalog say) => reason switch
    {
        Dimmed.Call => say.Get("You can record a message after the call."),
        Dimmed.Busy => say.Get("Wait until the current attachment is done."),
        Dimmed.NotSent => say.Get("Send or delete the voice message that wasn't sent first."),
        _ => null,
    };

    /// <summary>
    /// Whether the video button is hidden, dimmed or shown (S1.4): shown when the slot is a microphone (rows 7–10) in a family
    /// or direct chat with round video available; dimmed with the slot's sentence in rows 7 and 8; usable in row 9, because
    /// the not-sent rule is about voice; hidden everywhere else — and in every Windows build before Phase 3d.
    /// </summary>
    public static Door VideoDoor(DoorInputs inputs)
    {
        if (!inputs.FamilyOrDirectChat || !inputs.RoundAvailable)
        {
            return Door.Hidden;
        }
        var slot = ComposerSlot(inputs.Slot);
        return slot switch
        {
            { Kind: SlotKind.Dimmed, Reason: Dimmed.Call or Dimmed.Busy } => new(DoorKind.Dimmed, slot.Reason),
            { Kind: SlotKind.Dimmed } or { Kind: SlotKind.Microphone } => new(DoorKind.Shown),
            _ => Door.Hidden,
        };
    }

    /// <summary>The video button's label, where it is drawn — the English key (see <see cref="VideoDoorLabel"/>).</summary>
    public static string? DoorLabel(Door door) => door.Kind == DoorKind.Hidden ? null : VideoDoorLabel;

    /// <summary>Where a round video stops: <c>max_round_video_ms</c> − 500, 59.5 s at 60 000 — never a negative eternity.</summary>
    public static long RoundCapMs(long maxRoundVideoMs) => Math.Max(0, maxRoundVideoMs - RoundCapMarginMs);

    /// <summary>Where "10 seconds left" is shown and announced: <c>max_round_video_ms</c> − 10 000, 50 s at 60 000; 0 below ten seconds.</summary>
    public static long RoundWarningMs(long maxRoundVideoMs) => Math.Max(0, maxRoundVideoMs - RoundWarningLeadMs);

    /// <summary>A received circle's diameter, in effective pixels (S5.2).</summary>
    public static int RoundDiameter(WidthClass width) => width == WidthClass.Compact ? RoundDiameterCompact : RoundDiameterRegular;

    /// <summary>
    /// The drawing test (S5.1), the same on every client: exactly one attachment, <c>kind = video</c>, carrying
    /// <c>round: true</c>, and no body — compared EXACTLY, because the server stores a captionless attachment message's body
    /// as <c>""</c> and no port's idea of whitespace may make two clients disagree.
    /// </summary>
    public static bool IsRound(string body, IReadOnlyList<AttachmentFlags> attachments) =>
        body.Length == 0 && attachments is [{ Kind: "video", Round: true }];

    /// <summary>
    /// Whether the slot's tooltip may open while a press of <paramref name="pressing"/> is down (2026-10-06): never under a
    /// finger or a pen, where Windows opens a tooltip on a press-and-hold — a callout on a long press, which no client shows on
    /// the microphone any more — and as ever under a mouse, whose tooltip is a hover's, and for a key or a screen reader.
    /// </summary>
    public static bool TooltipDuring(SlotDevice pressing) => pressing is not (SlotDevice.Touch or SlotDevice.Pen);

    /// <summary>
    /// What a press on the slot does (S8.6). A click — any pointer, any length, the keyboard, a screen reader — activates,
    /// except a pen tap made with the barrel button down, which is the pen's right-click. The menu is the MICROPHONE's
    /// (rows 7–10): a mouse right-click, that barrel tap, Shift+F10 or the Menu key. Any other RightTapped — a touch or pen
    /// press-and-hold, if Windows raises one with holding off — and a context request at a pointer are ignored, so a long
    /// press simply clicks.
    /// </summary>
    /// <param name="barrelAtPress">The pen's barrel button was down when the press began (<c>IsBarrelButtonPressed</c> at PointerPressed).</param>
    /// <param name="microphone">The slot is a microphone (rows 7–10).</param>
    public static SlotResponse Respond(SlotPress press, SlotDevice device, bool barrelAtPress, bool microphone)
    {
        var barrel = device == SlotDevice.Pen && barrelAtPress;
        var wantsMenu = press switch
        {
            SlotPress.Click => barrel,
            SlotPress.RightTapped => device == SlotDevice.Mouse || barrel,
            SlotPress.ContextKey => true,
            _ => false,
        };
        if (!wantsMenu)
        {
            return press == SlotPress.Click ? SlotResponse.Activate : SlotResponse.Ignore;
        }
        return microphone ? SlotResponse.Menu : SlotResponse.Ignore;
    }

    /// <summary>
    /// The slot as drawn in one state (S8.6): Send E724, the microphone E720, Stop E71A, Save E73E — each with its name and
    /// tooltip. A dimmed microphone stays enabled and gives its reason as HelpText; only row 6 (and Save with nothing to
    /// save, and Send while a send is being written down) is disabled.
    /// </summary>
    /// <param name="sendHeld">A send's files are being written down or a place is being found — today's Send waits for them.</param>
    public static SlotFace Face(Slot slot, bool sendHeld, IStringCatalog say)
    {
        var record = say.Format("Record a voice message (%@)", RecordShortcut);
        switch (slot.Kind)
        {
            case SlotKind.SendVoice:
            {
                var name = say.Get("Send voice message");
                return new(SendGlyph, name, name, string.Empty, Enabled: true, LooksDimmed: false);
            }
            case SlotKind.StopRecording:
            {
                var name = say.Get("Stop recording");
                return new(StopGlyph, name, name, string.Empty, Enabled: true, LooksDimmed: false);
            }
            case SlotKind.Save:
            {
                var name = say.Get("Save");
                return new(SaveGlyph, name, name, string.Empty, Enabled: slot.Enabled == true, LooksDimmed: false);
            }
            case SlotKind.Send:
            {
                var name = say.Get("Send");
                return new(SendGlyph, name, name, string.Empty, Enabled: !sendHeld, LooksDimmed: false);
            }
            case SlotKind.Dimmed:
                return new(MicrophoneGlyph, say.Get("Record voice message"), record, Notice(slot.Reason, say) ?? string.Empty,
                    Enabled: true, LooksDimmed: true);
            case SlotKind.Microphone:
                return new(MicrophoneGlyph, say.Get("Record voice message"), record, string.Empty, Enabled: true, LooksDimmed: false);
            default:
            {
                // Row 6 — and row 1, whose recorder draws its own controls (Phase 3d): today's Send, disabled.
                var name = say.Get("Send");
                return new(SendGlyph, name, name, string.Empty, Enabled: false, LooksDimmed: false);
            }
        }
    }
}

/// <summary>
/// The slot's activation guard (S1.1) and the press reaching the slot (S8.6). For 600 ms after the slot's OWN activation
/// changed it — a send that emptied the composer, a click that started a recording, a Send that ended one or found it too
/// short, a Stop in row 3 — the slot ignores activation, and a press that went down while the guard ran is ignored WHOLE,
/// however late it lifts and however often the guard is armed again before it does. A double click on Send cannot start a
/// recording; a double click on the microphone cannot send one. Typing, pasting and staging never arm it, so "ok" followed
/// at once by Send still sends. Any change the PERSON makes to the composer — a character typed or deleted, a paste, a
/// suggestion taken, an item staged or taken off the strip (<see cref="Changed"/>, <see cref="TextChanged"/>) — LIFTS it,
/// for the microphone and for Send alike, exactly as <c>fc_text::record</c>'s <c>OtherAction</c> does on every other client:
/// "ok" sent and "x" typed and deleted at once leaves a microphone that records. A Send or a Save after the words have
/// changed since it was armed is never guarded either, even before the field has said so.
/// </summary>
/// <remarks>
/// <para>
/// <b>ENTER IN THE FIELD IS THE SLOT'S ACTIVATION</b> in rows 2 to 5 (S1.3; <c>fc_text::record</c>'s <c>guarded</c>: "a
/// port asks here before sending"), so it waits out the same guard: a second Enter after the slot's Stop in row 3 must not
/// send the words the note was staged beside. <b>A KEY HELD DOWN IS ONE PRESS</b>: its first may have been the slot's, or
/// the recording row's, and handed focus to the field (S2.4) — its repeats there never act (<see cref="IgnoresFieldKey"/>).
/// </para>
/// <para>
/// <b>A PRESS IS USED ONCE, AND ENDS.</b> The press under way — a pointer's from its going down, Enter's or Space's on the
/// focused slot — is what a click reads its pointer, a pen's barrel button and the guard from; the click uses it up, and its
/// end forgets it, so an activation no press began — Narrator's Invoke, Voice Access — reads none of a press that came
/// before it and never opened the menu or was guarded for it.
/// </para>
/// <para>Times are milliseconds on one monotonic clock (<see cref="Environment.TickCount64"/> in the window).</para>
/// </remarks>
public sealed class SlotGuard
{
    /// <summary>One press on the slot: its pointer (none for a key), a pen's barrel button as it went down, and whether the guard ran then.</summary>
    private sealed record Press(long Id, SlotDevice Device, bool Barrel, bool BeganGuarded);

    private long until = long.MinValue;
    private string? armedOver;
    private bool changedSinceArmed;
    private Press? live;
    private Press? lastPointer;
    private long lastId;
    private long menuFor = -1;

    /// <summary>
    /// The slot's own activation changed it, now, leaving <paramref name="draft"/> in the field. Armed again while it runs,
    /// it runs on to 600 ms after the later activation — never shorter.
    /// </summary>
    public void Arm(long nowMs, string draft)
    {
        until = Math.Max(until, nowMs + ComposerButton.ActivationGuardMs);
        armedOver = draft;
        changedSinceArmed = false;
    }

    /// <summary>
    /// The person changed the composer — a picture pasted, dropped or picked landed in the strip, an item was taken off it
    /// with its ✕, or the words changed (<see cref="TextChanged"/>): <c>fc_text::record</c>'s <c>OtherAction</c>. A change
    /// the person made is never guarded (S1.1), so the guard is lifted — the microphone records at the next click, and a
    /// Send or a Save goes, even one whose press went down while the guard still ran (its Send is the person's). A press on
    /// the microphone that went down under the guard stays ignored whole. While a recording runs (<paramref name="recording"/>)
    /// nothing in the box is the person's to change, and the guard that keeps a double click from sending stays. The slot's
    /// own staging — the note a Stop in row 3 stages — never tells it.
    /// </summary>
    public void Changed(bool recording)
    {
        if (recording)
        {
            return;
        }
        until = long.MinValue;
        changedSinceArmed = true;
    }

    /// <summary>
    /// The field's words are now <paramref name="draft"/>. Words that differ from those the guard was armed over are the
    /// person's change (<see cref="Changed"/>) — typed, deleted, pasted or a suggestion taken: the box's own changes are made
    /// before the slot arms the guard over what they leave (a send's clear), so they never differ from it.
    /// </summary>
    public void TextChanged(string draft, bool recording)
    {
        if (armedOver is not null && !string.Equals(draft, armedOver, StringComparison.Ordinal))
        {
            Changed(recording);
        }
    }

    /// <summary>Whether the guard is running now.</summary>
    public bool Running(long nowMs) => nowMs < until;

    /// <summary>A pointer goes down on the slot, now — a pen with its barrel button held, or not. Answers the press, for its end.</summary>
    public long PointerDown(long nowMs, SlotDevice device, bool barrel)
    {
        var press = new Press(++lastId, device, device == SlotDevice.Pen && barrel, Running(nowMs));
        live = press;
        lastPointer = press;
        return press.Id;
    }

    /// <summary>Enter or Space goes down on the focused slot, now — not a repeat, which is the same press. Answers the press, for its end.</summary>
    public long KeyDown(long nowMs)
    {
        var press = new Press(++lastId, SlotDevice.None, Barrel: false, Running(nowMs));
        live = press;
        return press.Id;
    }

    /// <summary>A press ended — lifted, cancelled, its capture lost, its key up, focus gone — after whatever click it made.</summary>
    public void PressEnded(long press)
    {
        if (live?.Id == press)
        {
            live = null;
        }
    }

    /// <summary>
    /// The slot's click (S8.6): the press under way is used up, and what the click does is answered — the slot's
    /// activation; the microphone's menu, for a pen tap made with the barrel button down (once, though it may also arrive as
    /// RightTapped); or nothing, because the guard runs, the press went down while it ran, or the barrel's menu is open.
    /// </summary>
    /// <param name="slot">What the slot is as the click arrives.</param>
    /// <param name="draft">The words in the field now: a Send or a Save after they changed — or something was staged — since the guard was armed is not guarded.</param>
    public SlotResponse Click(long nowMs, Slot slot, string draft)
    {
        var press = live;
        live = null;
        var response = ComposerButton.Respond(SlotPress.Click, press?.Device ?? SlotDevice.None, press?.Barrel ?? false, slot.IsMicrophone);
        return response switch
        {
            SlotResponse.Menu => MenuOnce(press),
            SlotResponse.Activate when Ignores(press?.BeganGuarded ?? false, nowMs, slot.Kind is SlotKind.Send or SlotKind.Save, draft) =>
                SlotResponse.Ignore,
            _ => response,
        };
    }

    /// <summary>
    /// A RightTapped on the slot (S8.6), which follows the press that made it: a mouse right-click or that press's barrel
    /// tap opens the microphone's menu — the barrel's once — and anything else, a touch or pen hold above all, nothing.
    /// </summary>
    public SlotResponse RightTapped(SlotDevice device, bool microphone)
    {
        var press = lastPointer;
        var response = ComposerButton.Respond(SlotPress.RightTapped, device, press?.Barrel ?? false, microphone);
        return response == SlotResponse.Menu && device == SlotDevice.Pen ? MenuOnce(press) : response;
    }

    /// <summary>
    /// Whether a key in the composer's field is ignored. A repeat always is: a key held down is one press, whose first went
    /// wherever focus was. Enter — a send or a save (<paramref name="sends"/>), the slot's activation in rows 2 to 5 — also
    /// waits out the guard, unless the words changed, or something was staged, since it was armed; Esc and the rest are not
    /// the slot's, and only repeat.
    /// </summary>
    public bool IgnoresFieldKey(long nowMs, bool repeat, bool sends, string draft) =>
        repeat || sends && Ignores(beganGuarded: false, nowMs, sends, draft);

    private bool Ignores(bool beganGuarded, long nowMs, bool sends, string draft) =>
        !(sends && armedOver is not null && (changedSinceArmed || !string.Equals(draft, armedOver, StringComparison.Ordinal)))
        && (beganGuarded || Running(nowMs));

    /// <summary>A barrel tap's menu opens once for its press, which may reach the slot as its click and as RightTapped both.</summary>
    private SlotResponse MenuOnce(Press? press)
    {
        if (press is { Barrel: true } barrel)
        {
            if (menuFor == barrel.Id)
            {
                return SlotResponse.Ignore;
            }
            menuFor = barrel.Id;
        }
        return SlotResponse.Menu;
    }
}

/// <summary>
/// Ctrl+Shift+R held down (S1.6, S8.6). A keyboard accelerator auto-repeats while its key is held — Microsoft's own
/// documentation says so, and that it cannot be changed — so a held chord would start a recording and then stop it, again and
/// again. A key held down is ONE press: the window notes each of the chord's key-downs as it tunnels in (PreviewKeyDown comes
/// before any accelerator) and swallows the repeats there; an invocation that still follows a repeat is ignored here.
/// </summary>
public sealed class ShortcutPresses
{
    /// <summary>An invocation this soon after a key-down is that key-down's: the same input, give or take a tick of the clock.</summary>
    public const long SameKeyMs = 50;

    private long seenAt;
    private bool seenRepeat;

    /// <summary>The chord's key went down, now — a repeat or not. Answers whether to swallow it: a repeat is.</summary>
    public bool KeyDown(long nowMs, bool repeat)
    {
        seenAt = nowMs;
        seenRepeat = repeat;
        return repeat;
    }

    /// <summary>Whether the accelerator, invoked now, is a fresh press — anything but the repeat just seen.</summary>
    public bool Fresh(long nowMs) => !(seenRepeat && nowMs >= seenAt && nowMs - seenAt <= SameKeyMs);
}
