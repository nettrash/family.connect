using System.Reflection;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Services;

/// <summary>
/// The server a Store build opens on, compiled in — the Windows counterpart of the iOS <c>FamilyConnect-nettrash</c> scheme's
/// <c>FCDefaultServerURL</c> and the Android <c>nettrash</c> flavour (root README, "Store builds with a predefined server").
/// </summary>
/// <remarks>
/// A build from source carries none and asks for an address on first run. A Store build is published with
/// <c>-p:FamilyConnectDefaultServer=https://fc.nettrash.me</c>, lands straight on sign-in, and "Change server" there still
/// reaches any other server. A server the reader chose is always kept over this one (<see cref="AppServices.SavedServer"/>).
/// </remarks>
internal static class DefaultServer
{
    private const string Key = "FamilyConnectDefaultServer";

    public static Uri? Address { get; } = Read();

    private static Uri? Read()
    {
        var value = typeof(DefaultServer).Assembly
            .GetCustomAttributes<AssemblyMetadataAttribute>()
            .FirstOrDefault(attribute => attribute.Key == Key)?.Value;
        return string.IsNullOrWhiteSpace(value) ? null : ServerUrl.Normalise(value);
    }
}
