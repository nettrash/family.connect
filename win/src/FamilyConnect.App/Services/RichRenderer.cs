using FamilyConnect.App.Logic;
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.Web.WebView2.Core;
using Windows.Storage.Streams;

namespace FamilyConnect.App.Services;

/// <summary>
/// A formula, a diagram or a graph as a picture for a chat bubble (issue #97; docs/capitals-and-rich-markdown-2026-10-17.md, "The
/// native host"): <c>null</c> for one that could not be drawn, which the bubble then shows as its source.
/// </summary>
/// <param name="Width">CSS pixels, which are DIPs here: the size it is shown at, at most.</param>
/// <param name="Baseline">CSS pixels from its top to its baseline — what lines an inline formula up with the words beside it.</param>
internal sealed record RichDrawn(ImageSource? Picture, double Width, double Height, double Baseline)
{
    /// <summary>Could not be drawn: the engine refused it, it drew nothing, it took too long, or there is no engine.</summary>
    public static readonly RichDrawn Failure = new(null, 0, 0, 0);
}

/// <summary>
/// The hidden web view that draws every rich block, and the two caches in front of it. Everything it decides — the job, the key,
/// the answers, the scale, what the page may load — is <see cref="RichJob"/>, <see cref="RichAnswer"/>, <see cref="RichCapture"/>,
/// <see cref="RichPage"/> and the caches in App.Logic, tested off Windows; what is here is only the doing of it.
/// </summary>
/// <remarks>
/// <para>
/// <b>WHERE IT LIVES.</b> A WebView2 initialises only inside a window's visual tree, so it lives in the main window's
/// <c>RichStage</c> canvas, sized to the page's viewport and parked at <c>Canvas.Left = -10000</c> — and VISIBLE, never collapsed.
/// Chromium takes a page to be hidden from the control's <c>Visibility</c>, not from where it is on screen, and a hidden page's
/// timers are throttled: md.win's off-canvas renderer learnt that a collapsed one turns a two-second diagram into a timeout.
/// </para>
/// <para>
/// <b>NOTHING LEAVES THE FOLDER.</b> The page is the app's copy of <c>web/rich</c> served from the virtual host
/// <see cref="RichPage.Host"/>; navigation is locked to <see cref="RichPage.Url"/>, every other request is answered 403 before it
/// leaves the process (<see cref="RichPage.Allows"/>), new windows, downloads and permissions are refused, and DevTools are off —
/// the DevTools PROTOCOL still answers <c>CallDevToolsProtocolMethodAsync</c>, which is how it is photographed.
/// </para>
/// <para>
/// <b>ONE JOB AT A TIME.</b> A job is <c>fcRich.drawAndPost(id, job)</c>, whose answer comes back as a web message, and is then
/// photographed with <c>Page.captureScreenshot</c> — the drawing's rectangle at the page's top left, at the screen's scale (capped
/// at 4096 pixels), with the page's default background overridden to clear so the PNG keeps its alpha. The next job waits for
/// the photograph, or it would draw over what is being photographed. Fifteen seconds without an answer fails the job and throws
/// the web view away, since an engine stuck in a loop would fail every job after it too.
/// </para>
/// <para>
/// <b>WHEN IT DIES.</b> If the page's process or the browser's goes, the job in flight fails and the next one makes a new web
/// view (and, for the browser, a new environment).
/// </para>
/// <para>
/// <b>KEPT.</b> A drawing is kept in memory and on disk (<see cref="AppFolders.RichPath"/>) under <see cref="RichJob.Key"/>, whose
/// version is read out of the shipped <c>fc-rich.js</c> — so a drawing kept on disk is shown without the browser process ever
/// being started. A failure is kept in memory only, so the next launch, maybe with a newer engine, tries again.
/// </para>
/// <para>The window's thread only: the web view, its events and the caches all belong to it. Never logs a source or an engine's
/// error, which quotes one.</para>
/// </remarks>
internal sealed class RichRenderer
{
    /// <summary>Far enough left that no monitor arrangement can put it on screen.</summary>
    private const double OffCanvasLeft = -10000;

    /// <summary>How many drawings are held decoded, and how many pixels between them (32 Mpx: ~128 MB at most).</summary>
    private const int MemoryEntries = 256;

    private const long MemoryPixels = 32L * 1024 * 1024;

    /// <summary>How many times a session tries to start the web view before it settles for showing sources.</summary>
    private const int MostStarts = 3;

    private readonly Canvas stage;
    private readonly string folder = Path.Combine(AppContext.BaseDirectory, "Assets", "Rich");
    private readonly RichStore store = new(AppFolders.RichPath);
    private readonly RichMemory<RichDrawn> memory = new(MemoryEntries, MemoryPixels);
    private readonly Dictionary<string, Task<RichDrawn>> drawing = new(StringComparer.Ordinal);
    private readonly Dictionary<int, TaskCompletionSource<RichResult>> waiting = [];
    private readonly SemaphoreSlim turn = new(1, 1);
    private WebView2? view;
    private CoreWebView2? core;
    private Task<bool>? starting;
    private int starts;
    private int asked;

    /// <param name="stage">The window's canvas the hidden web view is parked in.</param>
    public RichRenderer(Canvas stage)
    {
        this.stage = stage;
        try
        {
            Version = RichPage.VersionOf(File.ReadAllText(Path.Combine(folder, "fc-rich.js")));
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            Diagnostics.Write($"reading the rich renderer: {e.GetType().Name}");
        }
        if (Version is null)
        {
            Diagnostics.Write("the rich renderer is missing: formulas and diagrams show their source");
        }
        _ = Task.Run(() => store.Trim());
    }

    /// <summary><c>fcRich.version</c>, or null when the app's copy of the renderer is missing — and nothing is then drawn.</summary>
    public string? Version { get; }

    /// <summary>
    /// What is already held for <paramref name="job"/> — the drawing, or <see cref="RichDrawn.Failure"/> — or null when it has to
    /// be drawn (or read from disk) first. A bubble that is rebuilt asks this first, so it never flickers through its source.
    /// </summary>
    public RichDrawn? Held(RichJob job)
    {
        if (Version is not { } version)
        {
            return RichDrawn.Failure;
        }
        return memory.TryGet(job.Key(version), out var held) ? held : null;
    }

    /// <summary>The drawing of <paramref name="job"/>, from memory, disk or the page, in that order. Never throws.</summary>
    public Task<RichDrawn> DrawAsync(RichJob job)
    {
        if (Version is not { } version)
        {
            return Task.FromResult(RichDrawn.Failure);
        }
        var key = job.Key(version);
        if (memory.TryGet(key, out var held))
        {
            return Task.FromResult(held);
        }
        if (drawing.TryGetValue(key, out var running))
        {
            // The same block in two bubbles, or one bubble rebuilt while it was being drawn: one drawing for both.
            return running;
        }
        var task = DrawFreshAsync(job, key);
        if (!task.IsCompleted)
        {
            drawing[key] = task;
        }
        return task;
    }

    private async Task<RichDrawn> DrawFreshAsync(RichJob job, string key)
    {
        RichDrawn drawn;
        try
        {
            drawn = await FromDiskAsync(key) ?? await FromPageAsync(job, key);
        }
        catch (Exception e)
        {
            Diagnostics.Write($"drawing a {job.Kind} block: {e.GetType().Name}");
            drawn = RichDrawn.Failure;
        }
        finally
        {
            drawing.Remove(key);
        }
        memory.Put(key, drawn, Pixels(drawn));
        return drawn;
    }

    /// <summary>A drawing's weight in memory: its decoded pixels.</summary>
    private static long Pixels(RichDrawn drawn) =>
        drawn.Picture is BitmapImage { PixelWidth: var width, PixelHeight: var height } ? (long)width * height : 0;

    private async Task<RichDrawn?> FromDiskAsync(string key)
    {
        if (await Task.Run(() => store.Read(key)) is not { } kept)
        {
            return null;
        }
        return await DecodeAsync(kept.Png) is { } picture
            ? new RichDrawn(picture, kept.Metrics.Width, kept.Metrics.Height, kept.Metrics.Baseline)
            : null;
    }

    private async Task<RichDrawn> FromPageAsync(RichJob job, string key)
    {
        await turn.WaitAsync();
        try
        {
            if (!await StartAsync() || core is not { } page)
            {
                return RichDrawn.Failure;
            }
            var id = ++asked;
            var answer = new TaskCompletionSource<RichResult>();
            waiting[id] = answer;
            try
            {
                await page.ExecuteScriptAsync(job.Script(id));
            }
            catch (Exception e)
            {
                waiting.Remove(id);
                Diagnostics.Write($"asking the drawing page: {e.GetType().Name}");
                return RichDrawn.Failure;
            }
            if (await Task.WhenAny(answer.Task, Task.Delay(RichPage.Timeout)) != answer.Task)
            {
                waiting.Remove(id);
                Diagnostics.Write($"a {job.Kind} block took longer than {RichPage.Timeout.TotalSeconds:0} seconds");
                Reset();
                return RichDrawn.Failure;
            }
            var result = await answer.Task;
            // The engine's error is never written down: it quotes the source.
            if (!result.Ok || core != page)
            {
                return RichDrawn.Failure;
            }
            var scale = RichCapture.ScaleFor(Screen(), result.Width, result.Height);
            var png = RichCapture.Png(await page.CallDevToolsProtocolMethodAsync(
                "Page.captureScreenshot", RichCapture.Arguments(result.Width, result.Height, scale)));
            if (png is null || await DecodeAsync(png) is not { } picture)
            {
                Diagnostics.Write($"a {job.Kind} block drew but could not be photographed");
                return RichDrawn.Failure;
            }
            _ = Task.Run(() => store.Write(key, png, result));
            return new RichDrawn(picture, result.Width, result.Height, result.Baseline);
        }
        finally
        {
            turn.Release();
        }
    }

    /// <summary>The page, loaded and ready for jobs — started on first need and kept. False when there is no engine to start.</summary>
    private Task<bool> StartAsync()
    {
        if (starting is { IsCompleted: true } done && !done.Result && starts < MostStarts)
        {
            starting = null;
        }
        if (starting is null)
        {
            starts++;
            starting = StartCoreAsync();
        }
        return starting;
    }

    private async Task<bool> StartCoreAsync()
    {
        var web = new WebView2
        {
            Width = RichPage.ViewportWidth,
            Height = RichPage.ViewportHeight,
            Visibility = Visibility.Visible,
            IsHitTestVisible = false,
            IsTabStop = false,
            // Clear, so nothing of the page's own ground can reach a photograph (RichCapture.ClearBackground does the rest).
            DefaultBackgroundColor = Colors.Transparent,
        };
        // Nothing a screen reader should ever reach: the bubbles carry the sources as their names.
        AutomationProperties.SetAccessibilityView(web, AccessibilityView.Raw);
        Canvas.SetLeft(web, OffCanvasLeft);
        Canvas.SetTop(web, 0);
        stage.Children.Add(web);
        view = web;
        try
        {
            await web.EnsureCoreWebView2Async(await WebEngine.EnvironmentAsync());
            // EnsureCoreWebView2Async has been seen to come back with no core and no exception (md.win, 2026-09-26).
            if (view != web || web.CoreWebView2 is not { } page)
            {
                Diagnostics.Write("the drawing page's web view came up without a core");
                Discard(web);
                return false;
            }
            Lock(page);
            var loaded = new TaskCompletionSource<bool>();
            void Loaded(CoreWebView2 sender, CoreWebView2NavigationCompletedEventArgs args) => loaded.TrySetResult(args.IsSuccess);
            page.NavigationCompleted += Loaded;
            page.Navigate(RichPage.Url);
            var finished = await Task.WhenAny(loaded.Task, Task.Delay(RichPage.Timeout));
            page.NavigationCompleted -= Loaded;
            if (finished != loaded.Task || !await loaded.Task || view != web)
            {
                Diagnostics.Write("the drawing page did not load");
                Discard(web);
                return false;
            }
            // The viewport pinned in CSS pixels at a scale of 1, so the window's scale never reflows a drawing; and no default
            // ground, so a formula is photographed on nothing.
            await page.CallDevToolsProtocolMethodAsync("Emulation.setDeviceMetricsOverride", RichCapture.Metrics());
            await page.CallDevToolsProtocolMethodAsync("Emulation.setDefaultBackgroundColorOverride", RichCapture.ClearBackground);
            core = page;
            return true;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"starting the drawing page: {e.GetType().Name}");
            Discard(web);
            return false;
        }
    }

    /// <summary>Everything the page may not do, taken away before it loads.</summary>
    private void Lock(CoreWebView2 page)
    {
        var settings = page.Settings;
        settings.AreDevToolsEnabled = false;
        settings.AreDefaultContextMenusEnabled = false;
        settings.AreDefaultScriptDialogsEnabled = false;
        settings.AreHostObjectsAllowed = false;
        settings.AreBrowserAcceleratorKeysEnabled = false;
        settings.IsStatusBarEnabled = false;
        settings.IsZoomControlEnabled = false;
        settings.IsPinchZoomEnabled = false;
        settings.IsSwipeNavigationEnabled = false;
        settings.IsGeneralAutofillEnabled = false;
        settings.IsPasswordAutosaveEnabled = false;
        settings.IsBuiltInErrorPageEnabled = false;
        settings.IsScriptEnabled = true;
        settings.IsWebMessageEnabled = true;
        page.SetVirtualHostNameToFolderMapping(RichPage.Host, folder, CoreWebView2HostResourceAccessKind.Deny);
        page.AddWebResourceRequestedFilter("*", CoreWebView2WebResourceContext.All);
        page.WebResourceRequested += (_, args) =>
        {
            if (!RichPage.Allows(args.Request.Uri))
            {
                args.Response = page.Environment.CreateWebResourceResponse(null, 403, "Forbidden", string.Empty);
            }
        };
        page.NavigationStarting += (_, args) =>
        {
            if (!RichPage.IsPage(args.Uri))
            {
                args.Cancel = true;
            }
        };
        page.NewWindowRequested += (_, args) => args.Handled = true;
        page.DownloadStarting += (_, args) => args.Cancel = true;
        page.PermissionRequested += (_, args) => args.State = CoreWebView2PermissionState.Deny;
        page.WebMessageReceived += OnMessage;
        page.ProcessFailed += OnProcessFailed;
    }

    private void OnMessage(CoreWebView2 sender, CoreWebView2WebMessageReceivedEventArgs args)
    {
        if (!RichPage.IsPage(args.Source))
        {
            return;
        }
        string text;
        try
        {
            // fc-rich.js posts its {id, result} as a JSON STRING.
            text = args.TryGetWebMessageAsString();
        }
        catch (Exception)
        {
            return;
        }
        if (RichAnswer.Parse(text) is { } answer && waiting.Remove(answer.Id, out var job))
        {
            job.TrySetResult(answer.Result);
        }
    }

    private void OnProcessFailed(CoreWebView2 sender, CoreWebView2ProcessFailedEventArgs args)
    {
        var kind = args.ProcessFailedKind;
        // A GPU or utility process comes back by itself; the page's or the browser's does not.
        if (kind is not (CoreWebView2ProcessFailedKind.RenderProcessExited
            or CoreWebView2ProcessFailedKind.RenderProcessUnresponsive
            or CoreWebView2ProcessFailedKind.BrowserProcessExited))
        {
            return;
        }
        Diagnostics.Write($"the drawing page's process failed: {kind}");
        if (kind == CoreWebView2ProcessFailedKind.BrowserProcessExited)
        {
            WebEngine.Forget();
        }
        FailWaiting();
        core = null;
        // Not from inside the web view's own event: the teardown is the next turn's.
        var gone = view;
        stage.DispatcherQueue.TryEnqueue(() =>
        {
            if (view == gone)
            {
                Reset();
            }
        });
    }

    /// <summary>Throw the web view away: whatever is waiting fails, and the next job starts a new one.</summary>
    private void Reset()
    {
        FailWaiting();
        core = null;
        starting = null;
        if (view is { } old)
        {
            Discard(old);
        }
    }

    private void FailWaiting()
    {
        foreach (var answer in waiting.Values)
        {
            answer.TrySetResult(RichResult.Failed("gone"));
        }
        waiting.Clear();
    }

    private void Discard(WebView2 web)
    {
        if (view == web)
        {
            view = null;
            core = null;
        }
        stage.Children.Remove(web);
        try
        {
            web.Close();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"closing the drawing page: {e.GetType().Name}");
        }
    }

    /// <summary>The screen's own scale, for a photograph as sharp as the screen; 1 where it cannot be asked.</summary>
    private double Screen()
    {
        try
        {
            return stage.XamlRoot?.RasterizationScale ?? 1;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"reading the screen's scale: {e.GetType().Name}");
            return 1;
        }
    }

    private static async Task<BitmapImage?> DecodeAsync(byte[] png)
    {
        try
        {
            using var stream = new InMemoryRandomAccessStream();
            using (var writer = new DataWriter(stream))
            {
                writer.WriteBytes(png);
                await writer.StoreAsync();
                await writer.FlushAsync();
                writer.DetachStream();
            }
            stream.Seek(0);
            var picture = new BitmapImage();
            await picture.SetSourceAsync(stream);
            return picture;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"decoding a drawing: {e.GetType().Name}");
            return null;
        }
    }
}
