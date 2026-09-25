using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>
/// What a profile picture must be before it is sent, and what may be sent at all.
/// </summary>
/// <remarks>
/// The server stores what it is given after validating the type and the size and NEVER transcodes,
/// so the downscale is the client's job: a square JPEG whose longest edge is 512 px is what the
/// phones and the Mac send, and a Windows client that sent a 12-megapixel original would hand
/// every other client a download nobody needs (docs/protocol.md, "Profile pictures").
/// </remarks>
public static class AvatarRules
{
    /// <summary>The longest edge, after the downscale every client does before sending.</summary>
    public const int MaxEdge = 512;

    /// <summary>The two types the server takes. Nothing else is offered.</summary>
    public static readonly string[] Mimes = ["image/jpeg", "image/png"];

    /// <summary>What this client sends when it has re-encoded a picture itself.</summary>
    public const string SendAs = "image/jpeg";

    /// <summary>Whether a picture of this size has to be shrunk before it is sent.</summary>
    public static bool NeedsDownscale(int width, int height) =>
        width > MaxEdge || height > MaxEdge;

    /// <summary>
    /// The size to draw it at: the longest edge down to <see cref="MaxEdge"/>, never up — a
    /// picture already smaller is sent as it is rather than blown up to look worse.
    /// </summary>
    public static (int Width, int Height) Fit(int width, int height)
    {
        if (width <= 0 || height <= 0)
        {
            return (0, 0);
        }
        if (!NeedsDownscale(width, height))
        {
            return (width, height);
        }
        var scale = (double)MaxEdge / Math.Max(width, height);
        return (Math.Max(1, (int)Math.Round(width * scale)),
                Math.Max(1, (int)Math.Round(height * scale)));
    }

    /// <summary>Whether this is a type the server will take.</summary>
    public static bool IsAllowed(string mime) =>
        Mimes.Contains(mime, StringComparer.OrdinalIgnoreCase);
}

/// <summary>
/// Profile pictures, downloaded once per VERSION and kept.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE VERSION IS PART OF THE KEY.</b> A picture cached under the user's id alone never
/// changes when somebody changes theirs: <c>avatar_version</c> is the only signal there is — no
/// frame carries the picture, only the number — so a cache that ignores it shows a face the family
/// replaced weeks ago. It is the same mistake, in a different place, as caching a board note's
/// backdrop by the NOTE instead of by the attachment (issue #70).
/// </para>
/// <para>
/// <b>"NO PICTURE" IS AN ANSWER AND IT IS CACHED.</b> A user with none — and anybody outside the
/// family — is a 404, which is stable for that version: asking again on every redraw would be a
/// request per row per frame. A TRANSIENT failure caches nothing.
/// </para>
/// </remarks>
public sealed class AvatarCache(ApiClient api, IBlobStore blobs)
{
    /// <summary>What a missing picture is written as: present, and empty.</summary>
    private static readonly byte[] None = [];

    public static string KeyFor(long userId, int version) => $"avatar-{userId}-{version}";

    /// <summary>
    /// The bytes of somebody's picture, or null when there is none to draw — which is a real
    /// answer and not a failure: the caller draws initials.
    /// </summary>
    public async Task<(byte[]? Bytes, ApiError? Error)> BytesAsync(
        long userId, int version, CancellationToken ct = default)
    {
        if (version <= 0)
        {
            // 0 means no picture at all, and it says so without a request.
            return (null, null);
        }
        var key = KeyFor(userId, version);
        if (blobs.Read(key) is { } held)
        {
            return (held.Length == 0 ? null : held, null);
        }
        var answer = await api.Avatar(userId, ct).ConfigureAwait(false);
        if (answer.Ok && answer.Value is not null)
        {
            blobs.Write(key, answer.Value);
            return (answer.Value, null);
        }
        var error = answer.Error ?? ApiError.Transport("no answer");
        if (error.Transient)
        {
            // Nothing is written: the next look asks again rather than drawing initials for ever.
            return (null, error);
        }
        // A refusal — no picture, or not this caller's business to see. Stable for this version.
        blobs.Write(key, None);
        return (null, null);
    }
}
