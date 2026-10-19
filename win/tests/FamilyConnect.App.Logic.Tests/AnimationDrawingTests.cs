using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// An animation once it has arrived, and on its way (issue #96; docs/protocol.md, "Animations", "How it is drawn"): "GIF"
/// wherever a video says "Video", when a tile plays by itself, the viewer's title, the send that says so — and the sticker
/// shelf's one new door, for a pack item that was removed.
/// </summary>
public sealed class AnimationDrawingTests : IDisposable
{
    private const long Me = 7;
    private const long Chat = 42;
    private const string Sent = "2026-10-19T10:00:00Z";

    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    private static readonly AttachmentDto Gif =
        new(93, "video", "video/mp4", 182_330, 480, 270, 2100, HasPreview: true, Animation: true);

    private readonly Database cache = Database.OpenInMemory();
    private readonly ChatStore chats;

    public AnimationDrawingTests()
    {
        chats = new ChatStore(cache, () => Me);
        chats.Replace([new ChatRowDto(new ChatDto(Chat, "family", "The Smiths"))]);
        chats.Replace([new MemberDto(Me, "anna", "Anna"), new MemberDto(11, "bob", "Bob")], []);
    }

    public void Dispose() => cache.Dispose();

    private static MessageDto Message(long id, long sender, string body = "", params AttachmentDto[] media) =>
        new(id, Chat, sender, null, body, Sent, Attachments: media);

    /// <summary>"GIF" on the chat list where a video says "Video" — and the caption first, as any video's row has it.</summary>
    [Fact]
    public void AChatListRowSaysGifWhereAVideosSaysVideo()
    {
        var list = new ChatListModel(chats, () => Me);
        Assert.Equal("GIF", list.Preview(Message(1, 11, "", Gif), hidden: false));
        Assert.Equal("ha", list.Preview(Message(1, 11, "ha", Gif), hidden: false));
        Assert.Equal("Video", list.Preview(Message(1, 11, "", Gif with { Animation = false }), hidden: false));
        // Two flagged videos are not an animation, whatever they carry.
        Assert.Equal("Video", list.Preview(Message(1, 11, "", Gif, Gif with { Id = 94 }), hidden: false));
        Assert.Equal("Hidden — blocked member", list.Preview(Message(1, 11, "", Gif), hidden: true));
    }

    /// <summary>A reply quoting a caption-less animation says "GIF" where the server's excerpt is empty.</summary>
    [Fact]
    public void AQuoteOfAnAnimationSaysGif()
    {
        chats.Apply(Message(40, 11, "", Gif));
        chats.Apply(Message(39, 11, "", Gif with { Id = 90, Animation = false }));
        MessageDto ReplyTo(long id, string excerpt = "") =>
            new(50, Chat, Me, null, "lovely", Sent, ReplyTo: new ReplyToDto(id, 11, excerpt));

        Assert.Equal(new QuoteLine(40, "Bob", "GIF", false), Quotes.Of(ReplyTo(40), chats, Say)!.Reply);
        Assert.Equal(new QuoteLine(39, "Bob", "", false), Quotes.Of(ReplyTo(39), chats, Say)!.Reply);
        Assert.Equal(new QuoteLine(40, "Bob", "ha", false), Quotes.Of(ReplyTo(40, "ha"), chats, Say)!.Reply);
    }

    /// <summary>In the viewer it is a GIF — one flagged video, alone — and never a circle.</summary>
    [Fact]
    public void TheViewerCallsItAGif()
    {
        var album = new MediaAlbum([Gif], 0);
        Assert.True(album.IsAnimation);
        Assert.True(album.IsVideo);
        Assert.False(album.IsRound);
        Assert.Equal("GIF", album.Title(Say));
        Assert.False(new MediaAlbum([Gif with { Animation = false }], 0).IsAnimation);
        Assert.False(new MediaAlbum([Gif, Gif with { Id = 94 }], 0).IsAnimation);
        Assert.False(new MediaAlbum([Gif with { Kind = "photo" }], 0).IsAnimation);
    }

    /// <summary>
    /// A tile plays by itself only while the system wants motion and the video is within 8 MiB — the one video a tile
    /// downloads. Otherwise its poster, still, and a click plays it in the viewer.
    /// </summary>
    [Fact]
    public void ATilePlaysByItselfOnlyWhenWantedAndSmall()
    {
        Assert.Equal(8_388_608, AnimationPrep.TileBytes);
        Assert.True(AnimationPrep.PlaysInTile(true, 182_330));
        Assert.True(AnimationPrep.PlaysInTile(true, 8_388_608));
        Assert.False(AnimationPrep.PlaysInTile(true, 8_388_609));
        Assert.False(AnimationPrep.PlaysInTile(false, 182_330));
        Assert.False(AnimationPrep.PlaysInTile(true, null));
        Assert.False(AnimationPrep.PlaysInTile(true, 0));
    }

    /// <summary>
    /// The send says <c>animation: true</c> only for the server's own shape — ONE attachment, and it an animation this
    /// device made; beside anything else it goes as the ordinary video it also is.
    /// </summary>
    [Fact]
    public void ASendIsAnAnimationOnlyWhenTheAnimationIsAlone()
    {
        var made = new StagedMedia("video", "video/mp4", new byte[] { 0, 0, 0, 0 }, 480, 270, 2100, Animation: true);
        var photo = new StagedMedia("photo", "image/jpeg", new byte[] { 0xFF, 0xD8, 0xFF });
        Assert.True(AnimationSend.Is([made]));
        Assert.False(AnimationSend.Is([made, photo]));
        Assert.False(AnimationSend.Is([made with { Animation = false }]));
        Assert.False(AnimationSend.Is([photo with { Animation = true }]));
        Assert.False(AnimationSend.Is([]));
        // Its chip in the composer says what it will be.
        Assert.Equal("GIF", ComposerStaging.Label(made, Say));
        Assert.Equal("Video", ComposerStaging.Label(made with { Animation = false }, Say));
    }

    /// <summary>The flag is written with the row and goes on the wire with the message — REST and socket alike read the row.</summary>
    [Fact]
    public void AConversationQueuesAnAnimationWithItsFlag()
    {
        var outbox = new OutboxStore(cache);
        var api = new ApiClient(new HttpClient(new Refusing()), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"));
        var socket = new Silent();
        var pipeline = new SendPipeline(socket, outbox, chats, api, wait: (_, _) => Task.CompletedTask);
        var chat = new ConversationModel(Chat, chats, api, pipeline, socket, outbox: outbox);
        var row = chat.Send("ha", pendingFiles: ["0123456789abcdef0123456789abcdef"], animation: true);
        Assert.True(row.Animation);
        Assert.False(row.Round);
        Assert.True(Assert.Single(outbox.All()).Animation);
    }

    private sealed class Refusing : HttpMessageHandler
    {
        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken) =>
            Task.FromResult(new HttpResponseMessage(System.Net.HttpStatusCode.ServiceUnavailable));
    }

    private sealed class Silent : IFrameSender
    {
        public bool IsConnected => false;

        public Task<bool> TrySend(string frame, CancellationToken ct = default) => Task.FromResult(false);

        public event Action<ServerFrame>? Frame;

        public void Unused() => Frame?.Invoke(new ServerFrame.Pong());
    }

    // ---- the sticker shelf: a removed pack item goes ------------------------------------------------

    private sealed class Picture(string name)
    {
        public string Name { get; } = name;
    }

    [Fact]
    public void ARemovedStickerLeavesTheShelfAndIsLetGoUnlessShown()
    {
        var shown = new HashSet<Picture>();
        var letGo = new List<string>();
        var shelf = new StickerShelf<Picture>(1000, 10, shown.Contains, letGo: picture => letGo.Add(picture.Name));
        var a = new Picture("a");
        var b = new Picture("b");
        shelf.Put(1, a, 300);
        shelf.Put(~5L, b, 200);
        shown.Add(b);
        Assert.True(shelf.Remove(1));
        Assert.Equal(["a"], letGo);
        Assert.Equal(200, shelf.Held);
        Assert.False(shelf.TryGet(1, out _));
        // One on screen leaves the shelf without being let go under its image.
        Assert.True(shelf.Remove(~5L));
        Assert.Equal(["a"], letGo);
        Assert.Equal(0, shelf.Held);
        Assert.False(shelf.Remove(1));
        Assert.Equal(0, shelf.Count);
    }
}
