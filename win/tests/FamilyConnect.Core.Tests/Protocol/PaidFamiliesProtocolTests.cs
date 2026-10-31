using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.Core.Tests.Protocol;

/// <summary>
/// Paid families on the wire (docs/protocol.md, "Paid families", issue #89): <c>GET /me</c>'s new keys, the family's plan,
/// ceiling and deadline, the statistics' allowance, the six new codes, the <c>family_plan</c> frame, and <c>ai_error</c>'s
/// <c>"allowance_used"</c> with its <c>renews_at</c>. Every one of them absent is the old behaviour.
/// </summary>
public class PaidFamiliesProtocolTests : IDisposable
{
    private readonly Database database = Database.OpenInMemory();

    public void Dispose() => database.Dispose();

    private const string User = """{"id": 7, "username": "anna", "display_name": "Anna"}""";

    /// <summary>
    /// With subscriptions on, <c>GET /me</c> carries the switch, the caller's subscription, what a paid family gets and its
    /// days — and Apple's two keys, which this client has no use for and skips.
    /// </summary>
    [Fact]
    public void GetMeCarriesTheSubscriptionAndTheLimits()
    {
        var me = Wire.Decode<MeResponse>($$$"""
            {"user": {{{User}}}, "blocked_user_ids": [], "subscriptions_enabled": true,
             "subscription_products": ["me.nettrash.FamilyConnect.family.monthly", "me.nettrash.FamilyConnect.family.yearly"],
             "app_account_token": "6f1c3a2e-0000-4000-8000-000000000000",
             "unpaid_days": 14,
             "paid_family_limits": {"members": 25, "storage_bytes": 10737418240, "ai_tokens": 2000000, "images": 30,
                                    "transcript_ms": 7200000},
             "subscription": {"product_id": "me.nettrash.FamilyConnect.family.yearly", "environment": "Sandbox",
                              "expires_at": "2026-12-01T10:00:00Z", "active": true, "renews": false}}
            """)!;

        Assert.True(me.SubscriptionsEnabled);
        Assert.Equal(14, me.UnpaidDays);
        Assert.Equal(new PaidFamilyLimitsDto(25, 10_737_418_240, 2_000_000, 30, 7_200_000), me.PaidFamilyLimits);
        Assert.Equal(
            new SubscriptionDto("me.nettrash.FamilyConnect.family.yearly", "Sandbox", "2026-12-01T10:00:00Z", true, false),
            me.Subscription);
    }

    /// <summary>
    /// A server from before paid families says none of it, and every key reads as the old behaviour: off, no subscription, no
    /// limits. A subscription that ended is <c>active: false</c>, and an absent <c>renews</c> is "the server does not know".
    /// </summary>
    [Fact]
    public void AnOlderServerSaysNothingAndThatIsOff()
    {
        var older = Wire.Decode<MeResponse>($$"""{"user": {{User}}, "blocked_user_ids": []}""")!;
        Assert.False(older.SubscriptionsEnabled);
        Assert.Null(older.Subscription);
        Assert.Null(older.PaidFamilyLimits);
        Assert.Equal(0, older.UnpaidDays);

        var lapsed = Wire.Decode<MeResponse>($$$"""
            {"user": {{{User}}}, "subscriptions_enabled": true, "subscription": {"product_id": "x", "environment": "Production",
             "expires_at": "2026-10-01T00:00:00Z", "active": false}, "paid_family_limits": {"members": 25}}
            """)!;
        Assert.False(lapsed.Subscription!.Active);
        Assert.Null(lapsed.Subscription.Renews);
        // Each limit present only while it is on.
        Assert.Equal(new PaidFamilyLimitsDto(Members: 25), lapsed.PaidFamilyLimits);

        var none = Wire.Decode<MeResponse>($$"""{"user": {{User}}, "subscriptions_enabled": true, "subscription": null}""")!;
        Assert.True(none.SubscriptionsEnabled);
        Assert.Null(none.Subscription);
    }

    /// <summary>
    /// The family's ceiling, plan and deadline, on <c>GET /me</c>'s families and on the family's own document alike — and on
    /// a free family, or an older server, the deadline and the plan are simply absent.
    /// </summary>
    [Fact]
    public void TheFamilyCarriesItsCeilingPlanAndDeadline()
    {
        var unpaid = Wire.Decode<FamilyDto>(
            """{"id": 3, "name": "Ivanovs", "role": "member", "plan": "paid", "member_ceiling": 25, "delete_at": "2026-11-14T09:00:00Z"}""")!;
        Assert.Equal("paid", unpaid.Plan);
        Assert.Equal(25, unpaid.MemberCeiling);
        Assert.Equal("2026-11-14T09:00:00Z", unpaid.DeleteAt);

        var free = Wire.Decode<FamilyResponse>(
            """{"family": {"id": 3, "name": "Ivanovs", "plan": "free", "member_ceiling": 50}, "members": []}""")!.Family;
        Assert.Equal("free", free.Plan);
        Assert.Null(free.DeleteAt);

        var older = Wire.Decode<FamilyDto>("""{"id": 3, "name": "Ivanovs"}""")!;
        Assert.Null(older.Plan);
        Assert.Null(older.MemberCeiling);
        Assert.Null(older.DeleteAt);
    }

    /// <summary>The statistics' allowance, exactly as the protocol writes it — and absent for a free family.</summary>
    [Fact]
    public void TheStatisticsCarryAPaidFamilysAllowance()
    {
        var stats = Wire.Decode<StatsResponse>("""
            {"generated_at": "2026-11-03T10:00:00Z", "totals": {"members": 4, "messages": 10, "board_notes": 0}, "members": [],
             "allowance": {"renews_at": "2026-12-01T00:00:00Z",
                           "storage_bytes": {"used": 3221225472, "limit": 10737418240},
                           "ai_tokens": {"used": 812345, "limit": 2000000},
                           "images": {"used": 4, "limit": 30},
                           "transcript_ms": {"used": 600000, "limit": 7200000}}}
            """)!;
        Assert.Equal(
            new AllowanceDto(
                "2026-12-01T00:00:00Z",
                new AllowanceUseDto(3_221_225_472, 10_737_418_240),
                new AllowanceUseDto(812_345, 2_000_000),
                new AllowanceUseDto(4, 30),
                new AllowanceUseDto(600_000, 7_200_000)),
            stats.Allowance);

        // A limit the operator turned off is absent from the block.
        var some = Wire.Decode<StatsResponse>("""
            {"totals": {"members": 1, "messages": 0, "board_notes": 0},
             "allowance": {"renews_at": "2026-12-01T00:00:00Z", "images": {"used": 0, "limit": 30}}}
            """)!;
        Assert.Null(some.Allowance!.StorageBytes);
        Assert.Null(some.Allowance.AiTokens);
        Assert.Null(some.Allowance.TranscriptMs);

        var free = Wire.Decode<StatsResponse>("""{"totals": {"members": 1, "messages": 0, "board_notes": 0}}""")!;
        Assert.Null(free.Allowance);
    }

    /// <summary>The six codes of 2026-10-30, spelled as the protocol spells them, canonical, and every one a refusal.</summary>
    [Fact]
    public void TheSixNewCodesAreCanonicalRefusals()
    {
        foreach (var (code, wire, status) in new[]
                 {
                     (ErrorCodes.SubscriptionRequired, "subscription_required", 403),
                     (ErrorCodes.SubscriptionInvalid, "subscription_invalid", 400),
                     (ErrorCodes.SubscriptionTaken, "subscription_taken", 409),
                     (ErrorCodes.SubscriptionsDisabled, "subscriptions_disabled", 403),
                     (ErrorCodes.StorageAllowanceUsed, "storage_allowance_used", 403),
                     (ErrorCodes.AiAllowanceUsed, "ai_allowance_used", 403),
                 })
        {
            Assert.Equal(wire, code);
            Assert.Contains(code, ErrorCodes.All);
            Assert.True(new ApiError(code, "x", status).Canonical);
            Assert.False(new ApiError(code, "x", status).Transient);
            Assert.False(new ApiError(code, "x").Transient);
        }
        // Not the server's own disk: two different refusals.
        Assert.NotEqual(ErrorCodes.StorageFull, ErrorCodes.StorageAllowanceUsed);
    }

    /// <summary>
    /// <c>family_plan</c> names its family and carries its plan and deadline. An ABSENT <c>delete_at</c> — or a null, or one
    /// that is not a string — is the deadline gone; a frame with no family to name is no frame.
    /// </summary>
    [Fact]
    public void AFamilyPlanFrameNamesItsFamilyAndItsDeadline()
    {
        Assert.Equal(
            new ServerFrame.FamilyPlan(3, "paid", "2026-11-14T09:00:00Z"),
            ServerFrame.Parse("""{"type": "family_plan", "family_id": 3, "plan": "paid", "delete_at": "2026-11-14T09:00:00Z"}"""));
        Assert.Equal(
            new ServerFrame.FamilyPlan(3, "paid", null),
            ServerFrame.Parse("""{"type": "family_plan", "family_id": 3, "plan": "paid"}"""));
        Assert.Equal(
            new ServerFrame.FamilyPlan(3, "paid", null),
            ServerFrame.Parse("""{"type": "family_plan", "family_id": 3, "plan": "paid", "delete_at": null}"""));
        Assert.Equal(
            new ServerFrame.FamilyPlan(3, null, null),
            ServerFrame.Parse("""{"type": "family_plan", "family_id": 3, "plan": 7, "delete_at": false}"""));
        Assert.Null(ServerFrame.Parse("""{"type": "family_plan", "plan": "paid", "delete_at": "2026-11-14T09:00:00Z"}"""));
        Assert.Null(ServerFrame.Parse("""{"type": "family_plan", "family_id": 0, "plan": "free"}"""));
    }

    /// <summary>
    /// <c>"allowance_used"</c> is a reason this client knows, and it carries WHEN the allowance renews. A <c>renews_at</c>
    /// that is absent or unreadable is none — the reason still stands — and beside any other reason it is not read at all.
    /// </summary>
    [Fact]
    public void AnAssistantFailureCanSayTheAllowanceIsUsedAndWhenItRenews()
    {
        var renews = new DateTimeOffset(2026, 12, 1, 0, 0, 0, TimeSpan.Zero);
        Assert.Equal(
            new ServerFrame.AiError(42, 1339, AiErrorReason.AllowanceUsed, renews),
            ServerFrame.Parse("""
                {"type": "ai_error", "chat_id": 42, "message_id": 1339, "reason": "allowance_used",
                 "renews_at": "2026-12-01T00:00:00Z"}
                """));
        Assert.Equal(
            new ServerFrame.AiError(42, 1339, AiErrorReason.AllowanceUsed),
            ServerFrame.Parse("""{"type": "ai_error", "chat_id": 42, "message_id": 1339, "reason": "allowance_used"}"""));
        foreach (var unreadable in new[] { "\"soon\"", "7", "null", "{}" })
        {
            Assert.Equal(
                new ServerFrame.AiError(42, 1339, AiErrorReason.AllowanceUsed),
                ServerFrame.Parse(
                    $$"""{"type": "ai_error", "chat_id": 42, "message_id": 1339, "reason": "allowance_used", "renews_at": {{unreadable}}}"""));
        }
        Assert.Equal(
            new ServerFrame.AiError(42, 1339, AiErrorReason.Refused),
            ServerFrame.Parse("""
                {"type": "ai_error", "chat_id": 42, "message_id": 1339, "reason": "refused", "renews_at": "2026-12-01T00:00:00Z"}
                """));
        // Spelled differently it is a reason nobody defined, and an unknown reason is ABSENT.
        Assert.Equal(
            new ServerFrame.AiError(42, 1339),
            ServerFrame.Parse("""
                {"type": "ai_error", "chat_id": 42, "message_id": 1339, "reason": "Allowance_Used", "renews_at": "2026-12-01T00:00:00Z"}
                """));
    }

    /// <summary>
    /// The router passes a plan on to whoever holds the families — nothing about it is in the cache — and passes the renewal
    /// on with a used-up allowance.
    /// </summary>
    [Fact]
    public void TheRouterPassesThePlanAndTheRenewalOn()
    {
        var chats = new ChatStore(database, () => 7);
        var router = new FrameRouter(chats, new BoardStore(database));
        var plans = new List<ServerFrame.FamilyPlan>();
        var stopped = new List<(AiErrorReason? Reason, DateTimeOffset? RenewsAt)>();
        var changed = 0;
        router.FamilyPlanChanged += plans.Add;
        router.AiStopped += (_, _, reason, renews) => stopped.Add((reason, renews));
        router.ChatChanged += _ => changed++;
        router.RosterChanged += () => changed++;

        router.Hear(new ServerFrame.FamilyPlan(3, "paid", "2026-11-14T09:00:00Z"));
        router.Hear(new ServerFrame.FamilyPlan(3, "paid", null));
        var renews = new DateTimeOffset(2026, 12, 1, 0, 0, 0, TimeSpan.Zero);
        router.Hear(new ServerFrame.AiError(42, 1339, AiErrorReason.AllowanceUsed, renews));

        Assert.Equal([new(3, "paid", "2026-11-14T09:00:00Z"), new(3, "paid", null)], plans);
        Assert.Equal([(AiErrorReason.AllowanceUsed, renews)], stopped);
        Assert.Equal(0, changed);
    }
}
