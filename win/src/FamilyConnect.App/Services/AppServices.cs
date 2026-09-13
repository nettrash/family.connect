using System.Globalization;
using FamilyConnect.Core;

namespace FamilyConnect.App.Services;

/// <summary>What outlives a server: the reader's language, the probe, and the current connection.</summary>
internal sealed class AppServices : IAsyncDisposable
{
    /// <summary>
    /// The window's language is the DEVICE's — never the family's, which is what the assistant
    /// answers in (docs/protocol.md, "The family's language").
    /// </summary>
    public IStringCatalog Say { get; } =
        JsonCatalog.For(Languages.ForDisplay(CultureInfo.CurrentUICulture.Name));

    /// <summary>Numbers, dates and times in the reader's own format.</summary>
    public CultureInfo Culture => CultureInfo.CurrentCulture;

    /// <summary>For "Connect": a short wait, because a host that never answers is the common failure.</summary>
    public HttpClient Probe { get; } = new() { Timeout = TimeSpan.FromSeconds(15) };

    /// <summary>Whether the window is the one the reader is looking at — a read is reported only then.</summary>
    public bool Foreground { get; set; }

    public Uri? SavedServer => ServerSetting.Read();

    public Connection? Current { get; private set; }

    /// <summary>Talk to <paramref name="server"/> from now on.</summary>
    public async Task<Connection> UseAsync(Uri server)
    {
        var previous = ServerSetting.Read();
        if (Current is { } old)
        {
            Current = null;
            await old.DisposeAsync();
        }
        ServerSetting.Write(server);
        var next = new Connection(server, AppFolders.CachePath);
        if (previous is not null && previous != server)
        {
            // One cache, one server: another family's history must not be drawn for this one.
            next.Cache.WipeAll();
        }
        Current = next;
        return next;
    }

    public async ValueTask DisposeAsync()
    {
        if (Current is { } current)
        {
            Current = null;
            await current.DisposeAsync();
        }
        Probe.Dispose();
    }
}
