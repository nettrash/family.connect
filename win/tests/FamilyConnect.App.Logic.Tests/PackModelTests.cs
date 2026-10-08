using System.Buffers.Binary;
using System.Net;
using System.Text;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The family's sticker pack from the window's side (docs/protocol.md, "Sticker pack"): adding,
/// removing under the author-or-owner rule, deciding whether the pack holds a sent sticker, and
/// what one click in the panel sends.
/// </summary>
/// <remarks>
/// These are about the CHAT sticker. The board's cards are called stickers in this code too
/// (<see cref="BoardModelTests"/>), and nothing here touches them.
/// </remarks>
public class PackModelTests : IDisposable
{
    private const long Me = 7;
    private const long Ceiling = 8 * 1024;

    private static readonly DateTimeOffset Now = new(2026, 9, 13, 10, 0, 0, TimeSpan.Zero);

    private readonly Database cache = Database.OpenInMemory();
    private readonly PackStore pack;
    private readonly ChatStore chats;
    private readonly OutboxStore outbox;
    private DateTimeOffset now = Now;

    public PackModelTests()
    {
        pack = new PackStore(cache);
        chats = new ChatStore(cache, () => Me);
        outbox = new OutboxStore(cache);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        pack.SetLimits(new PackLimits(3, Ceiling));
    }

    public void Dispose() => cache.Dispose();

    // ---- fakes ---------------------------------------------------------------------------------

    private sealed record Asked(string Method, string Path, byte[] Body, string? ContentType);

    /// <summary>Answers by method and path, and keeps the BYTES of every body — a sticker is its bytes.</summary>
    private sealed class Wire : HttpMessageHandler
    {
        private readonly List<(string Method, string Endpoint, Queue<(HttpStatusCode Status, byte[]? Body)> Answers)> routes = [];

        public List<Asked> Requests { get; } = [];

        /// <summary>Called with each request as it arrives, BEFORE it is answered — where a frame that beats the answer lands.</summary>
        public Action<Asked>? Seen { get; set; }

        public Wire On(string method, string endpoint, params (HttpStatusCode Status, string? Json)[] answers)
        {
            routes.Add((method, "/api/v1" + endpoint, new Queue<(HttpStatusCode, byte[]?)>(
                answers.Select(answer => (answer.Status, answer.Json is null ? null : Encoding.UTF8.GetBytes(answer.Json))))));
            return this;
        }

        public Wire Bytes(long attachmentId, byte[] bytes)
        {
            routes.Add(("GET", $"/api/v1/attachments/{attachmentId}", new Queue<(HttpStatusCode, byte[]?)>([(HttpStatusCode.OK, bytes)])));
            return this;
        }

        public IEnumerable<string> Paths => Requests.Select(request => $"{request.Method} {request.Path}");

        protected override async Task<HttpResponseMessage> SendAsync(
            HttpRequestMessage request, CancellationToken cancellationToken)
        {
            var path = request.RequestUri!.PathAndQuery;
            var body = request.Content is null ? [] : await request.Content.ReadAsByteArrayAsync(cancellationToken);
            var asked = new Asked(request.Method.Method, path, body, request.Content?.Headers.ContentType?.MediaType);
            Requests.Add(asked);
            Seen?.Invoke(asked);
            var endpoint = path.Split('?')[0];
            foreach (var (method, where, answers) in routes)
            {
                if (method == request.Method.Method && where == endpoint)
                {
                    // The last answer stands for every later ask.
                    var (status, bytes) = answers.Count > 1 ? answers.Dequeue() : answers.Peek();
                    var response = new HttpResponseMessage(status);
                    if (bytes is not null)
                    {
                        response.Content = new ByteArrayContent(bytes);
                    }
                    return response;
                }
            }
            throw new HttpRequestException($"no route for {request.Method.Method} {path}");
        }
    }

    private sealed class Blobs : IBlobStore
    {
        public Dictionary<string, byte[]> Held { get; } = [];

        public byte[]? Read(string key) => Held.TryGetValue(key, out var bytes) ? bytes : null;

        public void Write(string key, ReadOnlyMemory<byte> bytes) => Held[key] = bytes.ToArray();
    }

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

    private sealed record Rig(PackModel Model, Wire Wire, Blobs Blobs, ApiClient Api);

    private Rig Build(Wire? wire = null)
    {
        wire ??= new Wire();
        var api = new ApiClient(
            new HttpClient(wire), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"));
        var blobs = new Blobs();
        return new Rig(new PackModel(pack, chats, api, new AttachmentCache(api, blobs), () => now), wire, blobs, api);
    }

    // ---- bytes ---------------------------------------------------------------------------------

    /// <summary>A WebP as its container says it: an extended header with a canvas, and padding to a length.</summary>
    private static byte[] WebP(int width = 512, int height = 512, bool animated = false, int length = 64, byte fill = 0xAB)
    {
        var file = new byte[Math.Max(length, 30)];
        Array.Fill(file, fill);
        "RIFF"u8.CopyTo(file);
        BinaryPrimitives.WriteUInt32LittleEndian(file.AsSpan(4), (uint)(file.Length - 8));
        "WEBP"u8.CopyTo(file.AsSpan(8));
        "VP8X"u8.CopyTo(file.AsSpan(12));
        BinaryPrimitives.WriteUInt32LittleEndian(file.AsSpan(16), 10);
        file[20] = (byte)(0x10 | (animated ? 0x02 : 0));
        file[21] = file[22] = file[23] = 0;
        Write24(file.AsSpan(24), width - 1);
        Write24(file.AsSpan(27), height - 1);
        return file;
    }

    private static byte[] Png(int width = 512, int height = 512, int length = 64)
    {
        var file = new byte[Math.Max(length, 33)];
        ((ReadOnlySpan<byte>)[0x89, (byte)'P', (byte)'N', (byte)'G', 0x0D, 0x0A, 0x1A, 0x0A]).CopyTo(file);
        BinaryPrimitives.WriteUInt32BigEndian(file.AsSpan(8), 13);
        "IHDR"u8.CopyTo(file.AsSpan(12));
        BinaryPrimitives.WriteUInt32BigEndian(file.AsSpan(16), (uint)width);
        BinaryPrimitives.WriteUInt32BigEndian(file.AsSpan(20), (uint)height);
        return file;
    }

    private static void Write24(Span<byte> at, int value)
    {
        at[0] = (byte)value;
        at[1] = (byte)(value >> 8);
        at[2] = (byte)(value >> 16);
    }

    private static PackItemDto Item(
        long id, long seq, long by = Me, long? size = 64, string mime = "image/webp", string? label = null, bool hasPreview = false) =>
        new(id, by, new AttachmentDto(70 + id, "photo", mime, size, 512, 512, HasPreview: hasPreview), "2026-09-13T10:00:00Z", seq, label);

    private static string Uploaded(long id, string mime = "image/webp") =>
        $$$"""{"attachment": {"id": {{{id}}}, "kind": "photo", "mime": "{{{mime}}}", "size": 64, "width": 512, "height": 512}}""";

    private static string Claimed(long id, long attachment, long seq, long by = Me) =>
        $$$"""
        {"item": {"id": {{{id}}}, "added_by": {{{by}}},
                  "attachment": {"id": {{{attachment}}}, "kind": "photo", "mime": "image/webp", "size": 64, "width": 512, "height": 512},
                  "created_at": "2026-09-13T10:00:00Z", "pack_seq": {{{seq}}}}}
        """;

    private static string Refusal(string code) => $$$"""{"error": {"code": "{{{code}}}", "message": "…"}}""";

    // ---- whether there is a pack at all ------------------------------------------------------

    /// <summary>
    /// A server that predates packs names no ceilings, and then nothing is offered — rather than a
    /// 404 when somebody taps a button.
    /// </summary>
    [Fact]
    public void ThePackIsOfferedOnlyWhereTheServerNamedItsCeilings()
    {
        var model = Build().Model;
        Assert.True(model.Offered);

        pack.SetLimits(null);

        Assert.False(model.Offered);
        Assert.False(model.IsFull);
    }

    // ---- adding --------------------------------------------------------------------------------

    /// <summary>
    /// THE BYPASS. A sticker goes up as the bytes it is — <c>kind=photo</c>, its own type, its own
    /// size — and with NO preview: a preview is a JPEG, which has no transparency and one frame.
    /// </summary>
    [Fact]
    public async Task AddingUploadsTheOriginalBytesUnpreparedAndThenClaimsThem()
    {
        var bytes = WebP(512, 384, animated: true, length: 4000);
        var rig = Build(new Wire()
            .On("POST", "/attachments", (HttpStatusCode.Created, Uploaded(71)))
            .On("POST", "/families/mine/pack", (HttpStatusCode.Created, Claimed(5, 71, 12))));
        pack.Replace([], 0);

        var (added, error) = await rig.Model.AddAsync(bytes, "  party cat ");

        Assert.Null(error);
        Assert.Equal(PackAdded.Added, added);
        Assert.Equal(
            ["POST /api/v1/attachments?kind=photo&width=512&height=384", "POST /api/v1/families/mine/pack"],
            rig.Wire.Paths);
        var upload = rig.Wire.Requests[0];
        // Byte for byte: not downscaled, not re-encoded, not stripped — and typed by what the bytes ARE.
        Assert.Equal(bytes, upload.Body);
        Assert.Equal("image/webp", upload.ContentType);
        Assert.Equal("""{"attachment_id":71,"label":"party cat"}""", Encoding.UTF8.GetString(rig.Wire.Requests[1].Body));
        // No `PUT …/preview` was ever sent.
        Assert.DoesNotContain(rig.Wire.Requests, request => request.Path.Contains("preview", StringComparison.Ordinal));

        Assert.Equal(5, Assert.Single(pack.Items()).Id);
        // The answer to this device's own POST moves no cursor.
        Assert.Equal(0, pack.Cursor);
        // And the bytes are kept under the id the pack holds them by, so the panel draws at once.
        Assert.Equal(bytes, rig.Blobs.Held["71"]);
    }

    [Fact]
    public async Task APngIsAStickerToo()
    {
        var bytes = Png(200, 100);
        var rig = Build(new Wire()
            .On("POST", "/attachments", (HttpStatusCode.Created, Uploaded(71, "image/png")))
            .On("POST", "/families/mine/pack", (HttpStatusCode.Created, Claimed(5, 71, 12))));

        var (added, _) = await rig.Model.AddAsync(bytes);

        Assert.Equal(PackAdded.Added, added);
        Assert.Equal("image/png", rig.Wire.Requests[0].ContentType);
        Assert.Equal("/api/v1/attachments?kind=photo&width=200&height=100", rig.Wire.Requests[0].Path);
        // No label was given, so no key was sent.
        Assert.Equal("""{"attachment_id":71}""", Encoding.UTF8.GetString(rig.Wire.Requests[1].Body));
    }

    /// <summary>
    /// ADDING THE SAME STICKER TWICE IS NOT AN ERROR AND NOT TWO STICKERS. The server answers
    /// <c>200</c> with the item that was there — whose attachment id is NOT the one just uploaded.
    /// </summary>
    [Fact]
    public async Task AddingWhatThePackAlreadyHoldsAnswersTheItemThatWasThere()
    {
        var bytes = WebP();
        pack.Replace([Item(5, 12)], 12);
        var rig = Build(new Wire()
            .On("POST", "/attachments", (HttpStatusCode.Created, Uploaded(99)))
            // The fresh upload (99) was dropped: the item carries the id the pack already had (75).
            .On("POST", "/families/mine/pack", (HttpStatusCode.OK, Claimed(5, 75, 12))));

        var (added, error) = await rig.Model.AddAsync(bytes);

        Assert.Null(error);
        Assert.Equal(PackAdded.AlreadyThere, added);
        Assert.Single(pack.Items());
        Assert.Equal(bytes, rig.Blobs.Held["75"]);
        Assert.False(rig.Blobs.Held.ContainsKey("99"));
    }

    /// <summary>
    /// THE FRAME CAN BEAT THE ANSWER. The server fans the actor's own <c>pack_item</c> frame out
    /// before the POST answers, so a genuinely NEW sticker is very often already in the cache when
    /// its claim comes back. That is still "Added": <c>201</c> says so, and the cache is not asked.
    /// </summary>
    [Fact]
    public async Task ANewStickerWhoseFrameArrivedFirstIsStillReportedAsAdded()
    {
        var bytes = WebP();
        pack.Replace([], 0);
        pack.CaughtUp(pack.Connection);
        var wire = new Wire()
            .On("POST", "/attachments", (HttpStatusCode.Created, Uploaded(71)))
            .On("POST", "/families/mine/pack", (HttpStatusCode.Created, Claimed(5, 71, 12)));
        var rig = Build(wire);
        wire.Seen = asked =>
        {
            if (asked.Method == "POST" && asked.Path.EndsWith("/families/mine/pack", StringComparison.Ordinal))
            {
                // The socket delivers this device's own add while the claim is still in the air.
                Assert.True(pack.Apply(Item(5, 12, size: 64) with { Attachment = new AttachmentDto(71, "photo", "image/webp", 64, 512, 512) }, SeqRoute.LiveFrame));
            }
        };

        var (added, error) = await rig.Model.AddAsync(bytes);

        Assert.Null(error);
        Assert.NotNull(pack.Item(5));
        Assert.Equal(PackAdded.Added, added);
        Assert.Equal("Added to family stickers", PackText.Sentence(added!.Value, EnglishCatalog.Instance));
        Assert.Equal(5, Assert.Single(pack.Items()).Id);
        // The frame moved the cursor (this connection had caught up); the answer moved nothing.
        Assert.Equal(12, pack.Cursor);
        Assert.Equal(bytes, rig.Blobs.Held["71"]);
    }

    /// <summary>The other order: the answer first, and the frame after it — one sticker, added once.</summary>
    [Fact]
    public async Task ANewStickerWhoseFrameArrivesAfterTheAnswerIsAddedOnce()
    {
        pack.Replace([], 0);
        pack.CaughtUp(pack.Connection);
        var rig = Build(new Wire()
            .On("POST", "/attachments", (HttpStatusCode.Created, Uploaded(71)))
            .On("POST", "/families/mine/pack", (HttpStatusCode.Created, Claimed(5, 71, 12))));

        var (added, error) = await rig.Model.AddAsync(WebP());

        Assert.Null(error);
        Assert.Equal(PackAdded.Added, added);
        Assert.Equal(0, pack.Cursor);
        // Then the frame: the same item at the same seq changes nothing drawn, and moves the cursor.
        Assert.False(pack.Apply(Item(5, 12), SeqRoute.LiveFrame));
        Assert.Equal(5, Assert.Single(pack.Items()).Id);
        Assert.Equal(12, pack.Cursor);
    }

    /// <summary>
    /// And the converse: <c>200</c> is "the pack already held it" even when THIS DEVICE had never
    /// heard of the item — a pack it has not read yet, or an add by somebody else it has not caught
    /// up with. The status says which, never the cache.
    /// </summary>
    [Fact]
    public async Task AStickerThePackAlreadyHeldIsSaidSoEvenWhenThisDeviceDidNotHoldIt()
    {
        var rig = Build(new Wire()
            .On("POST", "/attachments", (HttpStatusCode.Created, Uploaded(99)))
            .On("POST", "/families/mine/pack", (HttpStatusCode.OK, Claimed(5, 75, 12, by: 9))));
        Assert.Empty(pack.Items());

        var (added, error) = await rig.Model.AddAsync(WebP());

        Assert.Null(error);
        Assert.Equal(PackAdded.AlreadyThere, added);
        Assert.Equal("The family already has that sticker.", PackText.Sentence(added!.Value, EnglishCatalog.Instance));
        // The item is news all the same, and is held from here.
        Assert.Equal(5, Assert.Single(pack.Items()).Id);
    }

    // ---- the label -----------------------------------------------------------------------------

    /// <summary>
    /// AT MOST 64 CHARACTERS, COUNTED THE WAY THE SERVER COUNTS: Unicode scalar values after
    /// trimming. Not UTF-16 units — sixty-four emoji are 128 of those and are a fine label — and
    /// not what a screen shows: a family of four joined emoji is seven scalar values.
    /// </summary>
    [Fact]
    public void ALabelIsCountedInScalarValuesAfterTrimming()
    {
        Assert.Equal(64, PackLabel.MaxLength);
        Assert.Equal(0, PackLabel.Length(null));
        Assert.Equal(0, PackLabel.Length("   \t "));
        Assert.Equal(9, PackLabel.Length("  party cat \n"));
        Assert.Equal("party cat", PackLabel.Clean("  party cat \n"));
        Assert.Null(PackLabel.Clean("   "));
        Assert.Null(PackLabel.Clean(null));

        var sixtyFour = string.Concat(Enumerable.Repeat("🎉", 64));
        Assert.Equal(128, sixtyFour.Length);
        Assert.Equal(64, PackLabel.Length(sixtyFour));
        Assert.False(PackLabel.TooLong(sixtyFour));
        Assert.True(PackLabel.TooLong(sixtyFour + "🎉"));
        // Spaces at the ends are not counted, because they are not sent.
        Assert.False(PackLabel.TooLong("  " + new string('a', 64) + "  "));
        Assert.True(PackLabel.TooLong(new string('a', 65)));
        Assert.False(PackLabel.TooLong("Привет, 家族！"));
        Assert.Equal(7, PackLabel.Length("👨‍👩‍👧‍👦"));
        // A combining accent is a scalar value of its own, as the server counts it.
        Assert.Equal(2, PackLabel.Length("e\u0301"));
    }

    /// <summary>An over-long label is refused BEFORE ANY REQUEST, and in words.</summary>
    [Fact]
    public async Task AnOverLongLabelIsRefusedBeforeAnythingIsSent()
    {
        var rig = Build();
        var tooLong = new string('a', 65);

        var (added, error) = await rig.Model.AddAsync(WebP(), tooLong);
        var (fromMessage, fromMessageError) = await rig.Model.AddFromMessageAsync(
            new AttachmentDto(90, "photo", "image/webp", 64, 512, 512, Sticker: true), tooLong);

        Assert.Null(added);
        Assert.Null(fromMessage);
        Assert.Equal(PackLabel.TooLongCode, error!.Code);
        Assert.Equal(PackLabel.TooLongCode, fromMessageError!.Code);
        Assert.Empty(rig.Wire.Requests);
        Assert.Equal(
            "A sticker's label can be at most 64 characters.",
            PackText.Sentence(error, EnglishCatalog.Instance));
    }

    /// <summary>The longest label there is goes, trimmed, exactly as it was counted.</summary>
    [Fact]
    public async Task ALabelOfSixtyFourScalarValuesIsSentTrimmed()
    {
        var label = string.Concat(Enumerable.Repeat("🎉", 64));
        var rig = Build(new Wire()
            .On("POST", "/attachments", (HttpStatusCode.Created, Uploaded(71)))
            .On("POST", "/families/mine/pack", (HttpStatusCode.Created, Claimed(5, 71, 12))));

        var (added, error) = await rig.Model.AddAsync(WebP(), $"  {label} ");

        Assert.Null(error);
        Assert.Equal(PackAdded.Added, added);
        using var sent = System.Text.Json.JsonDocument.Parse(rig.Wire.Requests[1].Body);
        Assert.Equal(label, sent.RootElement.GetProperty("label").GetString());
    }

    /// <summary>"Add to family stickers" on a sent sticker carries the label it was given, too.</summary>
    [Fact]
    public async Task ASentStickerIsAddedWithTheLabelItWasGiven()
    {
        var bytes = WebP();
        var rig = Build(new Wire()
            .Bytes(90, bytes)
            .On("POST", "/attachments", (HttpStatusCode.Created, Uploaded(71)))
            .On("POST", "/families/mine/pack", (HttpStatusCode.Created, Claimed(5, 71, 12))));

        var (added, _) = await rig.Model.AddFromMessageAsync(
            new AttachmentDto(90, "photo", "image/webp", bytes.Length, 512, 512, Sticker: true), " party cat ");

        Assert.Equal(PackAdded.Added, added);
        Assert.Equal(
            """{"attachment_id":71,"label":"party cat"}""",
            Encoding.UTF8.GetString(rig.Wire.Requests.Last().Body));
    }

    /// <summary>
    /// REFUSED WHERE THE PERSON IS CHOOSING, from the ceilings this device already holds: nothing
    /// is uploaded to be told no.
    /// </summary>
    [Fact]
    public async Task APictureOverTheCeilingOrOfTheWrongKindIsRefusedBeforeAnythingIsSent()
    {
        var rig = Build();

        var (_, tooBig) = await rig.Model.AddAsync(WebP(length: (int)Ceiling + 1));
        var (_, jpeg) = await rig.Model.AddAsync(new byte[] { 0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0 });
        pack.Replace([Item(1, 1), Item(2, 2), Item(3, 3)], 3);
        var (_, full) = await rig.Model.AddAsync(WebP());

        Assert.Equal(ErrorCodes.PackItemTooLarge, tooBig!.Code);
        Assert.Equal(ErrorCodes.InvalidAttachment, jpeg!.Code);
        Assert.Equal(ErrorCodes.PackFull, full!.Code);
        Assert.True(rig.Model.IsFull);
        Assert.Empty(rig.Wire.Requests);
    }

    /// <summary>The server checks again — the pack may have filled since this device last looked — and its answer is passed on.</summary>
    [Fact]
    public async Task TheServersOwnRefusalIsPassedOnAndNothingIsHeld()
    {
        var rig = Build(new Wire()
            .On("POST", "/attachments", (HttpStatusCode.Created, Uploaded(71)))
            .On("POST", "/families/mine/pack", (HttpStatusCode.Conflict, Refusal("pack_full"))));

        var (added, error) = await rig.Model.AddAsync(WebP());

        Assert.Null(added);
        Assert.Equal(ErrorCodes.PackFull, error!.Code);
        Assert.Empty(pack.Items());
        Assert.Empty(rig.Blobs.Held);
    }

    /// <summary>
    /// <c>attachment_expired</c> means the sweep took the upload: upload it again — ONCE,
    /// automatically — and claim again. Nothing is shown; the label goes with the second claim too.
    /// </summary>
    [Fact]
    public async Task AnExpiredUploadKeepsItsLabelOnTheSecondClaim()
    {
        var rig = Build(new Wire()
            .On("POST", "/attachments", (HttpStatusCode.Created, Uploaded(71)), (HttpStatusCode.Created, Uploaded(72)))
            .On("POST", "/families/mine/pack",
                (HttpStatusCode.NotFound, Refusal("attachment_expired")),
                (HttpStatusCode.Created, Claimed(5, 72, 12))));

        var (added, error) = await rig.Model.AddAsync(WebP(), "party cat");

        Assert.Null(error);
        Assert.Equal(PackAdded.Added, added);
        Assert.Equal(
            ["POST /api/v1/attachments?kind=photo&width=512&height=512", "POST /api/v1/families/mine/pack",
             "POST /api/v1/attachments?kind=photo&width=512&height=512", "POST /api/v1/families/mine/pack"],
            rig.Wire.Paths);
        // The same bytes again, and the claim names the NEW upload.
        Assert.Equal(rig.Wire.Requests[0].Body, rig.Wire.Requests[2].Body);
        Assert.Equal("""{"attachment_id":72,"label":"party cat"}""", Encoding.UTF8.GetString(rig.Wire.Requests[3].Body));
    }

    /// <summary><c>attachment_expired</c> means the sweep took the upload: upload it again — once.</summary>
    [Fact]
    public async Task AnUploadTheSweepTookIsUploadedAgain()
    {
        var rig = Build(new Wire()
            .On("POST", "/attachments", (HttpStatusCode.Created, Uploaded(71)), (HttpStatusCode.Created, Uploaded(72)))
            .On("POST", "/families/mine/pack",
                (HttpStatusCode.NotFound, Refusal("attachment_expired")),
                (HttpStatusCode.Created, Claimed(5, 72, 12))));

        var (added, error) = await rig.Model.AddAsync(WebP());

        Assert.Null(error);
        Assert.Equal(PackAdded.Added, added);
        Assert.Equal(4, rig.Wire.Requests.Count);
        Assert.Equal("""{"attachment_id":72}""", Encoding.UTF8.GetString(rig.Wire.Requests[3].Body));
    }

    [Fact]
    public async Task AnUploadThatKeepsExpiringIsNotRetriedForEver()
    {
        var rig = Build(new Wire()
            .On("POST", "/attachments", (HttpStatusCode.Created, Uploaded(71)))
            .On("POST", "/families/mine/pack", (HttpStatusCode.NotFound, Refusal("attachment_expired"))));

        var (_, error) = await rig.Model.AddAsync(WebP());

        // Only the SECOND failure is shown: two uploads, two claims, and no third try.
        Assert.Equal(ErrorCodes.AttachmentExpired, error!.Code);
        Assert.Equal(4, rig.Wire.Requests.Count);
        Assert.Empty(pack.Items());
    }

    [Fact]
    public async Task AnUploadThatDidNotArriveIsATransientFailureAndClaimsNothing()
    {
        var rig = Build();

        var (added, error) = await rig.Model.AddAsync(WebP());

        Assert.Null(added);
        Assert.True(error!.Transient);
        Assert.Single(rig.Wire.Requests);
    }

    /// <summary>
    /// "Add to family stickers" on a sticker somebody SENT: the message's own bytes, uploaded again
    /// unprepared and claimed — read as the original even where the attachment says it has a preview.
    /// </summary>
    [Fact]
    public async Task ASentStickerIsAddedFromTheMessagesOwnBytes()
    {
        var bytes = WebP(fill: 0x11);
        var sent = new AttachmentDto(90, "photo", "image/webp", bytes.Length, 512, 512, HasPreview: true, Sticker: true);
        var rig = Build(new Wire()
            .Bytes(90, bytes)
            .On("POST", "/attachments", (HttpStatusCode.Created, Uploaded(91)))
            .On("POST", "/families/mine/pack", (HttpStatusCode.Created, Claimed(5, 91, 12, by: Me))));

        var (added, error) = await rig.Model.AddFromMessageAsync(sent);

        Assert.Null(error);
        Assert.Equal(PackAdded.Added, added);
        Assert.Equal("GET /api/v1/attachments/90", rig.Wire.Paths.First());
        Assert.Equal(bytes, rig.Wire.Requests[1].Body);
        Assert.DoesNotContain(rig.Wire.Requests, request => request.Path.Contains("preview", StringComparison.Ordinal));
    }

    // ---- who may remove ------------------------------------------------------------------------

    /// <summary>
    /// WHOEVER ADDED IT, OR THE FAMILY OWNER — a shape the board does not have: a member who left
    /// leaves their stickers behind, and under an author-only rule nobody could ever remove those.
    /// </summary>
    [Theory]
    [InlineData(7, 7, false, true)]   // mine
    [InlineData(9, 7, false, false)]  // somebody else's, and I am a member
    [InlineData(9, 7, true, true)]    // somebody else's, and I own the family
    [InlineData(7, 7, true, true)]
    [InlineData(0, 0, false, false)]  // nobody is signed in: an unknown adder is not "me"
    public void RemovalIsForWhoeverAddedItOrTheOwner(long addedBy, long reader, bool owner, bool may)
    {
        Assert.Equal(may, PackModel.MayRemove(Item(5, 12, by: addedBy), reader, owner));
    }

    [Fact]
    public void TheReaderIsWhoeverIsSignedIn()
    {
        var model = Build().Model;

        Assert.True(model.MayRemove(Item(5, 12, by: Me), readerOwnsFamily: false));
        Assert.False(model.MayRemove(Item(6, 13, by: 9), readerOwnsFamily: false));
        Assert.True(model.MayRemove(Item(6, 13, by: 9), readerOwnsFamily: true));
    }

    [Fact]
    public async Task RemovingTakesTheItemOutAtOnceAndMovesNoCursor()
    {
        pack.Replace([Item(5, 12), Item(6, 13)], 13);
        pack.Used(5, Now);
        var rig = Build(new Wire().On("DELETE", "/families/mine/pack/5", (HttpStatusCode.NoContent, null)));

        var error = await rig.Model.RemoveAsync(5);

        Assert.Null(error);
        Assert.Equal([6L], pack.Items().Select(item => item.Id));
        // The seq this removal was given is the server's to announce.
        Assert.Equal(13, pack.Cursor);
        // Remembered as gone: the frame for the ADD, arriving late, cannot put it back.
        Assert.False(pack.Apply(Item(5, 12), SeqRoute.LiveFrame));
        Assert.Empty(rig.Model.Panel().Recent);
    }

    [Fact]
    public async Task ARefusedRemovalLeavesTheItemWhereItIs()
    {
        pack.Replace([Item(5, 12, by: 9)], 12);
        var rig = Build(new Wire().On("DELETE", "/families/mine/pack/5", (HttpStatusCode.Forbidden, Refusal("not_pack_item_author"))));

        var error = await rig.Model.RemoveAsync(5);

        Assert.Equal(ErrorCodes.NotPackItemAuthor, error!.Code);
        Assert.Single(pack.Items());
    }

    /// <summary>
    /// <c>pack_item_not_found</c> ON A REMOVAL MEANS IT IS ALREADY GONE — which is what was asked
    /// for. The item is dropped here, remembered as gone, and NO ERROR comes back to be shown.
    /// </summary>
    [Fact]
    public async Task RemovingAnItemThatIsAlreadyGoneDropsItAndSaysNothing()
    {
        pack.Replace([Item(5, 12), Item(6, 13)], 13);
        pack.Used(5, Now);
        var rig = Build(new Wire().On("DELETE", "/families/mine/pack/5", (HttpStatusCode.NotFound, Refusal("pack_item_not_found"))));

        var error = await rig.Model.RemoveAsync(5);

        Assert.Null(error);
        Assert.Equal([6L], pack.Items().Select(item => item.Id));
        Assert.Empty(rig.Model.Panel().Recent);
        Assert.Equal(13, pack.Cursor);
        // Gone for good: an older copy arriving late cannot put it back.
        Assert.False(pack.Apply(Item(5, 12), SeqRoute.LiveFrame));
        Assert.Single(rig.Wire.Requests);
    }

    /// <summary>Any OTHER 404 is not "already gone", and a failure that is not a refusal leaves the item where it is.</summary>
    [Fact]
    public async Task OnlyThePacksOwnNotFoundIsReadAsGone()
    {
        pack.Replace([Item(5, 12)], 12);
        var rig = Build(new Wire().On(
            "DELETE", "/families/mine/pack/5",
            (HttpStatusCode.NotFound, Refusal("not_found")),
            (HttpStatusCode.BadGateway, null)));

        Assert.Equal("not_found", (await rig.Model.RemoveAsync(5))!.Code);
        Assert.True((await rig.Model.RemoveAsync(5))!.Transient);

        Assert.Single(pack.Items());
    }

    // ---- whether the pack holds a sent sticker ---------------------------------------------------

    /// <summary>
    /// Decided HERE, from bytes this device already has: an item whose size and type match and
    /// whose bytes are the same. Nothing on the wire names the item a message was sent from.
    /// </summary>
    [Fact]
    public async Task ThePackHoldsAStickerWhoseSizeTypeAndBytesMatch()
    {
        var bytes = WebP(fill: 0x11);
        pack.Replace([Item(5, 12, size: bytes.Length), Item(6, 13, size: bytes.Length)], 13);
        var rig = Build(new Wire().Bytes(90, bytes).Bytes(75, WebP(fill: 0x22)).Bytes(76, bytes));

        var held = await rig.Model.HoldsAsync(new AttachmentDto(90, "photo", "image/webp", bytes.Length, Sticker: true));

        Assert.True(held);
    }

    [Fact]
    public async Task TheSameSizeIsNotTheSameSticker()
    {
        var bytes = WebP(fill: 0x11);
        pack.Replace([Item(5, 12, size: bytes.Length)], 12);
        var rig = Build(new Wire().Bytes(90, bytes).Bytes(75, WebP(fill: 0x22)));

        Assert.False(await rig.Model.HoldsAsync(new AttachmentDto(90, "photo", "image/webp", bytes.Length, Sticker: true)));
    }

    /// <summary>A size or a type that matches nothing is answered without fetching a byte.</summary>
    [Fact]
    public async Task AStickerNoItemCouldBeIsNotHeldAndNothingIsFetched()
    {
        pack.Replace([Item(5, 12, size: 64), Item(6, 13, size: 64, mime: "image/png")], 13);
        var rig = Build();

        Assert.False(await rig.Model.HoldsAsync(new AttachmentDto(90, "photo", "image/webp", 65, Sticker: true)));
        Assert.False(await rig.Model.HoldsAsync(new AttachmentDto(90, "photo", "image/webp", Sticker: true)));
        Assert.False(await rig.Model.HoldsAsync(new AttachmentDto(90, "photo", "image/jpeg", 9999, Sticker: true)));
        Assert.Empty(rig.Wire.Requests);
    }

    /// <summary>
    /// Bytes that cannot be fetched read as "not held", which only offers what the pack may already
    /// have — and the server answers THAT with the item that was there.
    /// </summary>
    [Fact]
    public async Task AStickerWhoseBytesCannotBeReadIsOfferedRatherThanGuessedAt()
    {
        pack.Replace([Item(5, 12, size: 64)], 12);
        var rig = Build();

        Assert.False(await rig.Model.HoldsAsync(new AttachmentDto(90, "photo", "image/webp", 64, Sticker: true)));
    }

    // ---- sending -------------------------------------------------------------------------------

    /// <summary>
    /// What one click sends: the item's bytes AS CACHED, as one photo with NO preview — and read as
    /// the original even where <c>has_preview</c> is true, which it can be by dedup inheritance.
    /// </summary>
    [Fact]
    public async Task AStickerIsSentAsTheItemsOwnBytesWithNoPreview()
    {
        var bytes = WebP(512, 384, animated: true, length: 900);
        pack.Replace([Item(5, 12, size: bytes.Length, hasPreview: true)], 12);
        var rig = Build(new Wire().Bytes(75, bytes));

        var (media, error) = await rig.Model.ToSendAsync(pack.Item(5)!);

        Assert.Null(error);
        Assert.Equal("photo", media!.Kind);
        Assert.Equal("image/webp", media.Mime);
        Assert.Equal(bytes, media.Bytes.ToArray());
        Assert.Null(media.Preview);
        Assert.Equal((512, 512), (media.Width, media.Height));
        // The original, never `/preview`.
        Assert.Equal(["GET /api/v1/attachments/75"], rig.Wire.Paths);

        // Kept: the next click is not a download, and neither is a send with no network at all.
        await rig.Model.ToSendAsync(pack.Item(5)!);
        Assert.Single(rig.Wire.Requests);
    }

    /// <summary>
    /// A sticker is a COPY of bytes this device holds. An item it has never fetched, on a device
    /// that is offline, cannot be sent until they are here — and that is a transient failure, said
    /// as one.
    /// </summary>
    [Fact]
    public async Task AnItemWhoseBytesAreNotHereCannotBeSentOffline()
    {
        pack.Replace([Item(5, 12)], 12);
        var rig = Build();

        var (media, error) = await rig.Model.ToSendAsync(pack.Item(5)!);

        Assert.Null(media);
        Assert.True(error!.Transient);
    }

    /// <summary>The per-item ceiling binds a sticker MESSAGE too, and an operator may have lowered it since.</summary>
    [Fact]
    public async Task AnItemOverTodaysCeilingIsRefusedHereRatherThanAsAFailedBubble()
    {
        var bytes = WebP(length: 2000);
        pack.Replace([Item(5, 12, size: bytes.Length)], 12);
        pack.SetLimits(new PackLimits(200, 1000));
        var rig = Build(new Wire().Bytes(75, bytes));

        var (media, error) = await rig.Model.ToSendAsync(pack.Item(5)!);

        Assert.Null(media);
        Assert.Equal(ErrorCodes.PackItemTooLarge, error!.Code);
    }

    /// <summary>
    /// THE WHOLE SEND, OFFLINE FIRST. One click writes a row down with its staged bytes; the media
    /// pump uploads those bytes untouched and with no preview; and the message that is then posted
    /// says <c>sticker: true</c>, has no body, and may be a reply.
    /// </summary>
    [Fact]
    public async Task OneClickQueuesAMessageThatLandsAsAStickerWheneverTheNetworkLetsIt()
    {
        var bytes = WebP(512, 512, animated: true, length: 700);
        pack.Replace([Item(5, 12, size: bytes.Length)], 12);
        const string Delivered =
            """
            {"message": {"id": 1340, "chat_id": 42, "sender_id": 7, "client_msg_id": "c-1", "body": "",
             "created_at": "2026-09-13T10:00:00Z",
             "attachments": [{"id": 90, "kind": "photo", "mime": "image/webp", "size": 700, "sticker": true}]}}
            """;
        var rig = Build(new Wire()
            .Bytes(75, bytes)
            .On("POST", "/attachments", (HttpStatusCode.Created, Uploaded(90)))
            .On("POST", "/chats/42/messages", (HttpStatusCode.Created, Delivered)));
        var staging = new Staging();
        var media = new MediaOutbox(outbox, rig.Api, staging);
        var landed = new List<(bool Sticker, long Attachment, byte[] Bytes)>();
        media.Landed += (owed, attachment, file) => landed.Add((owed.Sticker, attachment.Id, file.Bytes.ToArray()));
        var sending = new SendPipeline(
            new NoSocket(), outbox, chats, rig.Api,
            now: () => now, nextId: () => "c-1", uploads: async ct => await media.PushAsync(ct));
        var chat = new ConversationModel(42, chats, rig.Api, sending, new NoSocket(), outbox: outbox);

        // The click.
        var (staged, _) = await rig.Model.ToSendAsync(pack.Item(5)!);
        staging.Files["0123456789abcdef0123456789abcdef"] = staged!;
        var row = chat.SendSticker("0123456789abcdef0123456789abcdef", replyToMessageId: 1337);
        rig.Model.Sent(5);

        // Written down BEFORE anything moves: this is the offline send.
        Assert.True(row.Sticker);
        Assert.Equal(string.Empty, row.Body);
        Assert.True(Assert.Single(chat.Pending()).OwesUploads);
        Assert.Single(rig.Wire.Requests);

        // The network comes back.
        Assert.Equal(1, await sending.FlushAsync(SendRules.FlushTrigger.ConnectivityRestored));

        var upload = rig.Wire.Requests[1];
        Assert.Equal("/api/v1/attachments?kind=photo&width=512&height=512", upload.Path);
        Assert.Equal("image/webp", upload.ContentType);
        Assert.Equal(bytes, upload.Body);
        Assert.Equal(
            """{"client_msg_id":"c-1","body":"","reply_to_message_id":1337,"attachment_ids":[90],"sticker":true}""",
            Encoding.UTF8.GetString(rig.Wire.Requests[2].Body));
        Assert.Equal(3, rig.Wire.Requests.Count);
        Assert.Empty(chat.Pending());
        // The upload is announced with the id the server gave it and the bytes that went — which is how the message
        // it becomes is drawn without downloading the picture back.
        Assert.Equal((true, 90L), (Assert.Single(landed).Sticker, landed[0].Attachment));
        Assert.Equal(bytes, landed[0].Bytes);
        // And it is drawn as what it is.
        Assert.NotNull(chats.Message(1340)!.StickerPicture);
        Assert.Equal([5L], rig.Model.Panel().Recent.Select(item => item.Id));
    }

    // ---- the panel -----------------------------------------------------------------------------

    [Fact]
    public void ThePanelLeadsWithWhatThisDeviceSentLastAndThenShowsTheWholePack()
    {
        pack.SetLimits(new PackLimits(200, Ceiling));
        pack.Replace([Item(5, 12), Item(6, 13), Item(7, 14), Item(8, 15)], 15);
        var model = Build().Model;
        Assert.Empty(model.Panel().Recent);

        model.Sent(7);
        now = now.AddMinutes(1);
        model.Sent(5);
        now = now.AddMinutes(1);
        model.Sent(7);

        var panel = model.Panel();
        Assert.Equal([7L, 5], panel.Recent.Select(item => item.Id));
        // The whole pack, in the order it was added, recents included: a sticker never moves under a finger.
        Assert.Equal([5L, 6, 7, 8], panel.All.Select(item => item.Id));
        Assert.False(panel.IsEmpty);
    }

    [Fact]
    public void RecentsAreCappedAndForgetWhatThePackNoLongerHolds()
    {
        var items = Enumerable.Range(1, 20).Select(id => Item(id, id)).ToList();
        var recents = Enumerable.Range(1, 20).Select(id => (long)id).Reverse().Prepend(99L).ToList();

        var panel = PackPanel.Of(items, recents);

        // The last SIXTEEN, the same on every client.
        Assert.Equal(16, PackPanel.RecentRoom);
        Assert.Equal(16, panel.Recent.Count);
        Assert.Equal(20, panel.Recent[0].Id);
        Assert.Equal(5, panel.Recent[^1].Id);
        Assert.DoesNotContain(panel.Recent, item => item.Id == 99);
        Assert.True(PackPanel.Of([], [5]).IsEmpty);
    }

    /// <summary>
    /// RECENTLY USED IS THIS DEVICE'S, AND IT SURVIVES A RESTART: it is in the cache file, not in
    /// a view or a session, so a model built again over the same cache — which is what a relaunch
    /// is — leads the panel with the same sixteen. It is cleared at sign-out, with everything else.
    /// </summary>
    [Fact]
    public void RecentsSurviveARestartAreSixteenAndAreClearedAtSignOut()
    {
        pack.SetLimits(new PackLimits(200, Ceiling));
        pack.Replace([.. Enumerable.Range(1, 20).Select(id => Item(id, id))], 20);
        var model = Build().Model;
        for (var id = 1; id <= 20; id++)
        {
            model.Sent(id);
            now = now.AddSeconds(1);
        }

        // A relaunch: new stores and a new model over the same cache file.
        var relaunched = new PackModel(
            new PackStore(cache), new ChatStore(cache, () => Me), Build().Api, new AttachmentCache(Build().Api, new Blobs()));
        var panel = relaunched.Panel();

        Assert.Equal(
            Enumerable.Range(5, 16).Reverse().Select(id => (long)id),
            panel.Recent.Select(item => item.Id));
        // Shown first, and the whole pack after them in the order it was added.
        Assert.Equal(20, panel.All.Count);

        // Sign-out (AppSession.SignOutAsync) wipes the cache; nothing of whose habits these were is left.
        cache.WipeAll();
        Assert.Empty(new PackStore(cache).Recents(PackPanel.RecentRoom));
        Assert.Empty(relaunched.Panel().Recent);
    }

    /// <summary>
    /// A BLOCK DOES NOT REACH THE PACK. A blocked member's sticker MESSAGE is hidden like anything
    /// they say; their pack ITEMS are drawn for everyone — a panel one sticker short for one member
    /// would be a quantity that moved when they blocked somebody.
    /// </summary>
    [Fact]
    public void ABlockedMembersItemsAreInThePanelLikeAnybodys()
    {
        pack.Replace([Item(5, 12, by: 9), Item(6, 13)], 13);
        chats.SetBlocked(9, true);
        var model = Build().Model;

        Assert.Equal([5L, 6], model.Items().Select(item => item.Id));
        Assert.Equal([5L, 6], model.Panel().All.Select(item => item.Id));
        // And their sticker MESSAGE is hidden, by the conversation's own rule, untouched by any of this.
        var list = new ChatListModel(chats, () => Me);
        var sticker = new MessageDto(
            1, 42, 9, null, string.Empty, "2026-09-13T10:00:00Z",
            Attachments: [new AttachmentDto(90, "photo", "image/webp", 64, Sticker: true)]);
        Assert.True(list.IsHidden(sticker));
        Assert.Equal("Hidden — blocked member", list.Preview(sticker, hidden: true));
    }
}

/// <summary>
/// What is decided about a picture somebody chose for the pack, before anything is uploaded
/// (docs/protocol.md, "512 × 512 is the CLIENT's rule").
/// </summary>
public class PackPickingTests
{
    private static readonly PackLimits Limits = new(200, 1000);

    private static byte[] WebP(int width, int height, bool animated, int length)
    {
        var file = new byte[Math.Max(length, 30)];
        "RIFF"u8.CopyTo(file);
        "WEBP"u8.CopyTo(file.AsSpan(8));
        "VP8X"u8.CopyTo(file.AsSpan(12));
        BinaryPrimitives.WriteUInt32LittleEndian(file.AsSpan(16), 10);
        file[20] = (byte)(animated ? 0x02 : 0);
        var (w, h) = (width - 1, height - 1);
        (file[24], file[25], file[26]) = ((byte)w, (byte)(w >> 8), (byte)(w >> 16));
        (file[27], file[28], file[29]) = ((byte)h, (byte)(h >> 8), (byte)(h >> 16));
        return file;
    }

    /// <summary>
    /// A FINISHED STICKER IS TAKEN AS GIVEN, whatever its pixel size, so long as it is within the
    /// byte ceiling: re-encoding it buys nothing and, for an animated one, is not possible.
    /// </summary>
    [Theory]
    [InlineData(512, 512, false)]
    [InlineData(96, 96, false)]
    [InlineData(2000, 2000, false)]
    [InlineData(2000, 2000, true)]
    public void AStickerWithinTheCeilingGoesUpAsItIs(int width, int height, bool animated)
    {
        var plan = PackPicking.For(WebP(width, height, animated, 1000), Limits, held: 0);

        Assert.Equal(PackPicking.Step.Take, plan.What);
        Assert.Null(plan.Refused);
    }

    /// <summary>
    /// 512 × 512 is THIS CLIENT's rule, when it MAKES one: a still picture over the ceiling and
    /// larger than the box is fitted into it — whole, proportions kept.
    /// </summary>
    [Fact]
    public void ALargeStillPictureOverTheCeilingIsFittedIntoTheBox()
    {
        var plan = PackPicking.For(WebP(2048, 1024, animated: false, 1001), Limits, held: 0);

        Assert.Equal(PackPicking.Step.Make, plan.What);
        Assert.Equal((512, 256), plan.Target);
    }

    /// <summary>NEVER RE-ENCODE AN ANIMATED ONE: over the ceiling it is simply too big.</summary>
    [Fact]
    public void AnAnimatedStickerOverTheCeilingIsRefusedAndNeverShrunk()
    {
        var plan = PackPicking.For(WebP(2048, 1024, animated: true, 1001), Limits, held: 0);

        Assert.Equal(PackPicking.Step.Refuse, plan.What);
        Assert.Equal(ErrorCodes.PackItemTooLarge, plan.Refused!.Code);
    }

    /// <summary>One that already fits the box cannot be made smaller by redrawing the same pixels.</summary>
    [Fact]
    public void AStillPictureAlreadyInTheBoxAndOverTheCeilingIsRefused()
    {
        var plan = PackPicking.For(WebP(512, 512, animated: false, 1001), Limits, held: 0);

        Assert.Equal(PackPicking.Step.Refuse, plan.What);
        Assert.Equal(ErrorCodes.PackItemTooLarge, plan.Refused!.Code);
    }

    /// <summary>The most basic thing wrong first, in the order the server checks: the type, the size, and only then the room.</summary>
    [Fact]
    public void TheRefusalIsTheMostBasicThingWrong()
    {
        byte[] jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0];

        // What the picture IS comes first: a moving one that is not a WebP is refused as that even into a full pack.
        Assert.Equal(PackPicking.AnimatedCode, PackPicking.For(Gif(frames: 2), Limits, held: 200).Refused!.Code);
        Assert.Equal(ErrorCodes.PackFull, PackPicking.For(jpeg, Limits, held: 200).Refused!.Code);
        Assert.Equal(ErrorCodes.PackItemTooLarge, PackPicking.For(WebP(512, 512, false, 1001), Limits, held: 200).Refused!.Code);
        Assert.Equal(ErrorCodes.PackFull, PackPicking.For(WebP(512, 512, false, 1000), Limits, held: 200).Refused!.Code);
        Assert.Equal(ErrorCodes.PackFull, PackPicking.For(WebP(2048, 2048, false, 1001), Limits, held: 200).Refused!.Code);
        Assert.Null(PackPicking.For(WebP(512, 512, false, 1000), Limits, held: 199).Refused);
    }

    // ---- what may be picked ------------------------------------------------------------------

    private static byte[] Png(int width, int height, int length, bool animated = false)
    {
        var file = new byte[Math.Max(length, 64)];
        ((ReadOnlySpan<byte>)[0x89, (byte)'P', (byte)'N', (byte)'G', 0x0D, 0x0A, 0x1A, 0x0A]).CopyTo(file);
        BinaryPrimitives.WriteUInt32BigEndian(file.AsSpan(8), 13);
        "IHDR"u8.CopyTo(file.AsSpan(12));
        BinaryPrimitives.WriteUInt32BigEndian(file.AsSpan(16), (uint)width);
        BinaryPrimitives.WriteUInt32BigEndian(file.AsSpan(20), (uint)height);
        // IHDR's crc (4), then the next chunk: an animation control chunk, or the image data.
        BinaryPrimitives.WriteUInt32BigEndian(file.AsSpan(33), 8);
        (animated ? "acTL"u8 : "IDAT"u8).CopyTo(file.AsSpan(37));
        return file;
    }

    /// <summary>A GIF of that many pictures, the way the format writes them.</summary>
    private static byte[] Gif(int frames)
    {
        var file = new List<byte>();
        file.AddRange("GIF89a"u8.ToArray());
        file.AddRange([2, 0, 2, 0, 0, 0, 0]);
        for (var frame = 0; frame < frames; frame++)
        {
            file.AddRange([0x21, 0xF9, 4, 0, 10, 0, 0, 0]);
            file.AddRange([0x2C, 0, 0, 0, 0, 2, 0, 2, 0, 0, 2, 2, 0x4C, 0x01, 0]);
        }
        file.Add(0x3B);
        return [.. file];
    }

    /// <summary>
    /// ANY STILL PICTURE THE PLATFORM CAN DECODE MAY BE PICKED. One that is not a WebP or a PNG —
    /// a JPEG, a HEIC, a still GIF — is MADE into a sticker: fitted whole into 512 × 512 and
    /// written as PNG, whatever its byte size. Never refused for its type, and never taken as given.
    /// </summary>
    [Theory]
    [InlineData(new byte[] { 0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, (byte)'J', (byte)'F', (byte)'I', (byte)'F', 0, 1 })]
    [InlineData(new byte[] { 0, 0, 0, 0x18, (byte)'f', (byte)'t', (byte)'y', (byte)'p', (byte)'h', (byte)'e', (byte)'i', (byte)'c' })]
    [InlineData(new byte[] { (byte)'B', (byte)'M', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 })]
    public void AStillPictureOfAnotherKindIsMadeIntoASticker(byte[] head)
    {
        var small = PackPicking.For(head, Limits, held: 0);
        var large = PackPicking.For([.. head, .. new byte[5000]], Limits, held: 0);

        Assert.Equal(PackPicking.Step.Make, small.What);
        Assert.Equal(PackPicking.Step.Make, large.What);
        Assert.Null(small.Refused);
        // The decoder says how big it is; the fit is this client's (TheBoxKeepsTheWholePictureAndItsShape).
        Assert.Null(small.Target);
    }

    [Fact]
    public void AStillGifIsMadeIntoAStickerToo()
    {
        Assert.Equal(PackPicking.Step.Make, PackPicking.For(Gif(frames: 1), Limits, held: 0).What);
    }

    /// <summary>
    /// AN ANIMATED PICTURE THAT IS NOT ALREADY AN ACCEPTABLE WEBP IS REFUSED, in words — never
    /// made into a sticker, which would silently keep one frame of it.
    /// </summary>
    [Fact]
    public void AnAnimatedGifIsRefusedAndNeverFlattened()
    {
        var plan = PackPicking.For(Gif(frames: 3), Limits, held: 0);

        Assert.Equal(PackPicking.Step.Refuse, plan.What);
        Assert.Equal(PackPicking.AnimatedCode, plan.Refused!.Code);
        Assert.Equal("Animated stickers must be WebP.", PackText.Sentence(plan.Refused, EnglishCatalog.Instance));
        // Not a server's code, and not a transient failure: trying again would refuse again.
        Assert.False(plan.Refused.Transient);
    }

    /// <summary>
    /// An animated PNG this client would have to REDRAW — one over the byte ceiling — would come
    /// out as its first frame: refused with the same sentence, whatever its pixel size. One within
    /// the ceiling is a finished sticker and goes up byte for byte, its animation untouched.
    /// </summary>
    [Fact]
    public void AnAnimatedPngThatWouldBeFlattenedIsRefused()
    {
        var big = PackPicking.For(Png(2048, 2048, 1001, animated: true), Limits, held: 0);
        var inTheBox = PackPicking.For(Png(512, 512, 1001, animated: true), Limits, held: 0);
        var within = PackPicking.For(Png(2048, 2048, 1000, animated: true), Limits, held: 0);

        Assert.Equal(PackPicking.AnimatedCode, big.Refused!.Code);
        Assert.Equal(PackPicking.AnimatedCode, inTheBox.Refused!.Code);
        Assert.Equal(PackPicking.Step.Take, within.What);
        // A STILL PNG of the same size is what the box is for.
        Assert.Equal(PackPicking.Step.Make, PackPicking.For(Png(2048, 1024, 1001), Limits, held: 0).What);
        Assert.Equal((512, 256), PackPicking.For(Png(2048, 1024, 1001), Limits, held: 0).Target);
    }

    /// <summary>A PNG within the ceiling is taken as given, like a WebP — whatever its pixel size.</summary>
    [Theory]
    [InlineData(96, 96)]
    [InlineData(512, 512)]
    [InlineData(2000, 1500)]
    public void APngWithinTheCeilingGoesUpAsItIs(int width, int height)
    {
        Assert.Equal(PackPicking.Step.Take, PackPicking.For(Png(width, height, 1000), Limits, held: 0).What);
    }

    /// <summary>A picture made into a sticker is asked again, as the bytes it now is — and may still be too big.</summary>
    [Fact]
    public void BytesAboutToGoUpAreAskedOnceMore()
    {
        Assert.Null(PackPicking.Refusal(WebP(512, 512, false, 1000), Limits, held: 0));
        Assert.Equal(ErrorCodes.PackItemTooLarge, PackPicking.Refusal(WebP(512, 512, false, 1001), Limits, held: 0)!.Code);
        Assert.Equal(ErrorCodes.InvalidAttachment, PackPicking.Refusal("GIF89a......"u8, Limits, held: 0)!.Code);
        Assert.Equal(ErrorCodes.PackFull, PackPicking.Refusal(WebP(512, 512, false, 10), Limits, held: 200)!.Code);
    }

    [Theory]
    [InlineData(2048, 1024, 512, 256)]
    [InlineData(1024, 2048, 256, 512)]
    [InlineData(4000, 3, 512, 1)]
    [InlineData(300, 200, 300, 200)]  // never scaled UP
    [InlineData(513, 513, 512, 512)]
    public void TheBoxKeepsTheWholePictureAndItsShape(int width, int height, int fittedWidth, int fittedHeight)
    {
        Assert.Equal((fittedWidth, fittedHeight), PackPicking.Fit(width, height));
    }

    [Fact]
    public void ARefusalIsSaidInThePacksOwnWords()
    {
        var say = EnglishCatalog.Instance;

        Assert.Equal("That picture is too big to be a sticker.", PackText.Sentence(new ApiError(ErrorCodes.PackItemTooLarge, "…", 413), say));
        Assert.Equal("A sticker has to be a WebP or PNG picture.", PackText.Sentence(new ApiError(ErrorCodes.InvalidAttachment, "…", 400), say));
        Assert.Equal("The family's stickers are full. Remove one to make room.", PackText.Sentence(new ApiError(ErrorCodes.PackFull, "…", 409), say));
        Assert.Equal(
            "Only whoever added a sticker, or the family owner, can remove it.",
            PackText.Sentence(new ApiError(ErrorCodes.NotPackItemAuthor, "…", 403), say));
        // A transient failure is never said as a refusal.
        Assert.Equal(
            "The sticker didn't reach the server. Check your connection and try again.",
            PackText.Sentence(ApiError.Transport("down"), say));
        Assert.Equal("Something went wrong. Try again.", PackText.Sentence(new ApiError(ErrorCodes.PackItemNotFound, "…", 404), say));
        Assert.Equal("The family already has that sticker.", PackText.Sentence(PackAdded.AlreadyThere, say));
        Assert.Equal("Added to family stickers", PackText.Sentence(PackAdded.Added, say));
    }

    [Fact]
    public void AStickerIsNamedForAScreenReaderAndThePackIsCounted()
    {
        var say = EnglishCatalog.Instance;
        var picture = new AttachmentDto(71, "photo", "image/webp");

        Assert.Equal("Sticker: party cat", PackText.Name(new PackItemDto(5, 7, picture, Label: "party cat"), say));
        Assert.Equal("Sticker", PackText.Name(new PackItemDto(5, 7, picture), say));
        Assert.Equal("12 of 200 stickers", PackText.Fullness(12, new PackLimits(200, 524_288), say));
    }
}
