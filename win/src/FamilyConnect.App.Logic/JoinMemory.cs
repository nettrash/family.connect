namespace FamilyConnect.App.Logic;

/// <summary>
/// Whose join request this device was waiting on. A refusal is never said by the server — the request just vanishes from
/// <c>GET /me</c> — so only a client that REMEMBERS waiting can tell "declined" from "never asked" (web <c>session.rs</c>
/// <c>awaiting_join</c>, ios <c>AppSettings.joinPending</c>), and a relaunch must not forget it.
/// </summary>
public interface IAwaitingJoin
{
    long? UserId { get; set; }
}

/// <summary>For a session that need not outlive its process — the tests.</summary>
public sealed class MemoryAwaitingJoin : IAwaitingJoin
{
    public long? UserId { get; set; }
}

/// <summary>
/// The composer's two assistant doors (web <c>composer.rs</c> <c>offers_ai</c> and <c>can_draw</c>, the Mac's sparkles and
/// paintbrush): "Ask the assistant" appends <c>@ai</c> in the family chat of a server that has an assistant; "Ask for a
/// picture" puts <c>/draw</c> first in the assistant's own chat when it can draw. Neither while a message is being edited —
/// an edit rewrites what was said, and a request cannot be added to it.
/// </summary>
public static class AssistantButtons
{
    public static (bool AskAssistant, bool AskPicture) Offered(string? chatKind, Core.Protocol.AssistantDto? assistant, bool editing) =>
        (chatKind == "family" && assistant is not null && !editing,
         chatKind == "ai" && assistant is { Images: true } && !editing);
}
