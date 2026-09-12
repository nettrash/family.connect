using System.Net;
using System.Text;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// A server that answers by path. The match is the WHOLE path, query aside, and anchored for a
/// reason paid for once already: a route that matched a substring answered `/chats/42/messages`
/// with `/me`'s body — `/messages` contains `/me` — and a fake that lies about which endpoint was
/// asked can only teach the wrong lesson.
/// </summary>
internal sealed class Server : HttpMessageHandler
{
    private readonly List<Func<string, (HttpStatusCode Status, string? Json)?>> routes = [];
    private readonly List<(string What, Func<Task<(HttpStatusCode Status, string? Json)>> Answer)> slow = [];

    public List<string> Asked { get; } = [];

    public Server On(string what, string? json, HttpStatusCode status = HttpStatusCode.OK)
    {
        routes.Add(path => Endpoint(path) == "/api/v1" + what ? (status, json) : null);
        return this;
    }

    /// <summary>An answer computed when asked — for a route a test needs to hold open.</summary>
    public Server OnAsync(string what, Func<Task<(HttpStatusCode Status, string? Json)>> answer)
    {
        slow.Add((what, answer));
        return this;
    }

    /// <summary>A different answer each time that endpoint is asked.</summary>
    public Server Then(string what, params (HttpStatusCode Status, string? Json)[] answers)
    {
        var asked = 0;
        routes.Add(path => Endpoint(path) == "/api/v1" + what
            ? answers[Math.Min(asked++, answers.Length - 1)]
            : null);
        return this;
    }

    private static string Endpoint(string pathAndQuery)
    {
        var query = pathAndQuery.IndexOf('?', StringComparison.Ordinal);
        return query < 0 ? pathAndQuery : pathAndQuery[..query];
    }

    protected override async Task<HttpResponseMessage> SendAsync(
        HttpRequestMessage request, CancellationToken cancellationToken)
    {
        var path = request.RequestUri!.PathAndQuery;
        Asked.Add(path);
        foreach (var (what, answer) in slow)
        {
            if (Endpoint(path) == "/api/v1" + what)
            {
                var computed = await answer().ConfigureAwait(false);
                return Answered(computed.Status, computed.Json);
            }
        }
        foreach (var route in routes)
        {
            if (route(path) is { } answer)
            {
                return Answered(answer.Status, answer.Json);
            }
        }
        throw new InvalidOperationException($"no route for {path}");
    }

    private static HttpResponseMessage Answered(HttpStatusCode status, string? json)
    {
        var response = new HttpResponseMessage(status);
        if (json is not null)
        {
            response.Content = new StringContent(json, Encoding.UTF8, "application/json");
        }
        return response;
    }
}
