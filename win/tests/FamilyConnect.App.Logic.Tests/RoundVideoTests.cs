using System.Net;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// Video messages on this client, Phase 2 (docs/audio-video-messages-2026-10-04.md, S5; docs/protocol.md, "Video
/// messages"): how a circle is drawn and named, the send that makes one, and everything else that has to know it is
/// not an ordinary video — the chat list, a quote, the viewer's title, Edit, and what pauses it.
/// </summary>
public sealed class RoundVideoTests : IDisposable
{
    private const long Me = 7;
    private const long Chat = 42;
    private const string Sent = "2026-10-05T10:00:00Z";

    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    private static readonly AttachmentDto Circle =
        new(91, "video", "video/mp4", 1649700, 480, 480, 23400, HasPreview: true, Round: true);

    private readonly Database cache = Database.OpenInMemory();
    private readonly ChatStore chats;
    private readonly OutboxStore outbox;

    public RoundVideoTests()
    {
        chats = new ChatStore(cache, () => Me);
        outbox = new OutboxStore(cache);
        chats.Replace([new ChatRowDto(new ChatDto(Chat, "family", "The Smiths"))]);
        chats.Replace([new MemberDto(Me, "anna", "Anna"), new MemberDto(11, "bob", "Bob")], []);
    }

    public void Dispose() => cache.Dispose();

    private static MessageDto Message(long id, long sender, string body = "", params AttachmentDto[] media) =>
        new(id, Chat, sender, null, body, Sent, Attachments: media);

    // ---- how it is drawn (S5.2, S6) -------------------------------------------------------------------

    /// <summary>240 on Windows, which is never a compact width — larger than a sticker, smaller than a video tile.</summary>
    [Fact]
    public void ACircleIsTwoHundredFortyAcrossLargerThanASticker()
    {
        Assert.Equal(240, RoundLook.Diameter);
        Assert.True(RoundLook.Diameter > StickerLook.Box);
        // The approved design's 48 play disc (the mockup's .round .pd), as Android draws it.
        Assert.Equal(48, RoundLook.PlayDisc);
        Assert.Equal(8, RoundLook.Dot);
        Assert.Equal(3, RoundLook.Ring);
    }

    /// <summary>The capsule says how long it is, and a screen reader hears the same with what it is (S6).</summary>
    [Fact]
    public void ACircleSaysHowLongItIsAndWhetherItWasPlayed()
    {
        Assert.Equal("0:23", RoundLook.Capsule(Circle));
        Assert.Equal("1:00", RoundLook.Capsule(Circle with { DurationMs = 60000 }));
        Assert.Null(RoundLook.Capsule(Circle with { DurationMs = null }));
        Assert.Null(RoundLook.Capsule(Circle with { DurationMs = 0 }));

        Assert.Equal("Video message, 0:23", RoundLook.Name(Circle, Say));
        Assert.Equal("Video message", RoundLook.Name(Circle with { DurationMs = null }, Say));

        Assert.Equal("Not played", RoundLook.Value(mine: false, played: false, Say));
        Assert.Equal("Played", RoundLook.Value(mine: false, played: true, Say));
    }

    /// <summary>
    /// THE DOT IS ONLY ON SOMEONE ELSE'S CIRCLE (S5.2, S6) — as on iOS, Android and the web: the reader's own circles never
    /// carry it, nor Narrator's "Not played" or "Played", whether or not this device has played them.
    /// </summary>
    [Theory]
    [InlineData(false, false, true)]
    [InlineData(false, true, false)]
    [InlineData(true, false, false)]
    [InlineData(true, true, false)]
    public void TheDotIsOnlyOnSomeoneElsesUnplayedCircle(bool mine, bool played, bool dot)
    {
        Assert.Equal(dot, RoundLook.ShowsDot(mine, played));
        Assert.Equal(mine ? null : dot ? "Not played" : "Played", RoundLook.Value(mine, played, Say));
    }

    /// <summary>
    /// THE DOT GOES AT THE END (S5.3: "at the end it returns to the poster and loses its dot"), as on every other client —
    /// not when it starts, not on a seek to the end of a clip never played here, never for a clip of unknown length.
    /// </summary>
    [Theory]
    [InlineData(true, 0.0, 23.4, false)] // it has only just started
    [InlineData(true, 12.0, 23.4, false)] // halfway
    [InlineData(true, 23.1, 23.4, false)] // just short of the end
    [InlineData(true, 23.15, 23.4, true)] // within the clock's tick of it
    [InlineData(true, 23.4, 23.4, true)] // at the end
    [InlineData(false, 23.4, 23.4, false)] // seeked to the end, never played
    [InlineData(true, 0.0, 0.0, false)] // a length not known
    [InlineData(true, 5.0, double.NaN, false)]
    [InlineData(true, double.NaN, 23.4, false)]
    public void ItCountsAsPlayedOnlyOncePlayedThrough(bool seenPlaying, double position, double total, bool through) =>
        Assert.Equal(through, RoundLook.PlayedThrough(seenPlaying, position, total));

    /// <summary>
    /// The viewer's one ring: none before it plays, as far as it has played while it plays or is paused part way — and none
    /// once it has FINISHED, when the mockup hides it and the circle is back to its poster, never a full ring left round it.
    /// </summary>
    [Theory]
    [InlineData(false, false, 0, 23.4, 0)]
    [InlineData(false, false, 11.7, 23.4, 0)]
    [InlineData(true, true, 11.7, 23.4, 0.5)]
    [InlineData(true, false, 11.7, 23.4, 0.5)]
    [InlineData(true, true, 23.4, 23.4, 1)]
    [InlineData(true, false, 23.4, 23.4, 0)]
    [InlineData(true, false, 23.3, 23.4, 0)]
    [InlineData(true, true, 5, 0, 0)]
    [InlineData(true, true, double.NaN, 23.4, 0)]
    public void TheViewerRingGoesWhenTheClipEnds(bool seenPlaying, bool playing, double position, double total, double ring) =>
        Assert.Equal(ring, RoundLook.ViewerRing(seenPlaying, playing, position, total), 6);

    /// <summary>
    /// The viewer's circle: as large as the room allows, never larger than the recording's 480 pixels and never smaller
    /// than the conversation's own circle.
    /// </summary>
    [Theory]
    [InlineData(1600, 1000, 120, 480)]
    [InlineData(600, 1000, 120, 480)]
    [InlineData(400, 1000, 120, 352)]
    [InlineData(1600, 500, 120, 332)]
    [InlineData(300, 300, 120, 200)]
    [InlineData(1600, 500, -50, 452)]
    public void TheViewersCircleFitsTheRoomWithinBounds(double width, double height, double chrome, double diameter)
    {
        Assert.Equal(diameter, RoundLook.ViewerDiameter(width, height, chrome));
    }

    [Fact]
    public void AViewerNotYetMeasuredGetsTheLargestCircle()
    {
        Assert.Equal(RoundLook.ViewerLargest, RoundLook.ViewerDiameter(0, 0, 100));
        Assert.Equal(RoundLook.ViewerLargest, RoundLook.ViewerDiameter(double.NaN, 900, 100));
    }

    // ---- what pauses it (S4's last column) -------------------------------------------------------------

    [Theory]
    [InlineData(PlaybackEvent.Call, Playing.VoiceNote, true)]
    [InlineData(PlaybackEvent.Call, Playing.RoundVideo, true)]
    [InlineData(PlaybackEvent.SessionLocked, Playing.VoiceNote, true)]
    [InlineData(PlaybackEvent.SessionLocked, Playing.RoundVideo, true)]
    [InlineData(PlaybackEvent.OutputChanged, Playing.VoiceNote, true)]
    [InlineData(PlaybackEvent.OutputChanged, Playing.RoundVideo, true)]
    // Minimised or hidden: a circle is looked at and pauses; a voice note is heard and plays on.
    [InlineData(PlaybackEvent.WindowHidden, Playing.RoundVideo, true)]
    [InlineData(PlaybackEvent.WindowHidden, Playing.VoiceNote, false)]
    // Only losing focus, still visible: everything plays on.
    [InlineData(PlaybackEvent.FocusLost, Playing.RoundVideo, false)]
    [InlineData(PlaybackEvent.FocusLost, Playing.VoiceNote, false)]
    // An ordinary video in the viewer is not in S4's column: this version leaves it as it was.
    [InlineData(PlaybackEvent.Call, Playing.Video, false)]
    [InlineData(PlaybackEvent.SessionLocked, Playing.Video, false)]
    [InlineData(PlaybackEvent.OutputChanged, Playing.Video, false)]
    [InlineData(PlaybackEvent.WindowHidden, Playing.Video, false)]
    public void WhatPausesWhatPlays(PlaybackEvent happened, Playing playing, bool pauses)
    {
        Assert.Equal(pauses, PlaybackPauses.Pauses(happened, playing));
    }

    /// <summary>A call, a lock and a hidden window reach playback through what stops a recording; the rest stop it outright.</summary>
    [Fact]
    public void TheWindowsInterruptionsReachPlayback()
    {
        Assert.Equal(PlaybackEvent.Call, PlaybackPauses.Of(RecordingEnd.Call));
        Assert.Equal(PlaybackEvent.SessionLocked, PlaybackPauses.Of(RecordingEnd.SessionLocked));
        Assert.Equal(PlaybackEvent.WindowHidden, PlaybackPauses.Of(RecordingEnd.WindowHidden));
        Assert.Null(PlaybackPauses.Of(RecordingEnd.LeftChat));
        Assert.Null(PlaybackPauses.Of(RecordingEnd.WindowClosed));
        Assert.Null(PlaybackPauses.Of(RecordingEnd.SignedOut));
        Assert.Null(PlaybackPauses.Of(RecordingEnd.Stopped));
        Assert.Null(PlaybackPauses.Of(RecordingEnd.Sent));
    }

    // ---- everything else that has to know (S5.4, S5.7) ------------------------------------------------

    /// <summary>NO EDIT on a circle, as on a sticker: the server refuses it and there is no balloon to hold words.</summary>
    [Fact]
    public void AVideoMessageIsNeverOfferedEdit()
    {
        Bubble Of(MessageDto message) => new(message, false, false, message.SenderId == Me);

        Assert.False(ConversationModel.MayEdit(Of(Message(1, Me, "", Circle))));
        // Even with words some server stored beside it.
        Assert.False(ConversationModel.MayEdit(Of(Message(1, Me, "look", Circle))));
        // An ordinary video with a caption still may be.
        Assert.True(ConversationModel.MayEdit(Of(Message(1, Me, "look", Circle with { Round = false }))));
    }

    /// <summary>"Video message" on the chat list, asked BEFORE "Video" — and the hidden row is still the hidden row.</summary>
    [Fact]
    public void AChatListRowSaysVideoMessageWhereAVideosSaysVideo()
    {
        var list = new ChatListModel(chats, () => Me);
        Assert.Equal("Video message", list.Preview(Message(1, 11, "", Circle), hidden: false));
        Assert.Equal("Video", list.Preview(Message(1, 11, "", Circle with { Round = false }), hidden: false));
        // Two videos are not a circle, whatever they carry.
        Assert.Equal("Video", list.Preview(Message(1, 11, "", Circle, Circle with { Id = 92 }), hidden: false));
        Assert.Equal("Hidden — blocked member", list.Preview(Message(1, 11, "", Circle), hidden: true));
        // A voice note keeps this client's own word for it.
        Assert.Equal("Voice message", list.Preview(Message(1, 11, "", new AttachmentDto(93, "audio")), hidden: false));
    }

    /// <summary>
    /// A reply quoting a circle says "Video message" where the server's excerpt is empty (S5.7), when this device holds
    /// the quoted message; a held one with words, and one it does not hold, stay as the server cut them.
    /// </summary>
    [Fact]
    public void AQuoteOfAVideoMessageSaysSo()
    {
        chats.Apply(Message(40, 11, "", Circle));
        chats.Apply(Message(39, 11, "", Circle with { Id = 90, Round = false }));
        MessageDto ReplyTo(long id, string excerpt = "") =>
            new(50, Chat, Me, null, "lovely", Sent, ReplyTo: new ReplyToDto(id, 11, excerpt));

        Assert.Equal(new QuoteLine(40, "Bob", "Video message", false), Quotes.Of(ReplyTo(40), chats, Say)!.Reply);
        Assert.Equal(new QuoteLine(39, "Bob", "", false), Quotes.Of(ReplyTo(39), chats, Say)!.Reply);
        Assert.Equal(new QuoteLine(38, "Bob", "", false), Quotes.Of(ReplyTo(38), chats, Say)!.Reply);
        Assert.Equal(new QuoteLine(40, "Bob", "the server's", false), Quotes.Of(ReplyTo(40, "the server's"), chats, Say)!.Reply);
        // And at the second level the same.
        var deeper = new MessageDto(51, Chat, 11, null, "yes", Sent,
            ReplyTo: new ReplyToDto(50, Me, "lovely", new QuoteParentDto(40, 11, "")));
        Assert.Equal(new QuoteLine(40, "Bob", "Video message", false), Quotes.Of(deeper, chats, Say)!.Parent);
    }

    /// <summary>
    /// In the viewer it is a video message, not "Video" — when it was opened AS one, from what S5.1's test of the whole
    /// message found. A flagged photo, a flagged video among several, or one opened from a message's tiles is the ordinary
    /// item it otherwise is.
    /// </summary>
    [Fact]
    public void TheViewerCallsItAVideoMessage()
    {
        var round = new MediaAlbum([Circle], 0, round: true);
        Assert.True(round.IsRound);
        Assert.Equal("Video message", round.Title(Say));
        var video = AttachmentText.DisplayName("video", null, Say);
        Assert.Equal(video, new MediaAlbum([Circle with { Round = false }], 0, round: true).Title(Say));
        // Flagged, but not opened as a video message: its tile, in an album.
        Assert.False(new MediaAlbum([Circle], 0).IsRound);
        Assert.Equal(video, new MediaAlbum([Circle], 0).Title(Say));
        // Two flagged videos: the S5.1 test fails, so each is a video.
        var two = new MediaAlbum([Circle, Circle with { Id = 92 }], 1, round: true);
        Assert.False(two.IsRound);
        Assert.Equal(video, two.Title(Say));
        // The flag on a photo.
        var photo = new MediaAlbum([new AttachmentDto(93, "photo", Name: "beach.jpg", Round: true)], 0, round: true);
        Assert.False(photo.IsRound);
        Assert.Equal("beach.jpg", photo.Title(Say));
    }

    // ---- sending one (docs/protocol.md, "Video messages") ---------------------------------------------

    private sealed class Staging : IMediaStore
    {
        public Dictionary<string, StagedMedia> Files { get; } = [];

        public StagedMedia? Read(string handle) => Files.TryGetValue(handle, out var staged) ? staged : null;

        public void Sweep(IReadOnlySet<string> keep)
        {
        }
    }

    private sealed class NoSocket : IFrameSender
    {
        public bool IsConnected => false;

        public Task<bool> TrySend(string frame, CancellationToken ct = default) => Task.FromResult(false);

        public event Action<ServerFrame>? Frame;

        public void Unused() => Frame?.Invoke(new ServerFrame.Pong());
    }

    /// <summary>
    /// THE STICKER'S PATTERN: the row is written down first, the square video goes up as a video with its poster, and
    /// only then is the message posted — with <c>round: true</c>, no words, and the reply it answers.
    /// </summary>
    [Fact]
    public async Task SendRoundUploadsTheSquareVideoAndItsPosterThenPostsTheFlag()
    {
        const string Handle = "0123456789abcdef0123456789abcdef";
        var server = new Server()
            .On("/attachments",
                """{"attachment": {"id": 91, "kind": "video", "mime": "video/mp4", "size": 6, "width": 480, "height": 480, "duration_ms": 23400}}""",
                HttpStatusCode.Created)
            .On("/attachments/91/preview", null, HttpStatusCode.NoContent)
            .On("/chats/42/messages",
                """{"message": {"id": 1341, "chat_id": 42, "sender_id": 7, "client_msg_id": "c-1", "body": "", "created_at": "2026-10-05T10:00:00Z", "attachments": [{"id": 91, "kind": "video", "mime": "video/mp4", "size": 6, "width": 480, "height": 480, "duration_ms": 23400, "has_preview": true, "round": true}]}}""",
                HttpStatusCode.Created);
        var api = new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"));
        var staging = new Staging();
        staging.Files[Handle] = new StagedMedia(
            "video", "video/mp4", "ftypmp"u8.ToArray(), 480, 480, 23400, Preview: "jpeg"u8.ToArray());
        var media = new MediaOutbox(outbox, api, staging);
        var landed = new List<(bool Round, long Attachment)>();
        media.Landed += (owed, attachment, _) => landed.Add((owed.Round, attachment.Id));
        var sending = new SendPipeline(
            new NoSocket(), outbox, chats, api, nextId: () => "c-1", uploads: async ct => await media.PushAsync(ct));
        var chat = new ConversationModel(Chat, chats, api, sending, new NoSocket(), outbox: outbox);

        var row = chat.SendRound(Handle, replyToMessageId: 41);

        Assert.True(row.Round);
        Assert.False(row.Sticker);
        Assert.Equal(string.Empty, row.Body);
        Assert.True(Assert.Single(chat.Pending()).OwesUploads);
        Assert.Empty(server.Asked);

        Assert.Equal(1, await sending.FlushAsync(SendRules.FlushTrigger.ConnectivityRestored));

        Assert.Equal("/api/v1/attachments?kind=video&width=480&height=480&duration_ms=23400", server.Asked[0]);
        Assert.Equal("/api/v1/attachments/91/preview", server.Asked[1]);
        Assert.Equal("/api/v1/chats/42/messages", server.Asked[2]);
        Assert.Equal(
            """{"client_msg_id":"c-1","body":"","reply_to_message_id":41,"attachment_ids":[91],"round":true}""",
            server.Bodies[^1]);
        Assert.Equal([(true, 91L)], landed);
        Assert.Empty(outbox.All());
        Assert.Equal(91, chats.Message(1341)!.RoundVideo!.Id);
    }

    /// <summary>
    /// The reader's own circle is drawn from the bytes this device already holds: the poster it made is kept under the
    /// attachment's preview key, so the delivered circle is not downloaded back to draw itself (S5.6).
    /// </summary>
    [Fact]
    public async Task AKeptPosterIsDrawnWithoutAskingTheServer()
    {
        var server = new Server();
        var api = new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"));
        var blobs = new Dictionary<string, byte[]>();
        var attachments = new AttachmentCache(api, new Blobs(blobs));

        attachments.RememberPreview(Circle with { HasPreview = false }, "jpeg"u8.ToArray());
        attachments.Remember(Circle with { HasPreview = false }, "ftypmp"u8.ToArray());

        var (poster, error) = await attachments.BytesAsync(Circle, preview: true);
        Assert.Null(error);
        Assert.Equal("jpeg"u8.ToArray(), poster);
        var (video, _) = await attachments.BytesAsync(Circle);
        Assert.Equal("ftypmp"u8.ToArray(), video);
        Assert.Empty(server.Asked);
        Assert.True(attachments.Holds(Circle, preview: true));
    }

    private sealed class Blobs(Dictionary<string, byte[]> held) : IBlobStore
    {
        public byte[]? Read(string key) => held.TryGetValue(key, out var bytes) ? bytes : null;

        public void Write(string key, ReadOnlyMemory<byte> bytes) => held[key] = bytes.ToArray();
    }
}
