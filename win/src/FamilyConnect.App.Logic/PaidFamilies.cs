using System.Globalization;
using System.Text.RegularExpressions;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>What a door to a new family shows.</summary>
public enum CreateDoor
{
    /// <summary>The create form, as it always was.</summary>
    Open,

    /// <summary>
    /// The operator has shut this door (<c>family_registration_enabled</c> or <c>member_families_enabled</c> false): drawn
    /// exactly as before — the gate's create card is not drawn, and "New Family…" is not offered. It keeps priority over the
    /// subscription, as the server checks it first.
    /// </summary>
    Shut,

    /// <summary>
    /// This server charges for a new family and the account holds no active subscription: the sentence saying where a family
    /// here is started, IN PLACE of the form. "Join a family" is untouched.
    /// </summary>
    AppleApp,
}

/// <summary>
/// Paid families on this client (docs/protocol.md, "Paid families"; docs/paid-families-2026-10-30.md, issue #89). Windows
/// cannot sell the subscription — only Apple's apps can — so what it does is say so: in place of the create doors, on a
/// banner while the family shown is unpaid, in the statistics' allowance, and in the two refusals. On a server whose
/// <c>subscriptions_enabled</c> is false or absent, NOTHING here changes anything.
/// </summary>
/// <remarks>
/// <para>
/// <b>A DEADLINE IS THE READER'S DAY; A RENEWAL IS THE UTC DAY.</b> Both are drawn as a day and a month — with the year only
/// when it is not this year — through ONE formatter (<see cref="Day"/>), but not in one zone (docs/protocol.md, "UTC calendar
/// day"). <c>delete_at</c> is a moment, shown in the reader's own zone. <c>renews_at</c> starts a UTC month, and is shown as
/// its UTC calendar day — always the 1st (<see cref="RenewalDay"/>): "30 November" to a reader west of Greenwich would name a
/// month that has not ended. <c>ai_allowance_used</c> over REST names no day, and is the 1st of the next UTC month
/// (<see cref="NextMonthUtc"/>), the same instant the <c>ai_error</c> frame's <c>renews_at</c> carries — so the statistics, a
/// failed answer, a transcript and a backdrop all name the same day.
/// </para>
/// <para>
/// <b>A MEMBER'S BANNER NAMES THE OWNER, OR WAITS FOR THEM.</b> The owner is the one person who can renew, and the sentence
/// is about them by name; until this device's roster holds them (a first pass still reading it) there is no banner rather
/// than one with a hole in it. Every roster read and frame redraws it.
/// </para>
/// </remarks>
public static class PaidFamilies
{
    /// <summary>
    /// Whether starting a family needs a subscription this account does not have: the server charges, and the account's
    /// <c>subscription.active</c> is not true. Joining never needs one.
    /// </summary>
    public static bool NeedsSubscription(SessionState state) =>
        state.SubscriptionsEnabled && state.Subscription?.Active != true;

    /// <summary>
    /// The family gate's "Create a Family", for an account in no family: shut where the operator shut
    /// <c>registration</c>, the sentence where a subscription is needed, else the form.
    /// </summary>
    public static CreateDoor FirstFamily(SessionState state) =>
        !state.FamilyRegistrationEnabled ? CreateDoor.Shut
        : NeedsSubscription(state) ? CreateDoor.AppleApp
        : CreateDoor.Open;

    /// <summary>
    /// "New Family…" from inside the app (docs/protocol.md, "Several families"): shut where it is not offered at all
    /// (<see cref="FamilyRules.OffersNewFamily"/>), and where it is, the sentence instead of the form while a subscription is
    /// needed — the entry stays, so the reader learns why rather than finding it gone.
    /// </summary>
    public static CreateDoor AnotherFamily(SessionState state) =>
        !FamilyRules.OffersNewFamily(state) ? CreateDoor.Shut
        : NeedsSubscription(state) ? CreateDoor.AppleApp
        : CreateDoor.Open;

    /// <summary>What a create door says on a server that charges, to an account that has not paid.</summary>
    public static string StartsInAppleApp(IStringCatalog say) =>
        say.Get("On this server, a new family is started in the Family Connect app on iPhone, iPad or Mac.");

    /// <summary>
    /// The upper bound of the owner's member-cap picker: the family's own <c>member_ceiling</c> — a paid family's is the
    /// lower paid limit — falling back to the operator's <c>max_family_members</c> from <c>GET /me</c> on a server that
    /// predates it.
    /// </summary>
    public static int MemberCeiling(SessionState state) => MemberCeiling(state.Family, state.MaxFamilyMembers);

    /// <inheritdoc cref="MemberCeiling(SessionState)"/>
    public static int MemberCeiling(FamilyDto? family, int maxFamilyMembers) =>
        family?.MemberCeiling is { } ceiling and > 0 ? ceiling : maxFamilyMembers;

    /// <summary>
    /// The banner over the shown family's chat list while it has a <c>delete_at</c>, or null while it has none. The owner is
    /// told where to renew; a member is told who can — the owner from <paramref name="roster"/>, and no banner until the
    /// roster names them.
    /// </summary>
    /// <param name="state">The session: the family shown, and whether this account owns it.</param>
    /// <param name="roster">The shown family's roster as this device holds it.</param>
    /// <param name="say">The catalogue.</param>
    /// <param name="culture">The reader's culture, for the date.</param>
    /// <param name="zone">The reader's zone, for the date.</param>
    /// <param name="now">The clock, for whether the date needs its year.</param>
    public static string? UnpaidBanner(
        SessionState state, IReadOnlyList<MemberDto> roster, IStringCatalog say, CultureInfo culture, TimeZoneInfo zone,
        DateTimeOffset now)
    {
        ArgumentNullException.ThrowIfNull(state);
        ArgumentNullException.ThrowIfNull(roster);
        if (!state.SubscriptionsEnabled || state.Family is not { } family || Times.Instant(family.DeleteAt) is not { } at)
        {
            return null;
        }
        var date = Day(DateTimeOffset.FromUnixTimeMilliseconds(at), culture, zone, now);
        if (state.IsOwner)
        {
            return say.Format(
                "“%@” will be deleted on %@ because your subscription has ended. Renew it in the Family Connect app on iPhone, iPad or Mac.",
                family.Name, date);
        }
        var owner = roster.FirstOrDefault(member => member.Owner && !member.IsFormer);
        return owner is { DisplayName.Length: > 0 }
            ? say.Format(
                "“%@” will be deleted on %@ because its subscription has ended. Only %@, the owner, can renew it.",
                family.Name, date, owner.DisplayName)
            : null;
    }

    /// <summary>The section Family Statistics adds for a paid family, or null — no block, or no limit in it.</summary>
    /// <param name="allowance">The statistics' <c>allowance</c>, absent for a free family.</param>
    /// <param name="say">The catalogue.</param>
    /// <param name="culture">The reader's culture: sizes, numbers and the date.</param>
    /// <param name="now">The clock, for whether the date needs its year.</param>
    public static AllowanceSection? Allowance(AllowanceDto? allowance, IStringCatalog say, CultureInfo culture, DateTimeOffset now)
    {
        if (allowance is null)
        {
            return null;
        }
        string Number(long value) => Math.Max(0, value).ToString("N0", culture);
        var rows = new List<AllowanceRow>();
        if (allowance.StorageBytes is { } storage)
        {
            rows.Add(new(say.Get("Storage"), say.Format(
                "%@ of %@",
                MediaText.DisplaySize(Math.Max(0, storage.Used), say, culture),
                MediaText.DisplaySize(Math.Max(0, storage.Limit), say, culture))));
        }
        if (allowance.AiTokens is { Limit: > 0 } tokens)
        {
            rows.Add(new(say.Get("Assistant"), say.Format("%lld%% used", Percent(tokens.Used, tokens.Limit))));
        }
        if (allowance.Images is { } images)
        {
            rows.Add(new(say.Get("Pictures"), say.Format("%@ of %@", Number(images.Used), Number(images.Limit))));
        }
        if (allowance.TranscriptMs is { } transcripts)
        {
            rows.Add(new(say.Get("Transcripts"), say.Format(
                "%lld of %lld min", MinutesUp(transcripts.Used), Math.Max(0, transcripts.Limit) / MillisecondsPerMinute)));
        }
        if (rows.Count == 0)
        {
            return null;
        }
        var renews = Times.Instant(allowance.RenewsAt) is { } at
            ? say.Format("Renews on %@", RenewalDay(DateTimeOffset.FromUnixTimeMilliseconds(at), culture, now))
            : null;
        return new AllowanceSection(say.Get("This Month’s Allowance"), rows, renews);
    }

    /// <summary>The assistant's tokens as a whole percentage: used / limit × 100, rounded DOWN — 100 only once it is used up — and never past 100.</summary>
    public static long Percent(long used, long limit) =>
        limit <= 0 ? 0 : (long)Math.Min(100, Math.Floor(Math.Max(0, used) * 100.0 / limit));

    /// <summary>Transcript milliseconds as the minutes USED: rounded up, so a second of use is a minute of it.</summary>
    public static long MinutesUp(long milliseconds) =>
        milliseconds <= 0 ? 0 : (milliseconds / MillisecondsPerMinute) + (milliseconds % MillisecondsPerMinute == 0 ? 0 : 1);

    private const long MillisecondsPerMinute = 60_000;

    /// <summary>What a refused upload says when the family's storage allowance is used — a terminal refusal, never retried.</summary>
    public static string StorageAllowanceUsed(IStringCatalog say) => say.Get("This family has used its storage allowance.");

    /// <summary>
    /// What a failed send's row says about WHY, or null for a refusal it has never put into words — which keeps "Try Again"
    /// and "Delete" alone, as before.
    /// </summary>
    public static string? SendRefusal(string? failedCode, IStringCatalog say) =>
        failedCode == ErrorCodes.StorageAllowanceUsed ? StorageAllowanceUsed(say) : null;

    /// <summary>
    /// What the assistant says when the family's month is used up: an <c>ai_error</c> with <c>"allowance_used"</c> (its
    /// <c>renews_at</c>), or <c>ai_allowance_used</c> on a transcript or a backdrop (<see cref="NextMonthUtc"/>) — naming the
    /// UTC day it renews (<see cref="RenewalDay"/>).
    /// </summary>
    public static string AiAllowanceUsed(DateTimeOffset renewsAt, IStringCatalog say, CultureInfo culture, DateTimeOffset now) =>
        say.Format("This family has used this month’s assistant allowance. It renews on %@.", RenewalDay(renewsAt, culture, now));

    /// <summary>When a monthly allowance starts again: the 1st of the next calendar month, UTC.</summary>
    public static DateTimeOffset NextMonthUtc(DateTimeOffset now)
    {
        var utc = now.ToUniversalTime();
        return new DateTimeOffset(utc.Year, utc.Month, 1, 0, 0, 0, TimeSpan.Zero).AddMonths(1);
    }

    /// <summary>
    /// When a monthly allowance renews, as its UTC calendar day in the reader's words — always the 1st, wherever the reader is
    /// (docs/protocol.md, "UTC calendar day"); the year only when it is not this year's, in UTC.
    /// </summary>
    public static string RenewalDay(DateTimeOffset renewsAt, CultureInfo culture, DateTimeOffset now) =>
        Day(renewsAt, culture, TimeZoneInfo.Utc, now);

    /// <summary>
    /// An instant as a day in <paramref name="zone"/>: the culture's own day and month ("14 November", "14. November",
    /// "11月14日"), and its long date without the weekday when the day is not in this year — both in that zone. A deadline is
    /// asked for in the reader's zone; a renewal in UTC (<see cref="RenewalDay"/>).
    /// </summary>
    public static string Day(DateTimeOffset at, CultureInfo culture, TimeZoneInfo zone, DateTimeOffset now)
    {
        ArgumentNullException.ThrowIfNull(culture);
        ArgumentNullException.ThrowIfNull(zone);
        var day = TimeZoneInfo.ConvertTime(at, zone);
        var today = TimeZoneInfo.ConvertTime(now, zone);
        return day.Year == today.Year
            ? day.ToString(culture.DateTimeFormat.MonthDayPattern, culture)
            : day.ToString(WithoutWeekday(culture.DateTimeFormat.LongDatePattern), culture);
    }

    /// <summary>
    /// A long-date pattern with its weekday taken out, with the comma and the space that went with it: "dddd, MMMM d, yyyy"
    /// is "MMMM d, yyyy", "yyyy年M月d日dddd" is "yyyy年M月d日". The culture's own order and words stay.
    /// </summary>
    public static string WithoutWeekday(string pattern) =>
        Weekday.Replace(pattern, " ").Trim();

    private static readonly Regex Weekday = new(@"\s*d{3,}\s*,?\s*|,?\s*d{3,}", RegexOptions.CultureInvariant);
}

/// <summary>One row of the allowance: what it is, and how much of it is used.</summary>
public sealed record AllowanceRow(string Label, string Value);

/// <summary>The statistics' allowance section: its heading, a row per limit, and when the monthly ones renew.</summary>
public sealed record AllowanceSection(string Title, IReadOnlyList<AllowanceRow> Rows, string? Renews);
