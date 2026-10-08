using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

/// <summary>What ended a recording that was running — the person, the five-minute limit, or something else.</summary>
public enum RecordingEnd
{
    /// <summary>The person's Stop — the row's Stop button, Esc, the slot in row 3, a second Ctrl+Shift+R.</summary>
    Stopped,

    /// <summary>The person's Send: the slot's arrow in row 2, which stops the recording and sends it (S2.5).</summary>
    Sent,

    /// <summary>Five minutes: the recording's own limit.</summary>
    Capped,

    /// <summary>The person's Cancel — after its question, where it asks one.</summary>
    Deleted,

    /// <summary>Leaving the chat: another chat, the rail's Board, Family or Settings, a notification opening another chat.</summary>
    LeftChat,

    /// <summary>A call rang, started or was placed — in any phase.</summary>
    Call,

    /// <summary>The window minimised, or hidden to the notification area by its close button with Keep running on.</summary>
    WindowHidden,

    /// <summary>The window really closing: Quit, or the close button with Keep running off.</summary>
    WindowClosed,

    /// <summary>The session locked or went away, the screen saver started, or the computer is going to sleep.</summary>
    SessionLocked,

    /// <summary>The recorder failed: the microphone gone or taken, the disk full.</summary>
    RecorderFailed,

    /// <summary>The session ended — signed out, expired, the family left, the account deleted.</summary>
    SignedOut,
}

/// <summary>Where a recording goes when it ends.</summary>
public enum RecordingFate
{
    /// <summary>Staged beside the composer, to listen to, caption and send — or throw away.</summary>
    Review,

    /// <summary>Kept on this device as a voice message that was not sent (S2.8), never sent by anything but its own row.</summary>
    Park,

    /// <summary>Gone.</summary>
    Discard,

    /// <summary>Sent now, through the outbox, with the reply the composer is primed with — the slot's Send and nothing else.</summary>
    Send,
}

/// <summary>A recording's end: where it goes, and what the composer says about it, if anything.</summary>
public sealed record RecordingOutcome(RecordingFate Fate, string? Sentence);

/// <summary>
/// What a recording takes from the composer as it is kept: the caption and the reply it goes with, and which of the two
/// the composer loses with it.
/// </summary>
public sealed record ComposerTake(string? Caption, long? ReplyTo, bool ClearsWords, bool ClearsReply)
{
    public static readonly ComposerTake Nothing = new(null, null, ClearsWords: false, ClearsReply: false);
}

/// <summary>
/// "Voice message not sent" (docs/audio-video-messages-2026-10-04.md, S2.8 and S4): a recording stopped by anything
/// but the person waits in its own row, with the reply it was recorded under and any caption, to be sent or deleted.
/// </summary>
/// <remarks>
/// <para>
/// <b>INTERRUPTIONS STOP AND KEEP, AND NEVER SEND OR DISCARD</b> (decision 12) — a call, the window hidden, minimised or
/// closed, the session locking or the computer sleeping, leaving the chat, the recorder failing. Under a second there is
/// nothing worth keeping, and it goes without a word. A sign-out takes everything recorded and not sent with it.
/// </para>
/// <para>
/// <b>FIVE MINUTES STOPS INTO REVIEW, NEVER INTO A SEND</b> (S2.5), and says so: nothing is ever sent by a limit running
/// out. <b>ONLY THE PERSON'S SEND SENDS</b> — the slot's arrow while a recording runs — and only a second or more of it:
/// a shorter one is "too short", never a blip in the family chat.
/// </para>
/// <para>
/// <b>A NOTE IN REVIEW BECOMES "NOT SENT" ONLY WHEN THE PERSON LEAVES THE CHAT</b> — or the window really closes, which
/// takes the chat with it — taking the words in the field as its caption and leaving the field empty, so a recording
/// nobody finished deciding about can never ride out with the next text. Everything else keeps it where it is.
/// </para>
/// </remarks>
public static class NotSent
{
    /// <summary>Where a recording that has just ended goes, and what is said about it.</summary>
    /// <param name="recorded">How long it ran, by the recorder's own clock.</param>
    /// <param name="bytes">How much could be read back — null when nothing could.</param>
    public static RecordingOutcome Ended(RecordingEnd why, TimeSpan recorded, int? bytes, IStringCatalog say)
    {
        var readable = bytes is > VoiceNotes.NothingAtOrBelowBytes;
        var worthKeeping = readable && recorded >= VoiceNotes.Shortest;
        // A second or more that cannot be read back is a recording lost, and the person is told rather than left to find
        // nothing; under a second there was nothing to lose.
        var lost = recorded >= VoiceNotes.Shortest ? say.Get("The recording stopped unexpectedly.") : null;
        return why switch
        {
            RecordingEnd.Deleted or RecordingEnd.SignedOut => new(RecordingFate.Discard, null),
            RecordingEnd.Stopped => worthKeeping
                ? new(RecordingFate.Review, null)
                : new(RecordingFate.Discard, lost ?? say.Get("That recording was too short.")),
            RecordingEnd.Sent => worthKeeping
                ? new(RecordingFate.Send, null)
                : new(RecordingFate.Discard, lost ?? say.Get("That recording was too short.")),
            RecordingEnd.Capped => worthKeeping
                ? new(RecordingFate.Review, say.Get("Recording stopped at five minutes."))
                : new(RecordingFate.Discard, lost),
            RecordingEnd.RecorderFailed => new(
                worthKeeping ? RecordingFate.Park : RecordingFate.Discard, say.Get("The recording stopped unexpectedly.")),
            _ => worthKeeping ? new(RecordingFate.Park, null) : new(RecordingFate.Discard, lost),
        };
    }

    /// <summary>
    /// Whether this ending also turns a voice note still in REVIEW into a not-sent one: leaving the chat, and the window
    /// really closing. A call, a hidden window, a locked session and a failed recorder leave it where it is.
    /// </summary>
    public static bool ParksReview(RecordingEnd why) => why is RecordingEnd.LeftChat or RecordingEnd.WindowClosed;

    /// <summary>
    /// What a RUNNING recording takes as an interruption parks it: the reply the composer is primed with — which leaves
    /// the composer only when nothing else in it would have carried it — and never the words, which stay the chat's own.
    /// An edit has no reply, and gives nothing.
    /// </summary>
    /// <param name="words">The composer's text.</param>
    /// <param name="replyTo">The reply it is primed with.</param>
    /// <param name="editing">An edit is open.</param>
    /// <param name="staged">Something is staged beside the words.</param>
    public static ComposerTake ForRecording(string words, long? replyTo, bool editing, bool staged)
    {
        if (editing || replyTo is null)
        {
            return ComposerTake.Nothing;
        }
        var nothingElse = string.IsNullOrWhiteSpace(words) && !staged;
        return new(null, replyTo, ClearsWords: false, ClearsReply: nothingElse);
    }

    /// <summary>
    /// What a voice note in REVIEW takes as the person leaves the chat (S2.8): the words in the field as its caption and
    /// the reply with them, and the composer is left empty — <c>takeComposer</c>'s caption and reply together. An edit's
    /// words are the edit's, never a caption.
    /// </summary>
    public static ComposerTake ForReview(string words, long? replyTo, bool editing)
    {
        if (editing)
        {
            return ComposerTake.Nothing;
        }
        var caption = string.IsNullOrWhiteSpace(words) ? null : words.TrimEnd();
        return new(caption, replyTo, ClearsWords: caption is not null, ClearsReply: replyTo is not null);
    }

    /// <summary>
    /// What a screen reader is told once a recording has gone where it went (S6, S2.5) — "Voice message sent", "Ready to
    /// review, 0:42", "Recording deleted" — and nothing for what an interruption kept or for an ending whose sentence the
    /// notice line already says and announces (<see cref="RecordingOutcome.Sentence"/>).
    /// </summary>
    /// <param name="went">Where it really went — a send that could not be written down lands in review instead.</param>
    /// <param name="durationMs">Its length, as it is kept.</param>
    public static string? Said(RecordingEnd why, RecordingFate went, int durationMs, IStringCatalog say) => (why, went) switch
    {
        (RecordingEnd.Sent, RecordingFate.Send) => say.Get("Voice message sent"),
        (RecordingEnd.Stopped, RecordingFate.Review) => say.Format("Ready to review, %@", MediaText.TimeLabel(durationMs / 1000.0)),
        (RecordingEnd.Deleted, RecordingFate.Discard) => say.Get("Recording deleted"),
        _ => null,
    };

    /// <summary>
    /// Whether this ending gives the keyboard back to the text field (S2.4): the person's own Send, Stop and Delete — so a
    /// second Enter cannot open the microphone again, and a note in review gets its caption where it is typed. An
    /// interruption leaves focus where it was.
    /// </summary>
    public static bool GivesFocusBack(RecordingEnd why) => why is RecordingEnd.Sent or RecordingEnd.Stopped or RecordingEnd.Deleted;

    /// <summary>
    /// What follows a not-sent row's own Send or ✕, whose button goes with its row (S2.8, S6): the keyboard goes back to the
    /// text field — never left nowhere, which loses a screen reader's place (WCAG 2.4.3) — and what happened is said: "Voice
    /// message sent", "Recording deleted".
    /// </summary>
    /// <param name="sent">The row's Send took it, rather than its ✕.</param>
    public static (bool FocusField, string Said) RowEnded(bool sent, int durationMs, IStringCatalog say) => sent
        ? (GivesFocusBack(RecordingEnd.Sent), Said(RecordingEnd.Sent, RecordingFate.Send, durationMs, say)!)
        : (GivesFocusBack(RecordingEnd.Deleted), Said(RecordingEnd.Deleted, RecordingFate.Discard, durationMs, say)!);

    /// <summary>"Voice message not sent · 0:42" — the row's own line.</summary>
    public static string Line(int durationMs, IStringCatalog say) =>
        say.Format("Voice message not sent · %@", MediaText.TimeLabel(durationMs / 1000.0));
}
