using System.Net.WebSockets;

namespace FamilyConnect.Core.Protocol;

/// <summary>
/// One text-frame WebSocket, behind an interface so the loop above it can be tested without a
/// server.
/// </summary>
/// <remarks>
/// The shape is deliberately small — connect, send text, receive text, close — because everything
/// interesting about this transport is the LOOP (<see cref="ChatSocket"/>), and a loop that can
/// only be exercised against a real endpoint is a loop nobody exercises.
/// </remarks>
public interface IWebSocket : IDisposable
{
    /// <summary>
    /// Open the upgrade. The token goes in <c>Authorization: Bearer</c> — a native client can set
    /// headers, and "a token in the query string is not a token at all" (docs/protocol.md,
    /// "WebSocket protocol"). The two subprotocols are the browser's workaround and not ours.
    /// </summary>
    Task ConnectAsync(Uri url, string? bearer, CancellationToken ct);

    Task SendAsync(string text, CancellationToken ct);

    /// <summary>The next frame, or null once the connection has closed.</summary>
    Task<string?> ReceiveAsync(CancellationToken ct);

    /// <summary>Why it closed, once it has.</summary>
    int? CloseCode { get; }
}

/// <summary>The real one.</summary>
public sealed class ClientWebSocketAdapter : IWebSocket
{
    private readonly ClientWebSocket socket = new();
    private readonly byte[] buffer = new byte[64 * 1024];

    public int? CloseCode => socket.CloseStatus is { } status ? (int)status : null;

    public async Task ConnectAsync(Uri url, string? bearer, CancellationToken ct)
    {
        if (bearer is { Length: > 0 })
        {
            socket.Options.SetRequestHeader("Authorization", "Bearer " + bearer);
        }
        // The server pings; this keeps the OS from tearing an idle connection down under us.
        socket.Options.KeepAliveInterval = TimeSpan.FromSeconds(30);
        await socket.ConnectAsync(url, ct).ConfigureAwait(false);
    }

    public Task SendAsync(string text, CancellationToken ct) =>
        socket.SendAsync(
            System.Text.Encoding.UTF8.GetBytes(text), WebSocketMessageType.Text,
            endOfMessage: true, ct);

    public async Task<string?> ReceiveAsync(CancellationToken ct)
    {
        var text = new System.Text.StringBuilder();
        while (true)
        {
            var received = await socket.ReceiveAsync(buffer, ct).ConfigureAwait(false);
            if (received.MessageType == WebSocketMessageType.Close)
            {
                return null;
            }
            text.Append(System.Text.Encoding.UTF8.GetString(buffer, 0, received.Count));
            if (received.EndOfMessage)
            {
                return text.ToString();
            }
        }
    }

    public void Dispose() => socket.Dispose();
}

/// <summary>
/// The realtime connection: one socket at a time, reconnected with full jitter, with the frames
/// handed to whoever is listening (docs/protocol.md, "WebSocket protocol").
/// </summary>
/// <remarks>
/// <para>
/// Three rules this loop exists to keep. A CONNECTION THAT DIED ON ARRIVAL EARNS NOTHING: a proxy
/// can accept the upgrade and drop it at once, and forgiving the ceiling at handshake made the
/// socket reconnect roughly twice a second for ever, each cycle firing a full resync — so the
/// reset is judged by how long the connection LASTED. An UNKNOWN FRAME IS IGNORED, because a newer
/// server may add one and dropping the connection over it would be worse than skipping it. And
/// <c>4401</c> IS NOT A NETWORK ERROR: the session expired mid-connection, so the client signs out
/// instead of retrying a token that will never work again.
/// </para>
/// <para>
/// Apple counterpart: <c>ChatSocket</c>. Android: <c>ChatSocketImpl</c>. Web: <c>web/src/socket.rs</c>.
/// </para>
/// </remarks>
public sealed class ChatSocket
{
    /// <summary>The close code the server sends when the session expires mid-connection.</summary>
    public const int SessionExpiredCode = 4401;

    /// <summary>
    /// How long a connection has to last to earn its reset. Shorter than this and the ceiling
    /// keeps climbing, which is what stops a flapping proxy becoming a resync storm.
    /// </summary>
    public static readonly TimeSpan DurableAfter = TimeSpan.FromSeconds(5);

    private readonly Func<IWebSocket> open;
    private readonly Func<Uri> url;
    private readonly ITokenStore tokens;
    private readonly ReconnectBackoff backoff;
    private readonly Func<TimeSpan, CancellationToken, Task> wait;
    private readonly Func<DateTimeOffset> now;

    private IWebSocket? live;

    public ChatSocket(
        Func<IWebSocket> open,
        Func<Uri> url,
        ITokenStore tokens,
        ReconnectBackoff? backoff = null,
        Func<TimeSpan, CancellationToken, Task>? wait = null,
        Func<DateTimeOffset>? now = null)
    {
        this.open = open;
        this.url = url;
        this.tokens = tokens;
        this.backoff = backoff ?? new ReconnectBackoff();
        this.wait = wait ?? Task.Delay;
        this.now = now ?? (() => DateTimeOffset.UtcNow);
    }

    /// <summary>Every frame this client understood, in arrival order.</summary>
    public event Action<ServerFrame>? Frame;

    /// <summary>A connection came up. The resync starts here, and so does the outbox flush.</summary>
    public event Action? Connected;

    /// <summary>It went away. Whether it comes back is this loop's business.</summary>
    public event Action? Disconnected;

    /// <summary>The session is gone: sign out, and do not reconnect.</summary>
    public event Action? SessionExpired;

    /// <summary>Whether a frame can be written right now.</summary>
    public bool IsConnected => live is not null;

    /// <summary>
    /// Connect, read frames, and reconnect when the connection goes — until
    /// <paramref name="ct"/> is cancelled or the session expires.
    /// </summary>
    public async Task RunAsync(CancellationToken ct)
    {
        while (!ct.IsCancellationRequested)
        {
            var socket = open();
            DateTimeOffset? connectedAt = null;
            var expired = false;
            try
            {
                await socket.ConnectAsync(url(), tokens.Token, ct).ConfigureAwait(false);
                connectedAt = now();
                live = socket;
                Connected?.Invoke();
                while (!ct.IsCancellationRequested)
                {
                    var text = await socket.ReceiveAsync(ct).ConfigureAwait(false);
                    if (text is null)
                    {
                        break;
                    }
                    // An unknown type parses to null and is skipped: a client that threw would
                    // drop the connection over something it was free to ignore.
                    if (ServerFrame.Parse(text) is { } frame)
                    {
                        Frame?.Invoke(frame);
                    }
                }
                expired = socket.CloseCode == SessionExpiredCode;
            }
            catch (OperationCanceledException) when (ct.IsCancellationRequested)
            {
                // The caller asked. Not a failure, and nothing to retry.
            }
            catch (Exception exception) when (exception is WebSocketException or HttpRequestException
                                                  or IOException or TimeoutException
                                                  or InvalidOperationException)
            {
                // Any transport failure is the same thing here: no connection. What it says about
                // the session is decided by the close code, not by the exception.
                expired = socket.CloseCode == SessionExpiredCode;
            }
            finally
            {
                live = null;
                socket.Dispose();
            }
            if (connectedAt is not null)
            {
                Disconnected?.Invoke();
            }
            if (expired)
            {
                // NOT a network error: the token will never work again, so retrying it would be a
                // client hammering a door it has no key to.
                SessionExpired?.Invoke();
                return;
            }
            if (ct.IsCancellationRequested)
            {
                return;
            }
            if (ReconnectBackoff.EarnsReset(connectedAt, now(), DurableAfter))
            {
                backoff.Reset();
            }
            try
            {
                await wait(backoff.NextDelay(), ct).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                return;
            }
        }
    }

    /// <summary>
    /// Write one frame, with the ack deadline as the WRITE deadline too: a socket whose TCP
    /// connection is dead absorbs a write, and the keepalive horizon is far longer than a person's
    /// patience (docs/protocol.md, "Sending on an unreliable network").
    /// </summary>
    /// <returns>Whether it went. False is not a refusal — the outbox decides what that means.</returns>
    public async Task<bool> TrySend(string frame, CancellationToken ct = default)
    {
        var socket = live;
        if (socket is null)
        {
            return false;
        }
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(ct);
        deadline.CancelAfter(SendRules.AckDeadline);
        try
        {
            await socket.SendAsync(frame, deadline.Token).ConfigureAwait(false);
            return true;
        }
        catch (Exception exception) when (exception is OperationCanceledException
                                              or WebSocketException or IOException
                                              or InvalidOperationException)
        {
            return false;
        }
    }
}
