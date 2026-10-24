using System.Globalization;

namespace FamilyConnect.Core.Store;

/// <summary>
/// How a store that keeps one of something PER FAMILY spells its <c>meta</c> keys (docs/protocol.md, "Several families"): the
/// key a cache from before several families used, with <c>@</c> and the family's id after it — <c>board.cursor.board_seq@3</c>.
/// No family is the old key itself, which is what the tests and a cache not yet adopted read
/// (<see cref="Database.AdoptLegacy"/>).
/// </summary>
internal static class FamilyKeys
{
    public static string Of(string key, long? familyId) =>
        familyId is { } family ? key + "@" + family.ToString(CultureInfo.InvariantCulture) : key;
}
