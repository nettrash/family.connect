using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>What one member row offers the person looking at it.</summary>
/// <param name="Message">A direct chat — never with yourself, and never with somebody you blocked, which would only be refused.</param>
/// <param name="Safety">Report and block: anybody but yourself.</param>
/// <param name="OwnerMenu">The owner's tools: a birthday for anyone, themselves included.</param>
/// <param name="Removable">A password reset and a removal: never yourself, never the owner.</param>
public readonly record struct MemberTools(bool Message, bool Safety, bool OwnerMenu, bool Removable);

/// <summary>
/// The family console's rules and words — the web client's family pane (<c>web/src/views/family.rs</c>,
/// <c>report.rs</c>), which is the Mac's with the iPhone's corrections.
/// </summary>
public static class FamilyText
{
    /// <summary>The protocol's four reasons, in the apps' order, each with the key it is said by.</summary>
    public static IReadOnlyList<(string Code, string Key)> Reasons { get; } =
        [("spam", "Spam"), ("harassment", "Harassment"), ("inappropriate", "Inappropriate"), ("other", "Something else")];

    /// <summary>Chosen to start with: the reason reporting most exists for.</summary>
    public const string FirstReason = "harassment";

    /// <summary>The members still in the family, sorted by name as the Mac sorts — without regard to case.</summary>
    public static IReadOnlyList<MemberDto> Roster(IEnumerable<MemberDto> members) =>
        [.. members
            .Where(member => !member.Deleted && !member.IsFormer)
            .OrderBy(member => member.DisplayName.ToLowerInvariant(), StringComparer.Ordinal)];

    public static MemberTools ToolsFor(MemberDto member, long me, bool owner, bool blocked)
    {
        var mine = member.Id == me;
        return new MemberTools(
            Message: !mine && !blocked,
            Safety: !mine,
            OwnerMenu: owner,
            Removable: owner && !mine && !member.Owner);
    }

    public static string MembersLine(int count, IStringCatalog say) => say.Plural("%lld members", count, count);

    /// <summary>A report's reason in words — one this client does not know is "Something else".</summary>
    public static string ReasonLabel(string reason, IStringCatalog say) =>
        say.Get(Reasons.FirstOrDefault(known => known.Code == reason).Key ?? "Something else");

    public static string Reported(ReportDto report, IStringCatalog say) =>
        say.Format("%@ reported %@", report.Reporter.DisplayName, report.Reported.DisplayName);

    /// <summary>
    /// What a reported message carried, as a chat-list preview says it — for a photo sent without
    /// words, all there is to say about it.
    /// </summary>
    public static string? Carried(IReadOnlyList<AttachmentDto>? attachments, IStringCatalog say)
    {
        if (attachments is not { Count: > 0 } all)
        {
            return null;
        }
        var first = all[0];
        return first.Kind switch
        {
            "photo" when all.Count > 1 => say.Plural("%lld Photos", all.Count, all.Count),
            "photo" => say.Get("Photo"),
            "video" => say.Get("Video"),
            "audio" => say.Get("Voice message"),
            "location" => say.Get("Location"),
            _ => first.Name ?? say.Get("File"),
        };
    }

    /// <summary>
    /// MANDATORY, and a protocol requirement rather than a nicety: somebody who reports without
    /// knowing the owner will read it has been surprised by their own app.
    /// </summary>
    public static string ReportDisclosure(bool aboutMessage, IStringCatalog say) =>
        aboutMessage
            ? say.Get("Your family owner will see this message and its text.")
            : say.Get("Your family owner will be told you reported this member.");

    /// <summary>
    /// Who reads a report about the ASSISTANT, said before it is sent. NOT the family owner: a
    /// private assistant thread belongs to its member alone, so the people who run the server are
    /// the ones told — and a member reporting a reply out of that thread has to know that before
    /// they send it, not afterwards (docs/protocol.md, "Reporting the assistant").
    /// </summary>
    public static string AssistantReportDisclosure(IStringCatalog say) =>
        say.Get("The people who run this server will see this reply and what you write here. Your family will not.");

    /// <summary>
    /// A failure with nothing more particular to say. The server ANSWERING with trouble of its own —
    /// <c>internal</c>, or a status with no protocol body, a proxy's 502 while it restarts — is not
    /// "can't reach the server", which would send somebody to check a network that is fine.
    /// </summary>
    public static string GenericFailure(ApiError error, IStringCatalog say) => error switch
    {
        { Code: ErrorCodes.TooManyRequests } or { Status: 429 } => say.Get("The server is busy. Try again in a moment."),
        { Code: ErrorCodes.Internal } or { Status: >= 500 } => say.Get("The server had a problem. Try again in a moment."),
        { Code: ErrorCodes.Transport } => say.Get("Can't reach the server. Check your connection."),
        { Status: > 0, Canonical: false } => say.Get("The server had a problem. Try again in a moment."),
        _ => say.Get("That didn't work. Try again."),
    };

    /// <summary>A full family leaves the request WAITING: full is a condition, not an answer.</summary>
    public static string RequestFailure(ApiError error, IStringCatalog say) =>
        error.Code == ErrorCodes.FamilyFull
            ? say.Get("The family is full. Raise the member limit or wait for somebody to leave — the request is still waiting.")
            : GenericFailure(error, say);
}
