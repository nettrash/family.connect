using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>
/// A call record in a bubble (ios <c>CallRecordView</c>, web <c>views/bubble.rs</c>; docs/protocol.md, "The record"): how it is
/// marked, and whether it calls back.
/// </summary>
public static class CallRecords
{
    /// <summary>A call the READER missed — somebody else's, never answered — is drawn in red. The caller's own unanswered call is not.</summary>
    public static bool IsMissed(CallRecordDto call, bool mine) => call.Outcome == "missed" && !mine;

    /// <summary>
    /// Whether a record offers "Call back", and what kind of call it places. Calling back is what a record is FOR, half the
    /// time: offered in a direct chat — a call lives in one — while the server carries calls, and never on a thread's panel.
    /// A video record calls back with video only where the server still carries video; otherwise it is a voice call.
    /// </summary>
    public static (bool Offered, bool Video) CallBack(
        CallRecordDto call, bool directChat, bool callsEnabled, bool videoCallsEnabled, bool inThread) =>
        (directChat && callsEnabled && !inThread, call.Video && videoCallsEnabled);
}
