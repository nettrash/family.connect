using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>
/// What a bubble is, beyond its words — the web client's <c>timeline</c> and bubble rules (ios
/// <c>MessagePresentation</c>): who counts as the assistant, an answer still being written, the seen tick, and who
/// may be reported or blocked from a message.
/// </summary>
public static class BubbleRules
{
    /// <summary>The assistant: its reserved account in the family chat, and anybody but the reader in its own chat.</summary>
    public static bool IsAssistant(MessageDto message, long me, bool assistantChat, long? assistantUserId) =>
        message.SenderId == assistantUserId || (assistantChat && message.SenderId != me);

    /// <summary>A member other than the reader — who the Safety items are about. Never the assistant.</summary>
    public static bool IsOtherMember(MessageDto message, long me, bool assistantChat, long? assistantUserId) =>
        message.SenderId != me && !IsAssistant(message, me, assistantChat, assistantUserId);

    /// <summary>
    /// An assistant answer not written yet: asked of the MESSAGE and nothing else — a numbered row carrying nothing,
    /// not the reader's, from somebody who can be the assistant. A set of ids a live delta touched would forget the
    /// state on relaunch, which is the blank bubble this rule exists to remove. <paramref name="body"/> is the body
    /// as drawn, streamed text included.
    /// </summary>
    public static bool Awaited(MessageDto message, string body, long me, bool assistantChat, long? assistantUserId) =>
        body.Length == 0
        && message.Media.Count == 0
        && message.Poll is null
        && message.Call is null
        && message.SenderId != me
        && message.Id != 0
        && (assistantChat || message.SenderId == assistantUserId);

    /// <summary>
    /// Whether the reader's own message carries a tick: a numbered one, anywhere but the family chat — where a
    /// seen state over many members is a row of faces nobody asked for (docs/protocol.md, "read" frames).
    /// </summary>
    public static bool ShowsTick(MessageDto message, long me, bool familyChat) =>
        !familyChat && message.SenderId == me && message.Id != 0;

    /// <summary>The tick is the double one: the other person's marker has reached it.</summary>
    public static bool Seen(MessageDto message, long me, bool familyChat, long peerReadUpTo) =>
        ShowsTick(message, me, familyChat) && message.Id <= peerReadUpTo;

    /// <summary>A message may be reported once it is numbered, and only another member's — hidden or not.</summary>
    public static bool MayReport(MessageDto message, long me, bool assistantChat, long? assistantUserId) =>
        message.Id != 0 && IsOtherMember(message, me, assistantChat, assistantUserId);

    /// <summary>
    /// An ASSISTANT reply may be reported, once it is numbered — a separate path from
    /// <see cref="MayReport"/> and deliberately so (docs/protocol.md, "Reporting the assistant").
    /// The assistant belongs to no family, so the member-report endpoint refuses it with
    /// <c>not_same_family</c>; what a member needs here is to say that a MODEL got something wrong,
    /// which is the operator's business and not the family owner's.
    /// </summary>
    public static bool MayReportAssistant(MessageDto message, long me, bool assistantChat, long? assistantUserId) =>
        message.Id != 0 && IsAssistant(message, me, assistantChat, assistantUserId);
}
