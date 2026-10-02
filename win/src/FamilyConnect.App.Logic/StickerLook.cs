namespace FamilyConnect.App.Logic;

/// <summary>
/// How a chat sticker is DRAWN — a drawing rule and not a wire one, written down in the protocol
/// because five clients must agree (docs/protocol.md, "How it is drawn").
/// </summary>
/// <remarks>
/// <para>
/// <b>NO BUBBLE, AND ONE FIXED BOX.</b> The picture alone, its transparency showing the chat
/// behind it, in a box that is the same for every sticker on this client — LARGER than an emoji
/// and smaller than a photograph — fitted whole and never cropped. Never at the picture's own
/// pixel size: that would make a 96-pixel sticker a speck and a 2000-pixel one a poster.
/// </para>
/// <para>
/// <b>THE SHAPE COMES FROM METADATA</b>, as a photo tile's does, so a row does not change height
/// when its picture lands; a sticker whose uploader could not say is given the whole box.
/// </para>
/// <para>
/// This is the CHAT sticker. The board's cards are <see cref="Sticker"/>, and nothing here is about them.
/// </para>
/// </remarks>
public static class StickerLook
{
    /// <summary>
    /// The box in a conversation, in effective pixels. A lone photo tile is up to 320 and the
    /// largest emoji-only message about a fifth of that: this sits between the two, where a
    /// messenger's stickers sit.
    /// </summary>
    public const double Box = 160;

    /// <summary>One cell of the panel over the composer.</summary>
    public const double PanelCell = 72;

    /// <summary>How many cells across the panel is.</summary>
    public const int PanelColumns = 5;

    /// <summary>One cell of the pack on the Family screen.</summary>
    public const double ManageCell = 88;

    /// <summary>
    /// The size a sticker is drawn at inside <paramref name="box"/>: the whole picture, its
    /// proportions kept, scaled UP as readily as down — the box is the size, not a ceiling.
    /// </summary>
    public static (double Width, double Height) Fit(int? width, int? height, double box = Box)
    {
        if (width is not > 0 || height is not > 0)
        {
            return (box, box);
        }
        var scale = Math.Min(box / width.Value, box / height.Value);
        // Never thinner than a pixel: a 4000 × 3 strip is still something to click.
        return (Math.Max(1, Math.Round(width.Value * scale)), Math.Max(1, Math.Round(height.Value * scale)));
    }
}

/// <summary>
/// The clock of an animated sticker: which frame is up, and whether animating it is worth the
/// memory at all.
/// </summary>
/// <remarks>
/// <para>
/// <b>ANIMATED WHERE THE PLATFORM CAN, FRAME ZERO WHERE IT CANNOT — AND BOTH ARE CORRECT.</b> The
/// window asks Windows Imaging for the frames; whether it gives any past the first is a fact
/// about the machine (the WebP codec is a Store extension), and a sticker drawn still is the same
/// sticker. What is decided HERE is only arithmetic, so it can be tested where no codec runs.
/// </para>
/// <para>
/// <b>A DURATION OF ZERO IS NOT ZERO.</b> Files in the wild say 0 or 10 ms for "as fast as you
/// can", and every browser draws those at 100 ms rather than spinning a core; so does this.
/// </para>
/// </remarks>
public static class StickerAnimation
{
    /// <summary>The most frames one sticker is animated with; past it, frame zero.</summary>
    public const int MaxFrames = 240;

    /// <summary>The most decoded pixels one animated sticker may hold, as BGRA bytes; past it, frame zero.</summary>
    public const long MaxDecodedBytes = 48L * 1024 * 1024;

    /// <summary>
    /// The most decoded bytes EVERY animated sticker a view keeps may hold between them — two of
    /// the largest one allowed, so the newest always fits whatever else is on screen. What is over
    /// it is drawn still (<see cref="StickerShelf{TPicture}"/>).
    /// </summary>
    public const long MaxHeldBytes = 2 * MaxDecodedBytes;

    /// <summary>How many decoded stickers a view keeps, still ones and unreadable ones included.</summary>
    public const int MaxKept = 64;

    /// <summary>What <paramref name="frames"/> frames of that size hold once decoded, as BGRA bytes.</summary>
    public static long DecodedBytes(int frames, int pixelWidth, int pixelHeight) =>
        (long)Math.Max(frames, 0) * Math.Max(pixelWidth, 0) * Math.Max(pixelHeight, 0) * 4;

    /// <summary>What a too-short frame stands for, in milliseconds — the browsers' rule.</summary>
    public const int FloorMs = 100;

    /// <summary>
    /// Whether to animate: more than one frame, and frames that fit the budget at the size they
    /// will be decoded. Anything else is drawn as its first frame.
    /// </summary>
    public static bool Worth(int frames, int pixelWidth, int pixelHeight) =>
        frames > 1
        && frames <= MaxFrames
        && pixelWidth > 0
        && pixelHeight > 0
        && DecodedBytes(frames, pixelWidth, pixelHeight) <= MaxDecodedBytes;

    /// <summary>
    /// The pixel size frames are decoded at: fitted into the box at the screen's own scale, and
    /// never larger than the picture is — a 512-pixel sticker in a 160-point box at 150% is
    /// decoded at 240, not 512, which is a fifth of the memory for the same picture.
    /// </summary>
    public static (int Width, int Height) DecodeSize(int width, int height, double box, double rasterScale)
    {
        if (width <= 0 || height <= 0)
        {
            return (1, 1);
        }
        var room = Math.Max(1, box * Math.Max(rasterScale, 1));
        var scale = Math.Min(1, Math.Min(room / width, room / height));
        return (
            Math.Max(1, (int)Math.Round(width * scale)),
            Math.Max(1, (int)Math.Round(height * scale)));
    }

    /// <summary>
    /// How long each of <paramref name="frames"/> frames stands. The file's own durations when it
    /// gave one per frame; the floor for every frame when it did not, because a count that
    /// disagrees with the decoder's is a file this clock cannot line up.
    /// </summary>
    public static IReadOnlyList<int> Clock(IReadOnlyList<int> fileDurations, int frames)
    {
        var clock = new int[Math.Max(frames, 0)];
        var trusted = fileDurations.Count == frames;
        for (var at = 0; at < clock.Length; at++)
        {
            clock[at] = trusted && fileDurations[at] > 10 ? fileDurations[at] : FloorMs;
        }
        return clock;
    }

    /// <summary>
    /// Which frame is up after <paramref name="elapsedMs"/>, looping for as long as it is looked
    /// at — a sticker is not a film with an end.
    /// </summary>
    public static int FrameAt(IReadOnlyList<int> clock, long elapsedMs)
    {
        if (clock.Count == 0)
        {
            return 0;
        }
        long total = 0;
        foreach (var duration in clock)
        {
            total += Math.Max(duration, 1);
        }
        var within = Math.Max(elapsedMs, 0) % total;
        for (var at = 0; at < clock.Count; at++)
        {
            within -= Math.Max(clock[at], 1);
            if (within < 0)
            {
                return at;
            }
        }
        return clock.Count - 1;
    }
}

/// <summary>
/// The decoded stickers one view keeps, so that a conversation redrawn on every change does not
/// decode them again — and the ceiling on what keeping them may cost.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE BUDGET IS FOR THE SHELF, NOT PER STICKER.</b> <see cref="StickerAnimation.MaxDecodedBytes"/>
/// says what ONE animated sticker may hold; a family that sends them the way messengers' users do
/// puts forty in a chat, and forty times that is gigabytes. So every animated picture is put here
/// with what its frames cost, and when the sum passes the budget the LEAST RECENTLY DRAWN give it
/// back: one no image is showing is let go whole, and one that is on screen is made a STILL —
/// frame zero, which is a correct drawing of a sticker. The newest is the one that keeps moving,
/// because the newest is the one at the bottom of the chat, where the reader is.
/// </para>
/// <para>
/// <b>A COST IS WHAT CAN BE GIVEN BACK.</b> A still picture and a picture made still are put at
/// zero: they are one frame each, and they leave by the count alone.
/// </para>
/// <para>
/// <b>"NOTHING HERE DECODES IT" IS AN ANSWER, AND IT IS KEPT.</b> A sticker no codec on this
/// machine reads (WebP without its extension) is remembered as that, so a rebuild does not read
/// the file and fail the decode again for every sticker in the chat, on every message that
/// arrives.
/// </para>
/// <para>
/// <b>AND IT KEEPS WHEN EACH ONE STARTED</b>, so the elements of a rebuilt conversation join the
/// animation where it was rather than sending every sticker on screen back to frame zero whenever
/// anybody in the chat sends or reacts.
/// </para>
/// <para>
/// No window and no picture type in here: what a picture is, whether one is on screen and how one
/// is made still are the view's to say, so the rule can be tested where no window exists. Not
/// thread-safe — it is the window thread's.
/// </para>
/// </remarks>
/// <param name="budget">The most bytes the pictures here may hold between them.</param>
/// <param name="most">The most pictures kept; past it the least recently drawn that no image shows are let go.</param>
/// <param name="showing">Whether an image in the window is drawing that picture right now.</param>
/// <param name="still">Make that picture a still one — frame zero kept, the rest given back.</param>
/// <param name="letGo">That picture is leaving the shelf and nothing shows it: give back what it holds.</param>
public sealed class StickerShelf<TPicture>(
    long budget,
    int most,
    Func<TPicture, bool>? showing = null,
    Action<TPicture>? still = null,
    Action<TPicture>? letGo = null)
    where TPicture : class
{
    private sealed class Kept(TPicture? picture, long cost, long drawn)
    {
        public TPicture? Picture { get; } = picture;

        public long Cost { get; set; } = cost;

        public long Drawn { get; set; } = drawn;

        public long? Started { get; set; }
    }

    private readonly Dictionary<long, Kept> kept = [];
    private long turn;

    /// <summary>What the pictures here hold between them, in bytes that could be given back.</summary>
    public long Held { get; private set; }

    public int Count => kept.Count;

    /// <summary>
    /// Whether this sticker has been decoded here — <paramref name="picture"/> is null when the
    /// answer was that nothing on this machine reads it. Asking is drawing: it counts as use.
    /// </summary>
    public bool TryGet(long id, out TPicture? picture)
    {
        if (kept.TryGetValue(id, out var entry))
        {
            entry.Drawn = ++turn;
            picture = entry.Picture;
            return true;
        }
        picture = null;
        return false;
    }

    /// <summary>
    /// Keep what a decode answered: the picture and what its frames cost, or null for "nothing
    /// here reads it". Older pictures make room for it, never the other way round.
    /// </summary>
    public void Put(long id, TPicture? picture, long cost)
    {
        if (kept.Remove(id, out var replaced))
        {
            Held -= replaced.Cost;
            if (replaced.Picture is { } old && !ReferenceEquals(old, picture) && !Showing(old))
            {
                letGo?.Invoke(old);
            }
        }
        var entry = new Kept(picture, picture is null ? 0 : Math.Max(cost, 0), ++turn)
        {
            // A sticker decoded again is the same sticker: its clock carries on.
            Started = replaced?.Started,
        };
        kept[id] = entry;
        Held += entry.Cost;
        // Least recently drawn first, and never the one just put: the newest is the one being looked at.
        foreach (var (other, held) in kept.Where(pair => !ReferenceEquals(pair.Value, entry))
                     .OrderBy(pair => pair.Value.Drawn).ToList())
        {
            var overCount = kept.Count > most;
            if (Held <= budget && !overCount)
            {
                break;
            }
            // Only a picture with something to give back answers for the bytes.
            var overBytes = Held > budget && held.Cost > 0;
            if (held.Picture is { } shown && Showing(shown))
            {
                // On screen: it cannot leave, but it can stop moving — and only the bytes ask that of it.
                if (overBytes)
                {
                    still?.Invoke(shown);
                    Held -= held.Cost;
                    held.Cost = 0;
                }
                continue;
            }
            if (!overBytes && !overCount)
            {
                continue;
            }
            kept.Remove(other);
            Held -= held.Cost;
            if (held.Picture is { } gone)
            {
                letGo?.Invoke(gone);
            }
        }
    }

    /// <summary>
    /// When this sticker's animation began, on the caller's clock: <paramref name="now"/> the
    /// first time it is asked, and that same moment for every image that draws it afterwards.
    /// </summary>
    public long Started(long id, long now) =>
        kept.TryGetValue(id, out var entry) ? entry.Started ??= now : now;

    /// <summary>The view is going: everything is let go, shown or not.</summary>
    public void Clear()
    {
        var all = kept.Values.ToList();
        kept.Clear();
        Held = 0;
        foreach (var entry in all)
        {
            if (entry.Picture is { } picture)
            {
                letGo?.Invoke(picture);
            }
        }
    }

    private bool Showing(TPicture picture) => showing?.Invoke(picture) ?? false;
}
