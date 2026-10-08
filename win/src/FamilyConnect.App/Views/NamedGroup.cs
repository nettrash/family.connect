using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;

namespace FamilyConnect.App.Views;

/// <summary>
/// A panel a screen reader MEETS: one group with the name and status set on it ("Voice message, 0:42", "Not played").
/// </summary>
/// <remarks>
/// A <c>Border</c> or a plain <c>Grid</c> has no automation peer, so a name set on one is never in the UI Automation tree
/// and Narrator never hears it — what it reads are the controls inside, one by one, with nothing saying what they belong
/// to. This is a <c>Grid</c> (which draws a background, a rounded border and padding as a <c>Border</c> does) whose peer is a
/// Group, so the name, the item status and the children under it are all where Narrator looks.
/// </remarks>
internal sealed partial class NamedGroup : Grid
{
    protected override AutomationPeer OnCreateAutomationPeer() => new Peer(this);

    private sealed partial class Peer(NamedGroup owner) : FrameworkElementAutomationPeer(owner)
    {
        protected override AutomationControlType GetAutomationControlTypeCore() => AutomationControlType.Group;

        protected override string GetClassNameCore() => nameof(NamedGroup);

        protected override bool IsControlElementCore() => true;

        protected override bool IsContentElementCore() => true;
    }
}
