namespace FamilyConnect.Core;

/// <summary>
/// The quote above a reply: the quoted body cut to at most 120 Unicode SCALAR values, never
/// mid-scalar — exactly as the server cuts <c>reply_to.excerpt</c> (docs/protocol.md, "Replies").
/// </summary>
/// <remarks>
/// A client cuts its own in two places and must cut identically, or the quote visibly changes
/// length when the server's copy lands: while its own reply is still pending, and when an edit
/// changes a message its local replies quote. Scalars, not grapheme clusters: a cut may land
/// inside a family emoji, because every client can reproduce that with nothing but a scalar count.
/// Held to <c>fc_text::excerpt</c> by <c>ChatOracleTests</c>.
/// </remarks>
public static class Excerpt
{
    public const int MaxScalars = 120;

    public static string Cut(string body)
    {
        var index = 0;
        var scalars = 0;
        while (index < body.Length)
        {
            if (scalars == MaxScalars)
            {
                return body[..index];
            }
            index += char.IsSurrogatePair(body, index) ? 2 : 1;
            scalars++;
        }
        return body;
    }
}
