using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>
/// Who may ask for the text of which recording, and what a failed ask says (docs/protocol.md, "Transcripts on
/// request", issue #62). One place, because the bubble that offers "Show text" and the rule the server enforces must
/// not drift apart: an offer the server then refuses is a button that lies.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE RULE, AS THE SERVER WRITES IT:</b> a member may ask about a recording on a message THEY sent, in any chat they
/// are in, direct chats included; about ANOTHER member's only in the family chat (its threads are the same chat), only
/// while the family's <c>ai_transcripts</c> is on; never about another member's in a direct chat, and never about the
/// assistant's own messages. The server adds one check this client cannot make — that the other SENDER has agreed to
/// the assistant — and answers it with the same <c>transcript_not_allowed</c>, which is drawn as "Not available for this
/// message."
/// </para>
/// <para>
/// <b>TWO FORMS, CHOSEN BY WHAT THE SERVER CAN READ</b> (<see cref="Form"/>): a voice note or audio file stored as a type
/// the provider reads, no bigger than <c>assistant.transcribe_max_bytes</c>, goes as the server's own copy; a video, an
/// Ogg file and an oversized recording go as sound this device makes from the file it holds
/// (<see cref="TranscriptSound"/>) — offered on a device that can make it, whatever the attachment's length: a
/// recording too long to fit is told so when the action is pressed, never hidden.
/// </para>
/// </remarks>
public static class TranscriptRules
{
    /// <summary>The provider's ceiling, the server's default, and never more (25 MiB).</summary>
    public const long CeilingBytes = 26_214_400;

    /// <summary>The stored types the server sends as they are — exactly its list, compared as it compares them.</summary>
    public static readonly IReadOnlyList<string> StoredTypes = ["audio/mp4", "audio/m4a", "audio/mpeg", "audio/wav"];

    /// <summary>
    /// Can this server turn a recording into text at all, and can the consent screen name who it goes to? A client
    /// that cannot name the processor offers no assistant, and a transcript IS the assistant's provider.
    /// </summary>
    public static bool IsAvailable(AssistantDto? assistant) =>
        assistant is { Transcribe: true } && AssistantConsent.IsAvailable(assistant.Processor);

    /// <summary>The most bytes the stored form may send here: what the server said, held to the ceiling.</summary>
    public static long MaxBytes(AssistantDto assistant)
    {
        ArgumentNullException.ThrowIfNull(assistant);
        return assistant.TranscribeMaxBytes is > 0 and var said ? Math.Min(said, CeilingBytes) : CeilingBytes;
    }

    /// <summary>
    /// The rule: may this reader ask about a recording on a message this sender sent, in a chat of this kind?
    /// </summary>
    /// <param name="chatKind">The chat's kind — a thread is its chat's, so a family thread is <c>family</c>.</param>
    /// <param name="senderId">Who sent the message the recording is on.</param>
    /// <param name="reader">Who is asking; 0 while unknown, which is nobody's own message.</param>
    /// <param name="assistantId">The assistant's account, whose messages are never asked about.</param>
    /// <param name="family">The family as the session holds it, for the owner's <c>ai_transcripts</c>.</param>
    public static bool MayAsk(string? chatKind, long senderId, long reader, long? assistantId, FamilyDto? family)
    {
        if (senderId <= 0 || (assistantId is { } assistant && senderId == assistant))
        {
            return false;
        }
        if (reader > 0 && senderId == reader)
        {
            // Your own voice, wherever you sent it: only your own consent, which the server asks for.
            return true;
        }
        // Somebody else's: the family chat only, and only while the owner allows it.
        return chatKind == "family" && family?.AiTranscripts == true;
    }

    /// <summary>
    /// Is this attachment one the server sends from its own stored copy — a recording of a type the provider reads, no
    /// bigger than the ceiling? A size the attachment does not state is not offered: the server would decide, and a
    /// button that may or may not work is the worse answer.
    /// </summary>
    public static bool SendsStoredCopy(AttachmentDto attachment, AssistantDto assistant)
    {
        ArgumentNullException.ThrowIfNull(attachment);
        return attachment.Kind == "audio"
               && attachment.Mime is { } mime
               && StoredTypes.Contains(mime, StringComparer.Ordinal)
               && attachment.Size is > 0 and var size
               && size <= MaxBytes(assistant);
    }

    /// <summary>
    /// Which form a recording's text is asked for in: the server's stored copy where it can send it; otherwise sound this
    /// device makes — a voice note, an audio file or a video, on a device that <paramref name="extracts"/>, WHATEVER its
    /// stated length (one too long to fit is told so at the press, <see cref="TranscriptSound.TooLong"/>); otherwise none.
    /// </summary>
    /// <param name="extracts">Whether this device can take a sound track out of a file it holds.</param>
    public static TranscriptForm Form(AttachmentDto attachment, AssistantDto assistant, bool extracts)
    {
        ArgumentNullException.ThrowIfNull(attachment);
        ArgumentNullException.ThrowIfNull(assistant);
        return SendsStoredCopy(attachment, assistant) ? TranscriptForm.Stored
            : extracts && attachment.Kind is "audio" or "video" ? TranscriptForm.Supplied
            : TranscriptForm.None;
    }

    /// <summary>
    /// The whole decision for one recording's "Show text": a server that transcribes, the rule, and a form to ask in
    /// (<see cref="Form"/>). Text this device already HOLDS is always shown — it was given to this member, and showing it
    /// sends nothing anywhere.
    /// </summary>
    /// <param name="extracts">Whether this device can make the sound itself; without it only the stored form is offered.</param>
    public static bool Offers(
        AssistantDto? assistant,
        FamilyDto? family,
        string? chatKind,
        long senderId,
        long reader,
        AttachmentDto attachment,
        bool held = false,
        bool extracts = false) =>
        held
        || (IsAvailable(assistant)
            && MayAsk(chatKind, senderId, reader, assistant!.UserId, family)
            && Form(attachment, assistant, extracts) != TranscriptForm.None);

    /// <summary>
    /// What a failed ask says under the player. The provider's refusal says so, and is final; a refusal of THIS
    /// recording or THIS member is "not available", and is final too; anything that says nothing about the request —
    /// no connection, the deadline, the provider's own failure — asks to try again.
    /// </summary>
    public static string FailureSentence(ApiError error, IStringCatalog say)
    {
        ArgumentNullException.ThrowIfNull(error);
        ArgumentNullException.ThrowIfNull(say);
        return MayRetry(error)
            ? say.Get("Couldn't get the text. Try again.")
            : error.Code switch
            {
                ErrorCodes.TranscriptRefused => say.Get("The assistant's provider refused this recording."),
                // What this device could not do with the file, in the words the other three clients use.
                TranscriptSound.TooLongCode => say.Get("This recording is too long to turn into text."),
                TranscriptSound.UnreadableCode => say.Get("Couldn't read the sound in this file."),
                _ => say.Get("Not available for this message."),
            };
    }

    /// <summary>
    /// Whether asking again could help: a transient failure, and the consent refusal that came back after a yes (the
    /// server had not caught up; the loop guard stopped this click, not the next one). Every other refusal is final.
    /// </summary>
    public static bool MayRetry(ApiError error)
    {
        ArgumentNullException.ThrowIfNull(error);
        return error.Code != ErrorCodes.TranscriptRefused
               && (error.Transient || error.Code == ErrorCodes.AssistantConsentRequired);
    }

    /// <summary>The text as drawn: what was said, or "No speech" for silence — an answer, not an error.</summary>
    public static string Shown(string? text, IStringCatalog say)
    {
        ArgumentNullException.ThrowIfNull(say);
        return string.IsNullOrWhiteSpace(text) ? say.Get("No speech") : text;
    }
}

/// <summary>Where one recording's text stands on this device.</summary>
public enum TranscriptPhase
{
    /// <summary>Folded away, or never asked for: the "Show text" action.</summary>
    Closed,

    /// <summary>Asked for and not answered yet: "Getting the text…".</summary>
    Asking,

    /// <summary>Shown under the player, with "Hide text".</summary>
    Open,

    /// <summary>The last ask failed; <see cref="TranscriptLook.Error"/> says why.</summary>
    Failed,
}

/// <summary>One recording's text as the bubble draws it.</summary>
/// <param name="Phase">Where it stands.</param>
/// <param name="Text">What was said, while <see cref="TranscriptPhase.Open"/>; <c>""</c> is silence.</param>
/// <param name="Error">Why the last ask failed, while <see cref="TranscriptPhase.Failed"/>.</param>
public sealed record TranscriptLook(TranscriptPhase Phase, string? Text = null, ApiError? Error = null)
{
    public static readonly TranscriptLook Closed = new(TranscriptPhase.Closed);

    /// <summary>The answer was silence, drawn as "No speech".</summary>
    public bool NoSpeech => Phase == TranscriptPhase.Open && string.IsNullOrWhiteSpace(Text);
}

/// <summary>
/// The text of recordings, asked for one at a time and kept on this device (docs/protocol.md, "Transcripts on
/// request"): the request with its own deadline, the consent question in <see cref="ConsentedAsk"/>'s order, and the
/// answer in <see cref="TranscriptStore"/> so that reopening a chat shows it without asking again.
/// </summary>
/// <remarks>
/// <para>
/// <b>WHAT IS SHOWN IS THIS MODEL'S; WHAT IS KEPT IS THE STORE'S.</b> A bubble is rebuilt on every redraw, so whether the
/// text is open, folded or still coming cannot live in the element; it lives here, per attachment, and the view draws
/// <see cref="Look"/> and redraws on <see cref="Changed"/>. Folding the text away keeps it: "Show text" again reads the
/// store and sends nothing.
/// </para>
/// <para>
/// <b>ONE ASK PER RECORDING AT A TIME.</b> A second click while the first is still asking — or still waiting on the
/// consent question — does nothing: the server would have made the second wait on the first call anyway.
/// </para>
/// <para>
/// <b>NOTHING HERE IS LOGGED.</b> The text of somebody's voice goes to the store and the screen and nowhere else.
/// </para>
/// <para>
/// <b>AN ASK BELONGS TO THE SESSION IT STARTED IN.</b> One model lives as long as the app, across sign-outs, and an ask
/// can take minutes. <see cref="Clear"/> — the session ended — cancels whatever is still out and makes it stale: an
/// answer that lands afterwards is neither kept nor drawn, because the sign-out has wiped the cache and the next account
/// would otherwise be shown the last one's words without asking, and an answer made from supplied sound must never reach
/// anybody but the member who asked.
/// </para>
/// </remarks>
public sealed class TranscriptModel(ApiClient api, TranscriptStore store, Func<DateTimeOffset>? clock = null)
{
    private readonly object gate = new();
    private readonly Dictionary<long, TranscriptLook> looks = [];
    private readonly HashSet<long> asking = [];
    private readonly Func<DateTimeOffset> now = clock ?? (() => DateTimeOffset.UtcNow);

    /// <summary>Which session an ask started in: <see cref="Clear"/> moves it on, and an ask from before writes nothing.</summary>
    private long session;

    /// <summary>Cancelled by <see cref="Clear"/>: whatever the ended session still had out stops.</summary>
    private CancellationTokenSource ended = new();

    /// <summary>A recording's look changed: the attachment id. May be raised off the UI thread.</summary>
    public event Action<long>? Changed;

    /// <summary>How this recording's text is drawn now.</summary>
    public TranscriptLook Look(long attachmentId)
    {
        lock (gate)
        {
            return looks.TryGetValue(attachmentId, out var look) ? look : TranscriptLook.Closed;
        }
    }

    /// <summary>Whether this device holds text for this recording — which it may show without asking anybody.</summary>
    public bool Holds(long attachmentId) => store.Find(attachmentId) is not null;

    /// <summary>
    /// "Show text": the kept text at once when this device holds it, and otherwise the question (when it is needed) and
    /// the request. Ends with the look set — open, failed, or closed again when the member said Not Now.
    /// </summary>
    /// <param name="asksFirst">Read when the click lands (<see cref="BackdropConsent.AsksFirst"/>).</param>
    /// <param name="ask">The consent question; true only on a yes the server has kept.</param>
    public Task ShowAsync(
        long chatId, long messageId, long attachmentId, Func<bool> asksFirst, Func<Task<bool>> ask,
        CancellationToken ct = default) =>
        ShowAsync(chatId, messageId, attachmentId, TranscriptForm.Stored, null, asksFirst, ask, ct);

    /// <summary>
    /// "Show text" in the form <see cref="TranscriptRules.Form"/> chose. <see cref="TranscriptForm.Supplied"/> makes the
    /// sound with <paramref name="sound"/> AFTER the consent question — nothing is downloaded or decoded for a member who
    /// says Not Now — and at most once per click, however many times the request goes. <see cref="TranscriptForm.Stored"/>
    /// asks for the server's copy, and falls back to the device's sound once if the server answers
    /// <c>not_transcribable</c> ("a client that sent no body may send the sound track instead").
    /// </summary>
    /// <remarks>
    /// An answer made from supplied sound is kept on THIS device only, marked as such (<see cref="KeptTranscript.Supplied"/>):
    /// the server never kept it, and it never replaces an answer from the stored copy.
    /// </remarks>
    /// <param name="sound">How this device makes the sound; null where it cannot, which leaves only the stored form.</param>
    public async Task ShowAsync(
        long chatId, long messageId, long attachmentId, TranscriptForm form, SoundSource? sound,
        Func<bool> asksFirst, Func<Task<bool>> ask, CancellationToken ct = default)
    {
        ArgumentNullException.ThrowIfNull(asksFirst);
        ArgumentNullException.ThrowIfNull(ask);
        long mine;
        CancellationToken sessionEnded;
        lock (gate)
        {
            if (!asking.Add(attachmentId))
            {
                return;
            }
            mine = session;
            sessionEnded = ended.Token;
        }
        using var linked = CancellationTokenSource.CreateLinkedTokenSource(ct, sessionEnded);
        var outer = ct;
        ct = linked.Token;
        try
        {
            if (store.Find(attachmentId) is { } kept)
            {
                Set(attachmentId, new TranscriptLook(TranscriptPhase.Open, kept.Text), mine);
                return;
            }
            if (form == TranscriptForm.None || (form == TranscriptForm.Supplied && sound is null))
            {
                // Nothing to ask with — the view offers no button then — so it is said, never sent and never thrown.
                Set(attachmentId, new TranscriptLook(TranscriptPhase.Failed, Error: TranscriptSound.Unavailable), mine);
                return;
            }
            ConsentedAsk.Outcome<TranscriptDto> outcome;
            var supplied = false;
            SuppliedSound? made = null;
            async Task<(TranscriptDto? Value, ApiError? Error)> SupplyAsync()
            {
                // Made once per click: the consent question may send the request twice, and the sound is the same.
                made ??= await MakeAsync(sound!, ct).ConfigureAwait(true);
                if (made.Bytes is not { } bytes)
                {
                    // Nothing outlives the click: the next one makes the sound again.
                    return (null, made.Error ?? TranscriptSound.Unreadable);
                }
                supplied = true;
                return await RequestAsync(chatId, messageId, attachmentId, bytes, ct).ConfigureAwait(true);
            }
            try
            {
                outcome = await ConsentedAsk.RunAsync(
                    asksFirst,
                    ask,
                    async () =>
                    {
                        Set(attachmentId, new TranscriptLook(TranscriptPhase.Asking), mine);
                        if (form == TranscriptForm.Supplied)
                        {
                            return await SupplyAsync().ConfigureAwait(true);
                        }
                        var stored = await RequestAsync(chatId, messageId, attachmentId, null, ct).ConfigureAwait(true);
                        return stored.Error?.Code == ErrorCodes.NotTranscribable && sound is not null
                            ? await SupplyAsync().ConfigureAwait(true)
                            : stored;
                    }).ConfigureAwait(true);
            }
            catch (Exception e) when (e is not OperationCanceledException || !outer.IsCancellationRequested)
            {
                // Said as the failure it is, never thrown at a click handler — and a session that ended is said as
                // nothing at all (Set drops it). The exception's TYPE only: no message text, which could carry anything.
                outcome = new ConsentedAsk.Outcome<TranscriptDto>(null, ApiError.Transport(e.GetType().Name), false);
            }
            if (outcome.Declined)
            {
                Set(attachmentId, TranscriptLook.Closed, mine);
            }
            else if (outcome.Value is { } transcript)
            {
                var text = transcript.Text ?? string.Empty;
                lock (gate)
                {
                    if (session != mine)
                    {
                        // Signed out while it was on its way: the cache was wiped, and this is not the next account's.
                        return;
                    }
                    store.Keep(new KeptTranscript(attachmentId, text, transcript.Language, supplied), now());
                }
                Set(attachmentId, new TranscriptLook(TranscriptPhase.Open, text), mine);
            }
            else
            {
                Set(attachmentId, new TranscriptLook(
                    TranscriptPhase.Failed, Error: outcome.Error ?? ApiError.Transport("no answer")), mine);
            }
        }
        finally
        {
            lock (gate)
            {
                // A session that ended took its asks with it, and the set may now hold the next session's.
                if (session == mine)
                {
                    asking.Remove(attachmentId);
                }
            }
        }
    }

    /// <summary>"Hide text": folded away, and still kept — showing it again asks nobody.</summary>
    public void Hide(long attachmentId)
    {
        lock (gate)
        {
            if (!looks.TryGetValue(attachmentId, out var look) || look.Phase != TranscriptPhase.Open)
            {
                return;
            }
        }
        Set(attachmentId, TranscriptLook.Closed);
    }

    /// <summary>
    /// Forget what is on screen — a session that ended — and stop whatever is still being asked: an answer that lands
    /// after this is not kept and not drawn. The store is wiped with the rest of the cache.
    /// </summary>
    public void Clear()
    {
        CancellationTokenSource stopping;
        lock (gate)
        {
            looks.Clear();
            asking.Clear();
            session++;
            stopping = ended;
            ended = new CancellationTokenSource();
        }
        // Not disposed: an ask still unwinding may hold a link to it, and cancelling is all it is for.
        stopping.Cancel();
    }

    /// <summary>
    /// The device's sound, held to the ceiling and to MPEG-4 — the server's own checks, made here so that a sound it would
    /// refuse is said before it is uploaded: over the ceiling is "too long", empty or not MPEG-4 "couldn't read". A stated
    /// length that cannot fit is "too long" before anything is downloaded. Whatever the maker throws is "couldn't read";
    /// only a cancel goes on as the cancel it is.
    /// </summary>
    private static async Task<SuppliedSound> MakeAsync(SoundSource sound, CancellationToken ct)
    {
        if (!TranscriptSound.CanFit(sound.DurationMs, sound.MaxBytes))
        {
            return SuppliedSound.Failed(TranscriptSound.TooLong);
        }
        SuppliedSound made;
        try
        {
            made = await sound.Make(ct).ConfigureAwait(true);
        }
        catch (Exception e) when (e is not OperationCanceledException || !ct.IsCancellationRequested)
        {
            return SuppliedSound.Failed(TranscriptSound.Unreadable);
        }
        if (made.Bytes is { } bytes)
        {
            if (bytes.LongLength == 0 || !MediaPrep.MatchesMagic(TranscriptSound.Mime, bytes))
            {
                return SuppliedSound.Failed(TranscriptSound.Unreadable);
            }
            if (!TranscriptSound.Fits(bytes.LongLength, sound.MaxBytes))
            {
                return SuppliedSound.Failed(TranscriptSound.TooLong);
            }
        }
        return made;
    }

    /// <param name="sound">The supplied sound, or null for the server's stored copy.</param>
    private async Task<(TranscriptDto? Value, ApiError? Error)> RequestAsync(
        long chatId, long messageId, long attachmentId, byte[]? sound, CancellationToken ct)
    {
        var answer = sound is null
            ? await api.Transcript(chatId, messageId, attachmentId, ct).ConfigureAwait(false)
            : await api.Transcript(chatId, messageId, attachmentId, sound, ct).ConfigureAwait(false);
        if (!answer.Ok)
        {
            return (null, answer.Error ?? ApiError.Transport("no answer"));
        }
        // A 2xx with no transcript in it is an answer this client cannot read, and terminal like every such answer.
        return answer.Value?.Transcript is { } transcript
            ? (transcript, null)
            : (null, new ApiError(ErrorCodes.Validation, "the answer carried no transcript", answer.Status));
    }

    private void Set(long attachmentId, TranscriptLook look) => Set(attachmentId, look, null);

    /// <param name="from">The session the ask started in; a look from a session that has since ended is dropped.</param>
    private void Set(long attachmentId, TranscriptLook look, long? from)
    {
        lock (gate)
        {
            if (from is { } started && started != session)
            {
                return;
            }
            looks[attachmentId] = look;
        }
        Changed?.Invoke(attachmentId);
    }
}
