namespace FamilyConnect.Core.Board;

/// <summary>
/// How a PICTURE is drawn on the wall (docs/protocol.md, "Board").
/// </summary>
/// <remarks>
/// <para>
/// A PHOTO IS DRAWN WHOLE: fitted in both dimensions and never cropped to fill its box. Issue #71
/// is what filling costs — a portrait photograph from a phone lost more than half its height on
/// every client that fitted the width alone, faces and all.
/// </para>
/// <para>
/// Web counterpart: <c>fc_text::board::fitted_picture</c>. Apple: <c>BoardPicture</c>. Android:
/// <c>BoardPicture</c>.
/// </para>
/// </remarks>
public static class BoardPicture
{
    /// <summary>
    /// The size a picture of <paramref name="pictureWidth"/> × <paramref name="pictureHeight"/>
    /// pixels takes inside a space, fitted in both dimensions: a tall photograph on a wide card
    /// comes back narrow, a wide one short, and neither comes back cropped.
    /// </summary>
    /// <remarks>
    /// Also the size of a BARE photo's card, which is the picture itself: the note's box hugs this,
    /// so the pin sits on the photograph rather than over bare wall.
    ///
    /// A picture the server never gave dimensions for takes the whole space, which costs a margin
    /// at worst — the picture is still drawn fitted inside it and never cropped.
    /// </remarks>
    public static (double Width, double Height) Fitted(
        double spaceWidth, double spaceHeight, int? pictureWidth, int? pictureHeight)
    {
        double width = pictureWidth ?? 0;
        double height = pictureHeight ?? 0;
        if (width <= 0 || height <= 0 || spaceWidth <= 0 || spaceHeight <= 0)
        {
            return (spaceWidth, spaceHeight);
        }
        var scale = Math.Min(spaceWidth / width, spaceHeight / height);
        // Never below a hairline: a panorama 20 000 pixels wide would round its height to nothing,
        // and a card of no height is a note nobody can click.
        return (Math.Max(1.0, Math.Min(spaceWidth, width * scale)),
                Math.Max(1.0, Math.Min(spaceHeight, height * scale)));
    }

    /// <summary>
    /// FITTED, never filled — the one word issue #71 was. Named rather than written into the XAML
    /// so a test can hold it, exactly as <c>NotePicture.contentMode</c> and
    /// <c>BoardPicture.scale</c> are named on Apple and Android.
    /// </summary>
    public const string Stretch = "Uniform";
}
