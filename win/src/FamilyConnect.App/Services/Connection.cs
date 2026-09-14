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
        Staging = new FolderMediaStore(AppFolders.StagingPath);
        Media = new MediaOutbox(Outbox, Api, Staging);
        Sending = new SendPipeline(Socket, Outbox, Chats, Api, uploads: PushMediaAsync);
        Router = new FrameRouter(Chats, Board);
        Attachments = new AttachmentCache(Api, new FileBlobStore(AppFolders.BlobsPath));
        Avatars = new AvatarCache(Api, new FileBlobStore(AppFolders.BlobsPath));
        Live = new LiveConnection(Session, Socket, new Resync(Api, Chats, Board, Sending), Sending, Router);
        // What the live frames leave on screen and nowhere else, listened for from the start so a frame that lands
        // while no conversation is open is not lost.
        PeerReads = new PeerReads();
        Answers = new AssistantAnswers();
        Router.PeerRead += (chatId, _, lastRead) => PeerReads.Apply(chatId, lastRead);
        Router.AiDelta += (chatId, messageId, text) => Answers.Delta(chatId, messageId, text, Chats.Message(messageId));
        Router.AiStopped += Answers.Stopped;
        Router.Edited += Answers.Finished;
        Session.Ended += _ =>
        {
            PeerReads.Clear();
            Answers.Clear();
        };
    }

    /// <summary>The other person's read marker in each direct chat, from the live frames.</summary>
    public PeerReads PeerReads { get; }

    /// <summary>The assistant's answers while they are written: the streamed text, and the ones that stopped.</summary>
    public AssistantAnswers Answers { get; }

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

    /// <summary>A send's files on disk, from Send until the message lands or is given up.</summary>
    public FolderMediaStore Staging { get; }

    /// <summary>The uploads queued messages owe, pushed at the start of every flush.</summary>
    public MediaOutbox Media { get; }

    public AppSession Session { get; }

    public FrameRouter Router { get; }

    public LiveConnection Live { get; }

    /// <summary>Attachment bytes: fetched once, kept, and a preview asked for only where one exists.</summary>
    public AttachmentCache Attachments { get; }

    /// <summary>Profile pictures, kept per version: a changed picture is a new key, never a stale face.</summary>
    public AvatarCache Avatars { get; }

    /// <summary>Whether a token is stored for this server — which is not the same as a session.</summary>
    public bool HasToken => Tokens.Token is not null;

    /// <summary>
    /// The uploads first, and then the sweep: files no queued row names any more — a send that
    /// landed, or one that was given up — are not kept.
    /// </summary>
    private async Task PushMediaAsync(CancellationToken ct)
    {
        await Media.PushAsync(ct).ConfigureAwait(false);
        try
        {
            Media.Sweep();
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            Diagnostics.Write($"sweeping staged files: {e.GetType().Name}");
        }
    }

    public async ValueTask DisposeAsync()
    {
        await Live.DisposeAsync().ConfigureAwait(false);
        Cache.Dispose();
        Http.Dispose();
    }
}
