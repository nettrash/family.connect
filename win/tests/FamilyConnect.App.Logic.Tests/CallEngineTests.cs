using System.Text.Json.Nodes;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// One call's signalling and state — web <c>calls.rs</c>'s own tests, and the ones its browser made unnecessary: the
/// engine's media is another process here, and its guards run on a clock a test can move.
/// </summary>
/// <remarks>
/// Every test body runs inside <see cref="Task.Run(Func{Task})"/>: there is no synchronization context there, so a delay
/// the clock completes resumes the engine at once, on the test's own thread, and the assertion after it sees the result.
/// </remarks>
public sealed class CallEngineTests
{
    private sealed class Wire : IFrameSender
    {
        public bool IsConnected { get; set; } = true;

        /// <summary>How many sends to refuse before the socket takes frames again.</summary>
        public int Refusals { get; set; }

        public List<string> Sent { get; } = [];

        public Task<bool> TrySend(string frame, CancellationToken ct = default)
        {
            if (Refusals > 0)
            {
                Refusals--;
                return Task.FromResult(false);
            }
            Sent.Add(frame);
            return Task.FromResult(true);
        }

        public event Action<ServerFrame>? Frame;

        public void Unused() => Frame?.Invoke(new ServerFrame.Pong());
    }

    private sealed class Media : ICallMedia
    {
        public MediaGrant Grant { get; set; } = MediaGrant.Microphone;

        /// <summary>Holds the microphone question open, as a person reading the prompt does.</summary>
        public TaskCompletionSource<MediaGrant>? Asking { get; set; }

        public bool Connects { get; set; } = true;

        public string? Offer { get; set; } = "v=offer";

        public string? Answer { get; set; } = "v=answer";

        public bool AcceptsRemote { get; set; } = true;

        /// <summary>Runs while the offer is being described — where a browser engine starts finding candidates.</summary>
        public Action? WhileDescribing { get; set; }

        public bool IsConnected { get; set; }

        public List<string> Log { get; } = [];

        public List<IceCandidate> Added { get; } = [];

        public Task<MediaGrant> OpenAsync(bool video)
        {
            Log.Add(video ? "open video" : "open");
            return Asking?.Task ?? Task.FromResult(Grant);
        }

        public Task<bool> ConnectAsync(IReadOnlyList<IceServerDto> servers, bool receiveVideoOnly)
        {
            Log.Add($"connect {servers.Count}{(receiveVideoOnly ? " receive-only" : string.Empty)}");
            return Task.FromResult(Connects);
        }

        public Task<string?> CreateOfferAsync()
        {
            Log.Add("offer");
            WhileDescribing?.Invoke();
            return Task.FromResult(Offer);
        }

        public Task<bool> SetRemoteAsync(bool isOffer, string sdp)
        {
            Log.Add($"remote {(isOffer ? "offer" : "answer")} {sdp}");
            return Task.FromResult(AcceptsRemote);
        }

        public Task<string?> CreateAnswerAsync()
        {
            Log.Add("answer");
            WhileDescribing?.Invoke();
            return Task.FromResult(Answer);
        }

        public Task AddCandidateAsync(IceCandidate candidate)
        {
            Added.Add(candidate);
            return Task.CompletedTask;
        }

        public void SetMuted(bool muted) => Log.Add($"muted {muted}");

        public void SetCamera(bool on) => Log.Add($"camera {on}");

        public void Ring(RingTone tone) => Log.Add($"ring {tone}");

        public void StopRinging() => Log.Add("quiet");

        public void Close() => Log.Add("close");

        public event Action<IceCandidate>? CandidateGathered;

        public event Action? Connected;

        public event Action? Failed;

        public event Action? MediaChanged;

        public void Gather(string candidate) => CandidateGathered?.Invoke(new IceCandidate(candidate, "0", 0));

        public void Up()
        {
            IsConnected = true;
            Connected?.Invoke();
        }

        public void Die() => Failed?.Invoke();

        public void Tracks() => MediaChanged?.Invoke();
    }

    /// <summary>A clock a test moves: every delay waits until the test says that much time has passed.</summary>
    private sealed class Clock
    {
        private readonly List<(TimeSpan After, TaskCompletionSource Done)> waiting = [];

        public DateTimeOffset Now { get; set; } = new(2026, 9, 14, 12, 0, 0, TimeSpan.Zero);

        public Task Delay(TimeSpan after, CancellationToken ct)
        {
            var done = new TaskCompletionSource();
            ct.Register(() => done.TrySetCanceled());
            waiting.Add((after, done));
            return done.Task;
        }

        public int Waiting(TimeSpan after) => waiting.Count(entry => entry.After == after && !entry.Done.Task.IsCompleted);

        public void Pass(TimeSpan after)
        {
            foreach (var (wanted, done) in waiting.ToList())
            {
                if (wanted == after)
                {
                    done.TrySetResult();
                }
            }
        }
    }

    private sealed record Rig(CallEngine Engine, Wire Socket, Media Media, Clock Clock, List<string> Asked);

    private static readonly IceCandidate Theirs = new("candidate:theirs", "0", 0);

    private static Rig Build(ApiResult<IceServersResponse>? servers = null)
    {
        var socket = new Wire();
        var media = new Media();
        var clock = new Clock();
        var asked = new List<string>();
        var answer = servers ?? ApiResult<IceServersResponse>.Success(new IceServersResponse([new IceServerDto(["stun:stun.example.com"])]));
        var ids = 0;
        var engine = new CallEngine(
            socket,
            _ =>
            {
                asked.Add("/calls/ice");
                return Task.FromResult(answer);
            },
            media,
            clock.Delay,
            () => clock.Now,
            () => $"call-{++ids}");
        return new Rig(engine, socket, media, clock, asked);
    }

    private static ServerFrame Offer(string callId = "in", bool video = false) => new ServerFrame.CallOffer(callId, 42, 9, "v=theirs", video);

    private static JsonNode Frame(string frame) => JsonNode.Parse(frame)!;

    private static string[] Types(Rig rig) => [.. rig.Socket.Sent.Select(frame => Frame(frame)["type"]!.GetValue<string>())];

    private static string? LastReason(Rig rig) => Frame(rig.Socket.Sent[^1])["reason"]?.GetValue<string>();

    private static IStringCatalog Say => EnglishCatalog.Instance;

    /// <summary>The protocol's order: the microphone, then the servers, then the offer the moment it is described.</summary>
    [Fact]
    public Task PlacingACallAsksForTheMicrophoneThenTheServersThenOffers() => Task.Run(async () =>
    {
        var rig = Build();

        await rig.Engine.PlaceAsync(42, 9, video: false);

        var call = rig.Engine.Call!;
        Assert.Equal(("call-1", 42L, 9L, true, CallStage.Dialling), (call.CallId, call.ChatId, call.PeerUserId, call.Outgoing, call.Stage));
        Assert.Equal(["open", "connect 1", "offer"], rig.Media.Log);
        var offer = Frame(Assert.Single(rig.Socket.Sent));
        Assert.Equal(("call_offer", "call-1", 42L, "v=offer"), (offer["type"]!.GetValue<string>(), offer["call_id"]!.GetValue<string>(), offer["chat_id"]!.GetValue<long>(), offer["sdp"]!.GetValue<string>()));
        Assert.Null(offer["video"]);
        Assert.Single(rig.Asked);
        Assert.Equal(1, rig.Clock.Waiting(CallEngine.OutgoingRingGuard));
        Assert.True(rig.Engine.Busy);
        Assert.Equal("Calling…", CallText.StatusLine(call, rig.Clock.Now, Say));
    });

    /// <summary>A candidate found while the offer was being described follows the offer: the server knows no call before it.</summary>
    [Fact]
    public Task ACandidateFoundBeforeTheOfferFollowsIt() => Task.Run(async () =>
    {
        var rig = Build();
        rig.Media.WhileDescribing = () => rig.Media.Gather("candidate:1");

        await rig.Engine.PlaceAsync(42, 9, video: false);
        rig.Media.Gather("candidate:2");

        Assert.Equal(["call_offer", "call_ice", "call_ice"], Types(rig));
        Assert.Equal("candidate:1", Frame(rig.Socket.Sent[1])["candidate"]!["candidate"]!.GetValue<string>());
        Assert.Equal("candidate:2", Frame(rig.Socket.Sent[2])["candidate"]!["candidate"]!.GetValue<string>());
    });

    /// <summary>The server reaching them is when the caller hears the tone — once, and for this call only.</summary>
    [Fact]
    public Task TheCallerHearsTheToneWhenItRingsThere() => Task.Run(async () =>
    {
        var rig = Build();
        await rig.Engine.PlaceAsync(42, 9, video: false);

        rig.Engine.Hear(new ServerFrame.CallRinging("theirs"));
        Assert.Equal(CallStage.Dialling, rig.Engine.Call!.Stage);

        rig.Engine.Hear(new ServerFrame.CallRinging("call-1"));
        rig.Engine.Hear(new ServerFrame.CallRinging("call-1"));

        Assert.Equal(CallStage.Ringing, rig.Engine.Call!.Stage);
        Assert.Single(rig.Media.Log, entry => entry == "ring Ringback");
        Assert.Equal("Ringing…", CallText.StatusLine(rig.Engine.Call!, rig.Clock.Now, Say));
    });

    /// <summary>An answer connects; candidates held for it go in; and the clock counts from the media coming UP.</summary>
    [Fact]
    public Task AnAnswerConnectsAndTheClockStartsWhenTheMediaDoes() => Task.Run(async () =>
    {
        var rig = Build();
        await rig.Engine.PlaceAsync(42, 9, video: false);
        rig.Engine.Hear(new ServerFrame.CallIce("call-1", Theirs));
        Assert.Empty(rig.Media.Added);

        rig.Engine.Hear(new ServerFrame.CallAnswer("call-1", "v=answer"));

        Assert.Contains("remote answer v=answer", rig.Media.Log);
        Assert.Equal(Theirs, Assert.Single(rig.Media.Added));
        var call = rig.Engine.Call!;
        Assert.Equal((CallStage.Connecting, true, (DateTimeOffset?)null), (call.Stage, call.Taken, call.AnsweredAt));
        Assert.Equal(1, rig.Clock.Waiting(CallEngine.AnswerGuard));
        Assert.Equal(0, rig.Clock.Waiting(CallEngine.OutgoingRingGuard));

        rig.Engine.Hear(new ServerFrame.CallIce("call-1", Theirs with { Candidate = "candidate:later" }));
        Assert.Equal(2, rig.Media.Added.Count);

        rig.Clock.Now += TimeSpan.FromSeconds(5);
        rig.Media.Up();
        var started = rig.Clock.Now;
        Assert.Equal((CallStage.Talking, (DateTimeOffset?)started), (rig.Engine.Call!.Stage, rig.Engine.Call!.AnsweredAt));
        Assert.Equal("0:12", CallText.StatusLine(rig.Engine.Call!, started.AddSeconds(12.9), Say));
        // A call that came up is not failed by the guard for one that never did.
        rig.Clock.Pass(CallEngine.AnswerGuard);
        Assert.Equal(CallStage.Talking, rig.Engine.Call!.Stage);

        // A second "up" changes nothing, and an answer for a call already taken is not applied again.
        rig.Clock.Now += TimeSpan.FromSeconds(3);
        rig.Media.Up();
        rig.Engine.Hear(new ServerFrame.CallAnswer("call-1", "v=again"));
        Assert.Equal(started, rig.Engine.Call!.AnsweredAt);
        Assert.DoesNotContain("remote answer v=again", rig.Media.Log);
    });

    /// <summary>A call nobody answers gives up at its guard — saying nothing: the server ended it long before — and then goes.</summary>
    [Fact]
    public Task ACallNobodyAnswersGivesUpAndGoes() => Task.Run(async () =>
    {
        var rig = Build();
        await rig.Engine.PlaceAsync(42, 9, video: false);

        rig.Clock.Pass(CallEngine.OutgoingRingGuard);

        var call = rig.Engine.Call!;
        Assert.Equal((CallStage.Ended, "timeout"), (call.Stage, call.EndedReason));
        Assert.Equal(["call_offer"], Types(rig));
        Assert.Equal("No answer", CallText.StatusLine(call, rig.Clock.Now, Say));
        Assert.Contains("close", rig.Media.Log);
        Assert.False(rig.Engine.Busy);

        rig.Clock.Pass(CallEngine.EndedLinger);
        Assert.Null(rig.Engine.Call);
    });

    /// <summary>An answered call that never comes up fails — and this one says so.</summary>
    [Fact]
    public Task AnAnsweredCallThatNeverComesUpFails() => Task.Run(async () =>
    {
        var rig = Build();
        await rig.Engine.PlaceAsync(42, 9, video: false);
        rig.Engine.Hear(new ServerFrame.CallAnswer("call-1", "v=answer"));

        rig.Clock.Pass(CallEngine.AnswerGuard);

        Assert.Equal("failed", rig.Engine.Call!.EndedReason);
        Assert.Equal("failed", LastReason(rig));
        Assert.Equal("Call failed", CallText.StatusLine(rig.Engine.Call!, rig.Clock.Now, Say));
    });

    /// <summary>The media dying ends the call as failed, once.</summary>
    [Fact]
    public Task MediaThatDiesEndsTheCallOnce() => Task.Run(async () =>
    {
        var rig = Build();
        await rig.Engine.PlaceAsync(42, 9, video: false);
        rig.Engine.Hear(new ServerFrame.CallAnswer("call-1", "v=answer"));
        rig.Media.Up();

        rig.Media.Die();
        rig.Media.Die();
        // A candidate the dying connection still reports belongs to no call.
        rig.Media.Gather("candidate:late");

        Assert.Equal((CallStage.Ended, "failed"), (rig.Engine.Call!.Stage, rig.Engine.Call!.EndedReason));
        Assert.Equal(["call_offer", "call_end"], Types(rig));
    });

    /// <summary>The reason follows the stage — and a call already over is not ended twice.</summary>
    [Fact]
    public Task TheReasonFollowsTheStage() => Task.Run(async () =>
    {
        var rig = Build();
        rig.Engine.Hear(Offer());
        rig.Engine.End();
        Assert.Equal(("decline", "decline"), (rig.Engine.Call!.EndedReason, LastReason(rig)));
        rig.Engine.End();
        Assert.Single(rig.Socket.Sent);
        Assert.Equal("decline", rig.Engine.Call!.EndedReason);

        rig = Build();
        await rig.Engine.PlaceAsync(42, 9, video: false);
        rig.Engine.End();
        Assert.Equal(("cancel", "cancel"), (rig.Engine.Call!.EndedReason, LastReason(rig)));

        rig = Build();
        await rig.Engine.PlaceAsync(42, 9, video: false);
        rig.Engine.Hear(new ServerFrame.CallAnswer("call-1", "v=answer"));
        rig.Media.Up();
        rig.Engine.End();
        Assert.Equal(("hangup", "hangup"), (rig.Engine.Call!.EndedReason, LastReason(rig)));
        Assert.Equal("Call ended", CallText.StatusLine(rig.Engine.Call!, rig.Clock.Now, Say));

        Assert.Equal("decline", CallText.EndReason(new CallState("c", 42, 9, Outgoing: false, Video: false, CallStage.Ringing)));
    });

    /// <summary>Answering: the microphone, the servers, the offer in with what was held for it, and the answer out.</summary>
    [Fact]
    public Task AnsweringTakesTheOfferAndAnswersIt() => Task.Run(async () =>
    {
        var rig = Build();
        rig.Engine.Hear(Offer());
        // An answer frame is the caller's to hear: a callee holding the call ignores one.
        rig.Engine.Hear(new ServerFrame.CallAnswer("in", "v=stray"));
        Assert.DoesNotContain("remote answer v=stray", rig.Media.Log);
        Assert.Equal((CallStage.Incoming, false, 9L), (rig.Engine.Call!.Stage, rig.Engine.Call!.Outgoing, rig.Engine.Call!.PeerUserId));
        Assert.Contains("ring Incoming", rig.Media.Log);
        Assert.Equal(1, rig.Clock.Waiting(CallEngine.IncomingRingGuard));
        Assert.Equal("Incoming call", CallText.StatusLine(rig.Engine.Call!, rig.Clock.Now, Say));
        rig.Engine.Hear(new ServerFrame.CallIce("in", Theirs));
        rig.Engine.Hear(new ServerFrame.CallIce("other", Theirs));
        rig.Media.Log.Clear();

        await rig.Engine.AnswerAsync();

        Assert.Equal(["open", "connect 1", "remote offer v=theirs", "answer", "quiet"], rig.Media.Log);
        Assert.Equal(Theirs, Assert.Single(rig.Media.Added));
        var answer = Frame(Assert.Single(rig.Socket.Sent));
        Assert.Equal(("call_answer", "in", "v=answer"), (answer["type"]!.GetValue<string>(), answer["call_id"]!.GetValue<string>(), answer["sdp"]!.GetValue<string>()));
        Assert.Equal((CallStage.Connecting, true), (rig.Engine.Call!.Stage, rig.Engine.Call!.Taken));
        Assert.Equal(1, rig.Clock.Waiting(CallEngine.AnswerGuard));
        Assert.Equal("Connecting…", CallText.StatusLine(rig.Engine.Call!, rig.Clock.Now, Say));
    });

    /// <summary>The media may already be up by the time the answer is in: the call is then talking at once.</summary>
    [Fact]
    public Task MediaAlreadyUpIsTalkingAtOnce() => Task.Run(async () =>
    {
        var rig = Build();
        rig.Engine.Hear(Offer());
        rig.Media.IsConnected = true;

        await rig.Engine.AnswerAsync();

        Assert.Equal(CallStage.Talking, rig.Engine.Call!.Stage);
        Assert.Equal(rig.Clock.Now, rig.Engine.Call!.AnsweredAt);
    });

    /// <summary>A candidate found before this device's answer went follows it: the server relays a callee's only once it has answered.</summary>
    [Fact]
    public Task TheAnswerIsTriedAgainAndItsCandidatesFollowIt() => Task.Run(async () =>
    {
        var rig = Build();
        rig.Engine.Hear(Offer());
        rig.Socket.Refusals = 2;
        rig.Media.WhileDescribing = () => rig.Media.Gather("candidate:early");

        await rig.Engine.AnswerAsync();
        rig.Media.Gather("candidate:meanwhile");
        Assert.Empty(rig.Socket.Sent);

        rig.Clock.Pass(CallEngine.SayEvery);
        Assert.Empty(rig.Socket.Sent);
        rig.Clock.Pass(CallEngine.SayEvery);

        Assert.Equal(["call_answer", "call_ice", "call_ice"], Types(rig));
        rig.Media.Gather("candidate:after");
        Assert.Equal(4, rig.Socket.Sent.Count);
    });

    /// <summary>Without a microphone, an answer is a refusal on the wire — and the reason on screen is the one that can be acted on.</summary>
    [Fact]
    public Task WithoutAMicrophoneAnAnswerIsARefusal() => Task.Run(async () =>
    {
        var rig = Build();
        rig.Media.Grant = MediaGrant.None;
        rig.Engine.Hear(Offer());

        await rig.Engine.AnswerAsync();

        Assert.Equal("microphone_denied", rig.Engine.Call!.EndedReason);
        Assert.Equal("decline", LastReason(rig));
        Assert.Empty(rig.Asked);
        Assert.Equal("Microphone access is needed for calls.", CallText.StatusLine(rig.Engine.Call!, rig.Clock.Now, Say));
    });

    /// <summary>Placing without a microphone says so, fetches nothing and sends nothing: the server never hears of it.</summary>
    [Fact]
    public Task PlacingWithoutAMicrophoneSendsNothing() => Task.Run(async () =>
    {
        var rig = Build();
        rig.Media.Grant = MediaGrant.None;

        await rig.Engine.PlaceAsync(42, 9, video: false);

        Assert.Equal("microphone_denied", rig.Engine.Call!.EndedReason);
        Assert.Empty(rig.Socket.Sent);
        Assert.Empty(rig.Asked);
    });

    /// <summary>A server that refuses calls ends the placing in its own words; it never heard of the call.</summary>
    [Fact]
    public Task AServerWithoutCallsEndsThePlacing() => Task.Run(async () =>
    {
        var rig = Build(ApiResult<IceServersResponse>.Failure(new ApiError("calls_disabled", "no", 403)));

        await rig.Engine.PlaceAsync(42, 9, video: false);

        Assert.Equal("calls_disabled", rig.Engine.Call!.EndedReason);
        Assert.Empty(rig.Socket.Sent);
        Assert.Equal("Calls are off on this server.", CallText.StatusLine(rig.Engine.Call!, rig.Clock.Now, Say));
    });

    /// <summary>An answer whose servers could not be reached goes on with none; one the server refuses is declined in its words.</summary>
    [Fact]
    public Task AnAnswerGoesOnWithoutServersItCouldNotReach() => Task.Run(async () =>
    {
        var rig = Build(ApiResult<IceServersResponse>.Failure(ApiError.Transport("down")));
        rig.Engine.Hear(Offer());
        await rig.Engine.AnswerAsync();
        Assert.Contains("connect 0", rig.Media.Log);
        Assert.Equal(CallStage.Connecting, rig.Engine.Call!.Stage);

        rig = Build(ApiResult<IceServersResponse>.Failure(new ApiError("calls_disabled", "no", 403)));
        rig.Engine.Hear(Offer());
        await rig.Engine.AnswerAsync();
        Assert.Equal(("calls_disabled", "decline"), (rig.Engine.Call!.EndedReason, LastReason(rig)));
        Assert.DoesNotContain(rig.Media.Log, entry => entry.StartsWith("connect", StringComparison.Ordinal));
    });

    /// <summary>A video call without a camera still connects — receiving the far side's picture — and says its camera is off.</summary>
    [Fact]
    public Task AVideoCallWithoutACameraStillConnects() => Task.Run(async () =>
    {
        var rig = Build();

        await rig.Engine.PlaceAsync(42, 9, video: true);

        Assert.Equal(["open video", "connect 1 receive-only", "offer"], rig.Media.Log);
        Assert.False(rig.Engine.Call!.Camera);
        Assert.True(Frame(rig.Socket.Sent[0])["video"]!.GetValue<bool>());

        rig = Build();
        rig.Media.Grant = MediaGrant.MicrophoneAndCamera;
        rig.Engine.Hear(Offer(video: true));
        Assert.Equal("Incoming video call", CallText.StatusLine(rig.Engine.Call!, rig.Clock.Now, Say));
        await rig.Engine.AnswerAsync();
        Assert.Contains("connect 1", rig.Media.Log);
        Assert.True(rig.Engine.Call!.Camera);
    });

    /// <summary>Mute and the camera: the state and the tracks both; the camera belongs to video calls alone.</summary>
    [Fact]
    public Task MuteAndCameraToggleTheirTracks() => Task.Run(async () =>
    {
        var rig = Build();
        rig.Media.Grant = MediaGrant.MicrophoneAndCamera;
        await rig.Engine.PlaceAsync(42, 9, video: true);
        Assert.True(rig.Engine.Call!.Camera);

        rig.Engine.ToggleMute();
        rig.Engine.ToggleCamera();
        Assert.Equal((true, false), (rig.Engine.Call!.Muted, rig.Engine.Call!.Camera));
        Assert.Contains("muted True", rig.Media.Log);
        Assert.Contains("camera False", rig.Media.Log);
        rig.Engine.ToggleMute();
        Assert.False(rig.Engine.Call!.Muted);

        rig = Build();
        await rig.Engine.PlaceAsync(42, 9, video: false);
        rig.Engine.ToggleCamera();
        Assert.False(rig.Engine.Call!.Camera);
        Assert.DoesNotContain(rig.Media.Log, entry => entry.StartsWith("camera", StringComparison.Ordinal));
    });

    /// <summary>THE SET-UP GUARD: a call cancelled while the microphone is being asked for is not placed, and its microphone is given back.</summary>
    [Fact]
    public Task ACallCancelledDuringSetUpIsNotPlaced() => Task.Run(async () =>
    {
        var rig = Build();
        rig.Media.Asking = new TaskCompletionSource<MediaGrant>();
        var placing = rig.Engine.PlaceAsync(42, 9, video: false);

        rig.Engine.End();
        rig.Media.Log.Clear();
        rig.Media.Asking.SetResult(MediaGrant.Microphone);
        await placing;

        Assert.Equal(["close"], rig.Media.Log);
        Assert.DoesNotContain("call_offer", Types(rig));
        Assert.Empty(rig.Asked);
        Assert.Equal("cancel", rig.Engine.Call!.EndedReason);
    });

    /// <summary>Every connection the callee has gets the offer: a second copy of the call held is the duplicate it is.</summary>
    [Fact]
    public Task ASecondCopyOfAnOfferStartsNothing() => Task.Run(async () =>
    {
        var rig = Build();
        rig.Engine.Hear(Offer());
        await rig.Engine.AnswerAsync();

        rig.Engine.Hear(Offer());

        Assert.Equal(CallStage.Connecting, rig.Engine.Call!.Stage);
        Assert.Single(rig.Media.Log, entry => entry == "ring Incoming");
    });

    /// <summary>An offer that crossed its own end does not ring again; another call does, and takes an ended one's place at once.</summary>
    [Fact]
    public Task AnEndedCallStaysEndedAndANewOneTakesItsPlace() => Task.Run(() =>
    {
        var rig = Build();
        rig.Engine.Hear(Offer("one"));
        rig.Engine.Decline();
        Assert.Equal(CallStage.Ended, rig.Engine.Call!.Stage);

        rig.Engine.Hear(Offer("two"));
        Assert.Equal(("two", CallStage.Incoming), (rig.Engine.Call!.CallId, rig.Engine.Call!.Stage));
        rig.Engine.Decline();
        rig.Clock.Pass(CallEngine.EndedLinger);
        Assert.Null(rig.Engine.Call);

        rig.Engine.Hear(Offer("one"));
        Assert.Null(rig.Engine.Call);
        rig.Engine.Hear(Offer("three"));
        Assert.Equal("three", rig.Engine.Call!.CallId);
        return Task.CompletedTask;
    });

    /// <summary>Only the newest calls are remembered as over: past the memory, an old id rings like any other.</summary>
    [Fact]
    public Task OnlyTheNewestEndedCallsAreRemembered() => Task.Run(() =>
    {
        var rig = Build();
        for (var index = 0; index <= CallEngine.Remembered; index++)
        {
            rig.Engine.Hear(Offer($"call {index}"));
            rig.Engine.Decline();
        }
        rig.Clock.Pass(CallEngine.EndedLinger);

        rig.Engine.Hear(Offer($"call {CallEngine.Remembered}"));
        Assert.Null(rig.Engine.Call);
        rig.Engine.Hear(Offer("call 1"));
        Assert.Null(rig.Engine.Call);
        rig.Engine.Hear(Offer("call 0"));
        Assert.Equal("call 0", rig.Engine.Call!.CallId);
        return Task.CompletedTask;
    });

    /// <summary>A frame naming a call this device does not hold is ignored in silence.</summary>
    [Fact]
    public Task AFrameForAnotherCallIsIgnored() => Task.Run(async () =>
    {
        var rig = Build();
        await rig.Engine.PlaceAsync(42, 9, video: false);
        rig.Engine.Hear(new ServerFrame.CallRinging("call-1"));

        rig.Engine.Hear(new ServerFrame.CallAnswer("theirs", "v=0"));
        rig.Engine.Hear(new ServerFrame.CallEnd("theirs", "hangup"));
        rig.Engine.Hear(new ServerFrame.Error("peer_busy", "busy", null, "theirs"));
        rig.Engine.Hear(new ServerFrame.CallIce("theirs", Theirs));
        rig.Engine.Hear(new ServerFrame.CallAnswer("call-1", "v=answer"));

        Assert.Equal(("call-1", CallStage.Connecting), (rig.Engine.Call!.CallId, rig.Engine.Call!.Stage));
        Assert.Empty(rig.Media.Added);
    });

    /// <summary>The server ends a call it cannot deliver BEFORE it answers the error, which then says better what happened.</summary>
    [Fact]
    public Task AnErrorRefinesTheCallItEnds() => Task.Run(async () =>
    {
        var rig = Build();
        await rig.Engine.PlaceAsync(42, 9, video: false);

        rig.Engine.Hear(new ServerFrame.CallEnd("call-1", "cancel"));
        Assert.Equal("cancel", rig.Engine.Call!.EndedReason);
        rig.Engine.Hear(new ServerFrame.Error("peer_unreachable", "nobody", null, "call-1"));

        Assert.Equal((CallStage.Ended, "unreachable"), (rig.Engine.Call!.Stage, rig.Engine.Call!.EndedReason));
        Assert.Equal("Unavailable", CallText.StatusLine(rig.Engine.Call!, rig.Clock.Now, Say));
        Assert.Equal(["call_offer"], Types(rig));
    });

    /// <summary>The buffer of candidates held for the remote description is capped where the server caps its own.</summary>
    [Fact]
    public Task TheCandidateBufferIsCapped() => Task.Run(async () =>
    {
        var rig = Build();
        rig.Engine.Hear(Offer());
        for (var index = 0; index < 80; index++)
        {
            rig.Engine.Hear(new ServerFrame.CallIce("in", Theirs with { Candidate = $"candidate:{index}" }));
        }

        await rig.Engine.AnswerAsync();

        Assert.Equal(CallEngine.HeldCandidates, rig.Media.Added.Count);
        Assert.Equal("candidate:0", rig.Media.Added[0].Candidate);
    });

    /// <summary>A callee left ringing gives up at its own guard, and the missed call is said the way the record says it.</summary>
    [Fact]
    public Task AnUnansweredRingGivesUpAsMissed() => Task.Run(() =>
    {
        var rig = Build();
        rig.Engine.Hear(Offer(video: true));

        rig.Clock.Pass(CallEngine.IncomingRingGuard);

        Assert.Equal("timeout", rig.Engine.Call!.EndedReason);
        Assert.Empty(rig.Socket.Sent);
        Assert.Equal("Missed video call", CallText.StatusLine(rig.Engine.Call!, rig.Clock.Now, Say));
        return Task.CompletedTask;
    });

    /// <summary>The app going says the reason its stage means, once, and gives the media back; a call already over says nothing.</summary>
    [Fact]
    public Task LeavingSaysWhatTheStageMeans() => Task.Run(async () =>
    {
        var rig = Build();
        await rig.Engine.PlaceAsync(42, 9, video: false);
        await rig.Engine.LeaveAsync();
        Assert.Equal("cancel", LastReason(rig));
        Assert.Contains("close", rig.Media.Log);

        rig = Build();
        rig.Engine.Hear(Offer());
        await rig.Engine.LeaveAsync();
        Assert.Equal("decline", LastReason(rig));

        rig = Build();
        rig.Engine.Hear(Offer());
        await rig.Engine.AnswerAsync();
        await rig.Engine.LeaveAsync();
        Assert.Equal("hangup", LastReason(rig));

        rig = Build();
        rig.Engine.Hear(Offer());
        rig.Engine.Decline();
        await rig.Engine.LeaveAsync();
        Assert.Single(rig.Socket.Sent);
    });

    /// <summary>Every change is announced, so a view redraws — a stream appearing included.</summary>
    [Fact]
    public Task EveryChangeIsAnnounced() => Task.Run(async () =>
    {
        var rig = Build();
        var changes = 0;
        rig.Engine.Changed += () => changes++;

        await rig.Engine.PlaceAsync(42, 9, video: false);
        var placed = changes;
        rig.Media.Tracks();

        Assert.True(placed >= 1);
        Assert.Equal(placed + 1, changes);
    });

    /// <summary>The apps' numbers: guards past the server's 45 s, two seconds on screen, ten tries, sixteen remembered, sixty-four held.</summary>
    [Fact]
    public void TheGuardsAndBuffersAreTheApps()
    {
        Assert.Equal(TimeSpan.FromSeconds(90), CallEngine.OutgoingRingGuard);
        Assert.Equal(TimeSpan.FromSeconds(60), CallEngine.IncomingRingGuard);
        Assert.Equal(TimeSpan.FromSeconds(30), CallEngine.AnswerGuard);
        Assert.Equal(TimeSpan.FromSeconds(2), CallEngine.EndedLinger);
        Assert.Equal(TimeSpan.FromMilliseconds(500), CallEngine.SayEvery);
        Assert.Equal((10, 16, 64), (CallEngine.SayAttempts, CallEngine.Remembered, CallEngine.HeldCandidates));
    }

    /// <summary>A call can be placed while the last one still says why it ended — and not while one is on.</summary>
    [Fact]
    public Task APlacedCallTakesAnEndedOnesPlaceButNotALiveOnes() => Task.Run(async () =>
    {
        var rig = Build();
        rig.Engine.Hear(Offer());
        await rig.Engine.PlaceAsync(42, 9, video: false);
        Assert.Equal("in", rig.Engine.Call!.CallId);
        Assert.Empty(rig.Socket.Sent);

        rig.Engine.Decline();
        await rig.Engine.PlaceAsync(42, 9, video: false);

        Assert.Equal(("call-1", CallStage.Dialling), (rig.Engine.Call!.CallId, rig.Engine.Call!.Stage));
        Assert.Equal(["call_end", "call_offer"], Types(rig));
    });

    /// <summary>An offer the socket would not take is a call that visibly did not happen: failed, and not tried again.</summary>
    [Fact]
    public Task AnOfferTheSocketWouldNotTakeFails() => Task.Run(async () =>
    {
        var rig = Build();
        rig.Socket.Refusals = 1;

        await rig.Engine.PlaceAsync(42, 9, video: false);

        Assert.Equal("failed", rig.Engine.Call!.EndedReason);
        Assert.Empty(rig.Socket.Sent);
        Assert.Equal(0, rig.Clock.Waiting(CallEngine.SayEvery));
    });

    /// <summary>An answer the media cannot take fails the call, and says so.</summary>
    [Fact]
    public Task AnAnswerTheMediaCannotTakeFailsTheCall() => Task.Run(async () =>
    {
        var rig = Build();
        await rig.Engine.PlaceAsync(42, 9, video: false);
        rig.Media.AcceptsRemote = false;

        rig.Engine.Hear(new ServerFrame.CallAnswer("call-1", "v=broken"));

        Assert.Equal(("failed", "failed"), (rig.Engine.Call!.EndedReason, LastReason(rig)));
    });

    /// <summary>A new call starts with none of the last one's candidates, held or waiting to follow an offer.</summary>
    [Fact]
    public Task ANewCallStartsWithNoneOfTheLastOnesCandidates() => Task.Run(async () =>
    {
        var rig = Build();
        rig.Engine.Hear(Offer("one"));
        rig.Engine.Hear(new ServerFrame.CallIce("one", Theirs));
        rig.Engine.Decline();
        rig.Engine.Hear(Offer("two"));
        await rig.Engine.AnswerAsync();
        Assert.Empty(rig.Media.Added);

        rig = Build();
        await rig.Engine.PlaceAsync(42, 9, video: false);
        rig.Engine.End();
        // Reported by the connection on its way out: it belongs to no call, and must not follow the next one's offer.
        rig.Media.Gather("candidate:stale");
        rig.Socket.Sent.Clear();
        rig.Media.WhileDescribing = () => rig.Media.Gather("candidate:new");
        await rig.Engine.PlaceAsync(42, 9, video: false);

        Assert.Equal(["call_offer", "call_ice"], Types(rig));
        Assert.Equal("candidate:new", Frame(rig.Socket.Sent[1])["candidate"]!["candidate"]!.GetValue<string>());
    });

    /// <summary>An engine detached from the media hears none of it: the window's media is the next server's engine's now.</summary>
    [Fact]
    public Task ADetachedEngineHearsNothingFromTheMedia() => Task.Run(async () =>
    {
        var rig = Build();
        await rig.Engine.PlaceAsync(42, 9, video: false);
        rig.Engine.Hear(new ServerFrame.CallAnswer("call-1", "v=answer"));
        var changes = 0;
        rig.Engine.Changed += () => changes++;

        rig.Engine.Detach();
        rig.Media.Gather("candidate:after");
        rig.Media.Up();
        rig.Media.Tracks();
        rig.Media.Die();

        Assert.Equal(CallStage.Connecting, rig.Engine.Call!.Stage);
        Assert.Equal(0, changes);
        Assert.Equal(["call_offer"], Types(rig));
    });

    /// <summary>What each refusal is called, and how each reason is said on either side of the call.</summary>
    [Fact]
    public void ARefusalIsNamedAndSaid()
    {
        Assert.Equal("busy", CallText.RefusalReason("peer_busy"));
        Assert.Equal("busy", CallText.RefusalReason("call_busy"));
        Assert.Equal("unreachable", CallText.RefusalReason("peer_unreachable"));
        Assert.Equal("blocked", CallText.RefusalReason("blocked"));
        Assert.Equal("calls_disabled", CallText.RefusalReason("calls_disabled"));
        Assert.Equal("video_calls_disabled", CallText.RefusalReason("video_calls_disabled"));
        Assert.Equal("failed", CallText.RefusalReason("invalid_call"));

        var call = new CallState("c", 42, 9, Outgoing: true, Video: false, CallStage.Ended);
        Assert.Equal("No answer", CallText.EndedLine("timeout", call, Say));
        Assert.Equal("Declined", CallText.EndedLine("decline", call, Say));
        Assert.Equal("Call ended", CallText.EndedLine("hangup", call, Say));
        Assert.Equal("Answered on another device", CallText.EndedLine("answered_elsewhere", call, Say));
        Assert.Equal("Busy", CallText.EndedLine("busy", call, Say));
        Assert.Equal("Unavailable", CallText.EndedLine("unavailable", call, Say));
        Assert.Equal("You've blocked them.", CallText.EndedLine("blocked", call, Say));
        Assert.Equal("Video calls are off on this server.", CallText.EndedLine("video_calls_disabled", call, Say));

        var callee = call with { Outgoing = false };
        Assert.Equal("Missed voice call", CallText.EndedLine("timeout", callee, Say));
        Assert.Equal("Call ended", CallText.EndedLine("decline", callee, Say));

        Assert.Equal("hangup", CallText.UnloadReason(callee with { Taken = true }));
        Assert.Equal("decline", CallText.UnloadReason(callee));
        Assert.Equal("cancel", CallText.UnloadReason(call));
        Assert.Equal("0:00", CallText.StatusLine(call with { Stage = CallStage.Talking }, DateTimeOffset.UnixEpoch, Say));
    }
}
