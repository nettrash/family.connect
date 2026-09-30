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
        // No HttpClient.Timeout: it would cap the backdrop's own 120 s deadline at the ordinary one. The API client
        // gives every request its deadline itself (docs/protocol.md, "Board").
        Http = ApiClient.NewHttpClient();
        Tokens = new LockerTokenStore(server);
        Api = new ApiClient(Http, server, Tokens);
        Cache = Database.Open(cachePath);
        Session = new AppSession(Api, Tokens, Cache, new AwaitingJoinFile(server));
        // The reader is whoever `GET /me` last said: read when asked, never captured, because a
        // sign-in as somebody else must not be drawn as the previous person's "You".
        Chats = new ChatStore(Cache, () => Session.State.Me?.Id ?? 0);
        Board = new BoardStore(Cache);
        Pack = new PackStore(Cache);
        Outbox = new OutboxStore(Cache);
        Socket = new ChatSocket(() => new ClientWebSocketAdapter(), () => Api.SocketUrl, Tokens);
        Staging = new FolderMediaStore(AppFolders.StagingPath);
        Media = new MediaOutbox(Outbox, Api, Staging);
        Sending = new SendPipeline(Socket, Outbox, Chats, Api, uploads: PushMediaAsync);
        Router = new FrameRouter(Chats, Board, Pack);
        Attachments = new AttachmentCache(Api, new FileBlobStore(AppFolders.BlobsPath));
        // A sticker on its way out is bytes this device already holds: kept under the id the server just gave them, so
        // the message it becomes is drawn at once rather than downloaded back.
        Media.Landed += (row, attachment, staged) =>
        {
            if (!row.Sticker)
            {
                return;
            }
            try
            {
                Attachments.Remember(attachment, staged.Bytes);
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                Diagnostics.Write($"keeping a sent sticker's bytes: {e.GetType().Name}");
            }
        };
        Avatars = new AvatarCache(Api, new FileBlobStore(AppFolders.BlobsPath));
        // The family's stickers: the pack this device keeps, and the bytes under their attachment ids in the same
        // cache every other picture is kept in.
        Stickers = new PackModel(Pack, Chats, Api, Attachments);
        Live = new LiveConnection(Session, Socket, new Resync(Api, Chats, Board, Sending, Pack), Sending, Router);
        // What the live frames leave on screen and nowhere else, listened for from the start so a frame that lands
        // while no conversation is open is not lost.
        PeerReads = new PeerReads();
        Answers = new AssistantAnswers();
        Previews = new LinkPreviews(() => LinkPreviewSetting.Enabled);
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

    /// <summary>The cards under links: the one place this app asks a host the family does not own, and only while switched on.</summary>
    public LinkPreviews Previews { get; }

    public Uri Server { get; }

    public HttpClient Http { get; }

    public LockerTokenStore Tokens { get; }

    public ApiClient Api { get; }

    public Database Cache { get; }

    public ChatStore Chats { get; }

    public BoardStore Board { get; }

    /// <summary>The family's sticker pack, kept as the board is kept (docs/protocol.md, "Sticker pack").</summary>
    public PackStore Pack { get; }

    /// <summary>What the window asks of the pack: the panel, who may remove what, and add, remove and send.</summary>
    public PackModel Stickers { get; }

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
        Previews.Dispose();
        Http.Dispose();
    }
}
