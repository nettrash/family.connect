namespace FamilyConnect.Core;

/// <summary>
/// The words a call record is drawn with (docs/protocol.md, "Voice calls"), ported from
/// <c>fc_text::call_record</c> — itself ported from <c>CallRecordText.swift</c> — and pinned to it
/// by the oracle.
/// </summary>
/// <remarks>
/// The server writes an ENGLISH PLACEHOLDER into a record's body for clients that predate calls
/// ("Voice call", "Missed voice call"). A client that knows the <c>call</c> object never shows that
/// body: it draws its own wording from the outcome, the duration and which side of the call the
/// reader was on. One decision for the bubble, the chat-list preview and any notification, so the
/// three can never say different things about the same call.
/// </remarks>
public static class CallRecordText
{
    /// <summary>The four outcomes the wire names. Anything else is still a call.</summary>
    public static class Outcome
    {
        public const string Completed = "completed";
        public const string Missed = "missed";
        public const string Declined = "declined";
        public const string Failed = "failed";
    }

    /// <summary>Which sentence a record is drawn with — the decision, kept apart from the words.</summary>
    /// <remarks><c>mine</c> throughout is whether the READER placed the call: a record's sender is
    /// the caller.</remarks>
    public abstract record Line
    {
        /// <summary>Answered and hung up, with the duration when the record carries one.</summary>
        public sealed record Completed(bool Video, long? DurationSecs) : Line;

        /// <summary>I called and nobody answered — kind-neutral: that is the whole of the news.</summary>
        public sealed record NoAnswer : Line;

        /// <summary>They called and I never answered.</summary>
        public sealed record Missed(bool Video) : Line;

        /// <summary>They said no to my call.</summary>
        public sealed record DeclinedByThem(bool Video) : Line;

        /// <summary>I said no to theirs.</summary>
        public sealed record DeclinedByMe(bool Video) : Line;

        /// <summary>The media never came up, or died. Kind-neutral: a failure first.</summary>
        public sealed record Failed(long? DurationSecs) : Line;

        /// <summary>An outcome this build does not know: still a call, and never nothing.</summary>
        public sealed record Unknown(bool Video) : Line;
    }

    /// <summary>The sentence for a record — the original's switch, in the original's order.</summary>
    public static Line Decide(string outcome, long? durationSecs, bool video, bool mine) =>
        outcome switch
        {
            Outcome.Completed => new Line.Completed(video, durationSecs),
            Outcome.Missed when mine => new Line.NoAnswer(),
            Outcome.Missed => new Line.Missed(video),
            Outcome.Declined when mine => new Line.DeclinedByThem(video),
            Outcome.Declined => new Line.DeclinedByMe(video),
            Outcome.Failed => new Line.Failed(durationSecs),
            _ => new Line.Unknown(video),
        };

    /// <summary>The sentence, in the reader's language. Its keys ARE the apps' own strings.</summary>
    public static string Say(Line line, IStringCatalog? words = null)
    {
        var say = words ?? EnglishCatalog.Instance;
        return line switch
        {
            Line.Completed(var video, long secs) =>
                say.Format(video ? "Video call · %@" : "Voice call · %@", Duration(secs)),
            Line.Completed(var video, null) => say.Get(video ? "Video call" : "Voice call"),
            // A record whose outcome this build never heard of is still a call, and the render
            // floor says never draw nothing.
            Line.Unknown(var video) => say.Get(video ? "Video call" : "Voice call"),
            Line.NoAnswer => say.Get("No answer"),
            Line.Missed(false) => say.Get("Missed voice call"),
            Line.Missed(true) => say.Get("Missed video call"),
            // The apps say each of these WHOLE rather than a kind with a word after it: a
            // language that puts the verb first cannot be handed "%@ declined".
            Line.DeclinedByThem(false) => say.Get("Voice call declined"),
            Line.DeclinedByThem(true) => say.Get("Video call declined"),
            Line.DeclinedByMe(false) => say.Get("Declined voice call"),
            Line.DeclinedByMe(true) => say.Get("Declined video call"),
            Line.Failed(long secs) => say.Format("Call failed · %@", Duration(secs)),
            Line.Failed(null) => say.Get("Call failed"),
            _ => say.Get("Voice call"),
        };
    }

    /// <summary>The one line a record is drawn with, decided and said.</summary>
    public static string Label(
        string outcome, long? durationSecs, bool video, bool mine, IStringCatalog? words = null) =>
        Say(Decide(outcome, durationSecs, video, mine), words);

    /// <summary>
    /// <c>3:42</c>, or <c>1:03:42</c> past an hour — the shape the in-call timer counts in. A
    /// negative duration is drawn as none at all.
    /// </summary>
    public static string Duration(long seconds)
    {
        var whole = Math.Max(seconds, 0);
        var hours = whole / 3600;
        var minutes = whole % 3600 / 60;
        var secs = whole % 60;
        return hours > 0 ? $"{hours}:{minutes:00}:{secs:00}" : $"{minutes}:{secs:00}";
    }
}
