namespace FamilyConnect.Core.Protocol;

/// <summary>
/// What the user types, turned into the two URLs this client speaks
/// (docs/protocol.md, "Transport"): REST under <c>{base}/api/v1</c> and the socket at
/// <c>{base}/api/v1/ws</c>, <c>wss://</c> for an https server and <c>ws://</c> for an http one.
/// </summary>
/// <remarks>
/// People type <c>chat.example.com</c>, <c>https://chat.example.com/</c> and
/// <c>192.168.1.10:8080</c>, and every client normalises the same way (the Apple client's
/// ServerURLNormalizer): a bare host gets <c>https://</c>, a trailing slash goes, and an explicit
/// scheme is kept — a family running on a LAN with no certificate types <c>http://</c> and means it.
/// </remarks>
public static class ServerUrl
{
    /// <summary>The normalised base, or null when there is no host in it at all.</summary>
    public static Uri? Normalise(string? typed)
    {
        var text = typed?.Trim();
        if (string.IsNullOrEmpty(text))
        {
            return null;
        }
        if (!text.Contains("://", StringComparison.Ordinal))
        {
            // A bare host is https: the one default that is safe to guess wrong in the strict
            // direction — a plain-text server has to be asked for by name.
            text = "https://" + text;
        }
        if (!Uri.TryCreate(text, UriKind.Absolute, out var url))
        {
            return null;
        }
        if (url.Scheme != Uri.UriSchemeHttp && url.Scheme != Uri.UriSchemeHttps)
        {
            return null;
        }
        if (string.IsNullOrEmpty(url.Host))
        {
            return null;
        }
        // The path is dropped on purpose: `{base}` is an origin, and a base with a path would
        // put `/api/v1` under whatever the user happened to paste.
        var builder = new UriBuilder(url.Scheme, url.Host) { Path = "/" };
        if (!url.IsDefaultPort)
        {
            builder.Port = url.Port;
        }
        return builder.Uri;
    }

    /// <summary>The REST root: <c>{base}/api/v1</c>, with no trailing slash.</summary>
    public static Uri Rest(Uri baseUrl) => new(baseUrl, "/api/v1");

    /// <summary>
    /// The socket URL. The token never travels in it: a native client sends
    /// <c>Authorization: Bearer</c> on the upgrade, which is what this one does — the two
    /// subprotocols are the browser's workaround, because a browser cannot set headers on an
    /// upgrade. "A token in the query string is not a token at all" (docs/protocol.md,
    /// "WebSocket protocol"), and a URL is written into every proxy log there is.
    /// </summary>
    public static Uri Socket(Uri baseUrl)
    {
        var scheme = baseUrl.Scheme == Uri.UriSchemeHttps ? "wss" : "ws";
        var builder = new UriBuilder(baseUrl) { Scheme = scheme, Path = "/api/v1/ws" };
        return builder.Uri;
    }
}
