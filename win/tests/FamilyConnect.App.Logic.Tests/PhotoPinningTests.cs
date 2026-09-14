using System.Net;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

public sealed class PhotoPinningTests : IDisposable
{
    private readonly Database cache = Database.OpenInMemory();
    private readonly BoardStore board;
    private readonly List<TimeSpan> pauses = [];

    public PhotoPinningTests() => board = new BoardStore(cache);

    public void Dispose() => cache.Dispose();

    private const string Uploaded = """{"attachment": {"id": 34, "kind": "photo", "width": 1600, "height": 1200}}""";

    private const string Pinned =
        """{"note": {"id": 70, "author_id": 7, "kind": "photo", "text": "", "board_seq": 5, "attachment": {"id": 34, "kind": "photo"}}}""";

    private static readonly StagedMedia Photo = new("photo", "image/jpeg", new byte[] { 1, 2, 3 }, 1600, 1200, Preview: new byte[] { 9 });

    private PhotoPinning Build(Server server) =>
        new(new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken")),
            board,
            (pause, _) =>
            {
                lock (pauses)
                {
                    pauses.Add(pause);
                }
                return Task.CompletedTask;
            });

    [Fact]
    public async Task APhotoIsUploadedPreviewedThenPinnedAsAPhotoNote()
    {
        var server = new Server()
            .On("/attachments", Uploaded)
            .On("/attachments/34/preview", null, HttpStatusCode.NoContent)
            .On("/families/mine/board/notes", Pinned);

        var outcome = await Build(server).PinAsync(Photo, (0.3, 0.4), "pink");

        Assert.True(outcome.Pinned);
        Assert.Equal(
            ["/api/v1/attachments?kind=photo&width=1600&height=1200", "/api/v1/attachments/34/preview", "/api/v1/families/mine/board/notes"],
            server.Asked);
        using var note = System.Text.Json.JsonDocument.Parse(server.Bodies[^1]);
        Assert.Equal(34, note.RootElement.GetProperty("attachment_id").GetInt64());
        Assert.Equal("photo", note.RootElement.GetProperty("kind").GetString());
        Assert.Equal(string.Empty, note.RootElement.GetProperty("text").GetString());
        Assert.Equal("pink", note.RootElement.GetProperty("color").GetString());
        Assert.Equal("medium", note.RootElement.GetProperty("size").GetString());
        Assert.NotNull(board.Note(70));
    }

    /// <summary>Best effort, but not best effort once: a preview the network lost is tried twice more.</summary>
    [Fact]
    public async Task APreviewTheNetworkLostIsTriedTwiceMoreBesideThePin()
    {
        var down = (HttpStatusCode.ServiceUnavailable, (string?)"""{"error": {"code": "internal", "message": "later"}}""");
        var server = new Server()
            .On("/attachments", Uploaded)
            .Then("/attachments/34/preview", down, down, down, down)
            .On("/families/mine/board/notes", Pinned);

        Assert.True((await Build(server).PinAsync(Photo, (0.3, 0.4), "pink")).Pinned);

        for (var look = 0; look < 100 && server.Asked.Count(path => path.EndsWith("/preview", StringComparison.Ordinal)) < 3; look++)
        {
            await Task.Delay(10);
        }
        Assert.Equal(3, server.Asked.Count(path => path.EndsWith("/preview", StringComparison.Ordinal)));
        Assert.Equal(PhotoPinning.PreviewRetries, pauses);
    }

    [Fact]
    public async Task ARefusedPreviewIsNotTriedAgain()
    {
        var server = new Server()
            .On("/attachments", Uploaded)
            .On("/attachments/34/preview", """{"error": {"code": "invalid_attachment", "message": "no"}}""", HttpStatusCode.BadRequest)
            .On("/families/mine/board/notes", Pinned);

        Assert.True((await Build(server).PinAsync(Photo, (0.3, 0.4), "pink")).Pinned);
        await Task.Delay(30);

        Assert.Single(server.Asked, path => path.EndsWith("/preview", StringComparison.Ordinal));
        Assert.Empty(pauses);
    }

    [Fact]
    public async Task ARefusedUploadPinsNothing()
    {
        var server = new Server().On(
            "/attachments", """{"error": {"code": "attachment_too_large", "message": "big"}}""", HttpStatusCode.RequestEntityTooLarge);

        var outcome = await Build(server).PinAsync(Photo, (0.3, 0.4), "pink");

        Assert.False(outcome.Pinned);
        Assert.Equal(ErrorCodes.AttachmentTooLarge, outcome.Error!.Code);
        Assert.Single(server.Asked);
        Assert.Equal("Couldn't pin that photo. That photo is too large to pin.",
            PhotoPinning.FailureText(outcome.Error, EnglishCatalog.Instance));
    }

    [Fact]
    public async Task AFullBoardRefusesThePinAndNothingIsApplied()
    {
        var server = new Server()
            .On("/attachments", Uploaded)
            .On("/attachments/34/preview", null, HttpStatusCode.NoContent)
            .On("/families/mine/board/notes", """{"error": {"code": "board_full", "message": "full"}}""", HttpStatusCode.Conflict);

        var outcome = await Build(server).PinAsync(Photo, (0.3, 0.4), "pink");

        Assert.Equal(ErrorCodes.BoardFull, outcome.Error!.Code);
        Assert.Empty(board.Notes());
    }

    /// <summary>One at a time, and said so: a second photo quietly dropped is a photo its sender thinks is on the wall.</summary>
    [Fact]
    public async Task OnePhotoIsPinnedAtATime()
    {
        var gate = new TaskCompletionSource<(HttpStatusCode, string?)>();
        var server = new Server()
            .OnAsync("/attachments", () => gate.Task)
            .On("/families/mine/board/notes", Pinned);
        var pinning = Build(server);

        var first = pinning.PinAsync(Photo with { Preview = null }, (0.3, 0.4), "pink");
        for (var look = 0; look < 100 && !pinning.Pinning; look++)
        {
            await Task.Delay(5);
        }
        Assert.True(pinning.Pinning);
        Assert.True((await pinning.PinAsync(Photo, (0.1, 0.1), "blue")).Busy);

        gate.SetResult((HttpStatusCode.OK, Uploaded));
        Assert.True((await first).Pinned);
        Assert.False(pinning.Pinning);
    }

    [Fact]
    public void APhotoLandsNearTheMiddleOrUnderThePointer()
    {
        var (x, y) = PhotoPinning.Scattered(() => 0.5);
        Assert.True(Math.Abs(0.35 - x) < 1e-9 && Math.Abs(0.30 - y) < 1e-9);
        Assert.True(Math.Abs(0.40 - PhotoPinning.Scattered(() => 1.0).X) < 1e-9);

        // A medium card centred under the pointer, on a wide wall: 150 x 110.
        var dropped = PhotoPinning.DroppedAt((575, 855), (1000, 1600));
        Assert.True(Math.Abs(0.5 - dropped.X) < 1e-9 && Math.Abs(0.5 - dropped.Y) < 1e-9);
        // Held inside the wall at the corner.
        Assert.Equal((0.0, 0.0), PhotoPinning.DroppedAt((5, 5), (1000, 1600)));
    }
}
