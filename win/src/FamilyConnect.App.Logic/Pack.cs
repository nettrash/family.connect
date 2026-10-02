using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>What an add came to, when it was not refused.</summary>
public enum PackAdded
{
    /// <summary>A new item: the pack is one sticker bigger.</summary>
    Added,

    /// <summary>
    /// The pack ALREADY held it — the server's <c>200</c>. Not an error and not two stickers:
    /// two members who both liked it do not fill the panel with copies.
    /// </summary>
    AlreadyThere,
}

/// <summary>The sticker panel over the composer: what this device sent lately, and then the whole pack.</summary>
/// <param name="Recent">
/// Newest first, and THIS DEVICE's alone — never on the wire, because it says something about a
/// person's habits and nothing about the family's pack.
/// </param>
/// <param name="All">The whole pack in the order the items were added, which never reshuffles under a finger.</param>
public sealed record PackPanel(IReadOnlyList<PackItemDto> Recent, IReadOnlyList<PackItemDto> All)
{
    /// <summary>
    /// How many recent stickers lead the panel: the last SIXTEEN, the same on every client. They
    /// are this DEVICE's — kept in its cache file, so they are there after a restart — and they go
    /// with everything else at sign-out.
    /// </summary>
    public const int RecentRoom = 16;

    public bool IsEmpty => All.Count == 0;

    /// <summary>
    /// The panel from what is held. A recent id the pack no longer holds is simply not there: the
    /// item was removed, and a removed sticker is not offered for sending.
    /// </summary>
    public static PackPanel Of(IReadOnlyList<PackItemDto> items, IReadOnlyList<long> recents)
    {
        var byId = items.ToDictionary(item => item.Id);
        List<PackItemDto> recent =
        [
            .. recents.Distinct()
                .Where(byId.ContainsKey)
                .Select(id => byId[id])
                .Take(RecentRoom),
        ];
        return new PackPanel(recent, [.. items.OrderBy(item => item.Id)]);
    }
}

/// <summary>
/// The family's sticker pack from the window's side (docs/protocol.md, "Sticker pack"): what the
/// panel shows, who may remove what, and the three writes — add, remove, send.
/// </summary>
/// <remarks>
/// <para>
/// <b>"STICKER" HERE IS THE CHAT ONE</b> — a small picture sent as its own message, the way a
/// messenger's stickers are. The cards on the board are called stickers in this code too
/// (<see cref="Sticker"/>, <c>StickerFace</c>); the two never meet, and everything about THIS one
/// is spelled <c>Pack</c> so that nobody has to read a name twice.
/// </para>
/// <para>
/// <b>A STICKER IS NEVER PREPARED.</b> Everything this client does to a photograph on its way up —
/// <c>MediaPreparing</c>'s decode, downscale, JPEG re-encode, metadata strip and preview — would
/// destroy exactly what makes a sticker one: a JPEG has no transparency and one frame. So nothing
/// here goes near that path. A pack item, and a sticker message, are built from the ORIGINAL
/// bytes as <see cref="StagedMedia"/> with no preview, and that is also what makes the copy free:
/// the message's upload hashes to the pack item's file.
/// </para>
/// <para>
/// <b>A SENT STICKER IS A COPY.</b> The message carries its own attachment and does not name the
/// pack item; the pack does not know about the message. So a send needs the item's BYTES, which
/// this device keeps under the attachment id — and an item whose bytes are not here yet, on a
/// device that is offline, cannot be sent until they are.
/// </para>
/// <para>
/// <b>BLOCKS DO NOT REACH THE PACK.</b> A blocked member's sticker MESSAGE is hidden like any
/// message of theirs — that is the conversation's rule and it is untouched — but their pack ITEMS
/// are drawn for everyone: an item is a picture the family keeps, and a panel one sticker short
/// for one member would be a quantity that moved when they blocked somebody. Nothing in this
/// class asks who is blocked, on purpose.
/// </para>
/// </remarks>
public sealed class PackModel(
    PackStore pack,
    ChatStore chats,
    ApiClient api,
    AttachmentCache attachments,
    Func<DateTimeOffset>? clock = null)
{
    private readonly Func<DateTimeOffset> now = clock ?? (() => DateTimeOffset.UtcNow);

    /// <summary>
    /// Whether this server has packs at all. One that predates them names no ceilings on
    /// <c>GET /families/mine</c>, and then there is no sticker button and no pack management —
    /// rather than a 404 when somebody taps one.
    /// </summary>
    public bool Offered => pack.Limits is not null;

    public PackLimits? Limits => pack.Limits;

    /// <summary>The pack in the order its items were added.</summary>
    public IReadOnlyList<PackItemDto> Items() => pack.Items();

    /// <summary>What the panel over the composer shows.</summary>
    public PackPanel Panel() => PackPanel.Of(pack.Items(), pack.Recents(PackPanel.RecentRoom));

    /// <summary>Whether the pack is at its ceiling — said beside the Add button, before a picture is even chosen.</summary>
    public bool IsFull => pack.Limits is { } limits && pack.Count() >= limits.MaxItems;

    /// <summary>
    /// Whoever added it, or the family's owner — a permission shape the board does not have, and
    /// the reason is the pack's other difference: a member who has left, or deleted their account,
    /// leaves their stickers behind, and under an author-only rule nobody could ever remove those.
    /// </summary>
    public static bool MayRemove(PackItemDto item, long reader, bool readerOwnsFamily) =>
        readerOwnsFamily || (reader != 0 && item.AddedBy == reader);

    public bool MayRemove(PackItemDto item, bool readerOwnsFamily) =>
        MayRemove(item, chats.Reader, readerOwnsFamily);

    /// <summary>A pack item's bytes — the ORIGINAL, always, whatever <c>has_preview</c> says.</summary>
    public Task<(byte[]? Bytes, ApiError? Error)> BytesAsync(PackItemDto item, CancellationToken ct = default) =>
        item.Attachment is { } picture
            ? attachments.BytesAsync(picture, preview: false, ct)
            : Task.FromResult<(byte[]?, ApiError?)>((null, new ApiError(ErrorCodes.PackItemNotFound, "the item has no picture")));

    // ---- adding --------------------------------------------------------------------------------

    /// <summary>
    /// Add a picture to the pack: upload the bytes AS THEY ARE, then claim them. Any member may.
    /// </summary>
    /// <remarks>
    /// <para>
    /// REFUSED HERE FIRST, with the server's own codes, where the limits are already known — so
    /// "that one is too big" arrives beside the picker rather than as a rejected request
    /// (docs/protocol.md, "Limits"). The server checks again, because the pack may have filled
    /// since this device last looked.
    /// </para>
    /// <para>
    /// <c>attachment_expired</c> at the claim means the unclaimed sweep took the upload, and the
    /// answer to that is to upload it again — ONCE, here, since the bytes are in hand — and claim
    /// again. Only a second failure is shown.
    /// </para>
    /// <para>
    /// <b>NEW OR ALREADY THERE IS THE HTTP STATUS, AND NOTHING ELSE.</b> <c>201</c> is a new item
    /// and <c>200</c> is the pack already holding it. Whether THIS DEVICE already holds the item
    /// says nothing: the server fans the actor's own <c>pack_item</c> frame out before the POST
    /// answers, so a genuinely new sticker is very often in the cache by the time its claim comes
    /// back — and was then reported as "The family already has that sticker."
    /// </para>
    /// </remarks>
    public async Task<(PackAdded? Added, ApiError? Error)> AddAsync(
        ReadOnlyMemory<byte> bytes, string? label = null, CancellationToken ct = default)
    {
        if (pack.Limits is not { } limits)
        {
            // A server with no packs. The window offers no way here; this is the backstop.
            return (null, new ApiError(ErrorCodes.PackItemNotFound, "this server has no sticker pack"));
        }
        // The window refuses an over-long label with words, beside the box it was typed in; this
        // is the backstop, and it too is before any request.
        if (PackLabel.TooLong(label))
        {
            return (null, PackLabel.Refused);
        }
        label = PackLabel.Clean(label);
        if (PackPicking.Refusal(bytes.Span, limits, pack.Count()) is { } refused)
        {
            return (null, refused);
        }
        var mime = StickerFile.Mime(bytes.Span) ?? StickerFile.Png;
        var size = StickerFile.Size(bytes.Span);
        ApiError? last = null;
        for (var attempt = 0; attempt < 2; attempt++)
        {
            // kind=photo, the sticker's own bytes, and NO preview: a preview is a JPEG, which is
            // the same destruction by another door.
            var upload = await api.Upload(
                "photo", mime, bytes, size?.Width, size?.Height, ct: ct).ConfigureAwait(false);
            if (!upload.Ok || upload.Value is null)
            {
                return (null, upload.Error ?? ApiError.Transport("no answer"));
            }
            var claim = await api.AddToPack(upload.Value.Attachment.Id, label, ct).ConfigureAwait(false);
            if (claim.Ok && claim.Value is { Item: { Attachment: { } kept } item })
            {
                // 201 is a new item, 200 is one the pack already held. Read from the STATUS: the
                // cache cannot say, because this device's own frame may have put the item there
                // before this answer arrived.
                var already = claim.Status == 200;
                // Under the per-item guard, as EVIDENCE: the answer to this device's own POST
                // moves no cursor, for the reason the board gives.
                pack.Apply(item, SeqRoute.Evidence);
                // The bytes are kept under the id the PACK holds them by — which is not the id
                // that was just uploaded when the pack already held these bytes. A disk that
                // will not take them costs a download later, and must not turn an add that
                // HAPPENED into one that is reported as failed.
                try
                {
                    attachments.Remember(kept, bytes);
                }
                catch (Exception e) when (e is IOException or UnauthorizedAccessException)
                {
                    // Not kept. The panel fetches them the first time it draws this item.
                }
                return (already ? PackAdded.AlreadyThere : PackAdded.Added, null);
            }
            last = claim.Error ?? ApiError.Transport("no answer");
            if (last.Code != ErrorCodes.AttachmentExpired)
            {
                break;
            }
        }
        return (null, last);
    }

    /// <summary>
    /// "Add to family stickers" on a sticker somebody SENT: the pack's own flow, with the message's
    /// bytes — uploaded again, unprepared, and claimed.
    /// </summary>
    public async Task<(PackAdded? Added, ApiError? Error)> AddFromMessageAsync(
        AttachmentDto picture, string? label = null, CancellationToken ct = default)
    {
        if (PackLabel.TooLong(label))
        {
            return (null, PackLabel.Refused);
        }
        var (bytes, error) = await attachments.BytesAsync(picture, preview: false, ct).ConfigureAwait(false);
        return bytes is null
            ? (null, error ?? ApiError.Transport("no answer"))
            : await AddAsync(bytes, label, ct).ConfigureAwait(false);
    }

    /// <summary>
    /// Whether the pack holds this sticker — decided HERE, from bytes this device already has,
    /// because nothing on the wire names the item a message was sent from: an item whose
    /// <c>size</c> and <c>mime</c> match, and whose bytes are the same.
    /// </summary>
    /// <remarks>
    /// A wrong "no" costs nothing. It offers "Add to family stickers" for something the pack
    /// already has, and the server answers <c>200</c> with the item that was there.
    /// </remarks>
    public async Task<bool> HoldsAsync(AttachmentDto picture, CancellationToken ct = default)
    {
        if (picture.Size is not { } size)
        {
            return false;
        }
        var mime = MediaPrep.Essence(picture.Mime ?? string.Empty);
        var candidates = pack.Items()
            .Where(item => item.Attachment is { Size: { } held } kept
                && held == size
                && MediaPrep.Essence(kept.Mime ?? string.Empty) == mime)
            .ToList();
        if (candidates.Count == 0)
        {
            return false;
        }
        var (sent, _) = await attachments.BytesAsync(picture, preview: false, ct).ConfigureAwait(false);
        if (sent is null)
        {
            return false;
        }
        foreach (var candidate in candidates)
        {
            var (kept, _) = await BytesAsync(candidate, ct).ConfigureAwait(false);
            if (kept is not null && kept.AsSpan().SequenceEqual(sent))
            {
                return true;
            }
        }
        return false;
    }

    // ---- removing ------------------------------------------------------------------------------

    /// <summary>
    /// Take an item out of the pack. Every message ever sent with it keeps its own copy and goes
    /// on drawing.
    /// </summary>
    /// <remarks>
    /// <c>pack_item_not_found</c> MEANS IT IS ALREADY GONE — removed by somebody else, or from a
    /// family this reader has left — and "gone" is what was asked for. The item is dropped here
    /// and NO ERROR is shown: a sentence about a sticker that is not there would be telling the
    /// person that what they wanted has happened.
    /// </remarks>
    public async Task<ApiError?> RemoveAsync(long itemId, CancellationToken ct = default)
    {
        var answer = await api.RemoveFromPack(itemId, ct).ConfigureAwait(false);
        if (!answer.Ok && answer.Error?.Code != ErrorCodes.PackItemNotFound)
        {
            return answer.Error ?? ApiError.Transport("no answer");
        }
        // The frame and the catch-up both carry the tombstone; dropping the item here as well is
        // what makes the sticker leave the panel as the click lands. No cursor moves: the seq this
        // removal was given is the server's to announce, and a cursor moved to a number nobody
        // issued would step the feed past whatever really holds it.
        pack.Removed(itemId);
        return null;
    }

    // ---- sending -------------------------------------------------------------------------------

    /// <summary>
    /// What a tap on a pack item sends: the item's bytes AS CACHED, as one <c>kind=photo</c>
    /// upload with no preview. The caller stages it and queues the row with <c>sticker</c> set —
    /// the outbox owns it from there, exactly as it owns any message.
    /// </summary>
    /// <remarks>
    /// The server does not check that the bytes are still in the pack, and deliberately: an item
    /// removed between the tap and the send must still go, and a send must survive an outbox that
    /// waited a day. So nothing here looks the item up again either.
    /// </remarks>
    public async Task<(StagedMedia? Media, ApiError? Error)> ToSendAsync(
        PackItemDto item, CancellationToken ct = default)
    {
        if (item.Attachment is not { } picture)
        {
            return (null, new ApiError(ErrorCodes.PackItemNotFound, "the item has no picture"));
        }
        var (bytes, error) = await BytesAsync(item, ct).ConfigureAwait(false);
        if (bytes is null)
        {
            return (null, error ?? ApiError.Transport("no answer"));
        }
        // What the bytes ARE, not what a row said of them: the upload is checked by magic number.
        var mime = StickerFile.Mime(bytes) ?? MediaPrep.Essence(picture.Mime ?? string.Empty);
        if (!StickerFile.IsStickerType(mime))
        {
            return (null, new ApiError(ErrorCodes.InvalidAttachment, "a sticker is a WebP or a PNG"));
        }
        // The per-item ceiling binds a sticker MESSAGE too, and an operator may have lowered it
        // since this item was added: refused here rather than as a failed bubble.
        if (pack.Limits is { } limits && bytes.Length > limits.MaxItemBytes)
        {
            return (null, new ApiError(ErrorCodes.PackItemTooLarge, "over the pack's per-item ceiling"));
        }
        var size = StickerFile.Size(bytes);
        return (new StagedMedia(
            "photo", mime, bytes,
            picture.Width ?? size?.Width,
            picture.Height ?? size?.Height), null);
    }

    /// <summary>This device just sent that item: it leads the panel next time.</summary>
    public void Sent(long itemId) => pack.Used(itemId, now());
}

/// <summary>
/// The few words whoever adds a sticker may give it (docs/protocol.md, "label"): optional, for a
/// screen reader, never drawn over the picture — and fixed when the item is added.
/// </summary>
/// <remarks>
/// COUNTED THE WAY THE SERVER COUNTS: Unicode scalar values, after trimming — not UTF-16 units,
/// which would call thirty-three emoji too long, and not what a screen shows, which would let a
/// family of joined emoji through as one. What is counted is exactly what is sent.
/// </remarks>
public static class PackLabel
{
    /// <summary>The most scalar values a label may be, after trimming.</summary>
    public const int MaxLength = 64;

    /// <summary>The code of a label refused HERE, before any request. Not a server code: the server's is <c>validation</c>.</summary>
    public const string TooLongCode = "pack_label_too_long";

    /// <summary>The label as it is sent: trimmed, and null when nothing is left — an empty label is no label.</summary>
    public static string? Clean(string? typed) =>
        typed?.Trim() is { Length: > 0 } label ? label : null;

    /// <summary>How long a label is, by the server's count: scalar values after trimming.</summary>
    public static int Length(string? typed) => Clean(typed)?.EnumerateRunes().Count() ?? 0;

    public static bool TooLong(string? typed) => Length(typed) > MaxLength;

    /// <summary>The refusal, for the one table of sentences (<see cref="PackText.Sentence(ApiError, IStringCatalog)"/>).</summary>
    /// <remarks>Given the status the server would give it, so it reads as the refusal it is and never as a failure worth retrying.</remarks>
    public static ApiError Refused => new(TooLongCode, "a sticker's label is at most 64 characters", 400);
}

/// <summary>
/// Where the sticker button is offered, and what stands between a click in the panel and the send.
/// </summary>
/// <remarks>
/// <para>
/// <b>IN EVERY CHAT A MESSAGE CAN BE SENT IN</b>: the family chat, a one-to-one chat, the
/// ASSISTANT's chat, and the THREAD composer. A sticker is a message, and a chat that takes
/// messages takes stickers.
/// </para>
/// <para>
/// <b>IN THE ASSISTANT'S CHAT IT IS ASKED ABOUT LIKE ANY MESSAGE THERE</b> (docs/protocol.md,
/// "Consenting to the assistant"). Everything sent in that chat reaches the model, and to the
/// model a sticker is a photo — so one click in the panel goes through the same question the Send
/// button does, never around it.
/// </para>
/// </remarks>
public static class PackSending
{
    /// <summary>What a click in the panel may do.</summary>
    public enum Gate
    {
        /// <summary>Send it.</summary>
        Send,

        /// <summary>It would reach the model and this member has not agreed: ask first, and send only on a yes.</summary>
        Ask,

        /// <summary>It would reach a model whose owner this server will not name: nothing is sent, as for any message there.</summary>
        Withheld,
    }

    /// <summary>
    /// The button by the conversation's composer: on a server that has packs, in any chat that is
    /// open, and not while a message is being edited — a sticker is its own message and an edit is
    /// somebody else's.
    /// </summary>
    public static bool Offered(string? chatKind, bool editing, bool packOffered) =>
        packOffered && !editing && chatKind is not null;

    /// <summary>The button by the THREAD's composer: wherever that composer can send, which needs a root to answer.</summary>
    public static bool OfferedInThread(bool hasRoot, bool packOffered) => packOffered && hasRoot;

    /// <summary>
    /// What stands between the click and the send. A sticker has no words, so in the family chat
    /// it cannot mention the assistant and is never asked about; in the assistant's own chat
    /// everything is.
    /// </summary>
    public static Gate For(string? chatKind, bool hasAssistant, string? processor, string? agreedAt) =>
        AssistantConsent.IsRequired(chatKind, string.Empty, processor, agreedAt) ? Gate.Ask
        : AssistantConsent.IsWithheldFromAnUnnamedAssistant(chatKind, string.Empty, hasAssistant, processor) ? Gate.Withheld
        : Gate.Send;
}

/// <summary>
/// What is decided about a picture somebody chose for the pack, BEFORE anything is uploaded.
/// </summary>
/// <remarks>
/// <para>
/// <b>ANY STILL PICTURE THIS MACHINE CAN DECODE MAY BE CHOSEN.</b> What a sticker IS on the wire
/// is a WebP or a PNG; what somebody may pick to make one from is wider than that.
/// </para>
/// <para>
/// <b>A FINISHED STICKER IS TAKEN AS GIVEN.</b> A WebP or a PNG within the byte ceiling goes up
/// untouched, whatever its pixel size and whether or not it moves: re-encoding somebody's finished
/// sticker buys nothing, and for an animated one is not possible.
/// </para>
/// <para>
/// <b>ANYTHING ELSE IS MADE INTO ONE</b> — a JPEG, a HEIC, a still GIF, or a still WebP or PNG
/// over the byte ceiling and larger than the box: fitted WHOLE into 512 × 512, its proportions and
/// its transparency kept, never scaled up, and written as PNG (Windows Imaging writes no WebP).
/// NEVER through the photo path, which draws on white and writes a JPEG. A WebP or PNG over the
/// ceiling that already fits the box is refused instead: redrawing the same pixels would not make
/// them fewer bytes.
/// </para>
/// <para>
/// <b>AN ANIMATED PICTURE THAT IS NOT ALREADY AN ACCEPTABLE WEBP IS REFUSED, IN WORDS</b> — an
/// animated GIF, or an animated PNG this client would have to redraw. Making a sticker of it would
/// keep one frame and quietly throw the rest away, and the person who picked a moving picture
/// would get a still one with no word about why. An animated WebP over the ceiling is simply too
/// big: it is the right kind of file, and nothing here can make it smaller.
/// </para>
/// </remarks>
public static class PackPicking
{
    /// <summary>The code of the "animated stickers must be WebP" refusal. Made here, never by the server.</summary>
    public const string AnimatedCode = "pack_animated_not_webp";

    /// <summary>What to do with a chosen picture.</summary>
    public enum Step
    {
        /// <summary>Upload these bytes as they are.</summary>
        Take,

        /// <summary>A still picture that is not a finished sticker: fit it into 512 × 512 as PNG, then ask again.</summary>
        Make,

        /// <summary>Animated and not WebP, too big, or the pack is full. <see cref="Plan.Refused"/> says which.</summary>
        Refuse,
    }

    /// <param name="Target">The pixel size to draw a <see cref="Step.Make"/> at; the decoder's own size, fitted, when the header did not say.</param>
    public readonly record struct Plan(Step What, ApiError? Refused = null, (int Width, int Height)? Target = null);

    /// <summary>What to do with a picture as it was chosen.</summary>
    public static Plan For(ReadOnlySpan<byte> bytes, PackLimits limits, int held)
    {
        // The most basic thing wrong first: what the picture IS, then its size, and only then
        // whether the pack is full.
        var full = held >= limits.MaxItems;
        if (StickerFile.Mime(bytes) is not { } mime)
        {
            // Not a WebP and not a PNG. A moving one cannot be made a sticker without losing the
            // movement; a still one is for the decoder to read — and to refuse, if it cannot.
            if (StickerFile.IsAnimatedGif(bytes))
            {
                return new Plan(Step.Refuse, Animated);
            }
            return full ? new Plan(Step.Refuse, Full) : new Plan(Step.Make);
        }
        if (bytes.Length <= limits.MaxItemBytes)
        {
            return full ? new Plan(Step.Refuse, Full) : new Plan(Step.Take);
        }
        if (StickerFile.IsAnimated(bytes))
        {
            // Never re-encoded. A WebP is the right kind and too big; an animated PNG would come
            // out of the redraw as its first frame.
            return new Plan(Step.Refuse, mime == StickerFile.WebP ? TooLarge : Animated);
        }
        var size = StickerFile.Size(bytes);
        if (size is { } known && known.Width <= StickerFile.Edge && known.Height <= StickerFile.Edge)
        {
            return new Plan(Step.Refuse, TooLarge);
        }
        return full
            ? new Plan(Step.Refuse, Full)
            : new Plan(Step.Make, Target: size is { } larger ? Fit(larger.Width, larger.Height) : null);
    }

    /// <summary>
    /// The refusal for bytes that are about to be uploaded AS THEY ARE, or null when they may go:
    /// what <see cref="PackModel.AddAsync"/> asks, and what a picture made into a sticker is asked again.
    /// </summary>
    public static ApiError? Refusal(ReadOnlySpan<byte> bytes, PackLimits limits, int held) =>
        StickerFile.Mime(bytes) is null ? NotASticker
        : bytes.Length > limits.MaxItemBytes ? TooLarge
        : held >= limits.MaxItems ? Full
        : null;

    /// <summary>A picture fitted whole into the 512 × 512 box: proportions kept, never scaled up, never below one pixel.</summary>
    public static (int Width, int Height) Fit(int width, int height)
    {
        var (w, h) = MediaPrep.FitWithin(
            (uint)Math.Max(width, 1), (uint)Math.Max(height, 1), StickerFile.Edge);
        return ((int)w, (int)h);
    }

    // The server's own codes, so one table of sentences answers a refusal wherever it was made.
    private static ApiError NotASticker => new(ErrorCodes.InvalidAttachment, "a sticker is a WebP or a PNG");

    private static ApiError TooLarge => new(ErrorCodes.PackItemTooLarge, "over the pack's per-item ceiling");

    private static ApiError Full => new(ErrorCodes.PackFull, "the pack is at its ceiling");

    /// <summary>The refusal of a moving picture that is not a WebP — also what the decoder answers when IT finds more than one frame.</summary>
    /// <remarks>Given a refusal's status: a code the server never wrote would otherwise read as a failure worth retrying.</remarks>
    public static ApiError Animated => new(AnimatedCode, "an animated sticker must be a WebP", 400);
}

/// <summary>What the window says about the pack.</summary>
public static class PackText
{
    /// <summary>
    /// A refusal in the reader's words. The ceilings are SAID, like <c>board_full</c>: the limit
    /// is the family's, and a sticker that silently did not appear is one its adder will go
    /// looking for.
    /// </summary>
    public static string Sentence(ApiError error, IStringCatalog say) => error.Code switch
    {
        ErrorCodes.PackItemTooLarge or ErrorCodes.AttachmentTooLarge =>
            say.Get("That picture is too big to be a sticker."),
        ErrorCodes.InvalidAttachment => say.Get("A sticker has to be a WebP or PNG picture."),
        // Said, never silently flattened to one frame.
        PackPicking.AnimatedCode => say.Get("Animated stickers must be WebP."),
        PackLabel.TooLongCode => LabelTooLong(say),
        ErrorCodes.PackFull => say.Get("The family's stickers are full. Remove one to make room."),
        ErrorCodes.NotPackItemAuthor =>
            say.Get("Only whoever added a sticker, or the family owner, can remove it."),
        _ when error.Transient => say.Get("The sticker didn't reach the server. Check your connection and try again."),
        _ => say.Get("Something went wrong. Try again."),
    };

    /// <summary>The over-long label, in words — said beside the box it was typed in, before any request.</summary>
    public static string LabelTooLong(IStringCatalog say) =>
        say.Format("A sticker's label can be at most %lld characters.", PackLabel.MaxLength);

    /// <summary>What an add that was NOT refused says.</summary>
    public static string Sentence(PackAdded added, IStringCatalog say) => added switch
    {
        PackAdded.AlreadyThere => say.Get("The family already has that sticker."),
        _ => say.Get("Added to family stickers"),
    };

    /// <summary>
    /// What a sticker is called to a screen reader: "Sticker", and the few words whoever added it
    /// gave when they gave any. Never drawn over the picture.
    /// </summary>
    public static string Name(PackItemDto item, IStringCatalog say) =>
        item.Label is { Length: > 0 } label ? say.Format("Sticker: %@", label) : say.Get("Sticker");

    /// <summary>"12 of 200 stickers" — how full the pack is, against the ceiling the server named.</summary>
    public static string Fullness(int held, PackLimits limits, IStringCatalog say) =>
        say.Format("%lld of %lld stickers", held, limits.MaxItems);
}
