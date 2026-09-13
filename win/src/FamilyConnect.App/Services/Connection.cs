using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Services;

/// <summary>
/// Everything that talks to ONE server, wired together — the composition the App.Logic tests build
/// with fakes, built here with the real socket, the real cache and the credential locker.
/// </summary>
/// <remarks>
/// A server is chosen before anything else exists, and the client, the cache and the socket are all
/// bound to it: changing server is therefore a new <see cref="Connection"/>, never a mutation of
/// this one.
/// </remarks>
internal sealed class Connection : IAsyncDisposable
{
    public Connection(Uri server, string cachePath)
    {
        Server = server;
        Http = new HttpClient();
        Tokens = new LockerTokenStore(server);
        Api = new ApiClient(Http, server, Tokens);
        Cache = Database.Open(cachePath);
        Session = new AppSession(Api, Tokens, Cache);
        // The reader is whoever `GET /me` last said: read when asked, never captured, because a
        // sign-in as somebody else must not be drawn as the previous person's "You".
        Chats = new ChatStore(Cache, () => Session.State.Me?.Id ?? 0);
        Board = new BoardStore(Cache);
        Outbox = new OutboxStore(Cache);
        Socket = new ChatSocket(() => new ClientWebSocketAdapter(), () => Api.SocketUrl, Tokens);
        Sending = new SendPipeline(Socket, Outbox, Chats, Api);
        Router = new FrameRouter(Chats, Board);
        Attachments = new AttachmentCache(Api, new FileBlobStore(AppFolders.BlobsPath));
        Live = new LiveConnection(Session, Socket, new Resync(Api, Chats, Board, Sending), Sending, Router);
    }

    public Uri Server { get; }

    public HttpClient Http { get; }

    public LockerTokenStore Tokens { get; }

    public ApiClient Api { get; }

    public Database Cache { get; }

    public ChatStore Chats { get; }

    public BoardStore Board { get; }

    public OutboxStore Outbox { get; }

    public ChatSocket Socket { get; }

    public SendPipeline Sending { get; }

    public AppSession Session { get; }

    public FrameRouter Router { get; }

    public LiveConnection Live { get; }

    /// <summary>Attachment bytes: fetched once, kept, and a preview asked for only where one exists.</summary>
    public AttachmentCache Attachments { get; }

    /// <summary>Whether a token is stored for this server — which is not the same as a session.</summary>
    public bool HasToken => Tokens.Token is not null;

    public async ValueTask DisposeAsync()
    {
        await Live.DisposeAsync().ConfigureAwait(false);
        Cache.Dispose();
        Http.Dispose();
    }
}
