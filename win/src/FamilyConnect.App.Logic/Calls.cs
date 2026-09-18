using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>Where a call has got to.</summary>
public enum CallStage
{
    /// <summary>Placed; the server has not said it reached them yet.</summary>
    Dialling,

    /// <summary>Ringing on their side.</summary>
    Ringing,

    /// <summary>Somebody is calling, and this device is one of those ringing.</summary>
    Incoming,

    /// <summary>Answered, and the media is coming up.</summary>
    Connecting,

    /// <summary>The media is up.</summary>
    Talking,

    /// <summary>Over. The panel says why for a moment, then goes.</summary>
    Ended,
}

/// <summary>A call as the screen needs it: what a view compares and redraws from.</summary>
/// <param name="Video">Decided when it was placed and fixed for its life: cameras toggle, the kind does not.</param>
/// <param name="Camera">This device's camera, on — never true on a voice call.</param>
/// <param name="Taken">Answered — here, or for the caller by the far side: what makes ending it a <c>hangup</c>.</param>
/// <param name="AnsweredAt">When the media came up, by this device's clock: the on-screen duration counts from here, so setting up is not billed to the conversation.</param>
/// <param name="EndedReason">Why it ended, while the panel says so: a protocol reason, or this client's own (<c>microphone_denied</c>).</param>
public sealed record CallState(
    string CallId,
    long ChatId,
    long PeerUserId,
    bool Outgoing,
    bool Video,
    CallStage Stage,
    bool Muted = false,
    bool Camera = false,
    bool Taken = false,
    DateTimeOffset? AnsweredAt = null,
    string? EndedReason = null);

/// <summary>What the media engine was given: nothing, the microphone, or the microphone and the camera.</summary>
public enum MediaGrant
{
    None,
    Microphone,
    MicrophoneAndCamera,
}

/// <summary>The two tones: somebody calling this device, and the ring the caller hears while it rings.</summary>
public enum RingTone
{
    Incoming,
    Ringback,
}

/// <summary>
/// What a call's media runs on — a peer connection, the microphone and camera, and the tones. On Windows a browser
/// engine's WebRTC; in a test, a fake. Everything here is one call's worth, and <see cref="Close"/> gives it all back.
/// </summary>
public interface ICallMedia
{
    /// <summary>
    /// The microphone, and the camera on a video call — asked again for the microphone alone when the camera is refused
    /// or missing, because a camera is never a reason to miss a call (docs/protocol.md, "Video").
    /// </summary>
    Task<MediaGrant> OpenAsync(bool video);

    /// <summary>A peer connection with these servers and what was opened; <paramref name="receiveVideoOnly"/> negotiates the far side's picture without a camera of our own.</summary>
    Task<bool> ConnectAsync(IReadOnlyList<IceServerDto> servers, bool receiveVideoOnly);

    /// <summary>An offer, set as the local description: its SDP, or null.</summary>
    Task<string?> CreateOfferAsync();

    Task<bool> SetRemoteAsync(bool isOffer, string sdp);

    /// <summary>An answer to the remote offer, set as the local description: its SDP, or null.</summary>
    Task<string?> CreateAnswerAsync();

    Task AddCandidateAsync(IceCandidate candidate);

    bool IsConnected { get; }

    void SetMuted(bool muted);

    void SetCamera(bool on);

    void Ring(RingTone tone);

    void StopRinging();

    /// <summary>Everything given back: tracks stopped — the microphone light goes out — and the connection closed.</summary>
    void Close();

    event Action<IceCandidate>? CandidateGathered;

    event Action? Connected;

    event Action? Failed;

    /// <summary>A stream appeared or went: a view attaches it again.</summary>
    event Action? MediaChanged;
}

/// <summary>A call's words — web <c>calls.rs</c> and <c>views/call.rs</c>, ios CallRecordText.</summary>
public static class CallText
{
    /// <summary>A refused call frame, as a reason to end on.</summary>
    public static string RefusalReason(string code) => code switch
    {
        "call_busy" or "peer_busy" => "busy",
        "peer_unreachable" => "unreachable",
        "video_calls_disabled" => "video_calls_disabled",
        // Not collapsed into one word: a person's own block is theirs to undo, and a server with no calls is a fact about the server.
        "blocked" => "blocked",
        "calls_disabled" => "calls_disabled",
        _ => "failed",
    };

    /// <summary>
    /// How ending a call is said for its stage (ios <c>performHangUp</c>): answered is a <c>hangup</c>, the caller giving up
    /// while it rings is a <c>cancel</c>, and the callee saying no is a <c>decline</c>.
    /// </summary>
    public static string EndReason(CallState call) => (call.Stage, call.Outgoing) switch
    {
        (CallStage.Incoming, _) => "decline",
        (CallStage.Dialling or CallStage.Ringing, true) => "cancel",
        (CallStage.Dialling or CallStage.Ringing, false) => "decline",
        _ => "hangup",
    };

    /// <summary>What an app on its way out owes the other side: a callee going is a refusal, never a missed call.</summary>
    public static string UnloadReason(CallState call) => call.Taken ? "hangup" : call.Outgoing ? "cancel" : "decline";

    /// <summary>Why the call on screen ended, in the apps' words. A call this person ended themselves says the plain thing.</summary>
    public static string EndedLine(string reason, CallState call, IStringCatalog say) => reason switch
    {
        "decline" when call.Outgoing => say.Get("Declined"),
        "timeout" when call.Outgoing => say.Get("No answer"),
        "timeout" => CallRecordText.Label("missed", null, call.Video, mine: false, say),
        "answered_elsewhere" => say.Get("Answered on another device"),
        "busy" => say.Get("Busy"),
        "unreachable" or "unavailable" => say.Get("Unavailable"),
        "blocked" => say.Get("You've blocked them."),
        "calls_disabled" => say.Get("Calls are off on this server."),
        "video_calls_disabled" => say.Get("Video calls are off on this server."),
        "microphone_denied" => say.Get("Microphone access is needed for calls."),
        "failed" => say.Get("Call failed"),
        _ => say.Get("Call ended"),
    };

    /// <summary>Where the call is, in words — and once it is up, how long it has been.</summary>
    public static string StatusLine(CallState call, DateTimeOffset now, IStringCatalog say) => call.Stage switch
    {
        CallStage.Dialling => say.Get("Calling…"),
        CallStage.Ringing => say.Get("Ringing…"),
        CallStage.Incoming => call.Video ? say.Get("Incoming video call") : say.Get("Incoming call"),
        CallStage.Connecting => say.Get("Connecting…"),
        CallStage.Talking => CallRecordText.Duration(call.AnsweredAt is { } answered ? (long)(now - answered).TotalSeconds : 0),
        _ => EndedLine(call.EndedReason ?? "hangup", call, say),
    };
}

/// <summary>
/// One call's signalling and state (docs/protocol.md, "Voice calls"; web <c>calls.rs</c>, ios CallManager): placing,
/// ringing, answering, the candidates, and the one way a call ends.
/// </summary>
/// <remarks>
/// <para>
/// <b>A FRAME IS APPLIED ONLY TO THE CALL THIS DEVICE HOLDS</b>, and every other is ignored in silence — the rule that
/// lets one person be signed in on several devices without the server tracking which is doing what.
/// </para>
/// <para>
/// <b>REMOTE CANDIDATES BEFORE THE REMOTE DESCRIPTION ARE HELD</b> (the server's own 64): a replayed offer is followed by
/// candidates gathered while a device woke, and a live relay promises no order. And <b>LOCAL CANDIDATES WAIT FOR THIS
/// DEVICE'S OFFER OR ANSWER</b>: the server knows no call before its offer, and relays a callee's candidates only once it
/// has answered — a browser engine in another process posts both, and nothing promises which lands first here.
/// </para>
/// <para>
/// <b>EVERY AWAIT IN SETTING A CALL UP IS FOLLOWED BY "STILL ON IT?"</b>: a call cancelled while the microphone was being
/// asked for must not go on to be placed, which would leave a live microphone behind a panel that has gone.
/// </para>
/// <para>Driven from one thread — the window's — as frames, media events and clicks all arrive there.</para>
/// </remarks>
public sealed class CallEngine
{
    /// <summary>Backstops for a <c>call_end</c> that never came — later than the server's 45 s, so its reason is normally the one shown. Neither says a word.</summary>
    public static readonly TimeSpan OutgoingRingGuard = TimeSpan.FromSeconds(90);

    public static readonly TimeSpan IncomingRingGuard = TimeSpan.FromSeconds(60);

    /// <summary>An answered call that never came up — this one DOES say <c>failed</c>, as nothing on the server ends it before a minute of silence.</summary>
    public static readonly TimeSpan AnswerGuard = TimeSpan.FromSeconds(30);

    /// <summary>How long a call that ended stays on screen saying why.</summary>
    public static readonly TimeSpan EndedLinger = TimeSpan.FromSeconds(2);

    /// <summary>A frame worth a few tries — ten, half a second apart (ios): the socket may be a moment from opening.</summary>
    public static readonly TimeSpan SayEvery = TimeSpan.FromMilliseconds(500);

    public const int SayAttempts = 10;

    /// <summary>How many just-ended calls are remembered, so a replayed offer for one cannot ring again.</summary>
    public const int Remembered = 16;

    /// <summary>How many candidates are held waiting — the server's own buffer.</summary>
    public const int HeldCandidates = 64;

    private readonly IFrameSender socket;
    private readonly Func<CancellationToken, Task<ApiResult<IceServersResponse>>> iceServers;
    private readonly ICallMedia media;
    private readonly Func<TimeSpan, CancellationToken, Task> delay;
    private readonly Func<DateTimeOffset> now;
    private readonly Func<string> newCallId;
    private readonly List<IceCandidate> early = [];
    private readonly List<IceCandidate> unsent = [];
    private readonly Queue<string> recentlyEnded = new();
    private string? offerSdp;
    private bool remoteSet;
    private bool described;
    private CancellationTokenSource? guard;
    private CancellationTokenSource? linger;
    private readonly Action onConnected;
    private readonly Action onFailed;
    private readonly Action onMediaChanged;

    public CallEngine(
        IFrameSender socket,
        Func<CancellationToken, Task<ApiResult<IceServersResponse>>> iceServers,
        ICallMedia media,
        Func<TimeSpan, CancellationToken, Task>? delay = null,
        Func<DateTimeOffset>? now = null,
        Func<string>? newCallId = null)
    {
        this.socket = socket;
        this.iceServers = iceServers;
        this.media = media;
        this.delay = delay ?? Task.Delay;
        this.now = now ?? (() => DateTimeOffset.UtcNow);
        this.newCallId = newCallId ?? (() => Guid.NewGuid().ToString());
        onConnected = () =>
        {
            if (Call is { } call)
            {
                Up(call.CallId);
            }
        };
        // A call that dies is reported by the client, from exactly here: the server deliberately does not end one over a
        // dropped socket (docs/protocol.md, "The sequence").
        onFailed = () =>
        {
            if (Call is { Stage: not CallStage.Ended } call)
            {
                Finish(call.CallId, "failed", say: "failed");
            }
        };
        onMediaChanged = () => Changed?.Invoke();
        media.CandidateGathered += Gathered;
        media.Connected += onConnected;
        media.Failed += onFailed;
        media.MediaChanged += onMediaChanged;
    }

    /// <summary>
    /// Done with the media: the window has moved to another server, and the media it keeps is another engine's now. Nothing
    /// the media says afterwards reaches this one.
    /// </summary>
    public void Detach()
    {
        media.CandidateGathered -= Gathered;
        media.Connected -= onConnected;
        media.Failed -= onFailed;
        media.MediaChanged -= onMediaChanged;
    }

    /// <summary>The call on screen, or null.</summary>
    public CallState? Call { get; private set; }

    /// <summary>On a call that is not over: a second one can be neither placed nor rung.</summary>
    public bool Busy => Call is { Stage: not CallStage.Ended };

    public event Action? Changed;

    /// <summary>One call frame, or a refusal naming a call.</summary>
    public void Hear(ServerFrame frame)
    {
        switch (frame)
        {
            case ServerFrame.CallOffer offer:
                Offered(offer.CallId, offer.ChatId, offer.FromUserId, offer.Sdp, offer.Video);
                break;
            case ServerFrame.CallRinging ringing:
                Ringing(ringing.CallId);
                break;
            case ServerFrame.CallAnswer answer:
                _ = AnsweredAsync(answer.CallId, answer.Sdp);
                break;
            case ServerFrame.CallIce ice:
                _ = CandidateAsync(ice.CallId, ice.Candidate);
                break;
            case ServerFrame.CallEnd end:
                // The server's reason, shown; nothing said back.
                Finish(end.CallId, end.Reason, say: null);
                break;
            case ServerFrame.Error { CallId: { } callId } error:
                // It may land AFTER the call_end the server sent first, and then it says better what happened.
                Finish(callId, CallText.RefusalReason(error.Code), say: null);
                break;
        }
    }

    /// <summary>
    /// Place a call in a direct chat, to its other member — in the protocol's order: the microphone first (a call nobody
    /// can be heard on is not worth ringing), then the servers, then the offer the moment it is described.
    /// </summary>
    public async Task PlaceAsync(long chatId, long peerUserId, bool video)
    {
        if (Call is { Stage: CallStage.Ended })
        {
            Clear();
        }
        if (Call is not null)
        {
            return;
        }
        var callId = newCallId();
        Set(new CallState(callId, chatId, peerUserId, Outgoing: true, video, CallStage.Dialling, Camera: video));
        var grant = await media.OpenAsync(video);
        if (!Still(callId))
        {
            media.Close();
            return;
        }
        if (grant == MediaGrant.None)
        {
            Finish(callId, "microphone_denied", say: null);
            return;
        }
        var servers = await iceServers(CancellationToken.None);
        if (!Still(callId))
        {
            media.Close();
            return;
        }
        if (!servers.Ok || servers.Value is null)
        {
            // The server never heard of this call: there is nothing to tell it.
            Finish(callId, CallText.RefusalReason(servers.Error?.Code ?? string.Empty), say: null);
            return;
        }
        var cameraless = video && grant != MediaGrant.MicrophoneAndCamera;
        var connected = await media.ConnectAsync(servers.Value.IceServers ?? [], cameraless);
        if (!Still(callId))
        {
            media.Close();
            return;
        }
        if (!connected)
        {
            Finish(callId, "failed", say: null);
            return;
        }
        if (cameraless)
        {
            Change(callId, call => call with { Camera = false });
        }
        var sdp = await media.CreateOfferAsync();
        if (!Still(callId))
        {
            media.Close();
            return;
        }
        if (sdp is null || !await socket.TrySend(ClientFrames.CallOffer(callId, chatId, sdp, video)))
        {
            // Placed but never offered — a call that visibly did not happen, to the person looking at it.
            Finish(callId, "failed", say: null);
            return;
        }
        Described(callId);
        RingGuard(callId, outgoing: true);
    }

    /// <summary>Answer the call this device is ringing with: the microphone, the servers, the offer in, the answer out.</summary>
    public async Task AnswerAsync()
    {
        if (Call is not { Stage: CallStage.Incoming } call || offerSdp is not { } offer)
        {
            return;
        }
        var callId = call.CallId;
        var grant = await media.OpenAsync(call.Video);
        if (!Still(callId))
        {
            media.Close();
            return;
        }
        if (grant == MediaGrant.None)
        {
            // A refusal on the wire, and the reason on screen is the one that can be acted on: leaving the caller ringing
            // at somebody who cannot speak is worse.
            Finish(callId, "microphone_denied", say: "decline");
            return;
        }
        var servers = await iceServers(CancellationToken.None);
        if (!Still(callId))
        {
            media.Close();
            return;
        }
        IReadOnlyList<IceServerDto> list = [];
        if (servers is { Ok: true, Value: { } answered })
        {
            list = answered.IceServers ?? [];
        }
        else if (servers.Error is { Transient: false } refusal)
        {
            // A refusal the server MEANS is the end of it. One it merely could not give leaves the candidates this
            // device finds on its own, which are enough on one network.
            Finish(callId, CallText.RefusalReason(refusal.Code), say: "decline");
            return;
        }
        var cameraless = call.Video && grant != MediaGrant.MicrophoneAndCamera;
        var connected = await media.ConnectAsync(list, cameraless);
        if (!Still(callId))
        {
            media.Close();
            return;
        }
        if (!connected)
        {
            HangUp("failed");
            return;
        }
        if (cameraless)
        {
            Change(callId, held => held with { Camera = false });
        }
        var offerIn = await media.SetRemoteAsync(isOffer: true, offer);
        if (!Still(callId))
        {
            return;
        }
        if (!offerIn)
        {
            HangUp("failed");
            return;
        }
        await RemoteIsSetAsync();
        var sdp = await media.CreateAnswerAsync();
        if (!Still(callId))
        {
            return;
        }
        if (sdp is null)
        {
            HangUp("failed");
            return;
        }
        // The answer is what takes the call, so it is worth a few tries.
        _ = SendAnswerAsync(callId, sdp);
        media.StopRinging();
        Change(callId, held => held with { Stage = CallStage.Connecting, Taken = true });
        ArmAnswerGuard(callId);
        CatchUp(callId);
    }

    /// <summary>Refuse it — there is no call_decline frame: a <c>call_end</c> that ends it on every one of their devices.</summary>
    public void Decline() => HangUp("decline");

    /// <summary>End it the way its stage means. A call already over is left alone.</summary>
    public void End()
    {
        if (Call is { Stage: not CallStage.Ended } call)
        {
            HangUp(CallText.EndReason(call));
        }
    }

    /// <summary>Say <c>call_end</c> with this reason, and put the call away.</summary>
    public void HangUp(string reason)
    {
        if (Call is { } call)
        {
            Finish(call.CallId, reason, say: reason);
        }
    }

    /// <summary>The app is going: the reason its stage means, said once — nothing runs after this to try again.</summary>
    public async Task LeaveAsync()
    {
        if (Call is not { Stage: not CallStage.Ended } call)
        {
            return;
        }
        await socket.TrySend(ClientFrames.CallEnd(call.CallId, CallText.UnloadReason(call)));
        Teardown();
    }

    public void ToggleMute()
    {
        if (Call is not { } call)
        {
            return;
        }
        var muted = !call.Muted;
        Set(call with { Muted = muted });
        media.SetMuted(muted);
    }

    /// <summary>The camera, off and on: a track disabled, nothing renegotiated — the kind never changes.</summary>
    public void ToggleCamera()
    {
        if (Call is not { Video: true } call)
        {
            return;
        }
        var on = !call.Camera;
        Set(call with { Camera = on });
        media.SetCamera(on);
    }

    // ---- frames ---------------------------------------------------------------------------------

    /// <summary>
    /// Somebody is calling. Every connection the callee has gets this, so a second copy of one held is the duplicate it
    /// is; and an offer for a call that just ended here crossed its own <c>call_end</c>.
    /// </summary>
    private void Offered(string callId, long chatId, long fromUserId, string sdp, bool video)
    {
        if (recentlyEnded.Contains(callId))
        {
            return;
        }
        // A call still saying why it ended is not a call: the next one takes its place at once.
        if (Call is { Stage: CallStage.Ended })
        {
            Clear();
        }
        // Already ringing with it, or on a call: their other devices may still take this one, and ending it here would
        // end it for all of them.
        if (Call is not null)
        {
            return;
        }
        Teardown();
        offerSdp = sdp;
        media.Ring(RingTone.Incoming);
        Set(new CallState(callId, chatId, fromUserId, Outgoing: false, video, CallStage.Incoming, Camera: video));
        RingGuard(callId, outgoing: false);
    }

    /// <summary>The server reached them: it rings on their side now, which is when the caller hears the tone.</summary>
    private void Ringing(string callId)
    {
        if (Call is { Stage: CallStage.Dialling } call && call.CallId == callId)
        {
            Set(call with { Stage = CallStage.Ringing });
            media.Ring(RingTone.Ringback);
        }
    }

    /// <summary>They took it, on one of their devices.</summary>
    private async Task AnsweredAsync(string callId, string sdp)
    {
        if (Call is not { Outgoing: true, Taken: false, Stage: not CallStage.Ended } call || call.CallId != callId)
        {
            return;
        }
        var set = await media.SetRemoteAsync(isOffer: false, sdp);
        if (!Still(callId))
        {
            return;
        }
        if (!set)
        {
            Finish(callId, "failed", say: "failed");
            return;
        }
        await RemoteIsSetAsync();
        media.StopRinging();
        Change(callId, held => held with { Stage = CallStage.Connecting, Taken = true });
        // The caller owes itself the callee's guard: nothing ends an answered call that never comes up for a minute.
        ArmAnswerGuard(callId);
        CatchUp(callId);
    }

    /// <summary>A relayed candidate: in now if the remote description is, and held until it is if not.</summary>
    private async Task CandidateAsync(string callId, IceCandidate candidate)
    {
        if (Call?.CallId != callId)
        {
            return;
        }
        if (remoteSet)
        {
            await media.AddCandidateAsync(candidate);
            return;
        }
        if (early.Count < HeldCandidates)
        {
            early.Add(candidate);
        }
    }

    // ---- the call's own machinery ----------------------------------------------------------------

    /// <summary>
    /// The one way a call ends here: the media given back, the reason left on screen for a moment, and the call remembered
    /// just long enough that a frame still in flight for it cannot start it again. <paramref name="say"/> is for the
    /// SERVER — only the protocol's four reasons — and null when the server said so first or never heard of the call.
    /// </summary>
    private void Finish(string callId, string shown, string? say)
    {
        if (Call is not { } call || call.CallId != callId)
        {
            return;
        }
        if (say is not null)
        {
            _ = SayRetryingAsync(ClientFrames.CallEnd(callId, say));
        }
        Teardown();
        recentlyEnded.Enqueue(callId);
        while (recentlyEnded.Count > Remembered)
        {
            recentlyEnded.Dequeue();
        }
        Set(call with { Stage = CallStage.Ended, EndedReason = shown });
        Linger(callId);
    }

    private void Teardown()
    {
        guard?.Cancel();
        guard = null;
        media.StopRinging();
        media.Close();
        early.Clear();
        unsent.Clear();
        offerSdp = null;
        remoteSet = false;
        described = false;
    }

    private void Clear()
    {
        linger?.Cancel();
        linger = null;
        Call = null;
    }

    private void Set(CallState call)
    {
        Call = call;
        Changed?.Invoke();
    }

    private void Change(string callId, Func<CallState, CallState> change)
    {
        if (Call is { } call && call.CallId == callId)
        {
            Set(change(call));
        }
    }

    /// <summary>Still ON this call — not merely still saying goodbye to it.</summary>
    private bool Still(string callId) => Call is { Stage: not CallStage.Ended } call && call.CallId == callId;

    /// <summary>A candidate this device found: after its offer or answer, or held until that has gone.</summary>
    private void Gathered(IceCandidate candidate)
    {
        if (Call is not { Stage: not CallStage.Ended } call)
        {
            return;
        }
        if (!described)
        {
            if (unsent.Count < HeldCandidates)
            {
                unsent.Add(candidate);
            }
            return;
        }
        _ = socket.TrySend(ClientFrames.CallIce(call.CallId, candidate));
    }

    /// <summary>This device's offer or answer has gone: what was found before it follows it now, in order.</summary>
    private void Described(string callId)
    {
        described = true;
        foreach (var candidate in unsent)
        {
            _ = socket.TrySend(ClientFrames.CallIce(callId, candidate));
        }
        unsent.Clear();
    }

    private async Task SendAnswerAsync(string callId, string sdp)
    {
        if (await SayRetryingAsync(ClientFrames.CallAnswer(callId, sdp)) && Still(callId))
        {
            Described(callId);
        }
    }

    private async Task<bool> SayRetryingAsync(string frame)
    {
        if (await socket.TrySend(frame))
        {
            return true;
        }
        for (var attempt = 0; attempt < SayAttempts; attempt++)
        {
            await delay(SayEvery, CancellationToken.None);
            if (await socket.TrySend(frame))
            {
                return true;
            }
        }
        return false;
    }

    /// <summary>The remote description is in: everything held back goes in now, in the order it came.</summary>
    private async Task RemoteIsSetAsync()
    {
        remoteSet = true;
        var held = early.ToArray();
        early.Clear();
        foreach (var candidate in held)
        {
            await media.AddCandidateAsync(candidate);
        }
    }

    /// <summary>The media is up: the call is a conversation, and the clock starts here rather than at the answer.</summary>
    private void Up(string callId)
    {
        if (Call is { Stage: CallStage.Connecting } call && call.CallId == callId)
        {
            Set(call with { Stage = CallStage.Talking, AnsweredAt = now() });
        }
    }

    /// <summary>The media may already be up — its event can land while held candidates were going in, before there was a Connecting stage to promote.</summary>
    private void CatchUp(string callId)
    {
        if (media.IsConnected)
        {
            Up(callId);
        }
    }

    private void RingGuard(string callId, bool outgoing) =>
        Arm(outgoing ? OutgoingRingGuard : IncomingRingGuard, () =>
        {
            if (Call is { Taken: false } call && call.CallId == callId)
            {
                Finish(callId, "timeout", say: null);
            }
        });

    private void ArmAnswerGuard(string callId) =>
        Arm(AnswerGuard, () =>
        {
            if (Call is { Stage: CallStage.Connecting } call && call.CallId == callId)
            {
                Finish(callId, "failed", say: "failed");
            }
        });

    private void Arm(TimeSpan after, Action then)
    {
        guard?.Cancel();
        var armed = new CancellationTokenSource();
        guard = armed;
        _ = WaitAsync(after, armed.Token, then);
    }

    /// <summary>The panel goes after a moment, unless another call has started meanwhile.</summary>
    private void Linger(string callId)
    {
        linger?.Cancel();
        var waiting = new CancellationTokenSource();
        linger = waiting;
        _ = WaitAsync(EndedLinger, waiting.Token, () =>
        {
            if (Call is { Stage: CallStage.Ended } call && call.CallId == callId)
            {
                Call = null;
                Changed?.Invoke();
            }
        });
    }

    private async Task WaitAsync(TimeSpan after, CancellationToken token, Action then)
    {
        try
        {
            await delay(after, token);
        }
        catch (OperationCanceledException)
        {
            return;
        }
        if (!token.IsCancellationRequested)
        {
            then();
        }
    }
}
