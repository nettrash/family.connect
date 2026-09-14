using System.Globalization;

namespace FamilyConnect.Core.Protocol;

/// <summary>The query strings uploads carry their facts in (docs/protocol.md, "Attachments").</summary>
public static class UploadQueries
{
    /// <summary>
    /// A place: the one upload with nothing to upload — three numbers in the query (docs/protocol.md, "Locations"), written
    /// the way the web client's <c>upload_query</c> writes them: seven places, and the accuracy as whole metres, rounded,
    /// never negative, and left out when it is not a number at all.
    /// </summary>
    public static string Location(double latitude, double longitude, double? accuracyM, string? name = null)
    {
        var query = new List<string>
        {
            "kind=location",
            // Seven places: a centimetre, and more than any fix is good for.
            "latitude=" + latitude.ToString("F7", CultureInfo.InvariantCulture),
            "longitude=" + longitude.ToString("F7", CultureInfo.InvariantCulture),
        };
        if (accuracyM is { } accuracy && double.IsFinite(accuracy))
        {
            query.Add("accuracy_m=" + ((long)Math.Max(0, Math.Round(accuracy, MidpointRounding.AwayFromZero))).ToString(CultureInfo.InvariantCulture));
        }
        if (!string.IsNullOrEmpty(name))
        {
            query.Add("name=" + Uri.EscapeDataString(name));
        }
        return string.Join("&", query);
    }
}
