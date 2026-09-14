using System.Text.Json;
using System.Text.Json.Serialization;
using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using Microsoft.UI.Xaml.Controls;
using Microsoft.Web.WebView2.Core;

namespace FamilyConnect.App.Services;

/// <summary>
/// A call's media on Windows: the browser engine's own WebRTC, in a page of the app's (<c>Assets/Call/call.html</c>) served
/// from a virtual https host — getUserMedia wants a secure origin — and driven by messages. Every decision is the engine's
/// (<see cref="CallEngine"/>); this is only the doing of it.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE MICROPHONE AND THE CAMERA ARE GRANTED TO THAT PAGE ALONE.</b> A permission request is answered yes only for the
/// virtual host, nothing else is ever navigated to, and a new window is never opened.
/// </para>
/// <para>
/// <b>STARTED ON FIRST USE, AND KEPT.</b> A browser engine is a process of its own: a window that never calls never starts
/// one, and a window that has called keeps it, so the next ring sounds at once.
/// </para>
/// <para>Driven from the window's thread, where the engine and the page's messages both arrive.</para>
/// </remarks>
internal sealed class WebViewCallMedia : ICallMedia
{
    private const string Host = "call.familyconnect.invalid";

    private static readonly JsonSerializerOptions Json = new() { DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull };

    private readonly WebView2 view;
    private readonly Dictionary<int, TaskCompletionSource<JsonElement?>> waiting = [];
    private readonly TaskCompletionSource<bool> ready = new();
    private int asked;
    private bool started;

    public WebViewCallMedia(WebView2 view)
    {
        this.view = view;
    }

    public bool IsConnected { get; private set; }

    public event Action<IceCandidate>? CandidateGathered;

    public event Action? Connected;

    public event Action? Failed;

    public event Action? MediaChanged;

    public async Task<MediaGrant> OpenAsync(bool video) =>
        Text(await AskAsync("open", new { video })) switch
        {
            "both" => MediaGrant.MicrophoneAndCamera,
            "microphone" => MediaGrant.Microphone,
            _ => MediaGrant.None,
        };

    public async Task<bool> ConnectAsync(IReadOnlyList<IceServerDto> servers, bool receiveVideoOnly)
    {
        IsConnected = false;
        var wanted = servers.Select(server => new { urls = server.Urls ?? [], username = server.Username, credential = server.Credential }).ToArray();
        return await AskAsync("connect", new { servers = wanted, receiveVideoOnly }) is { ValueKind: JsonValueKind.True };
    }

    public async Task<string?> CreateOfferAsync() => Text(await AskAsync("offer"));

    public async Task<bool> SetRemoteAsync(bool isOffer, string sdp) =>
        await AskAsync("remote", new { type = isOffer ? "offer" : "answer", sdp }) is { ValueKind: JsonValueKind.True };

    public async Task<string?> CreateAnswerAsync() => Text(await AskAsync("answer"));

    public async Task AddCandidateAsync(IceCandidate candidate) =>
        await AskAsync("candidate", new { candidate = candidate.Candidate, sdpMid = candidate.SdpMid, sdpMLineIndex = candidate.SdpMlineIndex });

    public void SetMuted(bool muted) => _ = AskAsync("mute", new { muted });

    public void SetCamera(bool on) => _ = AskAsync("camera", new { on });

    public void Ring(RingTone tone) => _ = AskAsync("ring", new { tone = tone.ToString() });

    public void StopRinging()
    {
        if (started)
        {
            _ = AskAsync("quiet");
        }
    }

    public void Close()
    {
        IsConnected = false;
        if (started)
        {
            _ = AskAsync("close");
        }
    }

    /// <summary>One request to the page, answered by its id — or null when the page could not be asked or said no.</summary>
    private async Task<JsonElement?> AskAsync(string op, object? args = null)
    {
        if (!await StartAsync())
        {
            return null;
        }
        var id = ++asked;
        var answer = new TaskCompletionSource<JsonElement?>();
        waiting[id] = answer;
        try
        {
            view.CoreWebView2.PostWebMessageAsJson(JsonSerializer.Serialize(new { id, op, args }, Json));
        }
        catch (Exception e)
        {
            waiting.Remove(id);
            Diagnostics.Write($"asking the call page: {e.GetType().Name}");
            return null;
        }
        return await answer.Task;
    }

    private async Task<bool> StartAsync()
    {
        if (!started)
        {
            started = true;
            try
            {
                // A ring has to sound before anybody has clicked in the page — which a browser otherwise refuses.
                var options = new CoreWebView2EnvironmentOptions { AdditionalBrowserArguments = "--autoplay-policy=no-user-gesture-required" };
                var environment = await CoreWebView2Environment.CreateWithOptionsAsync(null, null, options);
                await view.EnsureCoreWebView2Async(environment);
                var core = view.CoreWebView2;
                core.Settings.AreDevToolsEnabled = false;
                core.Settings.AreDefaultContextMenusEnabled = false;
                core.Settings.IsStatusBarEnabled = false;
                core.SetVirtualHostNameToFolderMapping(
                    Host, Path.Combine(AppContext.BaseDirectory, "Assets", "Call"), CoreWebView2HostResourceAccessKind.DenyCors);
                core.PermissionRequested += (_, args) =>
                {
                    var ours = Uri.TryCreate(args.Uri, UriKind.Absolute, out var asking) && asking.Host == Host;
                    args.State = ours && args.PermissionKind is CoreWebView2PermissionKind.Microphone or CoreWebView2PermissionKind.Camera
                        ? CoreWebView2PermissionState.Allow
                        : CoreWebView2PermissionState.Deny;
                };
                core.NavigationStarting += (_, args) =>
                {
                    if (!args.Uri.StartsWith($"https://{Host}/", StringComparison.Ordinal))
                    {
                        args.Cancel = true;
                    }
                };
                core.NavigationCompleted += (_, args) =>
                {
                    if (!args.IsSuccess)
                    {
                        Diagnostics.Write($"the call page did not load: {args.WebErrorStatus}");
                        ready.TrySetResult(false);
                    }
                };
                core.NewWindowRequested += (_, args) => args.Handled = true;
                core.ProcessFailed += (_, _) => Gone();
                core.WebMessageReceived += OnMessage;
                view.Source = new Uri($"https://{Host}/call.html");
            }
            catch (Exception e)
            {
                Diagnostics.Write($"starting the call page: {e.GetType().Name}");
                ready.TrySetResult(false);
            }
        }
        return await ready.Task;
    }

    private void OnMessage(CoreWebView2 sender, CoreWebView2WebMessageReceivedEventArgs args)
    {
        JsonElement message;
        try
        {
            using var document = JsonDocument.Parse(args.WebMessageAsJson);
            message = document.RootElement.Clone();
        }
        catch (JsonException)
        {
            return;
        }
        if (message.ValueKind != JsonValueKind.Object)
        {
            return;
        }
        if (message.TryGetProperty("event", out var happened))
        {
            switch (happened.GetString())
            {
                case "ready":
                    ready.TrySetResult(true);
                    break;
                case "candidate" when message.TryGetProperty("candidate", out var found) && found.ValueKind == JsonValueKind.Object:
                    CandidateGathered?.Invoke(new IceCandidate(
                        found.TryGetProperty("candidate", out var line) ? line.GetString() ?? string.Empty : string.Empty,
                        found.TryGetProperty("sdpMid", out var mid) && mid.ValueKind == JsonValueKind.String ? mid.GetString() : null,
                        found.TryGetProperty("sdpMLineIndex", out var index) && index.ValueKind == JsonValueKind.Number ? index.GetInt32() : null));
                    break;
                case "connected":
                    IsConnected = true;
                    Connected?.Invoke();
                    break;
                case "failed":
                    IsConnected = false;
                    Failed?.Invoke();
                    break;
                case "media":
                    MediaChanged?.Invoke();
                    break;
            }
            return;
        }
        if (message.TryGetProperty("id", out var id) && id.ValueKind == JsonValueKind.Number && waiting.Remove(id.GetInt32(), out var answer))
        {
            var ok = message.TryGetProperty("ok", out var said) && said.ValueKind == JsonValueKind.True;
            answer.TrySetResult(ok && message.TryGetProperty("value", out var value) ? value : null);
        }
    }

    /// <summary>The engine's process went: nothing waiting will be answered, and a call on it is over.</summary>
    private void Gone()
    {
        foreach (var answer in waiting.Values)
        {
            answer.TrySetResult(null);
        }
        waiting.Clear();
        if (IsConnected)
        {
            IsConnected = false;
            Failed?.Invoke();
        }
    }

    private static string? Text(JsonElement? value) =>
        value is { ValueKind: JsonValueKind.String } text ? text.GetString() : null;
}
