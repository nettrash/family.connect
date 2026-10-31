using System.Globalization;
using System.Net;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// Paid families from the app's side (docs/protocol.md, "Paid families"; issue #89): what the session learns, the two create
/// doors, the cap picker's range, the unpaid banner and the frame that moves it, the statistics' allowance, the dates they all
/// name, and the refusals. Windows cannot sell the subscription, so all of it is saying so — and on a server with
/// subscriptions off, none of it says anything.
/// </summary>
public class PaidFamiliesTests : IDisposable
{
    private static readonly IStringCatalog Say = EnglishCatalog.Instance;
    private static readonly CultureInfo British = CultureInfo.GetCultureInfo("en-GB");
    private static readonly DateTimeOffset Now = new(2026, 10, 30, 12, 0, 0, TimeSpan.Zero);

    /// <summary>Five hours behind UTC, all year — a zone where the 1st of a month at midnight UTC is still the last day of the one before.</summary>
    private static readonly TimeZoneInfo West = TimeZoneInfo.CreateCustomTimeZone("West", TimeSpan.FromHours(-5), "West", "West");

    private readonly Database cache = Database.OpenInMemory();

    public void Dispose() => cache.Dispose();

    private const string User = """{"id": 7, "username": "anna", "display_name": "Anna"}""";

    private static string Me(string families, string paid = "", bool registration = true, bool members = true) =>
        $$"""
        {"user": {{User}}, "blocked_user_ids": [], "families": [{{families}}], "pending_join_requests": [], "invitations": [],
         "family_registration_enabled": {{(registration ? "true" : "false")}},
         "member_families_enabled": {{(members ? "true" : "false")}}, "max_families_per_account": 5, "max_family_members": 50
         {{paid}}}
        """;

    private const string Active =
        """, "subscriptions_enabled": true, "subscription": {"product_id": "x", "environment": "Production", "expires_at": "2026-12-01T00:00:00Z", "active": true}""";

    private const string Lapsed =
        """, "subscriptions_enabled": true, "subscription": {"product_id": "x", "environment": "Production", "expires_at": "2026-10-01T00:00:00Z", "active": false}""";

    private const string Unsubscribed = """, "subscriptions_enabled": true, "subscription": null""";

    private const string Unpaid =
        """{"id": 3, "name": "Ivanovs", "role": "member", "plan": "paid", "member_ceiling": 25, "delete_at": "2026-11-14T09:00:00Z"}""";

    private const string Free = """{"id": 5, "name": "The Smiths", "role": "owner", "plan": "free", "member_ceiling": 50}""";

    private AppSession Session(Server server) =>
        new(new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken")),
            new MemoryTokenStore("t0ken"), cache);

    private async Task<AppSession> Signed(string me, Server? server = null)
    {
        var session = Session((server ?? new Server()).On("/me", me));
        Assert.Null(await session.RefreshAsync());
        return session;
    }

    private static SessionState State(
        bool enabled = true, SubscriptionDto? subscription = null, bool registration = true, bool members = true,
        IReadOnlyList<FamilyDto>? families = null, Gate gate = Gate.NoFamily) =>
        new(gate, Me: new UserDto(7, "anna", "Anna"), Family: families?.FirstOrDefault(), FamilyRegistrationEnabled: registration,
            MemberFamiliesEnabled: members, MaxFamiliesPerAccount: 5, Families: families, CurrentFamilyId: families?.FirstOrDefault()?.Id,
            SubscriptionsEnabled: enabled, Subscription: subscription, MaxFamilyMembers: 50);

    private static readonly SubscriptionDto Paying = new("x", "Production", "2026-12-01T00:00:00Z", Active: true);

    // ---- what the session learns ---------------------------------------------------------------------------------------------

    /// <summary>`GET /me`'s switch and the account's subscription reach the session; the family's plan keys ride on the family.</summary>
    [Fact]
    public async Task TheSessionLearnsTheSwitchTheSubscriptionAndTheFamilysPlan()
    {
        var session = await Signed(Me(Unpaid, Active));

        Assert.True(session.State.SubscriptionsEnabled);
        Assert.True(session.State.Subscription!.Active);
        Assert.Equal("paid", session.State.Family!.Plan);
        Assert.Equal(25, session.State.Family.MemberCeiling);
        Assert.Equal("2026-11-14T09:00:00Z", session.State.Family.DeleteAt);

        // An older server: off, and nothing held.
        var older = await Signed(Me("""{"id": 3, "name": "Ivanovs", "role": "member"}"""));
        Assert.False(older.State.SubscriptionsEnabled);
        Assert.Null(older.State.Subscription);
        Assert.Null(older.State.Family!.DeleteAt);
    }

    /// <summary>A subscription on a server whose switch is off is nothing to act on, and is not kept.</summary>
    [Fact]
    public async Task ASubscriptionOnAServerWithTheSwitchOffIsNotKept()
    {
        var session = await Signed(Me(Free,
            """, "subscriptions_enabled": false, "subscription": {"product_id": "x", "environment": "Production", "expires_at": "2026-12-01T00:00:00Z", "active": true}"""));
        Assert.False(session.State.SubscriptionsEnabled);
        Assert.Null(session.State.Subscription);
    }

    /// <summary>
    /// The family's fuller document is kept across a `GET /me`, but what `/me` says of payment is the newer read: a deadline it
    /// no longer carries is gone, and its ceiling is the one in force.
    /// </summary>
    [Fact]
    public async Task ADeadlineGetMeNoLongerCarriesIsGone()
    {
        var server = new Server()
            .Then("/me", (HttpStatusCode.OK, Me(Unpaid, Active)),
                (HttpStatusCode.OK, Me("""{"id": 3, "name": "Ivanovs", "role": "member", "plan": "paid", "member_ceiling": 20}""", Active)))
            .On("/families/3", """{"family": {"id": 3, "name": "Ivanovs", "invite_code": "ABCD2345", "plan": "paid", "member_ceiling": 25, "delete_at": "2026-11-14T09:00:00Z"}}""");
        var session = Session(server);
        await session.RefreshAsync();
        await session.RefreshFamilyAsync();
        Assert.Equal("ABCD2345", session.State.Family!.InviteCode);
        Assert.NotNull(session.State.Family.DeleteAt);

        await session.RefreshAsync();

        Assert.Equal("ABCD2345", session.State.Family!.InviteCode);
        Assert.Null(session.State.Family.DeleteAt);
        Assert.Equal(20, session.State.Family.MemberCeiling);
    }

    // ---- the family_plan frame -----------------------------------------------------------------------------------------------

    /// <summary>
    /// A <c>family_plan</c> frame puts the deadline on the family it NAMES, and one without <c>delete_at</c> takes it away —
    /// on the family shown and on the list alike — so the banner comes and goes while the app is open.
    /// </summary>
    [Fact]
    public async Task APlanFrameMovesTheDeadlineOfTheFamilyItNames()
    {
        var session = await Signed(Me($"{Free}, {Unpaid}", Active));
        Assert.Equal(5, session.State.CurrentFamilyId);
        var published = 0;
        session.Changed += _ => published++;

        // The family shown gets a deadline.
        session.ApplyPlan(new ServerFrame.FamilyPlan(5, "paid", "2026-11-20T08:00:00Z"));
        Assert.Equal("2026-11-20T08:00:00Z", session.State.Family!.DeleteAt);
        Assert.Equal("paid", session.State.Family.Plan);
        Assert.Equal("2026-11-20T08:00:00Z", session.State.AllFamilies.Single(family => family.Id == 5).DeleteAt);

        // The other family, not shown, loses its own — and the family shown keeps its.
        session.ApplyPlan(new ServerFrame.FamilyPlan(3, "paid", null));
        Assert.Null(session.State.AllFamilies.Single(family => family.Id == 3).DeleteAt);
        Assert.Equal("2026-11-20T08:00:00Z", session.State.Family!.DeleteAt);

        // Absent clears; an absent plan keeps the one held.
        session.ApplyPlan(new ServerFrame.FamilyPlan(5, null, null));
        Assert.Null(session.State.Family!.DeleteAt);
        Assert.Equal("paid", session.State.Family.Plan);
        Assert.Equal(3, published);

        // The same news twice, or news about a family this account is not in, publishes nothing.
        session.ApplyPlan(new ServerFrame.FamilyPlan(5, null, null));
        session.ApplyPlan(new ServerFrame.FamilyPlan(42, "paid", "2026-11-20T08:00:00Z"));
        Assert.Equal(3, published);
    }

    /// <summary>The frame reaches the session from the socket: the live connection wires the router to it.</summary>
    [Fact]
    public async Task APlanFrameFromTheSocketReachesTheSession()
    {
        var server = new Server().On("/me", Me(Free, Active));
        var api = new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"));
        var session = new AppSession(api, new MemoryTokenStore("t0ken"), cache);
        await session.RefreshAsync();
        var chats = new ChatStore(cache, () => 7);
        var board = new BoardStore(cache);
        var socket = new Socket();
        var sending = new SendPipeline(socket, new OutboxStore(cache), chats, api, wait: (_, _) => Task.CompletedTask);
        var router = new FrameRouter(chats, board);
        await using var live = new LiveConnection(session, socket, new Resync(api, chats, board, sending), sending, router);

        socket.Hear(ServerFrame.Parse("""{"type": "family_plan", "family_id": 5, "plan": "paid", "delete_at": "2026-11-20T08:00:00Z"}""")!);
        Assert.Equal("2026-11-20T08:00:00Z", session.State.Family!.DeleteAt);
        socket.Hear(ServerFrame.Parse("""{"type": "family_plan", "family_id": 5, "plan": "paid"}""")!);
        Assert.Null(session.State.Family!.DeleteAt);
    }

    /// <summary>A socket that never connects: frames are handed to it by the test.</summary>
    private sealed class Socket : ILiveSocket
    {
        public bool IsConnected => false;

        public event Action<ServerFrame>? Frame;

        // Never raised: this socket never connects, and its session never expires.
        public event Action? Connected { add { } remove { } }

        public event Action? Disconnected { add { } remove { } }

        public event Action? SessionExpired { add { } remove { } }

        public Task RunAsync(CancellationToken ct) => Task.Delay(Timeout.Infinite, ct).ContinueWith(_ => { }, TaskScheduler.Default);

        public Task<bool> TrySend(string frame, CancellationToken ct = default) => Task.FromResult(false);

        public void Hear(ServerFrame frame) => Frame?.Invoke(frame);
    }

    // ---- the create doors ----------------------------------------------------------------------------------------------------

    /// <summary>
    /// The family gate's "Create a Family": the operator's shut door keeps priority and stays as it was; on a server that
    /// charges, the sentence stands in place of the form until the account's subscription is active; anywhere else, the form.
    /// </summary>
    [Fact]
    public void TheGatesCreateDoorNeedsAnActiveSubscriptionWhereTheServerCharges()
    {
        Assert.Equal(CreateDoor.Open, PaidFamilies.FirstFamily(State(enabled: false)));
        Assert.Equal(CreateDoor.AppleApp, PaidFamilies.FirstFamily(State()));
        Assert.Equal(CreateDoor.AppleApp, PaidFamilies.FirstFamily(State(subscription: Paying with { Active = false })));
        Assert.Equal(CreateDoor.Open, PaidFamilies.FirstFamily(State(subscription: Paying)));
        // A shut door is shut whatever the subscription says.
        Assert.Equal(CreateDoor.Shut, PaidFamilies.FirstFamily(State(registration: false)));
        Assert.Equal(CreateDoor.Shut, PaidFamilies.FirstFamily(State(registration: false, subscription: Paying)));
        Assert.Equal(CreateDoor.Shut, PaidFamilies.FirstFamily(State(enabled: false, registration: false)));
        Assert.Equal(
            "On this server, a new family is started in the Family Connect app on iPhone, iPad or Mac.",
            PaidFamilies.StartsInAppleApp(Say));
    }

    /// <summary>
    /// "New Family…" stays where it is offered and says the sentence instead of opening the form; where it is not offered —
    /// the operator's switch, a family already owned — it is not, as before.
    /// </summary>
    [Fact]
    public void NewFamilySaysTheSentenceInsteadOfTheFormWhereTheServerCharges()
    {
        FamilyDto[] member = [new(3, "Ivanovs", Role: "member")];
        Assert.Equal(CreateDoor.Open, PaidFamilies.AnotherFamily(State(enabled: false, families: member, gate: Gate.Member)));
        Assert.Equal(CreateDoor.AppleApp, PaidFamilies.AnotherFamily(State(families: member, gate: Gate.Member)));
        Assert.Equal(CreateDoor.Open, PaidFamilies.AnotherFamily(State(subscription: Paying, families: member, gate: Gate.Member)));
        // Still offered — the entry stays — while it says the sentence.
        Assert.True(FamilyRules.OffersNewFamily(State(families: member, gate: Gate.Member)));
        Assert.Equal(CreateDoor.Shut, PaidFamilies.AnotherFamily(State(members: false, families: member, gate: Gate.Member)));
        Assert.Equal(
            CreateDoor.Shut,
            PaidFamilies.AnotherFamily(State(subscription: Paying, families: [new(3, "Ivanovs", Role: "owner")], gate: Gate.Owner)));
    }

    /// <summary>
    /// <c>subscription_required</c> on a create is said in its own words, and a shut door's own code keeps its own; the form
    /// stays where it was to say it.
    /// </summary>
    [Fact]
    public async Task ACreateRefusedForWantOfASubscriptionSaysSo()
    {
        Assert.Equal(
            "A new family on this server needs a subscription.",
            DoorSentences.Create(new ApiError(ErrorCodes.SubscriptionRequired, "x", 403), Say));
        Assert.Equal(
            "This server doesn't take new families.",
            DoorSentences.Create(new ApiError(ErrorCodes.FamilyRegistrationDisabled, "x", 403), Say));

        var server = new Server()
            .On("/me", Me("", Unsubscribed))
            .On("/families", """{"error": {"code": "subscription_required", "message": "a subscription is required"}}""", HttpStatusCode.Forbidden);
        var api = new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"));
        var session = new AppSession(api, new MemoryTokenStore("t0ken"), cache);
        await session.RefreshAsync();

        Assert.Equal("A new family on this server needs a subscription.", await new FamilyDoor(api, session).CreateAsync("Ivanovs"));
        Assert.Equal(Gate.NoFamily, session.State.Gate);

        var german = JsonCatalog.For("de");
        Assert.Equal(
            "Für eine neue Familie auf diesem Server ist ein Abo nötig.",
            DoorSentences.Create(new ApiError(ErrorCodes.SubscriptionRequired, "x", 403), german));
    }

    // ---- the cap picker ------------------------------------------------------------------------------------------------------

    /// <summary>The cap picker's range is the family's own ceiling, and the operator's on a server that predates it.</summary>
    [Fact]
    public void TheCapPickerDrawsItsRangeFromTheFamily()
    {
        Assert.Equal(25, PaidFamilies.MemberCeiling(new FamilyDto(3, "Ivanovs", MemberCeiling: 25), 50));
        Assert.Equal(50, PaidFamilies.MemberCeiling(new FamilyDto(3, "Ivanovs"), 50));
        Assert.Equal(50, PaidFamilies.MemberCeiling(new FamilyDto(3, "Ivanovs", MemberCeiling: 0), 50));
        Assert.Equal(50, PaidFamilies.MemberCeiling(null, 50));
        Assert.Equal(25, PaidFamilies.MemberCeiling(State(families: [new(3, "Ivanovs", Role: "owner", MemberCeiling: 25)], gate: Gate.Owner)));
    }

    // ---- the banner ----------------------------------------------------------------------------------------------------------

    private static MemberDto Member(long id, string name, string role = "member", bool left = false) =>
        new(id, name.ToLowerInvariant(), name, Role: role, HasLeft: left);

    private static readonly MemberDto[] Roster = [Member(7, "Anna"), Member(9, "Boris", "owner"), Member(11, "Carl")];

    /// <summary>
    /// A member's banner names the date and the owner, the one person who can renew; the owner's says where to renew. Both are
    /// about the family SHOWN, and its <c>delete_at</c> in the reader's own day.
    /// </summary>
    [Fact]
    public void TheBannerNamesTheDayAndWhoCanRenew()
    {
        var unpaid = new FamilyDto(3, "Ivanovs", Role: "member", Plan: "paid", DeleteAt: "2026-11-14T09:00:00Z");

        Assert.Equal(
            "“Ivanovs” will be deleted on 14 November because its subscription has ended. Only Boris, the owner, can renew it.",
            PaidFamilies.UnpaidBanner(State(families: [unpaid], gate: Gate.Member), Roster, Say, British, TimeZoneInfo.Utc, Now));
        Assert.Equal(
            "“Ivanovs” will be deleted on 14 November because your subscription has ended. Renew it in the Family Connect app on iPhone, iPad or Mac.",
            PaidFamilies.UnpaidBanner(
                State(families: [unpaid with { Role = "owner" }], gate: Gate.Owner), Roster, Say, British, TimeZoneInfo.Utc, Now));

        // In the reader's language and their date.
        Assert.Equal(
            "„Ivanovs“ wird am 14. November gelöscht, weil das Abo abgelaufen ist. Nur Boris (Besitzer:in) kann es verlängern.",
            PaidFamilies.UnpaidBanner(
                State(families: [unpaid], gate: Gate.Member), Roster, JsonCatalog.For("de"), CultureInfo.GetCultureInfo("de-DE"),
                TimeZoneInfo.Utc, Now));
    }

    /// <summary>
    /// No banner without a deadline, on a server with subscriptions off, for a deadline nobody can read — and, for a member,
    /// until the roster names the owner (a former owner is not the one who can renew).
    /// </summary>
    [Fact]
    public void ThereIsNoBannerWithoutADeadlineOrSomebodyToName()
    {
        var unpaid = new FamilyDto(3, "Ivanovs", Role: "member", Plan: "paid", DeleteAt: "2026-11-14T09:00:00Z");
        string? Banner(SessionState state, IReadOnlyList<MemberDto> roster) =>
            PaidFamilies.UnpaidBanner(state, roster, Say, British, TimeZoneInfo.Utc, Now);

        Assert.Null(Banner(State(families: [unpaid with { DeleteAt = null }], gate: Gate.Member), Roster));
        Assert.Null(Banner(State(enabled: false, families: [unpaid], gate: Gate.Member), Roster));
        Assert.Null(Banner(State(families: [unpaid with { DeleteAt = "soon" }], gate: Gate.Member), Roster));
        Assert.Null(Banner(State(), Roster));
        Assert.Null(Banner(State(families: [unpaid], gate: Gate.Member), [Member(7, "Anna")]));
        Assert.Null(Banner(State(families: [unpaid], gate: Gate.Member), [Member(7, "Anna"), Member(9, "Boris", "owner", left: true)]));
        // The owner's own banner names nobody, so it needs no roster.
        Assert.NotNull(Banner(State(families: [unpaid with { Role = "owner" }], gate: Gate.Owner), []));
    }

    // ---- dates ---------------------------------------------------------------------------------------------------------------

    /// <summary>
    /// A day in the reader's zone: the culture's own day and month this year, and its long date WITHOUT the weekday in another
    /// year — and an instant at midnight UTC is still the day before where the clock is behind.
    /// </summary>
    [Fact]
    public void ADateIsTheReadersDayWithTheYearOnlyWhenItIsNotThisOne()
    {
        var december = new DateTimeOffset(2026, 12, 1, 0, 0, 0, TimeSpan.Zero);
        Assert.Equal("1 December", PaidFamilies.Day(december, British, TimeZoneInfo.Utc, Now));
        Assert.Equal("30 November", PaidFamilies.Day(december, British, West, Now));
        Assert.Equal("1. Dezember", PaidFamilies.Day(december, CultureInfo.GetCultureInfo("de-DE"), TimeZoneInfo.Utc, Now));
        Assert.Equal("12月1日", PaidFamilies.Day(december, CultureInfo.GetCultureInfo("ja-JP"), TimeZoneInfo.Utc, Now));
        Assert.Equal("1 декабря", PaidFamilies.Day(december, CultureInfo.GetCultureInfo("ru-RU"), TimeZoneInfo.Utc, Now));

        var next = new DateTimeOffset(2027, 1, 1, 0, 0, 0, TimeSpan.Zero);
        var british = PaidFamilies.Day(next, British, TimeZoneInfo.Utc, Now);
        Assert.Equal("1 January 2027", british);
        Assert.DoesNotContain("Friday", british, StringComparison.Ordinal);
        Assert.Equal("2027年1月1日", PaidFamilies.Day(next, CultureInfo.GetCultureInfo("ja-JP"), TimeZoneInfo.Utc, Now));
        // The year is the reader's: midnight UTC on 1 January is still this year where the clock is behind.
        Assert.Equal("31 December", PaidFamilies.Day(next, British, West, Now));
    }

    /// <summary>The weekday goes from a long-date pattern with the comma and space that went with it, wherever it stood.</summary>
    [Fact]
    public void TheWeekdayIsTakenOutOfALongDate()
    {
        Assert.Equal("MMMM d, yyyy", PaidFamilies.WithoutWeekday("dddd, MMMM d, yyyy"));
        Assert.Equal("d MMMM yyyy", PaidFamilies.WithoutWeekday("dddd, d MMMM yyyy"));
        Assert.Equal("d. MMMM yyyy", PaidFamilies.WithoutWeekday("dddd, d. MMMM yyyy"));
        Assert.Equal("d MMMM yyyy", PaidFamilies.WithoutWeekday("dddd d MMMM yyyy"));
        Assert.Equal("yyyy年M月d日", PaidFamilies.WithoutWeekday("yyyy年M月d日dddd"));
        Assert.Equal("d MMMM yyyy", PaidFamilies.WithoutWeekday("d MMMM yyyy, dddd"));
        Assert.Equal("dd. MMMM yyyy.", PaidFamilies.WithoutWeekday("dddd, dd. MMMM yyyy."));
        Assert.Equal("d MMMM yyyy 'г.'", PaidFamilies.WithoutWeekday("d MMMM yyyy 'г.'"));
    }

    /// <summary>A monthly allowance starts again on the 1st of the next UTC month — over a year's end too.</summary>
    [Fact]
    public void AnAllowanceRenewsOnTheFirstOfTheNextUtcMonth()
    {
        Assert.Equal(new DateTimeOffset(2026, 11, 1, 0, 0, 0, TimeSpan.Zero), PaidFamilies.NextMonthUtc(Now));
        Assert.Equal(
            new DateTimeOffset(2027, 1, 1, 0, 0, 0, TimeSpan.Zero),
            PaidFamilies.NextMonthUtc(new DateTimeOffset(2026, 12, 31, 23, 59, 0, TimeSpan.Zero)));
        // An offset clock is read in UTC: 1 November 01:00 at +03:00 is still October there.
        Assert.Equal(
            new DateTimeOffset(2026, 11, 1, 0, 0, 0, TimeSpan.Zero),
            PaidFamilies.NextMonthUtc(new DateTimeOffset(2026, 11, 1, 1, 0, 0, TimeSpan.FromHours(3))));
    }

    /// <summary>
    /// A RENEWAL IS THE UTC DAY, a deadline the reader's (docs/protocol.md, "UTC calendar day"): the instant that starts
    /// December is "1 December" to a reader five hours behind UTC — in the statistics, on a failed answer, on a transcript and a
    /// backdrop — where the same reader's banner names their own day. The renewal paths take no zone at all, so the machine's
    /// cannot leak in; a stamp spelt with a western offset is still the UTC day it names.
    /// </summary>
    [Fact]
    public void ARenewalIsTheUtcDayWhereverTheReaderIs()
    {
        var december = new DateTimeOffset(2026, 12, 1, 0, 0, 0, TimeSpan.Zero);
        var spelledWest = new DateTimeOffset(2026, 11, 30, 19, 0, 0, TimeSpan.FromHours(-5));
        const string Renews = "This family has used this month’s assistant allowance. It renews on 1 December.";

        // What the reader's own day would have said, and what a deadline does say.
        Assert.Equal("30 November", PaidFamilies.Day(december, British, West, Now));
        Assert.Equal("1 December", PaidFamilies.RenewalDay(december, British, Now));
        Assert.Equal("1 December", PaidFamilies.RenewalDay(spelledWest, British, Now));
        Assert.Equal(
            "“Ivanovs” will be deleted on 30 November because its subscription has ended. Only Boris, the owner, can renew it.",
            PaidFamilies.UnpaidBanner(
                State(families: [new FamilyDto(3, "Ivanovs", Role: "member", DeleteAt: "2026-12-01T00:00:00Z")], gate: Gate.Member),
                Roster, Say, British, West, Now));

        // The statistics.
        Assert.Equal(
            "Renews on 1 December",
            PaidFamilies.Allowance(Full with { RenewsAt = "2026-11-30T19:00:00-05:00" }, Say, British, Now)!.Renews);
        // A failed answer, from the frame's renews_at.
        var answers = new AssistantAnswers();
        answers.Stopped(42, 5, AiErrorReason.AllowanceUsed, spelledWest);
        Assert.Equal(Renews, answers.FailureSentence(new MessageDto(5, 42, 99, null, "", "2026-11-30T23:00:00Z"), Say, British, Now));
        // A transcript and a backdrop: the 1st of the next UTC month, asked on 30 November at 20:00 five hours behind UTC —
        // already 1 December in UTC, so the allowance renews on 1 JANUARY.
        var lateWest = new DateTimeOffset(2026, 11, 30, 20, 0, 0, TimeSpan.FromHours(-5));
        var used = new ApiError(ErrorCodes.AiAllowanceUsed, "x", 403);
        Assert.Equal(
            "This family has used this month’s assistant allowance. It renews on 1 January 2027.",
            TranscriptRules.FailureSentence(used, Say, British, lateWest));
        Assert.Equal(Renews, NoteSheetText.BackdropFailure(used, Say, British, new DateTimeOffset(2026, 11, 30, 18, 0, 0, TimeSpan.FromHours(-5))));
        // The year is UTC's too: on 31 December at 20:00 five hours behind, it is already 2027 in UTC.
        Assert.Equal(
            "1 January",
            PaidFamilies.RenewalDay(
                new DateTimeOffset(2027, 1, 1, 0, 0, 0, TimeSpan.Zero), British,
                new DateTimeOffset(2026, 12, 31, 20, 0, 0, TimeSpan.FromHours(-5))));
    }

    // ---- the statistics' allowance -------------------------------------------------------------------------------------------

    private static readonly AllowanceDto Full = new(
        "2026-12-01T00:00:00Z",
        new AllowanceUseDto(3_221_225_472, 10_737_418_240),
        new AllowanceUseDto(812_345, 2_000_000),
        new AllowanceUseDto(4, 30),
        new AllowanceUseDto(600_000, 7_200_000));

    /// <summary>
    /// A row per limit, in the protocol's order: storage as this client's file sizes, the assistant as a whole percentage,
    /// pictures as counts, transcripts as minutes — and when the month renews.
    /// </summary>
    [Fact]
    public void TheAllowanceIsARowPerLimitAndTheDayItRenews()
    {
        var section = PaidFamilies.Allowance(Full, Say, British, Now)!;

        Assert.Equal("This Month’s Allowance", section.Title);
        Assert.Equal(
            [
                new AllowanceRow(
                    "Storage",
                    $"{MediaText.DisplaySize(3_221_225_472, Say, British)} of {MediaText.DisplaySize(10_737_418_240, Say, British)}"),
                new AllowanceRow("Assistant", "40% used"),
                new AllowanceRow("Pictures", "4 of 30"),
                new AllowanceRow("Transcripts", "10 of 120 min"),
            ],
            section.Rows);
        Assert.Equal("3.22 GB of 10.74 GB", section.Rows[0].Value);
        Assert.Equal("Renews on 1 December", section.Renews);
    }

    /// <summary>Only the limits the block carries; nothing for a free family, nor for a block with no limit in it.</summary>
    [Fact]
    public void TheAllowanceShowsOnlyWhatIsThere()
    {
        Assert.Null(PaidFamilies.Allowance(null, Say, British, Now));
        Assert.Null(PaidFamilies.Allowance(new AllowanceDto("2026-12-01T00:00:00Z"), Say, British, Now));

        var pictures = PaidFamilies.Allowance(
            new AllowanceDto(Images: new AllowanceUseDto(1234, 5000)), Say, British, Now)!;
        Assert.Equal([new AllowanceRow("Pictures", "1,234 of 5,000")], pictures.Rows);
        // No stamp, no line.
        Assert.Null(pictures.Renews);
    }

    /// <summary>
    /// The percentage is rounded DOWN — 100 only once the allowance is used — and capped at 100; minutes used are rounded UP,
    /// so a second of use is a minute, and the limit DOWN.
    /// </summary>
    [Fact]
    public void ThePercentageAndTheMinutesRoundTheWayTheyShould()
    {
        Assert.Equal(0, PaidFamilies.Percent(0, 2_000_000));
        Assert.Equal(0, PaidFamilies.Percent(19_999, 2_000_000));
        Assert.Equal(99, PaidFamilies.Percent(1_999_999, 2_000_000));
        Assert.Equal(100, PaidFamilies.Percent(2_000_000, 2_000_000));
        Assert.Equal(100, PaidFamilies.Percent(2_400_000, 2_000_000));
        Assert.Equal(0, PaidFamilies.Percent(-5, 2_000_000));
        Assert.Equal(0, PaidFamilies.Percent(5, 0));

        Assert.Equal(0, PaidFamilies.MinutesUp(0));
        Assert.Equal(1, PaidFamilies.MinutesUp(1));
        Assert.Equal(10, PaidFamilies.MinutesUp(600_000));
        Assert.Equal(11, PaidFamilies.MinutesUp(600_001));

        var section = PaidFamilies.Allowance(
            new AllowanceDto(
                AiTokens: new AllowanceUseDto(3_000_000, 2_000_000),
                TranscriptMs: new AllowanceUseDto(1, 7_259_999)),
            Say, British, Now)!;
        Assert.Equal(
            [new AllowanceRow("Assistant", "100% used"), new AllowanceRow("Transcripts", "1 of 120 min")],
            section.Rows);
        // A limit of 0 is a limit turned off, which has no percentage to show.
        Assert.Null(PaidFamilies.Allowance(new AllowanceDto(AiTokens: new AllowanceUseDto(5, 0)), Say, British, Now));
    }

    /// <summary>In another language: the words, the order a translation puts the numbers in, and its date.</summary>
    [Fact]
    public void TheAllowanceReadsInTheReadersLanguage()
    {
        var japanese = PaidFamilies.Allowance(
            Full, JsonCatalog.For("ja"), CultureInfo.GetCultureInfo("ja-JP"), Now)!;
        Assert.Equal("今月の上限", japanese.Title);
        Assert.Equal(new AllowanceRow("画像", "30 中 4"), japanese.Rows[2]);
        Assert.Equal(new AllowanceRow("文字起こし", "120 分中 10 分"), japanese.Rows[3]);
        Assert.Equal("12月1日 にリセット", japanese.Renews);

        var russian = PaidFamilies.Allowance(Full, JsonCatalog.For("ru"), CultureInfo.GetCultureInfo("ru-RU"), Now)!;
        Assert.Equal(new AllowanceRow("Ассистент", "Использовано 40%"), russian.Rows[1]);
    }

    // ---- the refusals --------------------------------------------------------------------------------------------------------

    /// <summary>
    /// An assistant row that failed for a used-up allowance REMEMBERS it — and the day it renews, from the frame — as it remembers
    /// a refusal; a frame with no readable day names the 1st of the next UTC month.
    /// </summary>
    [Fact]
    public void AnAnswerStoppedByTheAllowanceSaysWhenItRenews()
    {
        var answers = new AssistantAnswers();
        var row = new MessageDto(5, 42, 99, null, "", "2026-10-30T12:00:00Z");
        var bare = row with { Id = 6 };

        answers.Stopped(42, 5, AiErrorReason.AllowanceUsed, new DateTimeOffset(2026, 12, 1, 0, 0, 0, TimeSpan.Zero));
        answers.Stopped(42, 6, AiErrorReason.AllowanceUsed);

        Assert.True(answers.Failed(row));
        Assert.Equal(
            "This family has used this month’s assistant allowance. It renews on 1 December.",
            answers.FailureSentence(row, Say, British, Now));
        Assert.Equal(
            "This family has used this month’s assistant allowance. It renews on 1 November.",
            answers.FailureSentence(bare, Say, British, Now));
        // A finished answer says nothing.
        Assert.Null(answers.FailureSentence(row with { EditSeq = 3 }, Say, British, Now));

        Assert.Equal(
            "Эта семья исчерпала лимит ассистента на этот месяц. Он обновится 1 декабря.",
            AssistantAnswers.Sentence(
                AiErrorReason.AllowanceUsed, new DateTimeOffset(2026, 12, 1, 0, 0, 0, TimeSpan.Zero), JsonCatalog.For("ru"),
                CultureInfo.GetCultureInfo("ru-RU"), Now));
    }

    /// <summary>
    /// <c>ai_allowance_used</c> on a transcript or a backdrop names the 1st of the next UTC month, and is final: no "Try Again".
    /// </summary>
    [Fact]
    public void ATranscriptOrABackdropPastTheAllowanceSaysWhenItRenews()
    {
        var used = new ApiError(ErrorCodes.AiAllowanceUsed, "x", 403);
        const string Sentence = "This family has used this month’s assistant allowance. It renews on 1 November.";

        Assert.False(TranscriptRules.MayRetry(used));
        Assert.Equal(Sentence, TranscriptRules.FailureSentence(used, Say, British, Now));
        Assert.Equal(Sentence, NoteSheetText.BackdropFailure(used, Say, British, Now));
        // The others are what they always were.
        Assert.Equal("Couldn't draw that.", NoteSheetText.BackdropFailure(new ApiError(ErrorCodes.StorageFull, "x", 507), Say));
        Assert.Equal(
            "Not available for this message.",
            TranscriptRules.FailureSentence(new ApiError(ErrorCodes.TranscriptNotAllowed, "x", 403), Say, British, Now));
    }

    /// <summary>
    /// <c>storage_allowance_used</c> is a refusal with words of its own wherever an upload is refused: a chat send's row, a photo
    /// pinned to the board, a sticker added to the pack.
    /// </summary>
    [Fact]
    public void AnUploadPastTheStorageAllowanceSaysSo()
    {
        const string Sentence = "This family has used its storage allowance.";
        var used = new ApiError(ErrorCodes.StorageAllowanceUsed, "x", 403);

        Assert.Equal(Sentence, PaidFamilies.SendRefusal(ErrorCodes.StorageAllowanceUsed, Say));
        Assert.Null(PaidFamilies.SendRefusal(ErrorCodes.AttachmentTooLarge, Say));
        Assert.Null(PaidFamilies.SendRefusal(null, Say));
        Assert.Equal(Sentence, NoteSheetText.Failure(used, Say));
        Assert.Equal(Sentence, PackText.Sentence(used, Say));
        Assert.Equal("Эта семья исчерпала лимит хранилища.", PaidFamilies.SendRefusal(ErrorCodes.StorageAllowanceUsed, JsonCatalog.For("ru")));
    }

    /// <summary>
    /// The upload itself: refused once, the row FAILS with the code — it is not retried, and nothing later sends it again
    /// until somebody asks.
    /// </summary>
    [Fact]
    public async Task AnUploadPastTheStorageAllowanceFailsItsRowAndIsNotRetried()
    {
        var outbox = new OutboxStore(cache);
        new ChatStore(cache, () => 7).Replace([new ChatRowDto(new ChatDto(42, "family", "Ivanovs"))]);
        var row = new OutboxRow(Guid.NewGuid().ToString(), 42, "look", PendingFiles: ["a.jpg"], StagedFiles: ["a.jpg"]);
        outbox.Queue(row);
        var server = new Server().On(
            "/attachments", """{"error": {"code": "storage_allowance_used", "message": "no"}}""", HttpStatusCode.Forbidden);
        var media = new MediaOutbox(
            outbox,
            new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken")),
            new OneFile());
        var refusals = new List<ApiError>();
        media.Refused += (_, error) => refusals.Add(error);

        Assert.Equal(0, await media.PushAsync());
        Assert.Equal(0, await media.PushAsync());

        Assert.Equal(ErrorCodes.StorageAllowanceUsed, outbox.Find(row.ClientMsgId)!.FailedCode);
        Assert.Equal(ErrorCodes.StorageAllowanceUsed, Assert.Single(refusals).Code);
        Assert.Single(server.Asked);
    }

    /// <summary>One staged photo, whatever it is asked for.</summary>
    private sealed class OneFile : IMediaStore
    {
        public StagedMedia? Read(string handle) => new("photo", "image/jpeg", new byte[] { 1, 2, 3 }, 1600, 1200);

        public void Sweep(IReadOnlySet<string> keep)
        {
        }
    }
}
