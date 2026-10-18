using Microsoft.Web.WebView2.Core;

namespace FamilyConnect.App.Services;

/// <summary>
/// The app's one browser engine environment, shared by every web view it makes — the call's media page
/// (<see cref="WebViewCallMedia"/>) and the page that draws formulas and diagrams (<see cref="RichRenderer"/>).
/// </summary>
/// <remarks>
/// <para>
/// <b>ONE, BECAUSE WEBVIEW2 INSISTS.</b> Every web view of an app shares one user-data folder, and WebView2 refuses to start a
/// second environment on that folder with different options: whichever web view came second would fail to initialise. So the
/// options are decided once, here, for both — and the call's autoplay switch, which the drawing page has no use for, is
/// harmless to it.
/// </para>
/// <para>Made on first need. A browser process that exits takes the environment with it, and <see cref="Forget"/> lets the
/// next web view make a new one.</para>
/// </remarks>
internal static class WebEngine
{
    private static Task<CoreWebView2Environment>? environment;

    /// <summary>The environment, made the first time anything asks. A failure to make one is not kept: the next ask tries again.</summary>
    public static async Task<CoreWebView2Environment> EnvironmentAsync()
    {
        var made = environment ??= CreateAsync();
        try
        {
            return await made;
        }
        catch
        {
            if (environment == made)
            {
                environment = null;
            }
            throw;
        }
    }

    /// <summary>The browser process went: the environment it belonged to is no use to the next web view.</summary>
    public static void Forget() => environment = null;

    private static async Task<CoreWebView2Environment> CreateAsync()
    {
        // A ring has to sound before anybody has clicked in the call's page — which a browser otherwise refuses.
        var options = new CoreWebView2EnvironmentOptions { AdditionalBrowserArguments = "--autoplay-policy=no-user-gesture-required" };
        return await CoreWebView2Environment.CreateWithOptionsAsync(null, null, options);
    }
}
