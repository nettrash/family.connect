namespace FamilyConnect.Core;

/// <summary>Where a profile picture's square comes from, and the side it is drawn at.</summary>
/// <param name="X">The left edge of the largest centred square of the source.</param>
/// <param name="Y">Its top edge.</param>
/// <param name="Side">Its side, in source pixels.</param>
/// <param name="Edge">The side it is drawn at: never larger than <see cref="AvatarPrep.Edge"/>, never scaled up.</param>
public readonly record struct AvatarSquare(uint X, uint Y, uint Side, uint Edge);

/// <summary>
/// A profile picture as every client uploads it — <c>fc_text::avatar</c>, held to it by
/// <c>ChatOracleTests</c>: the largest CENTRED square, at most 512 across, as the first JPEG
/// quality that fits the byte budget.
/// </summary>
/// <remarks>
/// The budget is far under the server's 256 KiB on purpose: a family server sits behind a proxy
/// whose own body limit is small, and a proxy's 413 carries none of the protocol's explanation.
/// </remarks>
public static class AvatarPrep
{
    public const uint Edge = 512;

    /// <summary>Tried in turn; the last is sent anyway when none fits.</summary>
    public static IReadOnlyList<double> Qualities { get; } = [0.8, 0.65, 0.5, 0.4];

    public const int MaxBytes = 56 * 1024;

    /// <summary>The centre-cropped square of a picture, or null for one with no pixels.</summary>
    public static AvatarSquare? Square(uint width, uint height)
    {
        var side = Math.Min(width, height);
        return side == 0
            ? null
            : new AvatarSquare((width - side) / 2, (height - side) / 2, side, Math.Min(side, Edge));
    }
}
