using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>
/// Today's weather in the daily greeting, from a client's side (docs/protocol.md, "Today's weather, for places the owner
/// chose"; issue #72): whether the family screen offers the places field, whether it may be edited now, and what the
/// server holds.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE OWNER'S, AND ONLY WHERE THE SERVER SAYS SO.</b> The field is drawn for the owner alone — every other AI setting
/// of the family is too, and a member's screen has no assistant card at all — and only where <c>assistant.greeting_weather</c>
/// is true. An older server sends no such key, which reads as false: no field, rather than a list that does nothing.
/// </para>
/// <para>
/// <b>EDITABLE WHETHER OR NOT THE GREETING IS ON.</b> The list is the family's, kept by the server whatever
/// <c>ai_greeting</c> says, and an owner may choose the places first and turn the greeting on afterwards — so the field is
/// editable with the switch off as with it on (protocol.md, "The field is editable whether or not the greeting is on"),
/// as it is on iOS, Android and the web. Only another change of the family's on its way holds it.
/// </para>
/// <para>
/// <b>WHAT IS SHOWN AFTER A SAVE IS THE SERVER'S LIST</b>, from its answer: it may be shorter than what was sent (a repeat,
/// dropped) or spelt with less whitespace, and a server that predates the key keeps nothing at all.
/// </para>
/// </remarks>
public static class GreetingWeather
{
    /// <summary>Whether this server can put the weather in the greeting at all. A missing key is an older server: no.</summary>
    public static bool Offered(AssistantDto? assistant) => assistant?.GreetingWeather == true;

    /// <summary>Whether the family screen draws the places field: the owner's screen, on a server that offers it.</summary>
    public static bool Shown(SessionState state)
    {
        ArgumentNullException.ThrowIfNull(state);
        return state.IsOwner && Offered(state.Assistant);
    }

    /// <summary>
    /// Whether the places may be edited right now: whenever no other change of the family's is on its way — with the
    /// greeting on or off.
    /// </summary>
    public static bool Editable(bool idle) => idle;

    /// <summary>The places the server holds — none for an absent list (an older server) and none for a null inside one.</summary>
    public static IReadOnlyList<string> Saved(FamilyDto? family) =>
        family?.GreetingPlaces is { } places ? [.. places.Where(place => place is not null)] : [];
}

/// <summary>
/// The places field as it is being edited: up to three text fields, what they would send, and how they follow the server
/// without throwing away what the owner is typing.
/// </summary>
/// <remarks>
/// <para>
/// <b>A REDRAW IS NOT A RESET.</b> The family screen redraws on every frame that touches the family, and the server's list
/// only replaces the fields when it CHANGED and the fields hold nothing unsaved; even then, the empty fields at the end —
/// the ones the owner has just added — stay. What is typed while a save is on its way is kept, and is the next save.
/// </para>
/// <para>
/// <b>NOTHING IS SENT THAT CHANGES NOTHING</b>, and nothing is sent that the server would refuse: a blank field is no
/// place, and every rule a name must keep is kept as it is typed (<see cref="GreetingPlaces.Typed"/>).
/// </para>
/// </remarks>
public sealed class PlacesDraft
{
    private readonly List<string> fields = [];
    private IReadOnlyList<string> basis = [];

    /// <summary>What each field holds, in order.</summary>
    public IReadOnlyList<string> Fields => fields;

    /// <summary>The list this draft was last measured against: the server's, as far as this screen knows.</summary>
    public IReadOnlyList<string> Basis => basis;

    /// <summary>Whether another field may be added: fewer than three listed.</summary>
    public bool CanAdd => fields.Count < GreetingPlaces.MaxPlaces;

    /// <summary>The fields as a request, folded and with the blank ones left out.</summary>
    public GreetingPlaces.Checked Request => GreetingPlaces.Check(fields);

    /// <summary>Whether the fields ask for something other than what the server holds.</summary>
    public bool Unsaved => !Same(Request.Names, basis);

    /// <summary>One more, empty, field — while there are fewer than three.</summary>
    public bool Add()
    {
        if (!CanAdd)
        {
            return false;
        }
        fields.Add(string.Empty);
        return true;
    }

    /// <summary>Take one field out. An index that is not there changes nothing.</summary>
    public bool Remove(int index)
    {
        if (index < 0 || index >= fields.Count)
        {
            return false;
        }
        fields.RemoveAt(index);
        return true;
    }

    /// <summary>What a field now holds, as typed — answered as it will be kept (<see cref="GreetingPlaces.Typed"/>).</summary>
    public string Set(int index, string? text)
    {
        var typed = GreetingPlaces.Typed(text);
        if (index >= 0 && index < fields.Count)
        {
            fields[index] = typed;
        }
        return typed;
    }

    /// <summary>
    /// The server's list, as a redraw found it. A list it has already seen changes nothing; a new one replaces the fields
    /// when they hold nothing unsaved, and otherwise only becomes what the next save is measured against.
    /// </summary>
    public void Sync(IReadOnlyList<string> saved)
    {
        ArgumentNullException.ThrowIfNull(saved);
        if (Same(saved, basis))
        {
            return;
        }
        var untouched = !Unsaved;
        basis = [.. saved];
        if (untouched)
        {
            Replace(saved);
        }
    }

    /// <summary>
    /// The list the server KEPT, from its answer to the save of <paramref name="sent"/>: what the next save is measured
    /// against, and what the fields become — unless they were changed while the save was on its way, in which case they
    /// hold what the owner typed since, and that is the next save.
    /// </summary>
    public void Adopt(IReadOnlyList<string> sent, IReadOnlyList<string> kept)
    {
        ArgumentNullException.ThrowIfNull(sent);
        ArgumentNullException.ThrowIfNull(kept);
        basis = [.. kept];
        if (Same(Request.Names, sent))
        {
            Replace(kept);
        }
    }

    /// <summary>The list to send, or null when the fields ask for nothing new — or for something the server would refuse.</summary>
    public IReadOnlyList<string>? Pending()
    {
        var request = Request;
        return request.Ok && !Same(request.Names, basis) ? request.Names : null;
    }

    /// <summary>
    /// The fields become the server's list — and the empty fields at the end stay, because an empty field at the end is
    /// one the owner has just added and is about to type in.
    /// </summary>
    private void Replace(IReadOnlyList<string> places)
    {
        var trailing = 0;
        for (var index = fields.Count - 1; index >= 0 && GreetingPlaces.Fold(fields[index]).Length == 0; index--)
        {
            trailing++;
        }
        fields.Clear();
        fields.AddRange(places.Take(GreetingPlaces.MaxPlaces));
        for (; trailing > 0 && CanAdd; trailing--)
        {
            fields.Add(string.Empty);
        }
    }

    private static bool Same(IReadOnlyList<string> a, IReadOnlyList<string> b) =>
        a.Count == b.Count && a.SequenceEqual(b, StringComparer.Ordinal);
}
