using FamilyConnect.App.Services;
using FamilyConnect.Core;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Xaml.Shapes;
using static FamilyConnect.App.Views.Dialogs;

namespace FamilyConnect.App.Views;

/// <summary>
/// A person's picture, or their initials when they have none; the family's house (the web client's
/// <c>Avatar</c>, ios <c>InitialsAvatar</c>): a circle of the accent at a fifth of its strength, the initials or
/// the house in the accent itself, and the picture over both once it lands.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE INITIALS DRAW AT ONCE</b> and the picture replaces them when it arrives, so a row never waits on the
/// network to render. A picture is asked for only for a real version — 0 is "none" — and kept under the version,
/// which the protocol never reuses for another picture.
/// </para>
/// <para>
/// <b>DECORATIVE</b> beside the name it stands for, which is always drawn next to it: a screen reader hearing
/// "AS, Anna Smith" learns nothing twice.
/// </para>
/// </remarks>
internal sealed class AvatarFaces(Connection connection)
{
    /// <summary>
    /// ONE FETCH PER PICTURE, however many rows draw it at once: the load is kept, not its result, so the second row waits
    /// for the first one's bytes instead of asking again. Touched only on the window's thread.
    /// </summary>
    private readonly Dictionary<string, Task<BitmapImage?>> pictures = [];

    public FrameworkElement Face(string title, bool family, long? userId, int version, double size)
    {
        var resources = Application.Current.Resources;
        var ink = (Brush)resources["AccentTextFillColorPrimaryBrush"];
        var root = new Grid { Width = size, Height = size, VerticalAlignment = VerticalAlignment.Center };
        root.Children.Add(new Ellipse { Fill = (Brush)resources["AccentFillColorDefaultBrush"], Opacity = 0.2 });
        if (family)
        {
            root.Children.Add(new FontIcon
            {
                // Home, in Segoe Fluent Icons.
                Glyph = ((char)0xE80F).ToString(),
                FontSize = size * 0.41,
                Foreground = ink,
            });
        }
        else
        {
            root.Children.Add(new TextBlock
            {
                Text = AvatarText.Initials(title),
                FontSize = size * 0.36,
                FontWeight = FontWeights.SemiBold,
                Foreground = ink,
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
                TextLineBounds = TextLineBounds.Tight,
            });
            if (userId is { } id && version > 0)
            {
                var picture = new Ellipse();
                root.Children.Add(picture);
                _ = ShowAsync(picture, id, version);
            }
        }
        AutomationProperties.SetAccessibilityView(root, AccessibilityView.Raw);
        return root;
    }

    private async Task ShowAsync(Ellipse circle, long userId, int version)
    {
        var key = $"{userId}-{version}";
        if (!pictures.TryGetValue(key, out var loading))
        {
            loading = pictures[key] = LoadAsync(userId, version);
        }
        try
        {
            if (await loading is { } bitmap)
            {
                circle.Fill = new ImageBrush { ImageSource = bitmap, Stretch = Stretch.UniformToFill };
                return;
            }
        }
        catch (Exception e)
        {
            Diagnostics.Write($"drawing a profile picture: {e.GetType().Name}");
        }
        // No picture this time is not kept here — AvatarCache keeps a real "none" per version — so the next row asks again.
        if (pictures.TryGetValue(key, out var current) && current == loading)
        {
            pictures.Remove(key);
        }
    }

    private async Task<BitmapImage?> LoadAsync(long userId, int version)
    {
        var (bytes, _) = await connection.Avatars.BytesAsync(userId, version);
        return bytes is null ? null : await BitmapAsync(bytes);
    }
}
